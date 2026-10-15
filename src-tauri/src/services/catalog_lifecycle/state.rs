use super::{
    receipt::{self, Proof, Receipt},
    Failure,
};
use std::{fs, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Frozen {
    pub receipt: Option<Receipt>,
    pub proof: Proof,
}
pub(super) fn current(root: &Path) -> Result<Proof, Failure> {
    let prefix = receipt::private_path(root, "prefix")?;
    match fs::symlink_metadata(&prefix) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Proof::Missing),
        Err(_) => Err(Failure::Unreadable),
        Ok(_) => match super::provenance::inspect(&prefix) {
            Ok(identity) => Ok(Proof::Installed {
                hash: identity.hash,
                version: identity.version,
            }),
            Err(_) => super::provenance::empty_identity(&prefix).map(|hash| Proof::Empty { hash }),
        },
    }
}
pub(super) fn observe(root: &Path) -> Result<Frozen, Failure> {
    let record_path = receipt::private_path(root, "receipt.json")?;
    let receipt = receipt::read(&record_path)?;
    let proof = current(root)?;
    if receipt
        .as_ref()
        .is_some_and(|receipt| receipt.operation.is_some() || receipt.proof != proof)
        || receipt.is_none() && proof != Proof::Missing
    {
        return Err(Failure::Changed);
    }
    Ok(Frozen { receipt, proof })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn missing_is_installable_but_unowned_or_interrupted_prefix_is_never_adopted() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        assert_eq!(
            observe(&root).unwrap(),
            Frozen {
                receipt: None,
                proof: Proof::Missing
            }
        );
        fs::create_dir(root.join("prefix")).unwrap();
        assert!(observe(&root).is_err());
        fs::remove_dir(root.join("prefix")).unwrap();
        receipt::write(
            &root.join("receipt.json"),
            &Receipt {
                format: 1,
                operation: Some(uuid::Uuid::new_v4().to_string()),
                proof: Proof::Missing,
            },
        )
        .unwrap();
        assert!(observe(&root).is_err());
    }
    #[test]
    fn retained_empty_directories_have_physical_proof_but_never_authorize_unknown_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir_all(root.join("prefix/lib/node_modules/@github")).unwrap();
        let hash = super::super::provenance::empty_identity(&root.join("prefix")).unwrap();
        receipt::write(
            &root.join("receipt.json"),
            &Receipt {
                format: 1,
                operation: None,
                proof: Proof::Empty { hash },
            },
        )
        .unwrap();
        assert!(observe(&root).is_ok());
        fs::write(root.join("prefix/keep.txt"), "user content").unwrap();
        assert!(observe(&root).is_err());
        assert_eq!(
            fs::read_to_string(root.join("prefix/keep.txt")).unwrap(),
            "user content"
        );
    }
}
