//! Validate a single complete Ogg Opus stream, including page CRCs and lacing.
use crate::services::assistant::audio_input::{Info, INVALID};
pub(super) fn inspect(bytes: &[u8]) -> Result<Info, String> {
    let mut table = [0u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut n = (i as u32) << 24;
        for _ in 0..8 {
            n = if n & 0x80000000 != 0 {
                (n << 1) ^ 0x04c11db7
            } else {
                n << 1
            };
        }
        *entry = n;
    }
    let mut offset = 0usize;
    let mut sequence = 0u32;
    let mut serial = None;
    let mut packet = Vec::new();
    let mut packets = 0usize;
    let mut continued = false;
    let mut ended = false;
    let mut channels = 0u16;
    let mut skip = 0u64;
    let mut frames = 0u64;
    while offset < bytes.len() {
        if ended {
            return Err(INVALID.into());
        }
        let h = bytes.get(offset..offset + 27).ok_or(INVALID)?;
        if &h[..4] != b"OggS"
            || h[4] != 0
            || h[5] & !7 != 0
            || (h[5] & 1 != 0) != continued
            || (sequence == 0) != (h[5] & 2 != 0)
        {
            return Err(INVALID.into());
        }
        let stream = u32::from_le_bytes(h[14..18].try_into().map_err(|_| INVALID)?);
        if serial.is_some_and(|value| value != stream)
            || u32::from_le_bytes(h[18..22].try_into().map_err(|_| INVALID)?) != sequence
        {
            return Err(INVALID.into());
        }
        serial = Some(stream);
        sequence = sequence.checked_add(1).ok_or(INVALID)?;
        let count = h[26] as usize;
        if count == 0 {
            return Err(INVALID.into());
        }
        let lacing = bytes.get(offset + 27..offset + 27 + count).ok_or(INVALID)?;
        let size = lacing.iter().map(|n| *n as usize).sum::<usize>();
        let page_end = offset
            .checked_add(27 + count + size)
            .filter(|end| *end <= bytes.len())
            .ok_or(INVALID)?;
        let page = &bytes[offset..page_end];
        let expected = u32::from_le_bytes(h[22..26].try_into().map_err(|_| INVALID)?);
        let mut crc = 0u32;
        for (i, byte) in page.iter().enumerate() {
            let byte = if (22..26).contains(&i) { 0 } else { *byte };
            crc = (crc << 8) ^ table[((crc >> 24) as u8 ^ byte) as usize];
        }
        if crc != expected {
            return Err(INVALID.into());
        }
        let mut at = offset + 27 + count;
        for segment in lacing {
            let size = *segment as usize;
            if packet.len() + size > 65536 {
                return Err(INVALID.into());
            }
            packet.extend_from_slice(&bytes[at..at + size]);
            at += size;
            continued = *segment == 255;
            if !continued {
                if packets == 0 {
                    if packet.len() != 19
                        || &packet[..8] != b"OpusHead"
                        || packet[8] != 1
                        || !(1..=2).contains(&packet[9])
                        || packet[18] != 0
                    {
                        return Err(INVALID.into());
                    }
                    channels = packet[9] as u16;
                    skip = u16::from_le_bytes([packet[10], packet[11]]) as u64;
                } else if packets == 1 {
                    if packet.len() < 16 || &packet[..8] != b"OpusTags" {
                        return Err(INVALID.into());
                    }
                } else if packet.is_empty() {
                    return Err(INVALID.into());
                }
                packets += 1;
                packet.clear();
            }
        }
        ended = h[5] & 4 != 0;
        if ended {
            let granule = u64::from_le_bytes(h[6..14].try_into().map_err(|_| INVALID)?);
            frames = granule
                .checked_sub(skip)
                .filter(|n| *n > 0 && *n <= 48000 * 3600)
                .ok_or(INVALID)?;
            if continued || !packet.is_empty() {
                return Err(INVALID.into());
            }
        }
        offset = page_end;
    }
    if !ended || packets < 3 {
        return Err(INVALID.into());
    }
    Ok(Info {
        mime_type: "audio/ogg".into(),
        duration_ms: (frames * 1000).div_ceil(48000),
        sample_rate: 48000,
        channels,
    })
}
