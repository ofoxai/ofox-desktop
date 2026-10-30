//! Short audio input is validated natively before a gateway can see it.
mod mp3;
mod slot;
pub(super) use slot::Slot;
mod host;
use super::attachments;
pub use host::{DiscardRequest, Preview};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MAX_BYTES: usize = 25 * 1024 * 1024;
pub const MAX_SECONDS: u64 = 300;
pub const UNSUPPORTED: &str = "assistant_audio_unsupported";
pub const INVALID: &str = "assistant_audio_invalid";
pub const TOO_LONG: &str = "assistant_audio_too_long";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub mime_type: String,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
}

pub(super) struct Input {
    pub name: String,
    pub info: Info,
    pub bytes: Vec<u8>,
}
impl std::fmt::Debug for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Input")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}
impl Input {
    pub(super) fn from_path(path: &Path) -> Result<Self, String> {
        let bytes = attachments::read_source(path)?;
        Self::from_bytes(
            path.file_name().and_then(|v| v.to_str()).unwrap_or("audio"),
            bytes,
        )
    }
    pub(super) fn from_bytes(name: &str, bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(attachments::TOO_LARGE.into());
        }
        let info = inspect(&bytes)?;
        let clean = attachments::clean_name(name);
        let stem = clean
            .rsplit_once('.')
            .map_or(clean.as_str(), |(stem, _)| stem);
        let stem: String = stem.chars().take(100).collect();
        let stem = if stem.is_empty() { "audio" } else { &stem };
        let extension = if info.mime_type == "audio/wav" {
            "wav"
        } else {
            "mp3"
        };
        Ok(Self {
            name: format!("{stem}.{extension}"),
            info,
            bytes,
        })
    }
}
/// Generated speech has its own byte/duration budget; imports keep the short limit.
pub(super) fn inspect_generated(format: &str, bytes: &[u8]) -> Result<Info, String> {
    match format {
        "wav" if bytes.starts_with(b"RIFF") => wav(bytes, 3600),
        "mp3" => mp3::inspect(bytes, 3600),
        _ => Err(UNSUPPORTED.into()),
    }
}
fn inspect(bytes: &[u8]) -> Result<Info, String> {
    if bytes.starts_with(b"RIFF") {
        wav(bytes, MAX_SECONDS)
    } else if bytes.starts_with(b"ID3")
        || bytes.starts_with(&[0xff, 0xfb])
        || bytes
            .get(..2)
            .is_some_and(|b| b[0] == 0xff && b[1] & 0xe0 == 0xe0)
    {
        mp3::inspect(bytes, MAX_SECONDS)
    } else {
        Err(UNSUPPORTED.into())
    }
}

fn info_with_limit(
    mime: &str,
    frames: u64,
    rate: u32,
    channels: u16,
    max_seconds: u64,
) -> Result<Info, String> {
    if frames == 0 || !(8000..=48000).contains(&rate) || !(1..=2).contains(&channels) {
        return Err(INVALID.into());
    }
    if frames > rate as u64 * max_seconds {
        return Err(TOO_LONG.into());
    }
    Ok(Info {
        mime_type: mime.into(),
        duration_ms: (frames * 1000).div_ceil(rate as u64),
        sample_rate: rate,
        channels,
    })
}

/// PCM RIFF/WAVE only. Check the full container rather than trusting a
/// duration field, byte rate, file extension or decoder's permissive EOF.
fn wav(bytes: &[u8], max_seconds: u64) -> Result<Info, String> {
    let word = |at: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            bytes
                .get(at..at + 4)
                .ok_or(INVALID)?
                .try_into()
                .map_err(|_| INVALID)?,
        ))
    };
    let half = |at: usize| -> Result<u16, String> {
        Ok(u16::from_le_bytes(
            bytes
                .get(at..at + 2)
                .ok_or(INVALID)?
                .try_into()
                .map_err(|_| INVALID)?,
        ))
    };
    if bytes.len() < 44
        || bytes.get(8..12) != Some(b"WAVE")
        || word(4)? as u64 + 8 != bytes.len() as u64
    {
        return Err(INVALID.into());
    }
    let (mut offset, mut chunks, mut format, mut data) = (12usize, 0, None, None);
    while offset < bytes.len() {
        chunks += 1;
        if chunks > 4096 || bytes.len() - offset < 8 {
            return Err(INVALID.into());
        }
        let size = word(offset + 4)? as usize;
        let start = offset + 8;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or(INVALID)?;
        match &bytes[offset..offset + 4] {
            b"fmt " => {
                if format.is_some()
                    || !(size == 16 || (size == 18 && half(start + 16)? == 0))
                    || half(start)? != 1
                {
                    return Err(INVALID.into());
                }
                let (channels, rate, block, bits) = (
                    half(start + 2)?,
                    word(start + 4)?,
                    half(start + 12)?,
                    half(start + 14)?,
                );
                if !(1..=2).contains(&channels)
                    || !(8000..=48000).contains(&rate)
                    || ![8, 16, 24, 32].contains(&bits)
                    || block != channels * (bits / 8)
                    || word(start + 8)? != rate * block as u32
                {
                    return Err(INVALID.into());
                }
                format = Some((rate, channels, block));
            }
            b"data" if data.is_some() => return Err(INVALID.into()),
            b"data" => data = Some(size),
            _ => {}
        }
        offset = end
            .checked_add(size % 2)
            .filter(|end| *end <= bytes.len())
            .ok_or(INVALID)?;
    }
    let (rate, channels, block) = format.ok_or(INVALID)?;
    let data = data.ok_or(INVALID)?;
    if data % block as usize != 0 {
        return Err(INVALID.into());
    }
    info_with_limit(
        "audio/wav",
        (data / block as usize) as u64,
        rate,
        channels,
        max_seconds,
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, channels: u16, frames: u32) -> Vec<u8> {
        let size = frames * channels as u32 * 2;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + size).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
        out.extend_from_slice(&(channels * 2).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&size.to_le_bytes());
        out.resize(44 + size as usize, 0);
        out
    }

    #[test]
    fn a_pcm_wav_uses_actual_frames_and_content_instead_of_its_name() {
        let input = Input::from_bytes("../../recording.txt", wav(16000, 1, 4000)).unwrap();
        assert_eq!(input.name, "recording.wav");
        assert_eq!(input.info.mime_type, "audio/wav");
        assert_eq!(input.info.duration_ms, 250);
        assert_eq!(input.info.sample_rate, 16000);
        assert_eq!(input.info.channels, 1);
        let encoded = serde_json::to_value(&input.info).unwrap();
        assert_eq!(encoded["durationMs"], 250);
        assert!(encoded.get("bytes").is_none());
        assert_eq!(input.bytes.len(), 8044);
    }

    #[test]
    fn size_and_actual_duration_are_bounded_before_any_model_call() {
        assert_eq!(
            Input::from_bytes("a.wav", vec![0; MAX_BYTES + 1]).unwrap_err(),
            super::super::attachments::TOO_LARGE
        );
        assert!(Input::from_bytes("a.wav", wav(8000, 1, 8000 * 300)).is_ok());
        assert_eq!(
            Input::from_bytes("a.wav", wav(8000, 1, 8000 * 300 + 1)).unwrap_err(),
            TOO_LONG
        );
    }

    #[test]
    fn damaged_containers_and_unsupported_audio_never_become_uploadable() {
        for bytes in [
            vec![],
            b"fake.wav".to_vec(),
            b"OggSnot-supported".to_vec(),
            b"fLaCnot-supported".to_vec(),
        ] {
            assert_eq!(
                Input::from_bytes("renamed.wav", bytes).unwrap_err(),
                UNSUPPORTED
            );
        }
        let source = wav(16000, 1, 4000);
        for cut in [4, 12, 23, 43, source.len() - 1] {
            assert!(Input::from_bytes("broken.wav", source[..cut].to_vec()).is_err());
        }
        for (offset, value) in [(20, 3u32), (24, 192000), (28, 1), (40, u32::MAX)] {
            let mut bytes = source.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(Input::from_bytes("broken.wav", bytes).unwrap_err(), INVALID);
        }
        let mut trailing = source;
        trailing.push(0);
        assert_eq!(
            Input::from_bytes("trailing.wav", trailing).unwrap_err(),
            INVALID
        );
        assert!(Input::from_bytes("empty.wav", wav(16000, 1, 0)).is_err());
    }

    #[test]
    fn native_import_reads_the_open_file_and_retains_no_source_path() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.bin");
        std::fs::write(&path, wav(24000, 2, 12000)).unwrap();
        let input = Input::from_path(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(input.name, "source.wav");
        assert_eq!(input.info.duration_ms, 500);
        assert_eq!(input.info.channels, 2);
        assert_eq!(
            Input::from_path(root.path()).unwrap_err(),
            super::super::attachments::UNREADABLE
        );
    }

    #[test]
    fn mp3_is_fully_decoded_and_truncated_frames_cannot_hide_at_eof() {
        let bytes = include_bytes!("../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3");
        let input = Input::from_bytes("tone.wav", bytes.to_vec()).unwrap();
        assert_eq!(input.name, "tone.mp3");
        assert_eq!(input.info.mime_type, "audio/mpeg");
        assert_eq!(input.info.sample_rate, 16000);
        assert_eq!(input.info.channels, 1);
        assert!((990..=1100).contains(&input.info.duration_ms));
        for cut in [2, bytes.len() / 2, bytes.len() - 1] {
            assert_eq!(
                Input::from_bytes("truncated.mp3", bytes[..cut].to_vec()).unwrap_err(),
                INVALID
            );
        }
        let mut junk = bytes.to_vec();
        junk.extend_from_slice(b"unrelated trailing bytes");
        assert_eq!(Input::from_bytes("bad.mp3", junk).unwrap_err(), INVALID);
        let mut tagged = b"ID3\x04\0\0\0\0\0\x03abc".to_vec();
        tagged.extend_from_slice(bytes);
        assert_eq!(
            Input::from_bytes("tagged.mp3", tagged).unwrap().info,
            input.info
        );
        assert_eq!(
            Input::from_bytes("huge-tag.mp3", b"ID3\x04\0\0\x7f\x7f\x7f\x7f".to_vec()).unwrap_err(),
            INVALID
        );
    }

    #[test]
    fn mp3_cannot_invent_duration_with_a_forged_encoder_header() {
        let mut bytes =
            include_bytes!("../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3").to_vec();
        // The Xing/Info frame count is a metadata hint, not measured audio.
        let pos = bytes
            .windows(4)
            .position(|v| v == b"Info" || v == b"Xing")
            .unwrap();
        bytes[pos + 8..pos + 12].copy_from_slice(&1u32.to_be_bytes());
        match Input::from_bytes("forged.mp3", bytes) {
            Ok(input) => assert!(input.info.duration_ms >= 990),
            Err(error) => assert_eq!(error, INVALID),
        }
    }
}
