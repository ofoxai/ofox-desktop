//! Bound and check every Layer III frame before decoding; metadata duration
//! and a decoder's tolerant EOF/resynchronization cannot prove a whole file.
use super::{info, Info, INVALID, MAX_SECONDS, TOO_LONG};
use symphonia::core::{
    codecs::audio::AudioDecoderOptions,
    formats::{probe::Hint, FormatOptions, TrackType},
    io::MediaSourceStream,
};

fn payload(bytes: &[u8]) -> Result<&[u8], String> {
    let mut start = 0;
    let mut end = bytes.len();
    if bytes.starts_with(b"ID3") {
        if bytes.len() < 10
            || !(2..=4).contains(&bytes[3])
            || bytes[4] == 0xff
            || bytes[6..10].iter().any(|v| v & 0x80 != 0)
        {
            return Err(INVALID.into());
        }
        let size = bytes[6..10]
            .iter()
            .fold(0usize, |n, byte| (n << 7) | *byte as usize);
        let footer = bytes[3] == 4 && bytes[5] & 0x10 != 0;
        start = 10 + size + if footer { 10 } else { 0 };
        if size > 1024 * 1024
            || start > bytes.len()
            || (footer && bytes.get(start - 10..start - 7) != Some(b"3DI"))
        {
            return Err(INVALID.into());
        }
    }
    if end >= 128 && bytes.get(end - 128..end - 125) == Some(b"TAG") {
        end -= 128;
    }
    bytes
        .get(start..end)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| INVALID.into())
}

struct Frame {
    bytes: usize,
    rate: u32,
    channels: u16,
    samples: u64,
}
fn frame(bytes: &[u8]) -> Result<Frame, String> {
    let raw: [u8; 4] = bytes
        .get(..4)
        .ok_or(INVALID)?
        .try_into()
        .map_err(|_| INVALID)?;
    let h = u32::from_be_bytes(raw);
    let (version, layer, index, frequency) =
        ((h >> 19) & 3, (h >> 17) & 3, (h >> 12) & 15, (h >> 10) & 3);
    if h >> 21 != 0x7ff
        || version == 1
        || layer != 1
        || index == 0
        || index == 15
        || frequency == 3
        || h & 3 == 2
    {
        return Err(INVALID.into());
    }
    let divisor = match version {
        3 => 1,
        2 => 2,
        _ => 4,
    };
    let rate = [44100, 48000, 32000][frequency as usize] / divisor;
    let bitrate = if version == 3 {
        [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ][index as usize]
    } else {
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][index as usize]
    };
    let length = (if version == 3 { 144000 } else { 72000 }) * bitrate / rate + ((h >> 9) & 1);
    Ok(Frame {
        bytes: length as usize,
        rate,
        channels: if (h >> 6) & 3 == 3 { 1 } else { 2 },
        samples: if version == 3 { 1152 } else { 576 },
    })
}

pub(super) fn inspect(bytes: &[u8]) -> Result<Info, String> {
    let bytes = payload(bytes)?;
    let mut offset = 0usize;
    let mut frames = 0u64;
    let mut specification = None;
    while offset < bytes.len() {
        let f = frame(&bytes[offset..])?;
        if specification.is_some_and(|s| s != (f.rate, f.channels)) {
            return Err(INVALID.into());
        }
        specification = Some((f.rate, f.channels));
        offset = offset
            .checked_add(f.bytes)
            .filter(|end| *end <= bytes.len())
            .ok_or(INVALID)?;
        frames += f.samples;
        // At most one encoder-information frame and padding may be trimmed
        // by gapless decoding. Still bound CPU before allocating a decoder.
        if frames > f.rate as u64 * (MAX_SECONDS + 1) {
            return Err(TOO_LONG.into());
        }
    }
    let (rate, channels) = specification.ok_or(INVALID)?;
    let stream = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes.to_vec())),
        Default::default(),
    );
    let mut format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            stream,
            FormatOptions::default(),
            Default::default(),
        )
        .map_err(|_| INVALID)?;
    let track = format.default_track(TrackType::Audio).ok_or(INVALID)?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|v| v.audio())
        .ok_or(INVALID)?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default().verify(true))
        .map_err(|_| INVALID)?;
    let mut decoded = 0u64;
    while let Some(packet) = format.next_packet().map_err(|_| INVALID)? {
        if packet.track_id != track_id {
            return Err(INVALID.into());
        }
        let buffer = decoder.decode(&packet).map_err(|_| INVALID)?;
        if buffer.spec().rate() != rate || buffer.spec().channels().count() != channels as usize {
            return Err(INVALID.into());
        }
        decoded += buffer.frames() as u64;
        if decoded > rate as u64 * MAX_SECONDS {
            return Err(TOO_LONG.into());
        }
    }
    if decoder.finalize().verify_ok == Some(false) {
        return Err(INVALID.into());
    }
    info("audio/mpeg", decoded, rate, channels)
}
