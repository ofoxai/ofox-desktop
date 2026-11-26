use super::*;

fn fixture(root: &Path) -> PathBuf {
    let source = root.join("source");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/中文.txt"), b"snapshot fixture").unwrap();
    fs::write(source.join("empty"), b"").unwrap();
    source
}

#[test]
fn cold_snapshot_preserves_digest_and_prevents_replacement_until_drop() {
    let temporary = tempfile::tempdir().unwrap();
    let source = fixture(temporary.path());
    let expected = tree_digest(&files(&source, 1024).unwrap());
    let mut snapshot = RuntimeSnapshot::capture(
        &source,
        temporary.path(),
        &current_user().unwrap(),
        &expected,
        1024,
    )
    .unwrap();
    assert_eq!(snapshot.digest().unwrap(), expected);
    let path = final_path(&snapshot.spool).unwrap();
    assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
    assert!(fs::remove_file(&path).is_err());
    assert!(fs::rename(&path, temporary.path().join("replacement")).is_err());
    let destination = temporary.path().join("destination");
    fs::create_dir(&destination).unwrap();
    // The immutable snapshot, rather than the now changed public files,
    // supplies the bytes admitted to capability-readable staging.
    fs::write(source.join("nested/中文.txt"), b"source changed").unwrap();
    snapshot.write_to(&destination).unwrap();
    assert_eq!(tree_digest_at(&destination, 1024).unwrap(), expected);
    assert!(
        snapshot.write_to(&destination).is_err(),
        "never replace staging files"
    );
    drop(snapshot);
    assert_eq!(
        fs::symlink_metadata(&path).unwrap_err().kind(),
        std::io::ErrorKind::NotFound,
        "the held delete-on-close object is removed"
    );
}

#[test]
fn cold_snapshot_rejects_short_and_trailing_bytes() {
    for short in [true, false] {
        let temporary = tempfile::tempdir().unwrap();
        let source = fixture(temporary.path());
        let expected = tree_digest_at(&source, 1024).unwrap();
        let mut snapshot = RuntimeSnapshot::capture(
            &source,
            temporary.path(),
            &current_user().unwrap(),
            &expected,
            1024,
        )
        .unwrap();
        if short {
            snapshot.spool.set_len(1).unwrap();
        } else {
            snapshot.spool.seek(SeekFrom::End(0)).unwrap();
            snapshot.spool.write_all(b"extra").unwrap();
        }
        assert!(snapshot.digest().is_err());
        let destination = temporary.path().join("destination");
        fs::create_dir(&destination).unwrap();
        assert!(snapshot.write_to(&destination).is_err());
        let path = final_path(&snapshot.spool).unwrap();
        drop(snapshot);
        assert_eq!(
            fs::symlink_metadata(&path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
}

#[test]
fn cold_snapshot_cleans_up_changed_source_and_limit_failures() {
    let temporary = tempfile::tempdir().unwrap();
    let source = fixture(temporary.path());
    let expected = tree_digest_at(&source, 1024).unwrap();
    let parent = temporary.path().join("private");
    fs::create_dir(&parent).unwrap();
    let user = current_user().unwrap();
    fs::write(source.join("nested/中文.txt"), b"changed").unwrap();
    assert!(matches!(
        RuntimeSnapshot::capture(&source, &parent, &user, &expected, 1024),
        Err(error) if error == RUNTIME_CHANGED
    ));
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
    let expected = tree_digest_at(&source, 1024).unwrap();
    assert!(matches!(
        RuntimeSnapshot::capture(&source, &parent, &user, &expected, 1),
        Err(error) if error == UNSAFE_FILE
    ));
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
    fs::hard_link(source.join("nested/中文.txt"), source.join("alias")).unwrap();
    assert!(matches!(
        RuntimeSnapshot::capture(&source, &parent, &user, &expected, 1024),
        Err(error) if error == UNSAFE_FILE
    ));
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
}

// Called while the existing native control owns a verified, never-resumed
// LPAC process. AccessCheck examines the actual descriptor, so a sharing
// violation cannot masquerade as proof that the spool DACL denies LPAC read.
pub(in super::super) fn assert_spool_lpac_denied(process: HANDLE) {
    let temporary = tempfile::tempdir().unwrap();
    let spool = RuntimeSnapshot::create(temporary.path(), &current_user().unwrap()).unwrap();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        GetSecurityInfo(
            HANDLE(spool.as_raw_handle()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            Some(&mut descriptor),
        )
        .ok()
        .unwrap();
    }
    let _allocation = Local(descriptor.0);
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY | TOKEN_DUPLICATE, &mut token).unwrap() };
    let token = handle(token);
    let duplicate = access_impersonation_token(raw(&token), &Observer::default()).unwrap();
    for access in [1u32, 2, 0x20] {
        let mapping = GENERIC_MAPPING {
            GenericRead: 1,
            GenericWrite: 2,
            GenericExecute: 0x20,
            GenericAll: 0x23,
        };
        let mut privileges = PRIVILEGE_SET::default();
        let mut privilege_bytes = size_of::<PRIVILEGE_SET>() as u32;
        let mut granted = 0;
        let mut allowed = BOOL(0);
        unsafe {
            AccessCheck(
                descriptor,
                raw(&duplicate),
                access,
                &mapping,
                Some(&mut privileges),
                &mut privilege_bytes,
                &mut granted,
                &mut allowed,
            )
            .unwrap();
        }
        assert!(
            !allowed.as_bool(),
            "LPAC must not access the private snapshot"
        );
        assert_eq!(granted, 0);
    }
}
