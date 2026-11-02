use std::io::{Cursor, Read, Write};

use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

fn stored_archive() -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "safe.txt",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"test archive payload").unwrap();
    writer.finish().unwrap().into_inner()
}

fn read_payload(bytes: Vec<u8>) -> Result<Vec<u8>, zip::result::ZipError> {
    let mut archive = cc_switch_lib::zip_security::open(Cursor::new(bytes), 10)?;
    cc_switch_lib::zip_security::validate_headers(&mut archive)?;
    let mut entry = archive.by_index(0)?;
    let mut payload = Vec::new();
    entry.read_to_end(&mut payload)?;
    Ok(payload)
}

#[test]
fn reads_a_consistent_current_writer_archive() {
    assert_eq!(
        read_payload(stored_archive()).unwrap(),
        b"test archive payload"
    );
}

#[test]
fn reads_an_archive_written_by_zip_2_4_2() {
    let bytes = include_bytes!("fixtures/archive-security/zip-2.4.2-stored.zip");
    assert_eq!(
        read_payload(bytes.to_vec()).unwrap(),
        b"test archive payload"
    );
}

#[test]
fn reads_deflated_and_zip64_archives_written_by_zip_2_4_2() {
    let fixtures = [
        (
            "deflated",
            include_bytes!("fixtures/archive-security/zip-2.4.2-deflated.zip").as_slice(),
        ),
        (
            "zip64",
            include_bytes!("fixtures/archive-security/zip-2.4.2-zip64.zip").as_slice(),
        ),
    ];
    for (label, bytes) in fixtures {
        let mut archive = cc_switch_lib::zip_security::open(Cursor::new(bytes), 10).unwrap();
        cc_switch_lib::zip_security::validate_headers(&mut archive)
            .unwrap_or_else(|error| panic!("legacy {label}: {error}"));
        assert_eq!(archive.len(), 2);
        for (name, expected) in [
            ("safe.txt", b"test archive payload".as_slice()),
            ("notes/说明.txt", b"unicode filename fixture".as_slice()),
        ] {
            let mut entry = archive.by_name(name).unwrap();
            assert_eq!(entry.name().unwrap().as_ref(), name);
            let mut content = Vec::new();
            entry.read_to_end(&mut content).unwrap();
            assert_eq!(content, expected, "legacy {label}: {name}");
        }
    }
}

#[test]
fn refuses_a_local_filename_that_differs_from_the_central_directory() {
    let mut bytes = stored_archive();
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    assert_eq!(&bytes[30..38], b"safe.txt");
    bytes[30..38].copy_from_slice(b"evil.txt");

    assert!(
        read_payload(bytes).is_err(),
        "a ZIP member must not expose different filenames to local and central readers"
    );
}

#[test]
fn refuses_a_local_compression_method_that_differs_from_the_central_directory() {
    let mut bytes = stored_archive();
    // The central directory still describes the stored payload. Only the local
    // header claims deflate, so either parser must refuse this inconsistent ZIP.
    bytes[8..10].copy_from_slice(&8u16.to_le_bytes());

    assert!(
        read_payload(bytes).is_err(),
        "a ZIP member must not advertise two different compression methods"
    );
}

#[test]
fn metadata_does_not_scan_stored_payload_for_directory_signatures() {
    use std::cell::Cell;
    use std::io::{Seek, SeekFrom};
    use std::rc::Rc;
    struct Counted {
        inner: Cursor<Vec<u8>>,
        read: Rc<Cell<usize>>,
    }
    impl Read for Counted {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(out)?;
            self.read.set(self.read.get() + n);
            Ok(n)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "safe.txt",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(&b"PK\x01\x02".repeat(128 * 1024)).unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let read = Rc::new(Cell::new(0));
    let archive = cc_switch_lib::zip_security::open(
        Counted {
            inner: Cursor::new(bytes),
            read: read.clone(),
        },
        1,
    )
    .unwrap();
    assert_eq!(archive.len(), 1);
    assert!(
        read.get() < 150_000,
        "metadata read {} bytes from a stored payload",
        read.get()
    );
}

#[test]
fn raw_duplicate_directory_count_is_bounded_before_deduplication() {
    let mut bytes = stored_archive();
    let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    let end = bytes.len() - 22;
    let header = bytes[central..end].to_vec();
    bytes.splice(end..end, header.iter().copied());
    let end = bytes.len() - 22;
    bytes[end + 8..end + 10].copy_from_slice(&2u16.to_le_bytes());
    bytes[end + 10..end + 12].copy_from_slice(&2u16.to_le_bytes());
    bytes[end + 12..end + 16].copy_from_slice(&((header.len() * 2) as u32).to_le_bytes());
    assert!(cc_switch_lib::zip_security::open(Cursor::new(bytes), 1).is_err());
}

#[test]
fn refuses_ambiguous_footers_instead_of_falling_back() {
    let mut bytes = stored_archive();
    let end = bytes.len() - 22;
    let fake = bytes[end..].to_vec();
    bytes.extend_from_slice(&fake);
    assert!(cc_switch_lib::zip_security::open(Cursor::new(bytes), 10).is_err());
}

#[test]
fn malformed_directory_extra_fields_fail_without_footer_fallback() {
    let mut bytes = stored_archive();
    let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    // Claim a truncated extra field; this must not fall back to another footer.
    bytes[central + 30..central + 32].copy_from_slice(&4u16.to_le_bytes());
    assert!(cc_switch_lib::zip_security::open(Cursor::new(bytes), 10).is_err());
}

#[test]
fn reads_empty_archives() {
    let writer = ZipWriter::new(Cursor::new(Vec::new()));
    let bytes = writer.finish().unwrap().into_inner();
    assert!(cc_switch_lib::zip_security::open(Cursor::new(bytes), 10)
        .unwrap()
        .is_empty());
}

#[test]
fn reads_supported_extra_fields() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut options = zip::write::FullFileOptions::default();
    options
        .add_extra_field(0x5455, [1, 0, 0, 0, 0], false)
        .unwrap();
    writer.start_file("safe.txt", options).unwrap();
    writer.write_all(b"test archive payload").unwrap();
    assert_eq!(
        read_payload(writer.finish().unwrap().into_inner()).unwrap(),
        b"test archive payload"
    );
}

#[test]
fn refuses_an_extra_footer_signature_near_the_parser_window_boundary() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "safe.txt",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
    let mut payload = vec![b'x'; 70_000];
    payload[..4].copy_from_slice(b"PK\x05\x06");
    writer.write_all(&payload).unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    assert!(cc_switch_lib::zip_security::open(Cursor::new(bytes), 10).is_err());
}

#[test]
fn reads_a_small_archive_with_a_zip64_local_header_offset() {
    let mut bytes = stored_archive();
    let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    bytes[central + 42..central + 46].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[central + 30..central + 32].copy_from_slice(&12u16.to_le_bytes());
    let extra = [1, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let end = bytes.len() - 22;
    bytes.splice(end..end, extra);
    let end = bytes.len() - 22;
    let size = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().unwrap()) + 12;
    bytes[end + 12..end + 16].copy_from_slice(&size.to_le_bytes());
    // Prove the fixture is valid independently of the application preflight.
    let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    assert_eq!(archive.by_name("safe.txt").unwrap().header_start(), 0);
    assert_eq!(read_payload(bytes).unwrap(), b"test archive payload");
}
