//! Host-derived file scope. Exact shell files never grant their parent folder.
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Inside,
    Outside,
    Forbidden,
}

#[derive(Clone, Debug)]
pub struct CheckedPath {
    pub path: PathBuf,
    pub access: Access,
}

#[derive(Clone, Debug)]
pub struct Scope {
    roots: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

impl Scope {
    pub fn new(home: PathBuf, roots: Vec<PathBuf>, files: Vec<PathBuf>) -> Result<Self, String> {
        resolve(&home)?;
        Ok(Self {
            roots: roots.iter().map(|p| resolve(p)).collect::<Result<_, _>>()?,
            files: files.iter().map(|p| resolve(p)).collect::<Result<_, _>>()?,
        })
    }

    /// Inside one of the folders, not only one of the exact files (shell profiles).
    pub fn in_root(&self, path: &Path) -> bool {
        self.roots.iter().any(|root| path.starts_with(root))
    }

    pub fn add_folder(&mut self, folder: &Path) -> Result<PathBuf, String> {
        let checked = self.check(folder)?;
        if checked.access == Access::Forbidden || !checked.path.is_dir() {
            return Err("assistant_path_denied".into());
        }
        if !self.roots.contains(&checked.path) {
            self.roots.push(checked.path.clone());
        }
        Ok(checked.path)
    }

    pub fn check(&self, path: &Path) -> Result<CheckedPath, String> {
        if sensitive_path(&path.to_string_lossy()) {
            return Ok(CheckedPath {
                path: path.into(),
                access: Access::Forbidden,
            });
        }
        let path = resolve(path)?;
        let access = if sensitive_path(&path.to_string_lossy()) {
            Access::Forbidden
        } else if self.roots.iter().any(|root| path.starts_with(root)) || self.files.contains(&path)
        {
            Access::Inside
        } else {
            Access::Outside
        };
        Ok(CheckedPath { path, access })
    }
}

/// Resolve existing ancestors as well as the leaf; missing leaves are needed
/// for diagnostics and later journaled writes. Relative and device paths fail.
pub fn resolve(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("assistant_invalid_path".into());
    }
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(mut real) => {
                #[cfg(windows)]
                {
                    let value = real.to_string_lossy();
                    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
                        real = PathBuf::from(format!(r"\\{}", rest));
                    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
                        real = PathBuf::from(rest);
                    }
                }
                for name in missing.into_iter().rev() {
                    real.push(name);
                }
                return Ok(real);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A dangling symlink is not a missing file we may create.
                if std::fs::symlink_metadata(ancestor).is_ok() {
                    return Err("assistant_path_denied".into());
                }
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or("assistant_invalid_path")?
                        .to_owned(),
                );
                ancestor = ancestor.parent().ok_or("assistant_invalid_path")?;
            }
            Err(_) => return Err("assistant_path_denied".into()),
        }
    }
}

/// Also applied to requested and resolved paths; Windows syntax is tested on
/// every host so drive streams and device namespaces cannot bypass the policy.
pub fn sensitive_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_lowercase();
    if normalized.starts_with("//./") || normalized.starts_with("//?/") || normalized.contains('\0')
    {
        return true;
    }
    if normalized
        .char_indices()
        .any(|(i, c)| c == ':' && !(i == 1 && normalized.as_bytes()[0].is_ascii_alphabetic()))
    {
        return true;
    }
    let parts: Vec<_> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    if parts
        .iter()
        .any(|p| matches!(*p, ".ssh" | ".gnupg" | "keychains" | ".mozilla" | "mozilla"))
    {
        return true;
    }
    let leaf = parts.last().copied().unwrap_or("");
    if [".pem", ".key", ".p12", ".pfx", ".keychain", ".keychain-db"]
        .iter()
        .any(|suffix| leaf.ends_with(suffix))
        || leaf.starts_with("id_rsa")
        || leaf.starts_with("id_ed25519")
        || leaf.starts_with("id_ecdsa")
    {
        return true;
    }
    [
        "/google/chrome/",
        "/chromium/",
        "/microsoft/edge/",
        "/bravesoftware/",
        "/library/safari/",
        "/opera software/",
        "/.config/google-chrome/",
    ]
    .iter()
    .any(|segment| format!("{normalized}/").contains(segment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "ofox-scope-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn local_scope_checks_components_and_denies_sensitive_paths_even_inside_allowed_roots() {
        let tmp = Temp::new();
        let home = tmp.0.join("home");
        fs::create_dir_all(home.join(".codex")).unwrap();
        let config = home.join(".codex");
        fs::write(config.join("config.toml"), "model='x'").unwrap();
        fs::create_dir_all(home.join(".codex-other")).unwrap();
        fs::write(home.join(".codex-other/file"), "outside").unwrap();
        let scope = Scope::new(
            home.clone(),
            vec![config.clone()],
            vec![home.join(".zshrc")],
        )
        .unwrap();
        assert_eq!(
            scope.check(&config.join("config.toml")).unwrap().access,
            Access::Inside
        );
        assert_eq!(
            scope.check(&home.join(".codex-other/file")).unwrap().access,
            Access::Outside
        );
        for file in [
            ".ssh/id_ed25519",
            ".gnupg/private-keys-v1.d/key",
            "Library/Keychains/login.keychain-db",
            "Library/Application Support/Google/Chrome/Default/Cookies",
            "AppData/Local/Microsoft/Edge/User Data/Default/Login Data",
            ".codex/private.pem",
        ] {
            let path = home.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "sensitive").unwrap();
            let all = Scope::new(home.clone(), vec![home.clone()], vec![]).unwrap();
            assert_eq!(
                all.check(&path).unwrap().access,
                Access::Forbidden,
                "{file}"
            );
        }
        assert!(scope.check(std::path::Path::new("relative")).is_err());
    }

    #[test]
    fn local_scope_exact_shell_files_do_not_grant_the_home_directory() {
        let tmp = Temp::new();
        let home = tmp.0.clone();
        fs::write(home.join(".zshrc"), "export PATH=...\n").unwrap();
        fs::write(home.join("other"), "private").unwrap();
        let scope = Scope::new(home.clone(), vec![], vec![home.join(".zshrc")]).unwrap();
        assert_eq!(
            scope.check(&home.join(".zshrc")).unwrap().access,
            Access::Inside
        );
        assert_eq!(
            scope.check(&home.join("other")).unwrap().access,
            Access::Outside
        );
        assert_eq!(
            scope.check(&home.join("new-file")).unwrap().access,
            Access::Outside
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_scope_resolves_symlink_escape_and_freezes_allowed_directory_identity() {
        let tmp = Temp::new();
        let home = tmp.0.clone();
        let config = home.join("config");
        fs::create_dir(&config).unwrap();
        fs::write(home.join("private"), "outside").unwrap();
        std::os::unix::fs::symlink(home.join("private"), config.join("link")).unwrap();
        let scope = Scope::new(home.clone(), vec![config.clone()], vec![]).unwrap();
        assert_eq!(
            scope.check(&config.join("link")).unwrap().access,
            Access::Outside
        );
        fs::create_dir_all(home.join(".ssh")).unwrap();
        fs::write(home.join(".ssh/key"), "secret").unwrap();
        std::os::unix::fs::symlink(home.join(".ssh"), config.join("ssh")).unwrap();
        assert_eq!(
            scope.check(&config.join("ssh/key")).unwrap().access,
            Access::Forbidden
        );
    }

    #[test]
    fn local_scope_windows_device_paths_streams_and_browser_profiles_are_denied_lexically() {
        for path in [
            r"C:\Users\A\.ssh\id_rsa",
            r"C:\Users\A\AppData\Local\Google\Chrome\User Data\Default\Cookies",
            r"\\.\PhysicalDrive0",
            r"C:\config.json:secret",
            r"C:\Users\A\key.PEM",
        ] {
            assert!(sensitive_path(path), "{path}");
        }
        assert!(!sensitive_path(r"C:\Users\A\.codex\config.toml"));
    }
}
