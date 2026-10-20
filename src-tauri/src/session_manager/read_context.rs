//! Cooperative, bounded native transcript reads. Contexts live only for one read;
//! neither message bodies nor source receipts are persisted.
use super::{managed, source_cache::SourceStamp};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// One session's read: a long Claude Code session passes 40 MB and a Codex
/// rollout hundreds; past this it can be neither opened nor searched.
pub(crate) const SESSION_BYTES: u64 = 512 * 1024 * 1024;
/// One line: a Codex rollout's tool output or image runs to 18 MB; as the
/// Codex usage reader and magpie, a line past 32 MiB is refused.
pub(crate) const LINE_BYTES: usize = 32 * 1024 * 1024;
/// At least one whole session, or a session past it never finishes a batch.
pub(crate) const BATCH_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const BATCH_DURATION: Duration = Duration::from_secs(15);
const MAX_SQLITE_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SQLITE_BATCH_IO: u64 = 1024 * 1024 * 1024;

thread_local! {
    static CURRENT: RefCell<Option<Arc<ReadContext>>> = const { RefCell::new(None) };
}

#[cfg(test)]
type OpenObserver = Box<dyn Fn(&Path, bool)>;
#[cfg(test)]
type VerifyObserver = Box<dyn Fn(usize)>;
#[cfg(test)]
thread_local! {
    static OPEN_OBSERVER: RefCell<Option<OpenObserver>> = const { RefCell::new(None) };
    static VERIFY_OBSERVER: RefCell<Option<VerifyObserver>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_verify_observer<T>(
    observer: impl Fn(usize) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            VERIFY_OBSERVER.with(|observer| {
                observer.replace(None);
            });
        }
    }
    VERIFY_OBSERVER.with(|current| {
        current.replace(Some(Box::new(observer)));
    });
    let _reset = Reset;
    action()
}

#[cfg(test)]
pub(crate) fn with_open_observer<T>(
    observer: impl Fn(&Path, bool) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            OPEN_OBSERVER.with(|observer| {
                observer.replace(None);
            });
        }
    }
    OPEN_OBSERVER.with(|current| {
        current.replace(Some(Box::new(observer)));
    });
    let _reset = Reset;
    action()
}

struct Proof {
    path: PathBuf,
    stamp: SourceStamp,
    file: Option<File>,
}

#[derive(Default)]
pub(crate) struct SqliteBatch {
    copies: Mutex<BTreeMap<PathBuf, Arc<SqliteCopy>>>,
    copied_bytes: AtomicU64,
}

#[cfg(test)]
impl SqliteBatch {
    pub(crate) fn copied_bytes(&self) -> u64 {
        self.copied_bytes.load(Ordering::Relaxed)
    }
    pub(crate) fn copy_paths(&self) -> Vec<PathBuf> {
        self.copies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .map(|copy| copy.path.clone())
            .collect()
    }
}

struct SqliteCopy {
    _directory: tempfile::TempDir,
    path: PathBuf,
    sources: Vec<Proof>,
    absent: Vec<PathBuf>,
}

impl SqliteCopy {
    fn attach(&self, context: &ReadContext) -> Result<(), String> {
        context.check()?;
        let mut sources = Vec::new();
        for proof in &self.sources {
            context.check()?;
            context.validate(&proof.path)?;
            context.verify_expected(&proof.path, &proof.stamp)?;
            proof
                .stamp
                .verify(&proof.path)
                .map_err(|_| context.fail("changed"))?;
            let file = proof.file.as_ref().ok_or_else(|| context.fail("changed"))?;
            if !SourceStamp::of(file).is_ok_and(|stamp| stamp == proof.stamp) {
                return Err(context.fail("changed"));
            }
            sources.push(Proof {
                path: proof.path.clone(),
                stamp: proof.stamp.clone(),
                file: Some(
                    file.try_clone()
                        .map_err(|_| context.fail("sourceUnavailable"))?,
                ),
            });
        }
        for path in &self.absent {
            context.check()?;
            if !matches!(fs::symlink_metadata(path),Err(error) if error.kind() == io::ErrorKind::NotFound)
            {
                return Err(context.fail("changed"));
            }
        }
        context
            .proofs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .extend(sources);
        context
            .absent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .extend(self.absent.clone());
        Ok(())
    }
}

pub(crate) struct ReadContext {
    pub cancel: Arc<AtomicBool>,
    batch: Arc<AtomicU64>,
    deadline: Instant,
    roots: Vec<PathBuf>,
    bytes: AtomicU64,
    raw_bytes: AtomicU64,
    records: AtomicU64,
    sqlite_batch: Arc<SqliteBatch>,
    session_limit: u64,
    line_limit: usize,
    failure: Mutex<Option<String>>,
    expected: Mutex<Option<(PathBuf, SourceStamp)>>,
    proofs: Mutex<Vec<Proof>>,
    absent: Mutex<Vec<PathBuf>>,
}

impl ReadContext {
    pub fn new(
        cancel: Arc<AtomicBool>,
        batch: Arc<AtomicU64>,
        deadline: Instant,
        roots: Vec<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            cancel,
            batch,
            deadline,
            roots,
            bytes: AtomicU64::new(0),
            raw_bytes: AtomicU64::new(0),
            records: AtomicU64::new(0),
            sqlite_batch: Arc::new(SqliteBatch::default()),
            session_limit: SESSION_BYTES,
            line_limit: LINE_BYTES,
            failure: Mutex::new(None),
            expected: Mutex::new(None),
            proofs: Mutex::new(Vec::new()),
            absent: Mutex::new(Vec::new()),
        })
    }

    pub fn with_sqlite_batch(
        cancel: Arc<AtomicBool>,
        batch: Arc<AtomicU64>,
        deadline: Instant,
        roots: Vec<PathBuf>,
        sqlite_batch: Arc<SqliteBatch>,
    ) -> Arc<Self> {
        let mut context = Self::new(cancel, batch, deadline, roots);
        Arc::get_mut(&mut context).unwrap().sqlite_batch = sqlite_batch;
        context
    }

    /// Bind the eventual file handle (including a SQLite snapshot's source)
    /// to the candidate issued before the read starts.
    pub fn expect_source(&self, path: PathBuf, stamp: SourceStamp) {
        self.expected
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace((path, stamp));
    }

    fn verify_expected(&self, path: &Path, stamp: &SourceStamp) -> Result<(), String> {
        if self
            .expected
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .is_some_and(|(expected_path, expected_stamp)| {
                path == expected_path && stamp != expected_stamp
            })
        {
            return Err(self.fail("changed"));
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn with_session_limit(mut self: Arc<Self>, bytes: u64) -> Arc<Self> {
        Arc::get_mut(&mut self).unwrap().session_limit = bytes;
        self
    }

    #[cfg(test)]
    pub fn limited(roots: Vec<PathBuf>, bytes: u64, line: usize) -> Arc<Self> {
        let mut context = Self::new(
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU64::new(0)),
            Instant::now() + BATCH_DURATION,
            roots,
        );
        let context_mut = Arc::get_mut(&mut context).unwrap();
        context_mut.session_limit = bytes;
        context_mut.line_limit = line;
        context
    }

    #[cfg(test)]
    pub fn retained_file_handles(&self) -> usize {
        self.proofs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|proof| proof.file.is_some())
            .count()
    }

    fn fail(&self, code: &str) -> String {
        let mut failure = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        failure
            .get_or_insert_with(|| format!("session.search.{code}"))
            .clone()
    }

    pub fn check(&self) -> Result<(), String> {
        if let Some(error) = self
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            return Err(error.clone());
        }
        if self.cancel.load(Ordering::Acquire) {
            return Err(self.fail("cancelled"));
        }
        if Instant::now() >= self.deadline {
            return Err(self.fail("deadline"));
        }
        Ok(())
    }

    fn charge(&self, bytes: usize) -> Result<(), String> {
        self.check()?;
        let bytes = bytes as u64;
        if self
            .bytes
            .fetch_add(bytes, Ordering::Relaxed)
            .saturating_add(bytes)
            > self.session_limit
        {
            return Err(self.fail("sessionLimit"));
        }
        if self
            .batch
            .fetch_add(bytes, Ordering::Relaxed)
            .saturating_add(bytes)
            > BATCH_BYTES
        {
            return Err(self.fail("batchLimit"));
        }
        Ok(())
    }

    fn charge_raw(&self, bytes: usize, sqlite: bool) -> Result<(), String> {
        self.check()?;
        let bytes = bytes as u64;
        if sqlite {
            if self
                .sqlite_batch
                .copied_bytes
                .fetch_add(bytes, Ordering::Relaxed)
                .saturating_add(bytes)
                > MAX_SQLITE_BATCH_IO
            {
                return Err(self.fail("batchLimit"));
            }
        } else {
            if self
                .raw_bytes
                .fetch_add(bytes, Ordering::Relaxed)
                .saturating_add(bytes)
                > self.session_limit
            {
                return Err(self.fail("sessionLimit"));
            }
            if self
                .batch
                .fetch_add(bytes, Ordering::Relaxed)
                .saturating_add(bytes)
                > BATCH_BYTES
            {
                return Err(self.fail("batchLimit"));
            }
        }
        Ok(())
    }

    fn sqlite_copy(self: &Arc<Self>, path: &Path) -> Result<Arc<SqliteCopy>, String> {
        let cached = self
            .sqlite_batch
            .copies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(path)
            .cloned();
        if let Some(copy) = cached {
            copy.attach(self)?;
            return Ok(copy);
        }
        self.check()?;
        let proof_start = self
            .proofs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len();
        let absent_start = self
            .absent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len();
        let (main, _) = self.observe_file(path)?;
        let main_size = main.metadata().map_err(|_| self.fail("changed"))?.len();
        let mut wal = None;
        for suffix in ["-wal", "-journal"] {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            let sidecar = PathBuf::from(name);
            match fs::symlink_metadata(&sidecar) {
                Ok(_) if suffix == "-journal" => return Err(self.fail("changed")),
                Ok(_) => {
                    let (file, _) = self.observe_file(&sidecar)?;
                    wal = Some(file);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => self
                    .absent
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(sidecar),
                Err(_) => return Err(self.fail("changed")),
            }
        }
        let wal_size = wal
            .as_ref()
            .map(|file| file.metadata().map(|metadata| metadata.len()))
            .transpose()
            .map_err(|_| self.fail("changed"))?
            .unwrap_or(0);
        if main_size
            .checked_add(wal_size)
            .is_none_or(|size| size > MAX_SQLITE_SOURCE_BYTES)
        {
            return Err(self.fail("databaseLimit"));
        }
        let directory = tempfile::tempdir().map_err(|_| self.fail("sourceUnavailable"))?;
        let copy_path = directory.path().join("snapshot.db");
        let copy_file = |file: File, size: u64, destination: &Path| -> Result<(), String> {
            let input = RawInput {
                file,
                context: self.clone(),
                sqlite: true,
            };
            let mut target =
                File::create(destination).map_err(|_| self.fail("sourceUnavailable"))?;
            let copied = io::copy(&mut input.take(size + 1), &mut target)
                .map_err(|_| self.fail("sourceUnavailable"))?;
            target.flush().map_err(|_| self.fail("sourceUnavailable"))?;
            if copied != size {
                return Err(self.fail("changed"));
            }
            Ok(())
        };
        copy_file(main, main_size, &copy_path)?;
        if let Some(wal) = wal {
            copy_file(wal, wal_size, &directory.path().join("snapshot.db-wal"))?;
        }
        self.verify()?;
        let sources = self
            .proofs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .split_off(proof_start);
        let absent = self
            .absent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .split_off(absent_start);
        let copy = Arc::new(SqliteCopy {
            _directory: directory,
            path: copy_path,
            sources,
            absent,
        });
        copy.attach(self)?;
        self.sqlite_batch
            .copies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(path.to_owned(), copy.clone());
        Ok(copy)
    }

    fn validate(&self, path: &Path) -> Result<(), String> {
        managed::validate_path(path, &self.roots).map_err(|_| self.fail("changed"))
    }

    fn observe_file(&self, path: &Path) -> Result<(File, usize), String> {
        self.check()?;
        self.validate(path)?;
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW);
        }
        let file = options.open(path).map_err(|_| self.fail("changed"))?;
        if !file.metadata().map_err(|_| self.fail("changed"))?.is_file() {
            return Err(self.fail("changed"));
        }
        let stamp = SourceStamp::of(&file).map_err(|_| self.fail("changed"))?;
        self.verify_expected(path, &stamp)?;
        stamp.verify(path).map_err(|_| self.fail("changed"))?;
        let held = file.try_clone().map_err(|_| self.fail("changed"))?;
        let mut proofs = self.proofs.lock().unwrap_or_else(|e| e.into_inner());
        if proofs.len() >= 20_000 {
            return Err(self.fail("sessionLimit"));
        }
        let index = proofs.len();
        proofs.push(Proof {
            path: path.to_owned(),
            stamp,
            file: Some(held),
        });
        Ok((file, index))
    }

    pub fn verify(&self) -> Result<String, String> {
        self.check()?;
        let proofs = self.proofs.lock().unwrap_or_else(|e| e.into_inner());
        let mut hash = Sha256::new();
        hash.update(b"ofox-transcript-v1\0");
        let mut ordered: Vec<_> = proofs.iter().collect();
        ordered.sort_by(|left, right| left.path.cmp(&right.path));
        for (index, proof) in ordered.into_iter().enumerate() {
            #[cfg(test)]
            VERIFY_OBSERVER.with(|observer| {
                if let Some(observer) = observer.borrow().as_ref() {
                    observer(index);
                }
            });
            #[cfg(not(test))]
            let _ = index;
            self.check()?;
            self.validate(&proof.path)?;
            self.verify_expected(&proof.path, &proof.stamp)?;
            proof
                .stamp
                .verify(&proof.path)
                .map_err(|_| self.fail("changed"))?;
            if let Some(file) = &proof.file {
                if SourceStamp::of(file).map_err(|_| self.fail("changed"))? != proof.stamp {
                    return Err(self.fail("changed"));
                }
            }
            hash.update(proof.path.to_string_lossy().as_bytes());
            hash.update([0]);
            hash.update(serde_json::to_vec(&proof.stamp).map_err(|_| self.fail("changed"))?);
        }
        let mut absent = self
            .absent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        absent.sort();
        for path in &absent {
            self.check()?;
            if !matches!(fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
            {
                return Err(self.fail("changed"));
            }
            hash.update(path.to_string_lossy().as_bytes());
        }
        Ok(format!("{:x}", hash.finalize()))
    }
}

pub(crate) fn scope<T>(
    context: Arc<ReadContext>,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    struct Restore(Option<Arc<ReadContext>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|current| *current.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(CURRENT.with(|current| current.replace(Some(context.clone()))));
    context.check()?;
    let result = action();
    context.check()?;
    result
}

pub(crate) fn check() -> Result<(), String> {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .map_or(Ok(()), |context| context.check())
    })
}

pub(crate) fn charge_text(text: &str) -> Result<(), String> {
    CURRENT.with(|current| {
        if let Some(context) = current.borrow().as_ref() {
            if text.len() > context.line_limit {
                return Err(context.fail("lineLimit"));
            }
            context.charge(text.len())?;
            if context.records.fetch_add(1, Ordering::Relaxed) >= 100_000 {
                return Err(context.fail("sessionLimit"));
            }
        }
        Ok(())
    })
}

struct BoundedReader {
    inner: Box<dyn Read>,
    context: Arc<ReadContext>,
    line_bytes: usize,
    proof_index: usize,
}

/// Decoder input must be cancellable too: skippable/empty zstd frames can
/// consume arbitrary compressed input before producing one decoded byte.
struct RawInput {
    file: File,
    context: Arc<ReadContext>,
    sqlite: bool,
}
impl Read for RawInput {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.context.check().map_err(io::Error::other)?;
        let limit = bytes.len().min(64 * 1024);
        let read = self.file.read(&mut bytes[..limit])?;
        self.context
            .charge_raw(read, self.sqlite)
            .map_err(io::Error::other)?;
        Ok(read)
    }
}
impl Drop for BoundedReader {
    fn drop(&mut self) {
        let mut proofs = self
            .context
            .proofs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(proof) = proofs.get_mut(self.proof_index) {
            if let Some(file) = proof.file.take() {
                if !SourceStamp::of(&file).is_ok_and(|stamp| stamp == proof.stamp)
                    || proof.stamp.verify(&proof.path).is_err()
                {
                    self.context.fail("changed");
                }
            }
        }
    }
}
impl Read for BoundedReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.context.check().map_err(io::Error::other)?;
        let limit = bytes.len().min(64 * 1024);
        let read = self.inner.read(&mut bytes[..limit])?;
        self.context.charge(read).map_err(io::Error::other)?;
        for byte in &bytes[..read] {
            if *byte == b'\n' {
                self.line_bytes = 0;
                if self.context.records.fetch_add(1, Ordering::Relaxed) >= 100_000 {
                    return Err(io::Error::other(self.context.fail("sessionLimit")));
                }
            } else {
                self.line_bytes += 1;
                if self.line_bytes > self.context.line_limit {
                    return Err(io::Error::other(self.context.fail("lineLimit")));
                }
            }
        }
        Ok(read)
    }
}

pub(crate) fn open(path: &Path) -> Result<Box<dyn Read>, String> {
    let context = CURRENT.with(|current| current.borrow().clone());
    #[cfg(test)]
    OPEN_OBSERVER.with(|observer| {
        if let Some(observer) = observer.borrow().as_ref() {
            observer(path, context.is_some());
        }
    });
    let Some(context) = context else {
        return super::providers::dsh::open(path);
    };
    let (file, proof_index) = context.observe_file(path)?;
    let decoded: Box<dyn Read> = if path
        .extension()
        .is_some_and(|extension| extension == "zst" || extension == "zstd")
    {
        let raw = RawInput {
            file,
            context: context.clone(),
            sqlite: false,
        };
        let mut decoder =
            zstd::stream::read::Decoder::new(raw).map_err(|_| context.fail("sourceUnavailable"))?;
        decoder
            .window_log_max(23)
            .map_err(|_| context.fail("sourceUnavailable"))?;
        Box::new(decoder)
    } else {
        Box::new(file)
    };
    Ok(Box::new(BoundedReader {
        inner: decoded,
        context,
        line_bytes: 0,
        proof_index,
    }))
}

pub(crate) fn read_text(path: &Path) -> Result<String, String> {
    let mut text = String::new();
    open(path)?
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    check()?;
    Ok(text)
}

pub(crate) fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    open(path)?
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    check()?;
    Ok(bytes)
}

pub(crate) fn source_unavailable() {
    CURRENT.with(|current| {
        if let Some(context) = current.borrow().as_ref() {
            context.fail("sourceUnavailable");
        }
    });
}

pub(crate) fn file_count(count: usize) -> Result<(), String> {
    CURRENT.with(|current| {
        if let Some(context) = current.borrow().as_ref() {
            if count >= 20_000 {
                return Err(context.fail("sessionLimit"));
            }
        }
        Ok(())
    })
}

pub(crate) fn directory(path: &Path) -> Result<(), String> {
    CURRENT.with(|current| {
        if let Some(context) = current.borrow().as_ref() {
            context.check()?;
            context.validate(path)?;
            let stamp = SourceStamp::at(path).map_err(|_| context.fail("changed"))?;
            let mut proofs = context.proofs.lock().unwrap_or_else(|e| e.into_inner());
            if proofs.len() >= 20_000 {
                return Err(context.fail("sessionLimit"));
            }
            proofs.push(Proof {
                path: path.to_owned(),
                stamp,
                file: None,
            });
        }
        Ok(())
    })
}

pub(crate) struct SqliteRead {
    connection: rusqlite::Connection,
    _snapshot: Option<Arc<SqliteCopy>>,
}
impl std::ops::Deref for SqliteRead {
    type Target = rusqlite::Connection;
    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

fn configure_sqlite(
    conn: &rusqlite::Connection,
    context: Option<Arc<ReadContext>>,
) -> Result<(), String> {
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(|_| "session.search.sourceBusy".to_owned())?;
    conn.pragma_update(None, "query_only", true)
        .map_err(|_| "session.search.sourceBusy".to_owned())?;
    if let Some(context) = context {
        conn.progress_handler(1000, Some(move || context.check().is_err()));
    }
    conn.execute_batch("BEGIN DEFERRED")
        .map_err(|_| "session.search.sourceBusy".to_owned())?;
    Ok(())
}

/// Capture main/WAL handles before opening SQLite. Queries run only against a
/// private fixed copy, so an atomic replacement cannot bind proof B to inode A.
pub(crate) fn open_sqlite(path: &Path) -> Result<SqliteRead, String> {
    let context = CURRENT.with(|current| current.borrow().clone());
    #[cfg(test)]
    OPEN_OBSERVER.with(|observer| {
        if let Some(observer) = observer.borrow().as_ref() {
            observer(path, context.is_some());
        }
    });
    let snapshot = context
        .as_ref()
        .map(|context| context.sqlite_copy(path))
        .transpose()?;
    let target = snapshot
        .as_ref()
        .map_or(path, |snapshot| snapshot.path.as_path());
    let connection = rusqlite::Connection::open_with_flags(
        target,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "session.search.sourceUnavailable".to_owned())?;
    configure_sqlite(&connection, context)?;
    Ok(SqliteRead {
        connection,
        _snapshot: snapshot,
    })
}

#[cfg(all(test, unix))]
pub(crate) fn sqlite(_path: &Path, conn: &rusqlite::Connection) -> Result<(), String> {
    // An externally opened connection has no provable relationship to a later
    // pathname observation. Only open_sqlite can establish the source receipt.
    if CURRENT.with(|current| current.borrow().is_some()) {
        return Err("session.search.changed".into());
    }
    configure_sqlite(conn, None)
}

pub(crate) fn column(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<String> {
    let text = row
        .get_ref(index)?
        .as_str()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    charge_text(text).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(text.to_owned())
}

/// The same native JSONL projection contract used by WorkBuddy's reader, with
/// cooperative decoded-byte limits instead of a second unbounded file handle.
pub(crate) fn json_lines(
    path: &Path,
    wanted: impl Fn(&str) -> bool,
    mut accept: impl FnMut(i64, serde_json::Value) -> Result<(), String>,
) -> Result<crate::services::usage_source::JsonlProgress, String> {
    if CURRENT.with(|current| current.borrow().is_none()) {
        return crate::services::usage_source::json_lines(path, wanted, |offset, value| {
            accept(offset, value).map_err(crate::error::AppError::Config)
        })
        .map_err(|error| error.to_string());
    }
    use std::io::BufRead;
    let mut reader = io::BufReader::new(open(path)?);
    let mut offset = 0;
    let mut complete = true;
    let mut bytes = Vec::new();
    loop {
        check()?;
        bytes.clear();
        if reader
            .read_until(b'\n', &mut bytes)
            .map_err(|error| error.to_string())?
            == 0
        {
            break;
        }
        let terminated = bytes.last() == Some(&b'\n');
        if std::str::from_utf8(&bytes).is_ok_and(|line| line.trim().is_empty()) {
            offset += 1;
            continue;
        }
        let value = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|line| serde_json::from_str(line).ok());
        match value {
            Some(value) => {
                let line = std::str::from_utf8(&bytes)
                    .map_err(|_| "session.search.sourceUnavailable".to_owned())?;
                if wanted(line) {
                    accept(offset, value)?;
                }
            }
            None => {
                complete = false;
                if !terminated {
                    break;
                }
            }
        }
        offset += 1;
    }
    Ok(crate::services::usage_source::JsonlProgress { offset, complete })
}
