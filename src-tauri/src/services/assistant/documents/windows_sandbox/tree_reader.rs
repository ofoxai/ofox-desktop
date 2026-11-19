//! Reuse directory pins within one complete tree read. No file result is cached.
use super::*;

pub(super) struct TreeReader {
    root: PathBuf,
    physical_root: PathBuf,
    roots: Vec<File>,
    chain: Vec<(PathBuf, File)>,
}

impl TreeReader {
    pub(super) fn new(root: &Path) -> Result<Self, String> {
        let roots = pin_ancestors(root)?;
        let physical_root = final_path(roots.last().ok_or(UNSAFE_FILE)?)?;
        Ok(Self {
            root: root.to_path_buf(),
            physical_root,
            roots,
            chain: Vec::new(),
        })
    }

    fn validate_pins(&self) -> Result<(), String> {
        // A pin denies replacement/rename, but its directory can be emptied
        // and converted to a reparse point while the same handle stays open.
        for file in self
            .roots
            .iter()
            .chain(self.chain.iter().map(|(_, file)| file))
        {
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            unsafe {
                win(GetFileInformationByHandle(
                    HANDLE(file.as_raw_handle()),
                    &mut info,
                ))?;
            }
            if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT).0
                != FILE_ATTRIBUTE_DIRECTORY.0
            {
                return Err(UNSAFE_FILE.into());
            }
        }
        Ok(())
    }

    pub(super) fn open(&mut self, path: &Path, max: u64) -> Result<File, String> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| UNSAFE_FILE.to_owned())?;
        let parts: Vec<_> = relative.components().collect();
        if parts.is_empty()
            || parts
                .iter()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(UNSAFE_FILE.into());
        }
        let mut cursor = self.root.clone();
        let wanted: Vec<_> = parts[..parts.len() - 1]
            .iter()
            .map(|part| {
                cursor.push(part.as_os_str());
                cursor.clone()
            })
            .collect();
        let shared = self
            .chain
            .iter()
            .zip(&wanted)
            .take_while(|((existing, _), next)| existing == *next)
            .count();
        self.chain.truncate(shared);
        self.validate_pins()?;
        for directory in &wanted[shared..] {
            let file = open_path(directory, true)?;
            if !final_path(&file)?.starts_with(&self.physical_root) {
                return Err(UNSAFE_FILE.into());
            }
            self.chain.push((directory.clone(), file));
        }
        let file = open_path(path, false)?;
        if !final_path(&file)?.starts_with(&self.physical_root) || io(file.metadata())?.len() > max
        {
            return Err(UNSAFE_FILE.into());
        }
        self.validate_pins()?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn junction(link: &Path, target: &Path) {
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "create the private junction fixture"
        );
    }

    fn set_existing_junction(link: &Path, target: &Path) {
        #[link(name = "kernel32")]
        extern "system" {
            fn DeviceIoControl(
                handle: HANDLE,
                code: u32,
                input: *const c_void,
                input_length: u32,
                output: *mut c_void,
                output_length: u32,
                returned: *mut u32,
                overlapped: *mut c_void,
            ) -> i32;
        }
        let target = target.canonicalize().unwrap();
        let target = target.to_str().expect("UTF-8 fixture path");
        let print = target.strip_prefix(r"\\?\").unwrap_or(target);
        let substitute: Vec<_> = format!(r"\??\{print}").encode_utf16().collect();
        let display: Vec<_> = print.encode_utf16().collect();
        let sub_len = u16::try_from(substitute.len() * 2).unwrap();
        let print_len = u16::try_from(display.len() * 2).unwrap();
        let payload_len = 8usize + usize::from(sub_len) + 2 + usize::from(print_len) + 2;
        assert!(
            payload_len + 8 <= 16 * 1024,
            "bounded fixture reparse buffer"
        );
        let mut buffer = Vec::new();
        buffer.extend(0xa000_0003u32.to_le_bytes()); // IO_REPARSE_TAG_MOUNT_POINT
        buffer.extend(u16::try_from(payload_len).unwrap().to_le_bytes());
        buffer.extend(0u16.to_le_bytes());
        for number in [0, sub_len, sub_len + 2, print_len] {
            buffer.extend(number.to_le_bytes());
        }
        for number in substitute.into_iter().chain([0]).chain(display).chain([0]) {
            buffer.extend(number.to_le_bytes());
        }
        let name = wide_path(link).unwrap();
        let owned = unsafe {
            handle(
                CreateFileW(
                    PCWSTR(name.as_ptr()),
                    FILE_GENERIC_WRITE.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                    None,
                )
                .unwrap(),
            )
        };
        let mut returned = 0;
        let completed = unsafe {
            DeviceIoControl(
                raw(&owned),
                0x0009_00a4, // FSCTL_SET_REPARSE_POINT
                buffer.as_ptr().cast(),
                u32::try_from(buffer.len()).unwrap(),
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(completed, 0, "convert the private empty fixture directory");
    }

    #[test]
    fn scoped_tree_reader_keeps_the_complete_byte_digest() {
        let fixture = tempfile::tempdir().unwrap();
        for index in 0..60 {
            let parent = fixture.path().join(format!("group-{}/nested", index / 10));
            fs::create_dir_all(&parent).unwrap();
            fs::write(
                parent.join(format!("file-{index}")),
                format!("fixture {index} 中文 日本語"),
            )
            .unwrap();
        }
        let expected = tree_digest(&files(fixture.path(), MAX_TREE_BYTES).unwrap());
        assert_eq!(
            tree_digest_at(fixture.path(), MAX_TREE_BYTES).unwrap(),
            expected
        );
    }

    #[test]
    fn scoped_tree_reader_reopens_changed_files_and_rechecks_their_limits() {
        let fixture = tempfile::tempdir().unwrap();
        let leaf = fixture.path().join("data");
        fs::write(&leaf, b"first").unwrap();
        let mut reader = TreeReader::new(fixture.path()).unwrap();
        assert!(reader.open(&leaf, 4).is_err());
        assert!(reader.open(fixture.path(), 128).is_err());
        let mut bytes = Vec::new();
        reader
            .open(&leaf, 128)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"first");
        fs::write(&leaf, b"second").unwrap();
        bytes.clear();
        reader
            .open(&leaf, 128)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"second");
        fs::hard_link(&leaf, fixture.path().join("alias")).unwrap();
        assert!(reader.open(&leaf, 128).is_err());
    }

    #[test]
    fn scoped_tree_reader_rejects_inside_outside_and_ancestor_junctions() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("root");
        let inside = root.join("inside");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(inside.join("sentinel"), b"inside").unwrap();
        fs::write(outside.join("sentinel"), b"outside").unwrap();
        for (name, target) in [("inside-link", &inside), ("outside-link", &outside)] {
            let link = root.join(name);
            junction(&link, target);
            assert!(TreeReader::new(&root)
                .unwrap()
                .open(&link.join("sentinel"), 128)
                .is_err());
        }
        let alias = fixture.path().join("ancestor-link");
        junction(&alias, &root);
        assert!(TreeReader::new(&alias).is_err());
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
    }

    #[test]
    fn scoped_tree_reader_pins_the_root_and_current_parent_until_drop() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("root");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let leaf = nested.join("data");
        fs::write(&leaf, b"fixture").unwrap();
        let moved_root = fixture.path().join("moved-root");
        let moved_parent = root.join("moved-parent");
        {
            let mut reader = TreeReader::new(&root).unwrap();
            drop(reader.open(&leaf, 128).unwrap());
            for (from, to) in [(&root, &moved_root), (&nested, &moved_parent)] {
                assert_eq!(fs::rename(from, to).unwrap_err().raw_os_error(), Some(32));
            }
        }
        fs::rename(&nested, &moved_parent).unwrap();
        fs::rename(&moved_parent, &nested).unwrap();
        fs::rename(&root, &moved_root).unwrap();
        fs::rename(&moved_root, &root).unwrap();
    }

    #[test]
    fn scoped_tree_reader_rejects_a_retained_directory_converted_to_a_junction() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("root");
        let changing = root.join("changing");
        let target = root.join("target");
        fs::create_dir_all(&changing).unwrap();
        fs::create_dir(&target).unwrap();
        let leaf = changing.join("data");
        fs::write(&leaf, b"fixture").unwrap();
        fs::write(target.join("sentinel"), b"inside target").unwrap();
        let mut reader = TreeReader::new(&root).unwrap();
        drop(reader.open(&leaf, 128).unwrap());
        fs::remove_file(&leaf).unwrap();
        set_existing_junction(&changing, &target);
        let aliased = changing.join("sentinel");
        assert!(open_regular(&root, &aliased, 128).is_err());
        assert!(reader.open(&aliased, 128).is_err());
        drop(reader);
        assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"inside target");
    }
}
