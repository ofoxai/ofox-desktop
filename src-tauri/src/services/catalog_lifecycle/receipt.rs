//! Machine-local evidence; an interrupted operation is never silently adopted.
use super::Failure;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Receipt {
    pub format: u8,
    pub operation: Option<String>,
    pub proof: Proof,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields, rename_all = "camelCase")]
pub(super) enum Proof {
    Missing,
    Empty { hash: String },
    Installed { hash: String, version: String },
}
pub(super) fn read(path: &Path) -> Result<Option<Receipt>, Failure> {
    read_with(path, || {})
}
fn read_with(path: &Path, after_check: impl FnOnce()) -> Result<Option<Receipt>, Failure> {
    let parent = path.parent().ok_or(Failure::InvalidPackage)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Failure::InvalidPackage)?;
    private_path(parent, name)?;
    #[cfg(unix)]
    let bytes = match crate::config::read_private_with(path, 65536, after_check)
        .map_err(|_| Failure::Unreadable)?
    {
        Some(bytes) => bytes,
        None => return Ok(None),
    };
    #[cfg(not(unix))]
    let bytes = {
        after_check();
        super::provenance::read_regular(path, 65536)?
    };
    let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|_| Failure::InvalidPackage)?;
    if receipt.format != 1
        || receipt
            .operation
            .as_ref()
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return Err(Failure::InvalidPackage);
    }
    Ok(Some(receipt))
}
pub(super) fn write(path: &Path, receipt: &Receipt) -> Result<(), Failure> {
    write_with(path, receipt, || {})
}
fn write_with(path: &Path, receipt: &Receipt, after_check: impl FnOnce()) -> Result<(), Failure> {
    let parent = path.parent().ok_or(Failure::InvalidPackage)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Failure::InvalidPackage)?;
    let checked = private_path(parent, name)?;
    if fs::symlink_metadata(&checked)
        .is_ok_and(|meta| !meta.is_file() || meta.file_type().is_symlink())
    {
        return Err(Failure::InvalidPackage);
    }
    let bytes = serde_json::to_vec(receipt).map_err(|_| Failure::InvalidPackage)?;
    #[cfg(unix)]
    return crate::config::atomic_write_private_with(&checked, &bytes, after_check)
        .map_err(|_| Failure::Unreadable);
    #[cfg(not(unix))]
    {
        let _ = (checked, bytes, after_check);
        Err(Failure::InvalidPackage)
    }
}
/// Existing components must be physical directories. Does not create anything.
pub(super) fn private_path(root: &Path, leaf: &str) -> Result<PathBuf, Failure> {
    use std::path::Component;
    if !root.is_absolute()
        || Path::new(leaf)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(Failure::InvalidPackage);
    }
    let path = root.join(leaf);
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(Failure::InvalidPackage),
            Ok(meta) if current != path && !meta.is_dir() => return Err(Failure::InvalidPackage),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Failure::Unreadable),
        }
    }
    Ok(path)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    #[test]
    fn replacing_a_checked_parent_with_an_external_link_cannot_supply_a_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(root.join("managed")).unwrap();
        fs::create_dir(root.join("outside")).unwrap();
        fs::write(
            root.join("outside/receipt.json"),
            r#"{"format":1,"operation":null,"proof":{"kind":"missing"}}"#,
        )
        .unwrap();
        assert!(read_with(&root.join("managed/receipt.json"), || {
            fs::rename(root.join("managed"), root.join("saved")).unwrap();
            symlink(root.join("outside"), root.join("managed")).unwrap();
        })
        .is_err());
    }
    #[test]
    fn reading_existing_or_dangling_parent_link_is_not_missing_or_owned() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(root.join("outside")).unwrap();
        fs::write(
            root.join("outside/receipt.json"),
            r#"{"format":1,"operation":null,"proof":{"kind":"missing"}}"#,
        )
        .unwrap();
        symlink(root.join("outside"), root.join("alias")).unwrap();
        assert!(read(&root.join("alias/receipt.json")).is_err());
        fs::remove_file(root.join("alias")).unwrap();
        symlink(root.join("nonexistent"), root.join("alias")).unwrap();
        assert!(read(&root.join("alias/receipt.json")).is_err());
    }
    #[test]
    fn link_inserted_after_write_check_cannot_overwrite_an_outside_user_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let target = root.join("outside.txt");
        let path = root.join("receipt.json");
        fs::write(&target, "keep user bytes").unwrap();
        let result = write_with(
            &path,
            &Receipt {
                format: 1,
                operation: None,
                proof: Proof::Missing,
            },
            || symlink(&target, &path).unwrap(),
        );
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "keep user bytes");
    }
    #[test]
    fn receipt_roundtrip_preserves_intent_and_never_defaults_corruption() {
        let home = tempfile::tempdir().unwrap();
        let path = fs::canonicalize(home.path()).unwrap().join("receipt.json");
        let receipt = Receipt {
            format: 1,
            operation: Some(uuid::Uuid::new_v4().to_string()),
            proof: Proof::Missing,
        };
        assert_eq!(read(&path).unwrap(), None);
        write(&path, &receipt).unwrap();
        assert_eq!(read(&path).unwrap(), Some(receipt));
        for body in [
            "{",
            r#"{"format":2,"operation":null,"proof":{"kind":"missing"}}"#,
            r#"{"format":1,"operation":null,"proof":{"kind":"missing"},"unknown":true}"#,
        ] {
            fs::write(&path, body).unwrap();
            assert!(read(&path).is_err());
        }
    }
    #[test]
    fn receipt_and_private_paths_reject_links_and_parent_traversal() {
        let home = tempfile::tempdir().unwrap();
        let root_home = fs::canonicalize(home.path()).unwrap();
        let target = root_home.join("user-file");
        fs::write(&target, "keep user data").unwrap();
        let path = root_home.join("receipt.json");
        symlink(&target, &path).unwrap();
        let receipt = Receipt {
            format: 1,
            operation: None,
            proof: Proof::Missing,
        };
        assert!(read(&path).is_err());
        assert!(write(&path, &receipt).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep user data");
        let root = root_home.join("catalog");
        fs::create_dir(&root).unwrap();
        symlink(home.path(), root.join("escape")).unwrap();
        assert!(private_path(&root, "escape/target").is_err());
        assert!(private_path(&root, "../outside").is_err());
        assert_eq!(
            private_path(&root, "copilot/prefix").unwrap(),
            root.join("copilot/prefix")
        );
    }
}
