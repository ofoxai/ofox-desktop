//! Explicit speech synthesis contracts; no chat-model mutation or auto replay.
pub(super) mod http;
mod ogg;
use super::{audio, catalog::Model, types::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_OUTPUT_BYTES: usize = 50 * 1024 * 1024;
pub const MAX_INPUT_CHARS: usize = 4_000;
pub const MAX_STYLE_CHARS: usize = 1_000;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeechRequest {
    pub session_id: String,
    pub expected_owner: String,
    pub expected_generation: u64,
    pub run_id: String,
    pub operation_id: String,
    pub model_id: String,
    pub input: String,
    pub voice: String,
    pub response_format: String,
    pub language: Option<String>,
    pub instructions: Option<String>,
    pub source_message_id: Option<String>,
}
impl SpeechRequest {
    pub(super) fn validate(&self) -> Result<(), String> {
        for id in [&self.session_id, &self.run_id, &self.operation_id] {
            validate_id(id)?;
        }
        if let Some(id) = &self.source_message_id {
            validate_id(id)?;
        }
        if self.input.trim().is_empty()
            || self.input.chars().count() > MAX_INPUT_CHARS
            || self
                .instructions
                .as_ref()
                .is_some_and(|s| s.chars().count() > MAX_STYLE_CHARS)
            || self
                .language
                .as_deref()
                .is_some_and(|s| !["zh", "en", "ja"].contains(&s))
        {
            return Err(audio::INVALID.into());
        }
        Ok(())
    }
    pub(super) fn body(&self, model: &Model) -> Result<Value, String> {
        use super::catalog::{AudioFamily, Kind};
        self.validate()?;
        let caps = model
            .audio
            .as_ref()
            .filter(|_| model.kind == Kind::Speech && model.id == self.model_id)
            .ok_or(audio::INVALID)?;
        if !caps.response_formats.contains(&self.response_format) || self.response_format == "pcm" {
            return Err(audio::INVALID.into());
        }
        let voice = self.voice.as_bytes();
        let valid_voice = match caps.family {
            AudioFamily::GeminiTts => {
                (1..=64).contains(&voice.len()) && voice.iter().all(u8::is_ascii_alphabetic)
            }
            AudioFamily::ElevenlabsTts => {
                voice.len() == 20 && voice.iter().all(u8::is_ascii_alphanumeric)
            }
            _ => false,
        };
        if !valid_voice
            || (!caps.instructions && self.instructions.as_ref().is_some_and(|s| !s.is_empty()))
        {
            return Err(audio::INVALID.into());
        }
        let mut body = serde_json::json!({"model": self.model_id, "input": self.input, "voice": self.voice, "response_format": self.response_format});
        if let Some(language) = &self.language {
            let language = if caps.family == AudioFamily::GeminiTts {
                match language.as_str() {
                    "zh" => "cmn-CN",
                    "en" => "en-US",
                    _ => "ja-JP",
                }
            } else {
                language
            };
            body["language_code"] = serde_json::json!(language);
        }
        if let Some(style) = self.instructions.as_ref().filter(|s| !s.trim().is_empty()) {
            body["instructions"] = serde_json::json!(style);
        }
        Ok(body)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputInfo {
    pub mime_type: String,
    pub size_bytes: u64,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
}
pub(super) fn inspect(format: &str, bytes: &[u8]) -> Result<OutputInfo, String> {
    if bytes.is_empty() || bytes.len() > MAX_OUTPUT_BYTES {
        return Err(audio::UNKNOWN.into());
    }
    let info = if format == "opus" {
        ogg::inspect(bytes)
    } else {
        super::audio_input::inspect_generated(format, bytes)
    }
    .map_err(|_| audio::UNKNOWN)?;
    Ok(OutputInfo {
        mime_type: info.mime_type,
        size_bytes: bytes.len() as u64,
        duration_ms: info.duration_ms,
        sample_rate: info.sample_rate,
        channels: info.channels,
    })
}
pub(super) fn estimate(model: &Model, input: &str) -> Option<String> {
    use rust_decimal::Decimal;
    let price = &model.audio.as_ref()?.pricing;
    if price.unit != super::catalog::AudioBillingUnit::Characters {
        return None;
    }
    let rate = price
        .input_text_per_character
        .as_deref()?
        .parse::<Decimal>()
        .ok()
        .filter(|n| *n >= Decimal::ZERO)?;
    rate.checked_mul(Decimal::from(input.chars().count() as u64))
        .map(|n| n.normalize().to_string())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use serde_json::json;
    pub(super) fn model(id: &str) -> Model {
        super::super::catalog::parse_models(json!({"data":[{"id":id,"supported_endpoints":["audio/speech"],"pricing":{"input_text_per_character":"0.0002","prompt":"0.000001","output_audio":"0.00002"}}]}),super::super::catalog::Source::V1).remove(0)
    }
    pub(super) fn request(model: &str, format: &str, voice: &str) -> SpeechRequest {
        SpeechRequest {
            session_id: id(),
            expected_owner: "a".repeat(64),
            expected_generation: 1,
            run_id: id(),
            operation_id: id(),
            model_id: model.into(),
            input: "今天下午三点开会。".into(),
            voice: voice.into(),
            response_format: format.into(),
            language: Some("zh".into()),
            instructions: None,
            source_message_id: None,
        }
    }
    #[test]
    fn speech_requests_follow_exact_family_contracts_and_map_languages() {
        let mut r = request("google/gemini-3.8-flash-tts", "wav", "Kore");
        r.instructions = Some("语气平静".into());
        assert!(r.validate().is_ok());
        let body = r.body(&model(&r.model_id)).unwrap();
        assert_eq!(body["language_code"], "cmn-CN");
        assert_eq!(body["instructions"], "语气平静");
        assert!(body.get("stream_format").is_none());
        assert!(body.get("provider_options").is_none());
        r = request("elevenlabs/eleven_v4", "mp3", "21m00Tcm4TlvDq8ikWAM");
        let m = model(&r.model_id);
        assert_eq!(r.body(&m).unwrap()["language_code"], "zh");
        assert_eq!(estimate(&m, &r.input).as_deref(), Some("0.0018"));
        r.language = None;
        assert!(r.body(&m).unwrap().get("language_code").is_none());
        r.instructions = Some("unsupported".into());
        assert!(r.body(&m).is_err());
    }
    #[test]
    fn speech_refuses_empty_oversized_unknown_and_unsupported_parameters() {
        let base = request("google/gemini-3.8-flash-tts", "wav", "Kore");
        for r in [
            SpeechRequest {
                input: " ".into(),
                ..base.clone()
            },
            SpeechRequest {
                input: "中".repeat(4001),
                ..base.clone()
            },
            SpeechRequest {
                instructions: Some("a".repeat(1001)),
                ..base.clone()
            },
            SpeechRequest {
                language: Some("invented".into()),
                ..base.clone()
            },
        ] {
            assert!(r.validate().is_err());
        }
        for r in [
            SpeechRequest {
                response_format: "pcm".into(),
                ..base.clone()
            },
            SpeechRequest {
                response_format: "mp3".into(),
                ..base.clone()
            },
            SpeechRequest {
                voice: "../Kore".into(),
                ..base.clone()
            },
        ] {
            assert!(r.body(&model(&base.model_id)).is_err());
        }
        assert!(estimate(&model(&base.model_id), &base.input).is_none());
    }
    pub(in crate::services::assistant) fn wav(seconds: u32) -> Vec<u8> {
        let size = 8000 * 2 * seconds;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + size).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&8000u32.to_le_bytes());
        b.extend_from_slice(&16000u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&size.to_le_bytes());
        b.resize(44 + size as usize, 0);
        b
    }
    #[test]
    fn speech_opus_checks_the_complete_ogg_opus_container() {
        let bytes = include_bytes!("../../../tests/fixtures/assistant-audio/tone-48k-mono.opus");
        let info = inspect("opus", bytes).unwrap();
        assert_eq!(info.mime_type, "audio/ogg");
        assert_eq!(info.sample_rate, 48000);
        assert_eq!(info.channels, 1);
        assert!(info.duration_ms > 0);
        let mut corrupt = bytes.to_vec();
        corrupt[22] ^= 1;
        assert!(inspect("opus", &corrupt).is_err());
        assert!(inspect("opus", &bytes[..bytes.len() - 1]).is_err());
    }
    #[test]
    fn speech_output_is_validated_from_bytes_without_the_short_input_duration_limit() {
        let info = inspect("wav", &wav(301)).unwrap();
        assert_eq!(info.duration_ms, 301000);
        assert_eq!(info.size_bytes, 4816044);
        let mp3 = include_bytes!("../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3");
        assert_eq!(inspect("mp3", mp3).unwrap().mime_type, "audio/mpeg");
        for (format, bytes) in [
            ("wav", br#"{"error":"not audio"}"#.as_slice()),
            ("mp3", wav(1).as_slice()),
            ("opus", b"OggSfake".as_slice()),
        ] {
            assert!(inspect(format, bytes).is_err());
        }
    }
}
