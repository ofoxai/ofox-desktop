//! Tauri merges `tauri.windows.conf.json` with JSON Merge Patch, which replaces
//! arrays wholesale: the Windows `main` window entry must repeat every field of
//! the base entry, or Windows silently falls back to Tauri defaults (800×600,
//! not centered).

use serde_json::{Map, Value};
use std::{collections::BTreeSet, path::PathBuf};

/// Fields that intentionally differ on Windows: no overlay title bar there.
const PLATFORM_FIELDS: [&str; 2] = ["title", "titleBarStyle"];

fn read(file: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let text = std::fs::read_to_string(&path).expect("read Tauri config");
    serde_json::from_str(&text).expect("parse Tauri config")
}

fn main_window(config: &Value) -> Map<String, Value> {
    config["app"]["windows"]
        .as_array()
        .expect("app.windows array")
        .iter()
        .find(|window| window["label"] == "main")
        .and_then(Value::as_object)
        .cloned()
        .expect("main window entry")
}

#[test]
fn windows_main_window_repeats_every_base_field() {
    let base = main_window(&read("tauri.conf.json"));
    let windows = main_window(&read("tauri.windows.conf.json"));
    for (key, value) in &base {
        let windows_value = windows
            .get(key)
            .unwrap_or_else(|| panic!("Windows main window is missing `{key}`"));
        if !PLATFORM_FIELDS.contains(&key.as_str()) {
            assert_eq!(windows_value, value, "Windows `{key}` differs from base");
        }
    }
}

/// The bundled `scripts/installer` tree is the macOS one-click installer. As
/// per-user WiX file components it fails ICE38/ICE64, and Windows never runs it.
#[test]
fn windows_bundle_drops_the_macos_installer_resources() {
    let base = read("tauri.conf.json");
    assert!(base["bundle"]["resources"]
        .as_array()
        .expect("base bundle.resources")
        .iter()
        .any(|resource| resource == "../scripts/installer/**/*"));
    let windows = read("tauri.windows.conf.json");
    let bundle = windows["bundle"]
        .as_object()
        .expect("Windows bundle override");
    assert_eq!(bundle.keys().collect::<Vec<_>>(), ["resources"]);
    let installer = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/installer")
        .canonicalize()
        .unwrap();
    assert!(expanded_resources(&windows)
        .iter()
        .all(|resource| !resource.starts_with(&installer)));
}

#[test]
fn windows_main_window_is_titled_with_the_product_name() {
    let product = read("tauri.conf.json")["productName"].clone();
    let windows = main_window(&read("tauri.windows.conf.json"));
    assert_eq!(windows["title"], product);
}

fn expanded_resources(config: &Value) -> Vec<PathBuf> {
    let patterns: Vec<String> = config["bundle"]["resources"]
        .as_array()
        .expect("bundle.resources")
        .iter()
        .map(|pattern| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(pattern.as_str().expect("resource glob"))
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    tauri::utils::resources::ResourcePaths::new(&patterns, true)
        .map(|resource| {
            resource
                .expect("resource glob resolves")
                .canonicalize()
                .unwrap()
        })
        .collect()
}

#[test]
fn every_platform_bundles_the_logo_notices_without_duplicate_images() {
    let logo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../src/assets/agent-logos")
        .canonicalize()
        .unwrap();
    let expected: BTreeSet<String> = [
        "LICENSE-Aider.txt",
        "LICENSE-Crush.md",
        "LICENSE-Lobe-Icons.txt",
        "LICENSE-Magpie.txt",
        "NOTICE.md",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for config in ["tauri.conf.json", "tauri.windows.conf.json"] {
        let resources = expanded_resources(&read(config));
        let logo_resources: Vec<_> = resources
            .iter()
            .filter(|resource| resource.starts_with(&logo_root))
            .collect();
        assert_eq!(logo_resources.len(), expected.len(), "{config}");
        let notices = logo_resources
            .into_iter()
            .map(|resource| {
                assert!(resource.is_file());
                resource.file_name().unwrap().to_string_lossy().into_owned()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(notices, expected, "{config}");
    }
}
