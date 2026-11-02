//! Shared bounded ZIP metadata and header policy for application imports.

use std::io::{Read, Seek, SeekFrom};
use zip::result::{ZipError, ZipResult};

fn invalid() -> ZipError {
    ZipError::InvalidArchive("unbounded or ambiguous ZIP metadata".into())
}
fn little_endian(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .rev()
        .fold(0, |value, &byte| value << 8 | u64::from(byte))
}
fn read_at(reader: &mut (impl Read + Seek), at: u64, len: usize) -> ZipResult<Vec<u8>> {
    reader.seek(SeekFrom::Start(at))?;
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Bound raw central records before ZIP allocates metadata. Imports accept a
/// single-disk, zero-offset archive with one unambiguous footer, including ZIP64.
/// Extra fields are left to ZIP's parser after their aggregate size is bounded.
fn checked_entries(reader: &mut (impl Read + Seek), max_entries: usize) -> ZipResult<(u64, u64)> {
    const END: usize = 22;
    const LOCATOR: usize = 20;
    const RECORD64: usize = 56;
    const HEADER: usize = 46;
    // Bound directory bytes independently of compressed member sizes.
    let max_directory = (max_entries as u64)
        .saturating_mul(2048)
        .clamp(1024 * 1024, 32 * 1024 * 1024);
    let len = reader.seek(SeekFrom::End(0)).map_err(|_| invalid())?;
    // ZIP 9 searches roughly 64 KiB; cover its entire window conservatively.
    let tail_len = len.min(128 * 1024);
    let tail = read_at(reader, len - tail_len, tail_len as usize)?;
    // A malformed directory must never expose a second EOCD through fallback.
    // Ambiguous footer signatures (including inside comments/data) are refused.
    if tail
        .windows(4)
        .filter(|part| *part == b"PK\x05\x06")
        .count()
        != 1
    {
        return Err(invalid());
    }
    let at = tail
        .windows(4)
        .rposition(|part| part == b"PK\x05\x06")
        .filter(|&at| at + END <= tail.len())
        .ok_or_else(invalid)?;
    let field = |from: usize, size: usize| little_endian(&tail[at + from..at + from + size]);
    let end = len - tail_len + at as u64;
    if at + END + field(20, 2) as usize != tail.len() || field(4, 2) != 0 || field(6, 2) != 0 {
        return Err(invalid());
    }
    let (on_disk, total) = (field(8, 2), field(10, 2));
    let classic_size = field(12, 4);
    let classic_offset = field(16, 4);
    let has_locator = at
        .checked_sub(LOCATOR)
        .is_some_and(|locator| tail[locator..locator + 4] == *b"PK\x06\x07");
    let (count, directory, directory_end) = if has_locator
        || total == u64::from(u16::MAX)
        || field(12, 4) == u64::from(u32::MAX)
        || field(16, 4) == u64::from(u32::MAX)
    {
        // The crate then reads only the ZIP64 record.
        let locator = at.checked_sub(LOCATOR).ok_or_else(invalid)?;
        let locator_field =
            |from: usize, size: usize| little_endian(&tail[locator + from..locator + from + size]);
        let locator_at = end - LOCATOR as u64;
        let record_at = locator_field(8, 8);
        if tail[locator..locator + 4] != *b"PK\x06\x07"
            || locator_field(4, 4) != 0
            || locator_field(16, 4) > 1
            || record_at
                .checked_add(RECORD64 as u64)
                .is_none_or(|end| end > locator_at)
        {
            return Err(invalid());
        }
        let record = read_at(reader, record_at, RECORD64)?;
        let field = |from: usize, size: usize| little_endian(&record[from..from + size]);
        let count = field(32, 8);
        if record[..4] != *b"PK\x06\x06"
            || field(4, 8) < 44
            || field(4, 8) > 1024 * 1024
            || field(4, 8).checked_add(12) != Some(locator_at - record_at)
            || field(14, 2) > field(12, 2)
            || field(16, 4) != 0
            || field(20, 4) != 0
            || field(24, 8) != count
            || on_disk != total
            || (total != u64::from(u16::MAX) && total != count)
            || (classic_size != u64::from(u32::MAX) && classic_size != field(40, 8))
            || (classic_offset != u64::from(u32::MAX) && classic_offset != field(48, 8))
        {
            return Err(invalid());
        }
        (count, (field(48, 8), field(40, 8)), record_at)
    } else if on_disk == total {
        (total, (field(16, 4), field(12, 4)), end)
    } else {
        return Err(invalid());
    };
    if count > max_entries as u64 {
        return Err(invalid());
    }
    let (start, size) = directory;
    if start.checked_add(size) != Some(directory_end) {
        return Err(invalid());
    }
    if size > max_directory {
        return Err(invalid());
    }
    let headers = read_at(reader, start, size as usize)?;
    let (mut at, mut found) = (0, 0u64);
    while at < headers.len() {
        let header = headers.get(at..at + HEADER).ok_or_else(invalid)?;
        let field = |from: usize, size: usize| little_endian(&header[from..from + size]);
        if header[..4] != *b"PK\x01\x02" {
            return Err(invalid());
        }
        // ZIP64 may move the local offset into an extra field. Resolve and
        // validate local headers only after bounded ZIP metadata parsing.
        at += HEADER + field(28, 2) as usize + field(30, 2) as usize + field(32, 2) as usize;
        found += 1;
    }
    if at != headers.len() || found != count {
        return Err(invalid());
    }
    Ok((count, start))
}

/// Open bounded, unambiguous metadata without scanning payload signatures.
/// The caller must first bound the input size and then validate local headers
/// before reading or publishing members.
pub fn open<R: Read + Seek>(mut reader: R, max_entries: usize) -> ZipResult<zip::ZipArchive<R>> {
    let (count, directory) = checked_entries(&mut reader, max_entries)?;
    let archive = zip::ZipArchive::with_config(
        zip::read::Config {
            archive_offset: zip::read::ArchiveOffset::Known(0),
        },
        reader,
    )?;
    if archive.len() as u64 != count
        || archive.offset() != 0
        || archive.central_directory_start() != directory
    {
        return Err(invalid());
    }
    Ok(archive)
}

/// A patched ZIP library alone does not enable strict local/central reads.
pub fn validate_headers<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> ZipResult<()> {
    archive.validate_header_consistency()
}
