//! A cold runtime snapshot backed by one private, exclusively held file.
//! No capability-readable directory exists until these exact bytes are verified.
use super::*;
use std::io::{Seek, SeekFrom};
use windows::Win32::Storage::FileSystem::{DELETE, FILE_FLAG_DELETE_ON_CLOSE};

pub(super) struct RuntimeSnapshot {
    // Never reopened by name or exposed to a child. CREATE_NEW, an explicit
    // protected owner/System DACL and share mode zero prevent another writer
    // from changing the verified snapshot, even in a permissive parent.
    spool: File,
    manifest: Vec<(PathBuf, u64)>,
}

impl RuntimeSnapshot {
    fn create(parent: &Path, user: &str) -> Result<File, String> {
        let pins = pin_ancestors(parent)?;
        let physical_parent = final_path(pins.last().ok_or(UNSAFE_FILE)?)?;
        let path = parent.join(format!("snapshot-{}", uuid::Uuid::new_v4().simple()));
        let name = wide_path(&path)?;
        let descriptor = security_descriptor(&private_sddl(user))?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: BOOL(0),
        };
        let handle = unsafe {
            win(CreateFileW(
                PCWSTR(name.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0 | DELETE.0,
                Default::default(),
                Some(&attributes),
                CREATE_NEW,
                FILE_FLAG_DELETE_ON_CLOSE | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            ))?
        };
        let spool = unsafe { File::from_raw_handle(handle.0) };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            win(GetFileInformationByHandle(
                HANDLE(spool.as_raw_handle()),
                &mut info,
            ))?;
        }
        if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT).0 != 0
            || info.nNumberOfLinks != 1
            || final_path(&spool)?.parent() != Some(physical_parent.as_path())
        {
            return Err(UNSAFE_FILE.into());
        }
        Ok(spool)
    }

    pub(super) fn capture(
        source: &Path,
        parent: &Path,
        user: &str,
        expected_digest: &str,
        max: u64,
    ) -> Result<Self, String> {
        let mut snapshot = Self {
            spool: Self::create(parent, user)?,
            manifest: Vec::new(),
        };
        let captured = tree_digest_stream(
            source,
            max,
            Some((&mut snapshot.spool, &mut snapshot.manifest)),
        )?;
        // Check both the source read and the actual retained file. Failure
        // drops the original DELETE_ON_CLOSE handle without path cleanup.
        if captured != expected_digest || snapshot.digest()? != expected_digest {
            return Err(RUNTIME_CHANGED.into());
        }
        Ok(snapshot)
    }

    fn digest(&mut self) -> Result<String, String> {
        io(self.spool.seek(SeekFrom::Start(0)))?;
        let mut hash = Sha256::new();
        for (relative, length) in &self.manifest {
            let name = relative.to_string_lossy();
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update(length.to_le_bytes());
            Self::read_exact_chunks(&mut self.spool, *length, |chunk| {
                hash.update(chunk);
                Ok(())
            })?;
        }
        Self::eof(&mut self.spool)?;
        Ok(format!("{:x}", hash.finalize()))
    }

    fn read_exact_chunks(
        spool: &mut File,
        mut remaining: u64,
        mut consume: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut buffer = [0u8; 64 * 1024];
        while remaining != 0 {
            let wanted = remaining.min(buffer.len() as u64) as usize;
            let length = io(spool.read(&mut buffer[..wanted]))?;
            if length == 0 {
                return Err(UNSAFE_FILE.into());
            }
            consume(&buffer[..length])?;
            remaining -= length as u64;
        }
        Ok(())
    }

    fn eof(spool: &mut File) -> Result<(), String> {
        if io(spool.read(&mut [0u8; 1]))? != 0 {
            return Err(UNSAFE_FILE.into());
        }
        Ok(())
    }

    pub(super) fn write_to(&mut self, root: &Path) -> Result<(), String> {
        io(self.spool.seek(SeekFrom::Start(0)))?;
        for (relative, length) in &self.manifest {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                io(fs::create_dir_all(parent))?;
            }
            let mut destination = io(fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path))?;
            Self::read_exact_chunks(&mut self.spool, *length, |chunk| {
                io(destination.write_all(chunk))
            })?;
        }
        Self::eof(&mut self.spool)
    }
}

#[cfg(test)]
pub(super) mod tests;
