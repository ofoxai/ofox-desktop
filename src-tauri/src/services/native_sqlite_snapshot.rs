//! Bounded copies of a stable native SQLite main/WAL pair. Native SHM is never
//! opened or created. Callers own schema/row decoding and map static errors.
use crate::{error::AppError, session_manager::source_cache::SourceStamp};
use rusqlite::{Connection, OpenFlags};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

type Error = fn(&str) -> AppError;

/// SQLite resolves a leaf symlink before finding its WAL. A raw main-file
/// copy beside alias-named sidecars would silently lose that target WAL.
pub(crate) fn validate_paths(path: &Path, error: Error) -> Result<(), AppError> {
    for component in [
        path.to_owned(),
        sidecar(path, "-wal"),
        sidecar(path, "-journal"),
    ] {
        match fs::symlink_metadata(component) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(error("sourceSymlinkUnsupported"))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(error("sourceUnreadable")),
        }
    }
    Ok(())
}
struct Component {
    path: PathBuf,
    file: File,
    stamp: SourceStamp,
    error: Error,
}
impl Component {
    fn open(path: PathBuf, budget: u64, error: Error) -> Result<Self, AppError> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW);
        }
        let file = options.open(&path).map_err(|_| error("sourceUnreadable"))?;
        if !file
            .metadata()
            .map_err(|_| error("sourceUnreadable"))?
            .is_file()
        {
            return Err(error("sourceUnreadable"));
        }
        let stamp = SourceStamp::of(&file).map_err(|_| error("sourceUnreadable"))?;
        if stamp.size > budget {
            return Err(error("sourceTooLarge"));
        }
        Ok(Self {
            path,
            file,
            stamp,
            error,
        })
    }
    fn verify(&self) -> Result<(), AppError> {
        self.stamp
            .verify(&self.path)
            .map_err(|_| (self.error)("sourceChanged"))?;
        if SourceStamp::of(&self.file).map_err(|_| (self.error)("sourceChanged"))? != self.stamp {
            return Err((self.error)("sourceChanged"));
        }
        Ok(())
    }
    fn copy_to(&mut self, destination: &Path) -> Result<(), AppError> {
        let mut target = File::create(destination).map_err(|_| (self.error)("snapshotFailed"))?;
        let copied = std::io::copy(&mut (&mut self.file).take(self.stamp.size + 1), &mut target)
            .map_err(|_| (self.error)("snapshotFailed"))?;
        target.flush().map_err(|_| (self.error)("snapshotFailed"))?;
        if copied != self.stamp.size {
            return Err((self.error)("sourceChanged"));
        }
        self.verify()
    }
}
fn exists(path: &Path, error: Error) -> Result<bool, AppError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(error("sourceUnreadable")),
    }
}
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
pub(crate) struct SqliteSnapshot {
    main: Component,
    wal: Option<Component>,
    path: PathBuf,
    error: Error,
}
impl SqliteSnapshot {
    pub(crate) fn open(path: &Path, budget: u64, error: Error) -> Result<Self, AppError> {
        validate_paths(path, error)?;
        if !exists(path, error)? {
            return Err(error("sourceMissing"));
        }
        // A paused rollback writer may have uncommitted main pages. Copying
        // without its rollback journal would not establish a native snapshot.
        if exists(&sidecar(path, "-journal"), error)? {
            return Err(error("sourceBusy"));
        }
        let main = Component::open(path.to_owned(), budget, error)?;
        let wal_path = sidecar(path, "-wal");
        let wal = exists(&wal_path, error)?
            .then(|| Component::open(wal_path, budget, error))
            .transpose()?;
        if main
            .stamp
            .size
            .checked_add(wal.as_ref().map_or(0, |f| f.stamp.size))
            .is_none_or(|size| size > budget)
        {
            return Err(error("sourceTooLarge"));
        }
        let result = Self {
            main,
            wal,
            path: path.to_owned(),
            error,
        };
        result.verify()?;
        Ok(result)
    }
    pub(crate) fn verify(&self) -> Result<(), AppError> {
        validate_paths(&self.path, self.error)?;
        self.main.verify()?;
        if exists(&sidecar(&self.path, "-journal"), self.error)?
            || exists(&sidecar(&self.path, "-wal"), self.error)? != self.wal.is_some()
        {
            return Err((self.error)("sourceChanged"));
        }
        if let Some(wal) = &self.wal {
            wal.verify()?;
        }
        Ok(())
    }
    pub(crate) fn read<T>(
        &mut self,
        after_copy: impl FnOnce(),
        read: impl FnOnce(&Connection) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let temporary = tempfile::tempdir().map_err(|_| (self.error)("snapshotFailed"))?;
        let copy = temporary.path().join("snapshot.db");
        self.main.copy_to(&copy)?;
        if let Some(wal) = &mut self.wal {
            wal.copy_to(&sidecar(&copy, "-wal"))?;
        }
        self.verify()?;
        after_copy();
        let conn = Connection::open_with_flags(
            &copy,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| (self.error)("invalidDatabase"))?;
        conn.busy_timeout(Duration::from_millis(500))
            .map_err(|_| (self.error)("invalidDatabase"))?;
        conn.pragma_update(None, "query_only", true)
            .map_err(|_| (self.error)("invalidDatabase"))?;
        let before: i64 = conn
            .pragma_query_value(None, "data_version", |r| r.get(0))
            .map_err(|_| (self.error)("invalidDatabase"))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|_| (self.error)("invalidDatabase"))?;
        let result = read(&tx)?;
        tx.commit().map_err(|_| (self.error)("invalidDatabase"))?;
        let after: i64 = conn
            .pragma_query_value(None, "data_version", |r| r.get(0))
            .map_err(|_| (self.error)("invalidDatabase"))?;
        if before != after {
            return Err((self.error)("sourceChanged"));
        }
        self.verify()?;
        Ok(result)
    }
}
