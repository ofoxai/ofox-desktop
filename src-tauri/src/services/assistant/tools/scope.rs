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
    protected: Vec<PathBuf>,
    readable: Vec<(PathBuf, PathBuf)>,
}

impl Scope {
    pub fn new(home: PathBuf, roots: Vec<PathBuf>, files: Vec<PathBuf>) -> Result<Self, String> {
        resolve(&home)?;
        Ok(Self {
            protected: vec![],
            readable: vec![],
            roots: roots.iter().map(|p| resolve(p)).collect::<Result<_, _>>()?,
            files: files.iter().map(|p| resolve(p)).collect::<Result<_, _>>()?,
        })
    }

    pub fn protect(
        &mut self,
        private: &Path,
        denied: &[PathBuf],
        readable: Vec<(PathBuf, PathBuf)>,
    ) -> Result<(), String> {
        self.protected = vec![private.into(), resolve(private)?];
        self.protected.extend_from_slice(denied);
        self.readable = readable;
        Ok(())
    }
    pub fn write_forbidden(&self, requested: &Path) -> bool {
        let real = resolve(requested).ok();
        self.protected
            .iter()
            .any(|p| requested.starts_with(p) || real.as_ref().is_some_and(|r| r.starts_with(p)))
    }
    pub fn protected_roots(&self) -> &[PathBuf] {
        &self.protected
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
        let requested = path;
        let path = resolve(path)?;
        let private = self.write_forbidden(requested);
        let permitted = self.readable.iter().any(|(target, real)| {
            path.starts_with(real)
                && (requested.starts_with(target)
                    || requested.starts_with(real)
                    || !self.protected.iter().any(|p| requested.starts_with(p)))
        });
        let access = if sensitive_path(&path.to_string_lossy()) || (private && !permitted) {
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

#[cfg(test)]
mod private_tests {
    use super::*;
    use crate::services::assistant::tools::{
        commands, files,
        policy::{self, Category, Decision, Operation, PermissionMode},
    };
    #[test]
    fn private_overlay_prunes_outside_traversal_and_never_grants_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let private = tmp.path().join("private");
        let skill = private.join("assistant/skills/one");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "needle").unwrap();
        std::fs::write(private.join("secret.txt"), "needle").unwrap();
        std::fs::write(tmp.path().join("public.txt"), "needle").unwrap();
        let mut scope = Scope::new(tmp.path().into(), vec![], vec![]).unwrap();
        scope
            .protect(
                &private,
                &[],
                vec![(skill.clone(), resolve(&skill).unwrap())],
            )
            .unwrap();
        assert_eq!(
            files::read(&scope, &skill.join("SKILL.md"), true).unwrap(),
            "needle"
        );
        assert!(files::read(&scope, &private.join("secret.txt"), true).is_err());
        let found = files::find(&scope, tmp.path(), "*.txt", 100, true).unwrap();
        assert_eq!(found.len(), 1);
        let matches = files::grep(&scope, tmp.path(), "needle", 100, true).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(files::list(&scope, tmp.path(), 100, true).unwrap().len(), 1);
        for mode in [
            PermissionMode::Default,
            PermissionMode::AutoEdit,
            PermissionMode::FullAuto,
        ] {
            for path in [
                skill.join("SKILL.md"),
                skill.join("new"),
                private.join("state.json"),
            ] {
                assert_eq!(
                    policy::decide(
                        true,
                        mode,
                        Operation {
                            category: Category::WriteConfig,
                            read_only_command: false,
                            forbidden: scope.write_forbidden(&path)
                        }
                    ),
                    Decision::Deny
                );
            }
        }
        for command in [
            "echo x > SKILL.md",
            "cp /tmp/x SKILL.md",
            "mv SKILL.md x",
            "rm SKILL.md",
            "Set-Content SKILL.md x",
            "Copy-Item /tmp/x SKILL.md",
            "Rename-Item SKILL.md x",
            "Remove-Item SKILL.md",
        ] {
            assert!(
                commands::protected_operation(command, &skill, tmp.path(), &scope),
                "{command}"
            );
        }
        assert!(commands::protected_operation(
            "rm -rf .",
            tmp.path(),
            tmp.path(),
            &scope
        ));
        assert!(!commands::protected_operation(
            "cat SKILL.md",
            &skill,
            tmp.path(),
            &scope
        ));
        assert!(!commands::protected_operation(
            "python3 script.py",
            &skill,
            tmp.path(),
            &scope
        ));
        assert!(commands::protected_operation(
            &format!("cat '{}'", private.join("secret.txt").display()),
            tmp.path(),
            tmp.path(),
            &scope
        ));
    }
    #[cfg(unix)]
    #[test]
    fn precise_artifact_exception_does_not_follow_replaced_or_escaping_links() {
        let tmp = tempfile::tempdir().unwrap();
        let private = tmp.path().join("private");
        let target = private.join("assistant/skills/one");
        let tree = private.join("library/artifacts/hash/tree");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("file"), "ok").unwrap();
        std::os::unix::fs::symlink(&tree, &target).unwrap();
        let mut scope = Scope::new(tmp.path().into(), vec![], vec![]).unwrap();
        scope
            .protect(
                &private,
                &[],
                vec![(target.clone(), resolve(&tree).unwrap())],
            )
            .unwrap();
        assert_eq!(
            files::read(&scope, &target.join("file"), true).unwrap(),
            "ok"
        );
        assert!(scope.write_forbidden(&target.join("new")));
        assert!(scope.write_forbidden(&tree.join("file")));
        assert_eq!(
            scope.check(tree.parent().unwrap()).unwrap().access,
            Access::Forbidden
        );
        std::os::unix::fs::symlink(tmp.path(), tree.join("escape")).unwrap();
        assert_eq!(
            scope.check(&target.join("escape")).unwrap().access,
            Access::Forbidden
        );
        std::fs::remove_file(&target).unwrap();
        std::os::unix::fs::symlink(tmp.path(), &target).unwrap();
        assert_eq!(
            scope.check(&target.join("public")).unwrap().access,
            Access::Forbidden
        );
    }
}

#[cfg(all(test, windows))]
mod windows_skill_paths {
    use super::*;
    #[test]
    fn trusted_canonical_identity_keeps_public_paths_ordinary() {
        let tmp = tempfile::tempdir().unwrap();
        let lexical = tmp.path().join("技能 with spaces");
        std::fs::create_dir(&lexical).unwrap();
        let canonical = std::fs::canonicalize(&lexical).unwrap();
        assert!(canonical.to_string_lossy().starts_with(r"\\?\"));
        assert_eq!(resolve(&canonical).unwrap(), resolve(&lexical).unwrap());
        let scope = Scope::new(tmp.path().into(), vec![], vec![]).unwrap();
        assert_eq!(scope.check(&canonical).unwrap().access, Access::Forbidden);
        assert_ne!(scope.check(&lexical).unwrap().access, Access::Forbidden);
    }
}
