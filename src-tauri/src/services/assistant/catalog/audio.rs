//! Documented audio contracts; catalog prices remain live and apex-specific.
use super::{million, unit_price, Kind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioFamily {
    GeminiTts,
    ElevenlabsTts,
    GptTranscription,
    ElevenlabsTranscription,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioBillingUnit {
    Tokens,
    Seconds,
    Characters,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VoiceKind {
    Name,
    Id,
}

/// Decimal USD rates. A missing component is unknown, never a free request.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioPricing {
    pub unit: AudioBillingUnit,
    pub input_audio_per_second: Option<String>,
    pub input_text_per_character: Option<String>,
    pub input_text_per_million: Option<String>,
    pub input_audio_per_million: Option<String>,
    pub output_text_per_million: Option<String>,
    pub output_audio_per_million: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCapabilities {
    pub family: AudioFamily,
    pub response_formats: Vec<String>,
    pub default_format: String,
    pub voice_kind: Option<VoiceKind>,
    /// Documented examples, not the provider's exhaustive voice inventory.
    pub suggested_voices: Vec<String>,
    pub instructions: bool,
    pub prompt: bool,
    pub language: bool,
    pub word_timestamps: bool,
    pub pricing: AudioPricing,
}

pub(super) fn capabilities(id: &str, kind: Kind, pricing: &Value) -> Option<AudioCapabilities> {
    use AudioFamily::*;
    // Exact IDs and endpoint kind: neither a naming resemblance nor a generic
    // supported_parameters list establishes a new provider's wire contract.
    let family = match (kind, id) {
        (
            Kind::Speech,
            "google/gemini-2.5-flash-lite-preview-tts"
            | "google/gemini-2.5-flash-tts"
            | "google/gemini-2.5-pro-tts"
            | "google/gemini-3.8-flash-lite-tts"
            | "google/gemini-3.8-flash-tts",
        ) => GeminiTts,
        (Kind::Speech, "elevenlabs/eleven_v4") => ElevenlabsTts,
        (
            Kind::Transcription,
            "openai/gpt-transcribe"
            | "openai/gpt-4o-mini-transcribe"
            | "openai/gpt-4o-transcribe-diarize",
        ) => GptTranscription,
        (Kind::Transcription, "elevenlabs/scribe_v2") => ElevenlabsTranscription,
        _ => return None,
    };
    let unit = match family {
        ElevenlabsTts => AudioBillingUnit::Characters,
        ElevenlabsTranscription => AudioBillingUnit::Seconds,
        GptTranscription if id == "openai/gpt-transcribe" => AudioBillingUnit::Seconds,
        _ => AudioBillingUnit::Tokens,
    };
    let tokens = unit == AudioBillingUnit::Tokens;
    let prices = AudioPricing {
        input_audio_per_second: (unit == AudioBillingUnit::Seconds)
            .then(|| unit_price(&pricing["input_audio_per_second"]))
            .flatten(),
        input_text_per_character: (unit == AudioBillingUnit::Characters)
            .then(|| unit_price(&pricing["input_text_per_character"]))
            .flatten(),
        input_text_per_million: tokens.then(|| million(&pricing["prompt"])).flatten(),
        input_audio_per_million: tokens.then(|| million(&pricing["audio"])).flatten(),
        output_text_per_million: tokens.then(|| million(&pricing["completion"])).flatten(),
        output_audio_per_million: tokens.then(|| million(&pricing["output_audio"])).flatten(),
        unit,
    };
    let (formats, voice_kind, voices) = match family {
        GeminiTts => (
            vec!["wav", "pcm"],
            Some(VoiceKind::Name),
            vec!["Kore", "Puck", "Zephyr"],
        ),
        ElevenlabsTts => (
            vec!["mp3", "opus", "wav", "pcm"],
            Some(VoiceKind::Id),
            vec!["21m00Tcm4TlvDq8ikWAM"],
        ),
        GptTranscription => (vec!["json"], None, vec![]),
        ElevenlabsTranscription => (vec!["json", "verbose_json"], None, vec![]),
    };
    Some(AudioCapabilities {
        default_format: formats[0].into(),
        response_formats: formats.into_iter().map(str::to_owned).collect(),
        voice_kind,
        suggested_voices: voices.into_iter().map(str::to_owned).collect(),
        instructions: family == GeminiTts,
        prompt: family == GptTranscription && id != "openai/gpt-4o-transcribe-diarize",
        language: true,
        word_timestamps: family == ElevenlabsTranscription,
        family,
        pricing: prices,
    })
}
