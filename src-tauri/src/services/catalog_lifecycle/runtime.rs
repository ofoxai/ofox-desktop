use super::{provenance, Failure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Runtime {
    pub node: PathBuf,
    pub npm: PathBuf,
    pub identity: String,
    pub engine: Option<String>,
}
impl Runtime {
    pub fn pair(node: &Path) -> Result<Self, Failure> {
        let node = fs::canonicalize(node).map_err(|_| Failure::Unreadable)?;
        if node.file_name() != Some(std::ffi::OsStr::new("node")) {
            return Err(Failure::InvalidPackage);
        }
        let base = node
            .parent()
            .and_then(Path::parent)
            .ok_or(Failure::InvalidPackage)?;
        let npm_root = base.join("lib/node_modules/npm");
        let npm = npm_root.join("bin/npm-cli.js");
        let (tree, manifest) = provenance::tree_identity(&npm_root)?;
        let manifest = manifest.ok_or(Failure::InvalidPackage)?;
        let builtin = npm_root.join("npmrc");
        match fs::symlink_metadata(&builtin) {
            Ok(_) => {
                let bytes = provenance::read_regular(&builtin, 65536)?;
                let text = std::str::from_utf8(&bytes).map_err(|_| Failure::InvalidPackage)?;
                if text.split(['\r', '\n']).any(|line| {
                    let line = line.trim();
                    !line.is_empty() && !line.starts_with('#') && !line.starts_with(';')
                }) {
                    return Err(Failure::InvalidPackage);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Failure::Unreadable),
        }
        if manifest["name"] != "npm"
            || manifest["bin"]["npm"] != "bin/npm-cli.js"
            || manifest["version"]
                .as_str()
                .and_then(|text| semver::Version::parse(text).ok())
                .is_none()
        {
            return Err(Failure::InvalidPackage);
        }
        provenance::read_regular(&npm, 1024 * 1024)?;
        let mut digest = Sha256::new();
        digest.update(provenance::file_identity(&node)?);
        digest.update(tree);
        Ok(Self {
            node,
            npm,
            identity: format!("{:x}", digest.finalize()),
            engine: manifest
                .get("engines")
                .and_then(|engines| engines.get("node"))
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .ok_or(Failure::InvalidPackage)
                })
                .transpose()?,
        })
    }
}
pub(super) fn parse_probe(runtime: &Runtime, bytes: &[u8]) -> Result<String, Failure> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| Failure::InvalidPackage)?;
    let version = value["version"].as_str().ok_or(Failure::InvalidPackage)?;
    if value["platform"] != "darwin"
        || value["arch"] != "arm64"
        || value["execPath"].as_str() != runtime.node.to_str()
        || semver::Version::parse(version)
            .ok()
            .is_none_or(|version| version.major < 22 || !version.pre.is_empty())
        || runtime.engine.as_ref().is_some_and(|engine| {
            crate::commands::node_requirement::engine_satisfied(version, engine) != Some(true)
        })
    {
        return Err(Failure::InvalidPackage);
    }
    Ok(version.into())
}
pub(super) fn environment(work: &Path, node: &Path) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for (key, value) in [
        ("HOME", work.to_path_buf()),
        ("TMPDIR", work.join("tmp")),
        ("NPM_CONFIG_USERCONFIG", work.join("user.npmrc")),
        ("NPM_CONFIG_GLOBALCONFIG", work.join("global.npmrc")),
        ("NPM_CONFIG_CACHE", work.join("cache")),
    ] {
        env.insert(key.into(), value.to_string_lossy().into_owned());
    }
    env.insert(
        "PATH".into(),
        format!(
            "{}:/usr/bin:/bin",
            node.parent().unwrap_or(Path::new("/usr/bin")).display()
        ),
    );
    env.insert("LANG".into(), "en_US.UTF-8".into());
    env.insert("CI".into(), "true".into());
    env
}

#[cfg(all(test, unix))]
pub(super) mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    #[test]
    fn does_not_inherit_authentication_from_a_paired_npm_builtin_config() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let node = fixture(&root);
        for config in [
            "//registry.npmjs.org/:_authToken=fixture-private-value\n",
            "# comment\r//registry.npmjs.org/:_authToken=fixture-private-value\r",
            "; comment\r//registry.npmjs.org/:_authToken=fixture-private-value\r",
            "# comment\r\n//registry.npmjs.org/:_authToken=fixture-private-value\n",
        ] {
            fs::write(root.join("lib/node_modules/npm/npmrc"), config).unwrap();
            assert!(Runtime::pair(&node).is_err(), "accepted configured npmrc");
        }
        fs::write(
            root.join("lib/node_modules/npm/npmrc"),
            "# comment\r; another comment\r\n\n",
        )
        .unwrap();
        assert!(Runtime::pair(&node).is_ok());
    }
    #[test]
    fn native_probe_requires_supported_arch_version_execpath_and_npm_engine() {
        let runtime = Runtime {
            node: PathBuf::from("/trusted/bin/node"),
            npm: PathBuf::new(),
            identity: String::new(),
            engine: Some("^20.17.0 || >=22.9.0".into()),
        };
        let good = serde_json::json!({"version":"22.12.0","platform":"darwin","arch":"arm64","execPath":"/trusted/bin/node"});
        assert_eq!(
            parse_probe(&runtime, good.to_string().as_bytes()).unwrap(),
            "22.12.0"
        );
        for (key, value) in [
            ("arch", "x64"),
            ("platform", "linux"),
            ("version", "20.19.0"),
            ("execPath", "/other/node"),
        ] {
            let mut bad = good.clone();
            bad[key] = value.into();
            assert!(parse_probe(&runtime, bad.to_string().as_bytes()).is_err());
        }
        let mut incompatible = runtime.clone();
        incompatible.engine = Some(">=24".into());
        assert!(parse_probe(&incompatible, good.to_string().as_bytes()).is_err());
        incompatible.engine = Some("unknown version constraint".into());
        assert!(parse_probe(&incompatible, good.to_string().as_bytes()).is_err());
    }
    pub(crate) fn fixture(root: &Path) -> PathBuf {
        fs::create_dir_all(root.join("bin")).unwrap();
        let npm = root.join("lib/node_modules/npm");
        fs::create_dir_all(npm.join("bin")).unwrap();
        fs::create_dir(npm.join("lib")).unwrap();
        fs::write(root.join("bin/node"), b"fixture node binary").unwrap();
        fs::write(
            npm.join("package.json"),
            br#"{"name":"npm","version":"10.9.0","bin":{"npm":"bin/npm-cli.js"}}"#,
        )
        .unwrap();
        fs::write(npm.join("bin/npm-cli.js"), "require('../lib/cli.js')").unwrap();
        fs::write(npm.join("lib/cli.js"), "// original dependency").unwrap();
        symlink(
            "../lib/node_modules/npm/bin/npm-cli.js",
            root.join("bin/npm"),
        )
        .unwrap();
        root.join("bin/node")
    }
    #[test]
    fn paired_runtime_freezes_entire_npm_dependency_tree_and_node() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let node = fixture(&root);
        let first = Runtime::pair(&node).unwrap();
        fs::write(
            root.join("lib/node_modules/npm/lib/cli.js"),
            "// changed dependency",
        )
        .unwrap();
        assert_ne!(Runtime::pair(&node).unwrap().identity, first.identity);
        fs::write(&node, b"changed node binary").unwrap();
        assert_ne!(Runtime::pair(&node).unwrap().identity, first.identity);
    }
    #[test]
    fn runtime_cannot_pair_foreign_npm_or_follow_external_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let node = fixture(&root);
        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join("dependency.js"), "outside").unwrap();
        symlink(
            other.path().join("dependency.js"),
            root.join("lib/node_modules/npm/lib/outside.js"),
        )
        .unwrap();
        assert!(Runtime::pair(&node).is_err());
    }
    #[test]
    fn npm_environment_is_closed_and_never_inherits_credentials_or_user_config() {
        let env = environment(Path::new("/private/job"), Path::new("/trusted/bin/node"));
        assert_eq!(env.get("HOME").map(String::as_str), Some("/private/job"));
        assert_eq!(
            env.get("PATH").map(String::as_str),
            Some("/trusted/bin:/usr/bin:/bin")
        );
        assert_eq!(
            env.get("NPM_CONFIG_USERCONFIG").map(String::as_str),
            Some("/private/job/user.npmrc")
        );
        assert_eq!(
            env.get("NPM_CONFIG_GLOBALCONFIG").map(String::as_str),
            Some("/private/job/global.npmrc")
        );
        assert!(!env.contains_key("NODE_OPTIONS"));
        assert!(!env.contains_key("NPM_TOKEN"));
    }
}
