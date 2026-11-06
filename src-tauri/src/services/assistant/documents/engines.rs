//! The document engines bundled beside the runtime, found through the
//! `engines` map of its `manifest.json`.
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// Where each bundled engine is. `None` when the manifest does not list it
/// or its file is not there: the formats it makes are then not offered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Engines {
    pub officecli: Option<PathBuf>,
    pub typst: Option<PathBuf>,
}

/// Read `base/manifest.json`. A missing or unreadable manifest, field or
/// file is no engine, not an error.
pub fn engines_at(base: &Path) -> Engines {
    let manifest = std::fs::read(base.join("manifest.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
    let locate = |name: &str| {
        let relative = Path::new(manifest["engines"][name]["path"].as_str()?);
        // The manifest is ours, but a path that leaves `base` is no engine.
        let inside = relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)));
        let path = base.join(relative);
        (inside && path.is_file()).then_some(path)
    };
    Engines {
        officecli: locate("officecli"),
        typst: locate("typst"),
    }
}

impl Engines {
    /// The deliverable formats this machine can make. Text formats need no
    /// engine.
    pub fn formats(&self) -> Vec<&'static str> {
        let mut formats = vec!["md", "txt", "csv", "html", "json"];
        if self.officecli.is_some() {
            formats.extend(["pptx", "docx", "xlsx"]);
        }
        if self.typst.is_some() {
            formats.push("pdf");
        }
        formats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn runtime_with(manifest: serde_json::Value, files: &[&str]) -> tempfile::TempDir {
        let base = tempfile::tempdir().unwrap();
        std::fs::write(base.path().join("manifest.json"), manifest.to_string()).unwrap();
        for file in files {
            let path = base.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        base
    }

    #[test]
    fn engines_at_reads_paths_from_manifest() {
        let base = runtime_with(
            json!({"engines": {
                "officecli": {"path": "engines/officecli/officecli"},
                "typst": {"path": "engines/typst/typst"},
            }}),
            &["engines/officecli/officecli", "engines/typst/typst"],
        );
        let engines = engines_at(base.path());
        let officecli = engines.officecli.unwrap();
        let typst = engines.typst.unwrap();
        assert!(officecli.is_absolute() && typst.is_absolute());
        assert_eq!(officecli, base.path().join("engines/officecli/officecli"));
        assert_eq!(typst, base.path().join("engines/typst/typst"));
    }

    #[test]
    fn engines_at_ignores_missing_files_and_absent_field() {
        let base = runtime_with(
            json!({"engines": {"officecli": {"path": "engines/officecli/officecli"}}}),
            &[],
        );
        assert_eq!(engines_at(base.path()), Engines::default());
        let base = runtime_with(json!({"sdkVersion": "1.5.0"}), &[]);
        assert_eq!(engines_at(base.path()), Engines::default());
    }

    #[test]
    fn engines_at_survives_a_missing_or_broken_manifest() {
        let base = tempfile::tempdir().unwrap();
        assert_eq!(engines_at(base.path()), Engines::default());
        std::fs::write(base.path().join("manifest.json"), "{not json").unwrap();
        assert_eq!(engines_at(base.path()), Engines::default());
    }

    #[test]
    fn engines_at_stays_inside_the_runtime_folder() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("runtime");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(root.path().join("typst"), "").unwrap();
        let manifest = |path: &std::path::Path| {
            std::fs::write(
                base.join("manifest.json"),
                json!({"engines": {"typst": {"path": path}}}).to_string(),
            )
            .unwrap();
        };
        manifest(std::path::Path::new("../typst"));
        assert_eq!(engines_at(&base), Engines::default());
        manifest(&root.path().join("typst"));
        assert_eq!(engines_at(&base), Engines::default());
    }

    #[test]
    fn formats_depend_on_engines() {
        assert_eq!(
            Engines::default().formats(),
            ["md", "txt", "csv", "html", "json"]
        );
        let both = Engines {
            officecli: Some("officecli".into()),
            typst: Some("typst".into()),
        };
        assert_eq!(
            both.formats(),
            ["md", "txt", "csv", "html", "json", "pptx", "docx", "xlsx", "pdf"]
        );
        let office_only = Engines {
            officecli: Some("officecli".into()),
            typst: None,
        };
        assert!(!office_only.formats().contains(&"pdf"));
        assert!(office_only.formats().contains(&"docx"));
    }
}
