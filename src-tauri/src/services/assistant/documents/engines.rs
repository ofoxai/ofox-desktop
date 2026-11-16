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
    /// Always `None` off macOS: the Python engine runs only in
    /// `sandbox-exec` (see `documents::create`).
    pub python: Option<PythonEngine>,
}

/// The Python deck engine: the runtime's Node runs the runner script on the
/// Pyodide assets (`node <runner> --assets <assets> …`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonEngine {
    /// The manifest's `node`.
    pub node: PathBuf,
    pub runner: PathBuf,
    /// The folder of the manifest's `assets.json`, the runner's `--assets`.
    pub assets: PathBuf,
}

/// Read `base/manifest.json`. A missing or unreadable manifest, field or
/// file is no engine, not an error.
pub fn engines_at(base: &Path) -> Engines {
    let manifest = read_json(&base.join("manifest.json"));
    // The manifest is ours, but a path that leaves `base` is no engine.
    let file = |field: &Value| file_inside(base, field);
    let engine = |name: &str| &manifest["engines"][name];
    let python = || {
        let assets = file(&engine("python")["path"])?;
        let folder = assets.parent()?.to_path_buf();
        if !assets_staged(&assets, &folder) {
            return None;
        }
        Some(PythonEngine {
            node: file(&manifest["node"])?,
            runner: file(&engine("python")["runner"])?,
            assets: folder,
        })
    };
    Engines {
        officecli: file(&engine("officecli")["path"]),
        typst: file(&engine("typst")["path"]),
        python: if cfg!(target_os = "macos") {
            python()
        } else {
            None
        },
    }
}

/// The JSON in `path`; `Null` when it cannot be read or parsed.
fn read_json(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null)
}

/// `folder/<field>` when `field` is a relative path that stays inside
/// `folder` and names a file there.
fn file_inside(folder: &Path, field: &Value) -> Option<PathBuf> {
    let relative = Path::new(field.as_str()?);
    let inside = relative
        .components()
        .all(|part| matches!(part, Component::Normal(_)));
    let path = folder.join(relative);
    (inside && path.is_file()).then_some(path)
}

/// Whether every file the Python engine's `assets` (its `assets.json`)
/// lists under `files` (each wheel, the font, its licence) is a file in its
/// `folder`: a partial copy is no engine, rather than one that fails to load.
fn assets_staged(assets: &Path, folder: &Path) -> bool {
    read_json(assets)["files"]
        .as_array()
        .is_some_and(|files| files.iter().all(|f| file_inside(folder, f).is_some()))
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

    const PYTHON_FILES: [&str; 5] = [
        "node/node",
        "runtime/src/python/runner.mjs",
        "engines/python/assets.json",
        "engines/python/wheels/numpy.whl",
        "engines/python/fonts/NotoSansSC-Regular.otf",
    ];

    /// The staged files `assets.json` lists, in the engine's folder.
    const ASSETS: &str = r#"{"files": ["wheels/numpy.whl", "fonts/NotoSansSC-Regular.otf"]}"#;

    fn python_manifest(node: &str, runner: &str) -> serde_json::Value {
        json!({"node": node, "engines": {"python": {
            "path": "engines/python/assets.json",
            "runner": runner,
            "version": "314.0.7",
        }}})
    }

    /// [`runtime_with`] whose `assets.json`, when it is among `files`,
    /// holds `assets`.
    fn python_runtime(
        manifest: serde_json::Value,
        files: &[&str],
        assets: &str,
    ) -> tempfile::TempDir {
        let base = runtime_with(manifest, files);
        let path = base.path().join("engines/python/assets.json");
        if path.is_file() {
            std::fs::write(path, assets).unwrap();
        }
        base
    }

    #[test]
    fn engines_at_reads_the_python_engine() {
        let manifest = python_manifest("node/node", "runtime/src/python/runner.mjs");
        let base = python_runtime(manifest.clone(), &PYTHON_FILES, ASSETS);
        let python = engines_at(base.path()).python;
        if cfg!(target_os = "macos") {
            let python = python.unwrap();
            assert_eq!(python.node, base.path().join("node/node"));
            assert_eq!(
                python.runner,
                base.path().join("runtime/src/python/runner.mjs")
            );
            // The runner's `--assets`: the folder of `assets.json`.
            assert_eq!(python.assets, base.path().join("engines/python"));
        } else {
            // It runs only in macOS's sandbox-exec: elsewhere there is none.
            assert_eq!(python, None);
        }
        // Without any one of its files there is no engine: Node, the
        // runner, `assets.json`, or a staged file `assets.json` lists (an
        // interrupted or partial copy).
        for missing in PYTHON_FILES {
            let present: Vec<&str> = PYTHON_FILES
                .into_iter()
                .filter(|file| *file != missing)
                .collect();
            let base = python_runtime(manifest.clone(), &present, ASSETS);
            assert_eq!(engines_at(base.path()).python, None, "{missing}");
        }
        // Nor with a path that leaves the runtime folder.
        for (node, runner) in [
            ("../node/node", "runtime/src/python/runner.mjs"),
            ("node/node", "/runtime/src/python/runner.mjs"),
        ] {
            let base = python_runtime(python_manifest(node, runner), &PYTHON_FILES, ASSETS);
            assert_eq!(engines_at(base.path()).python, None, "{node} {runner}");
        }
        // Nor when `assets.json` lists no files, or a file outside the
        // engine's folder (which is there), or is no JSON.
        let mut outside = PYTHON_FILES.to_vec();
        outside.push("engines/numpy.whl");
        for assets in [
            "{}",
            r#"{"files": "wheels/numpy.whl"}"#,
            r#"{"files": ["wheels/numpy.whl", 1]}"#,
            r#"{"files": ["../numpy.whl"]}"#,
            r#"{"files": ["/etc/hosts"]}"#,
            r#"{"files": ["wheels"]}"#,
            "",
        ] {
            let base = python_runtime(manifest.clone(), &outside, assets);
            assert_eq!(engines_at(base.path()).python, None, "{assets}");
        }
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
            python: None,
        };
        assert_eq!(
            both.formats(),
            ["md", "txt", "csv", "html", "json", "pptx", "docx", "xlsx", "pdf"]
        );
        let office_only = Engines {
            officecli: Some("officecli".into()),
            typst: None,
            python: None,
        };
        assert!(!office_only.formats().contains(&"pdf"));
        assert!(office_only.formats().contains(&"docx"));
    }
}
