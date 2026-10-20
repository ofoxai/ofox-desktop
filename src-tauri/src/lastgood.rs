//! Last good generation of a state file: a file that is there but not valid is
//! unknown, never empty.
//!
//! A machine that went down mid-write can leave a state file at its full length
//! with every byte zero. Reading that as "no settings" and saving over it would
//! make the loss final. Here, before each save the file there now is kept as
//! `<file>.bak` when it is valid, or copied aside as `<file>.bad-<time>` when it
//! is not; a read of a file that is not valid gives the `.bak`'s bytes instead
//! (and says so), or reports the file unreadable. Ported from magpie's
//! `internal/lastgood`.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

use crate::config::{atomic_write_with, AtomicOpts};
use crate::error::AppError;

/// Backups and bad copies hold the same data as the file, so they get its mode.
const FILE_MODE: u32 = 0o600;

/// What `.bad-<time>` names try per second before giving up.
const MAX_BAD_COPY_NAMES: usize = 1000;

/// A state file read through [`read`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LgRead {
    /// The bytes to use: the file's own, or the `.bak`'s when `recovered`.
    pub bytes: Vec<u8>,
    /// The file was not valid and `bytes` are its last good generation.
    pub recovered: bool,
    /// Where the invalid bytes were copied aside; `None` when the file was valid
    /// or the copy could not be made.
    pub bad_copy: Option<PathBuf>,
    /// What was wrong with the file; `None` when it was valid.
    pub why: Option<String>,
}

/// Why [`read`] gave no bytes.
#[derive(Debug, thiserror::Error)]
pub enum LgError {
    /// No file there. A crash leaves a file renamed over as it was or as it
    /// became, never gone, so a missing file was removed on purpose: its `.bak`
    /// is not looked at.
    #[error("not found")]
    NotFound,
    /// The file is there, but neither it nor its `.bak` is valid: unknown,
    /// never empty.
    #[error("{} can't be read ({why}) and has no good backup", path.display())]
    Unreadable {
        path: PathBuf,
        why: String,
        bad_copy: Option<PathBuf>,
    },
    /// The file could not be read at all (permissions, a directory...).
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
}

/// `<path>.bak`, where `path`'s last good generation is kept.
pub fn bak_path(path: &Path) -> PathBuf {
    with_suffix(path, ".bak")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

/// Empty, whitespace only, or every byte zero: what a half-finished write leaves.
/// Never a good copy, whatever `valid` says.
fn is_blank(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| *b == 0 || b.is_ascii_whitespace())
}

fn is_good(bytes: &[u8], valid: &impl Fn(&[u8]) -> bool) -> bool {
    !is_blank(bytes) && valid(bytes)
}

/// What is wrong with a bad file, for the log and the GUI.
fn describe(bytes: &[u8]) -> String {
    if !bytes.is_empty() && bytes.iter().all(|b| *b == 0) {
        format!("{} bytes, every one zero", bytes.len())
    } else if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        "it is empty".to_string()
    } else {
        "it doesn't parse".to_string()
    }
}

/// Read `path`. Valid contents come back as they are. A file there that is not
/// valid is copied aside (`<path>.bad-<time>`) and gives its `.bak`'s bytes when
/// those are valid (`recovered`), else [`LgError::Unreadable`].
pub fn read(path: &Path, valid: impl Fn(&[u8]) -> bool) -> Result<LgRead, LgError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(LgError::NotFound),
        Err(source) => {
            return Err(LgError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    if is_good(&bytes, &valid) {
        return Ok(LgRead {
            bytes,
            recovered: false,
            bad_copy: None,
            why: None,
        });
    }

    let why = describe(&bytes);
    let bad_copy = match keep_bad(path, &bytes) {
        Ok(copy) => Some(copy),
        Err(err) => {
            log::warn!(
                "{} can't be read ({why}) and couldn't be kept: {err}",
                display_name(path)
            );
            None
        }
    };
    match fs::read(bak_path(path)) {
        Ok(bak) if is_good(&bak, &valid) => {
            log::warn!(
                "{} can't be read ({why}); its last good copy, {}, is used",
                display_name(path),
                display_name(&bak_path(path)),
            );
            Ok(LgRead {
                bytes: bak,
                recovered: true,
                bad_copy,
                why: Some(why),
            })
        }
        _ => Err(LgError::Unreadable {
            path: path.to_path_buf(),
            why,
            bad_copy,
        }),
    }
}

/// Call before `path` is replaced. The file there now, when valid, becomes the
/// last good generation (`<path>.bak`, written atomically); one that is not valid
/// is copied aside and the `.bak` left as it is, so an unreadable file is never
/// written over before a copy of it is kept. A missing file needs neither. An
/// error means the write must not go on.
pub fn keep(path: &Path, valid: impl Fn(&[u8]) -> bool) -> Result<(), AppError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(AppError::io(path, err)),
    };
    if !is_good(&bytes, &valid) {
        keep_bad(path, &bytes)?;
        return Ok(());
    }
    let bak = bak_path(path);
    if fs::read(&bak).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    atomic_write_with(
        &bak,
        &bytes,
        AtomicOpts {
            mode: Some(FILE_MODE),
        },
    )
}

/// Copy `bytes`, `path`'s contents that are not valid, aside as
/// `<path>.bad-<YYYYMMDD-HHMMSS>` and give that name. Two different bad files
/// within one second are both kept (`-2`, `-3`...); the same bad bytes kept
/// already, under any time, are not kept again.
pub fn keep_bad(path: &Path, bytes: &[u8]) -> Result<PathBuf, AppError> {
    keep_bad_at(path, bytes, Local::now())
}

fn keep_bad_at(path: &Path, bytes: &[u8], now: DateTime<Local>) -> Result<PathBuf, AppError> {
    if let Some(existing) = kept_already(path, bytes) {
        return Ok(existing);
    }
    let stamp = with_suffix(path, &format!(".bad-{}", now.format("%Y%m%d-%H%M%S")));
    for attempt in 1..=MAX_BAD_COPY_NAMES {
        let name = if attempt == 1 {
            stamp.clone()
        } else {
            with_suffix(&stamp, &format!("-{attempt}"))
        };
        if fs::symlink_metadata(&name).is_ok() {
            continue;
        }
        match create_new_private(&name) {
            Ok(mut file) => {
                let written = file.write_all(bytes).and_then(|()| file.sync_all());
                drop(file);
                if let Err(err) = written {
                    let _ = fs::remove_file(&name);
                    return Err(AppError::io(&name, err));
                }
                log::warn!(
                    "{} can't be read ({}); it is kept as {}",
                    display_name(path),
                    describe(bytes),
                    display_name(&name),
                );
                return Ok(name);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(AppError::io(&name, err)),
        }
    }
    Err(AppError::Config(format!(
        "{} can't be read and there is no free name to keep it under",
        display_name(path)
    )))
}

fn create_new_private(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    options.open(path)
}

/// The copy of `bytes` kept aside for `path` before, whatever second it was kept
/// in.
fn kept_already(path: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let prefix = format!("{}.bad-", path.file_name()?.to_string_lossy());
    fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path())
        .find(|candidate| {
            fs::metadata(candidate).is_ok_and(|meta| meta.len() == bytes.len() as u64)
                && fs::read(candidate).is_ok_and(|old| old == bytes)
        })
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn json_valid(bytes: &[u8]) -> bool {
        serde_json::from_slice::<serde_json::Value>(bytes).is_ok()
    }

    fn zeros(n: usize) -> Vec<u8> {
        vec![0; n]
    }

    fn settings_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("settings.json")
    }

    fn bad_copies(path: &Path) -> Vec<PathBuf> {
        let prefix = format!("{}.bad-", path.file_name().unwrap().to_string_lossy());
        let mut found: Vec<PathBuf> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&prefix)
            })
            .collect();
        found.sort();
        found
    }

    fn at(second: u32, millis: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 10, 12, 0, second).unwrap()
            + chrono::Duration::milliseconds(i64::from(millis))
    }

    #[test]
    fn a_zeroed_file_is_read_from_its_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        let good = br#"{"language":"zh"}"#;
        fs::write(&path, good).unwrap();
        keep(&path, json_valid).unwrap();
        fs::write(&path, zeros(10783)).unwrap();

        let read = read(&path, json_valid).unwrap();

        assert_eq!(read.bytes, good);
        assert!(read.recovered);
        assert_eq!(read.why.as_deref(), Some("10783 bytes, every one zero"));
    }

    #[test]
    fn a_recovered_read_copies_the_bad_bytes_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        fs::write(bak_path(&path), br#"{"good":true}"#).unwrap();
        fs::write(&path, br#"{"half"#).unwrap();

        let read = read(&path, json_valid).unwrap();

        let copy = read.bad_copy.expect("a bad copy is made");
        assert_eq!(fs::read(copy).unwrap(), br#"{"half"#);
        assert_eq!(
            fs::read(&path).unwrap(),
            br#"{"half"#,
            "read does not rewrite"
        );
    }

    #[test]
    fn a_valid_file_is_read_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        fs::write(&path, br#"{"a":1}"#).unwrap();
        fs::write(bak_path(&path), br#"{"a":0}"#).unwrap();

        let read = read(&path, json_valid).unwrap();

        assert_eq!(read.bytes, br#"{"a":1}"#);
        assert!(!read.recovered);
        assert!(read.bad_copy.is_none());
        assert!(bad_copies(&path).is_empty());
    }

    #[test]
    fn a_missing_file_is_not_found_and_its_backup_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        fs::write(bak_path(&path), br#"{"good":true}"#).unwrap();

        assert!(matches!(read(&path, json_valid), Err(LgError::NotFound)));
    }

    #[test]
    fn a_bad_file_without_a_backup_is_unreadable_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        fs::write(&path, zeros(64)).unwrap();

        let err = read(&path, json_valid).unwrap_err();

        let LgError::Unreadable { why, bad_copy, .. } = err else {
            panic!("expected Unreadable, got {err:?}");
        };
        assert_eq!(why, "64 bytes, every one zero");
        assert_eq!(fs::read(bad_copy.unwrap()).unwrap(), zeros(64));
    }

    #[test]
    fn a_bad_file_with_a_bad_backup_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        fs::write(&path, br#"{"half"#).unwrap();
        fs::write(bak_path(&path), zeros(8)).unwrap();

        assert!(matches!(
            read(&path, json_valid),
            Err(LgError::Unreadable { .. })
        ));
    }

    #[test]
    fn empty_whitespace_and_zero_files_are_bad_even_when_valid_says_yes() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        for blank in [&b""[..], &b" \n\t"[..], &[0, 0, 0][..]] {
            fs::write(&path, blank).unwrap();
            assert!(
                matches!(read(&path, |_| true), Err(LgError::Unreadable { .. })),
                "{blank:?} must not be a good copy"
            );
        }
    }

    #[test]
    fn the_same_bad_file_is_copied_aside_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        fs::write(&path, zeros(32)).unwrap();

        let _ = read(&path, json_valid);
        let _ = read(&path, json_valid);
        keep(&path, json_valid).unwrap();
        keep(&path, json_valid).unwrap();

        assert_eq!(bad_copies(&path).len(), 1);
    }

    #[test]
    fn two_bad_files_in_one_second_are_both_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);

        let first = keep_bad_at(&path, &zeros(10), at(0, 100)).unwrap();
        let second = keep_bad_at(&path, br#"{"half"#, at(0, 900)).unwrap();
        let third = keep_bad_at(&path, &zeros(11), at(0, 950)).unwrap();

        let names: Vec<String> = [first, second, third]
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "settings.json.bad-20261010-120000",
                "settings.json.bad-20261010-120000-2",
                "settings.json.bad-20261010-120000-3",
            ]
        );
        assert_eq!(bad_copies(&path).len(), 3);
    }

    #[test]
    fn the_same_bad_bytes_are_not_kept_again_in_a_later_second() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);

        let first = keep_bad_at(&path, &zeros(10), at(1, 0)).unwrap();
        let again = keep_bad_at(&path, &zeros(10), at(5, 0)).unwrap();
        let other = keep_bad_at(&path, &zeros(12), at(5, 0)).unwrap();

        assert_eq!(again, first);
        assert_ne!(other, first);
        assert_eq!(bad_copies(&path).len(), 2);
    }

    #[test]
    fn keep_makes_a_valid_file_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        fs::write(&path, br#"{"a":1}"#).unwrap();

        keep(&path, json_valid).unwrap();

        assert_eq!(fs::read(bak_path(&path)).unwrap(), br#"{"a":1}"#);
        assert!(bad_copies(&path).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn the_backup_and_a_bad_copy_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        fs::write(&path, br#"{"a":1}"#).unwrap();
        keep(&path, json_valid).unwrap();
        fs::write(&path, zeros(8)).unwrap();
        keep(&path, json_valid).unwrap();

        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&bak_path(&path)), 0o600);
        assert_eq!(mode(&bad_copies(&path)[0]), 0o600);
    }

    #[test]
    fn keep_leaves_the_good_backup_when_the_file_is_bad() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        fs::write(bak_path(&path), br#"{"good":true}"#).unwrap();
        fs::write(&path, zeros(16)).unwrap();

        keep(&path, json_valid).unwrap();

        assert_eq!(fs::read(bak_path(&path)).unwrap(), br#"{"good":true}"#);
        assert_eq!(bad_copies(&path).len(), 1);
    }

    #[test]
    fn keep_of_a_missing_file_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);

        keep(&path, json_valid).unwrap();

        assert!(!bak_path(&path).exists());
        assert!(bad_copies(&path).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn keep_fails_when_a_bad_file_cannot_be_copied_aside() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(&dir);
        fs::write(&path, zeros(64)).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let writable_anyway = fs::File::create(dir.path().join("probe")).is_ok();
        let result = keep(&path, json_valid);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        if writable_anyway {
            return; // running as root: the folder mode does not stop the copy
        }

        assert!(result.is_err(), "a bad file not kept must stop the write");
    }

    #[test]
    fn describe_says_what_is_wrong() {
        assert_eq!(describe(&zeros(3)), "3 bytes, every one zero");
        assert_eq!(describe(b""), "it is empty");
        assert_eq!(describe(b"  \n"), "it is empty");
        assert_eq!(describe(br#"{"a":"#), "it doesn't parse");
    }
}
