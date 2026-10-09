//! Tauri merges `tauri.windows.conf.json` with JSON Merge Patch, which replaces
//! arrays wholesale: the Windows `main` window entry must repeat every field of
//! the base entry, or Windows silently falls back to Tauri defaults (800×600,
//! not centered).

use serde_json::{Map, Value};

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
    assert_eq!(bundle["resources"], Value::Array(Vec::new()));
}

#[test]
fn windows_main_window_is_titled_with_the_product_name() {
    let product = read("tauri.conf.json")["productName"].clone();
    let windows = main_window(&read("tauri.windows.conf.json"));
    assert_eq!(windows["title"], product);
}
