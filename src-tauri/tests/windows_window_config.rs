//! Tauri merges `tauri.windows.conf.json` with JSON Merge Patch, which replaces
//! arrays wholesale: the Windows `main` window entry must repeat every field of
//! the base entry, or Windows silently falls back to Tauri defaults (800×600,
//! not centered).

use serde_json::{Map, Value};
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
};

#[test]
fn every_platform_bundles_the_audio_decoder_license_and_source_notice() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for config in ["tauri.conf.json", "tauri.windows.conf.json"] {
        let entries = expanded_resource_entries(&resource_config(config));
        for name in ["SYMPHONIA-MPL-2.0.txt", "SYMPHONIA-NOTICE.md"] {
            let source = root
                .join("../docs/licenses")
                .join(name)
                .canonicalize()
                .unwrap();
            assert!(entries.contains(&(source, PathBuf::from("licenses").join(name))));
        }
    }
}

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

fn resource_config(file: &str) -> Value {
    if file == "tauri.windows.conf.json" {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let (merged, _) =
            tauri::utils::config::parse::read_from(tauri::utils::platform::Target::Windows, root)
                .expect("merge Windows config using Tauri's RFC 7396 parser");
        let _: tauri::Config = serde_json::from_value(merged.clone())
            .expect("merged Windows config must deserialize after resource removals");
        merged
    } else {
        read(file)
    }
}

fn resource_sources(config: &Value) -> Vec<&str> {
    match &config["bundle"]["resources"] {
        Value::Array(resources) => resources
            .iter()
            .map(|resource| resource.as_str().expect("resource glob"))
            .collect(),
        Value::Object(resources) => resources.keys().map(String::as_str).collect(),
        _ => panic!("bundle.resources must be an array or source-to-target map"),
    }
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
    assert!(resource_sources(&base)
        .iter()
        .any(|resource| resource.starts_with("../scripts/installer/")));
    let windows = read("tauri.windows.conf.json");
    let bundle = windows["bundle"]
        .as_object()
        .expect("Windows bundle override");
    assert_eq!(bundle.keys().collect::<Vec<_>>(), ["resources"]);
    let installer = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/installer")
        .canonicalize()
        .unwrap();
    let effective = resource_config("tauri.windows.conf.json");
    assert!(
        !resource_sources(&effective)
            .iter()
            .any(|resource| resource.starts_with("../scripts/installer/")),
        "Windows map must explicitly remove inherited installer sources"
    );
    assert!(expanded_resources(&effective)
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
    expanded_resource_entries(config)
        .into_iter()
        .map(|(source, _target)| source)
        .collect()
}

fn expanded_resource_entries(config: &Value) -> Vec<(PathBuf, PathBuf)> {
    let absolute = |source: &str| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(source)
            .to_string_lossy()
            .into_owned()
    };
    let resolve = |resource: Result<tauri::utils::resources::Resource, tauri::utils::Error>| {
        let resource = resource.expect("resource glob resolves");
        (
            resource.path().canonicalize().expect("resource exists"),
            resource.target().to_owned(),
        )
    };
    match &config["bundle"]["resources"] {
        Value::Array(resources) => {
            let patterns: Vec<String> = resources
                .iter()
                .map(|source| absolute(source.as_str().expect("resource glob")))
                .collect();
            tauri::utils::resources::ResourcePaths::new(&patterns, true)
                .iter()
                .map(resolve)
                .collect()
        }
        Value::Object(resources) => {
            let patterns: HashMap<String, String> = resources
                .iter()
                .map(|(source, target)| {
                    (
                        absolute(source),
                        target.as_str().expect("resource target").to_owned(),
                    )
                })
                .collect();
            tauri::utils::resources::ResourcePaths::from_map(&patterns, true)
                .iter()
                .map(resolve)
                .collect()
        }
        _ => panic!("bundle.resources must be an array or source-to-target map"),
    }
}

#[test]
fn resource_expansion_accepts_lists_and_explicit_maps() {
    let source = "src/services/MAGPIE_LICENSE";
    let list = serde_json::json!({"bundle":{"resources":[source]}});
    let map = serde_json::json!({"bundle":{"resources":{(source):"licenses/MAGPIE_LICENSE"}}});
    assert_eq!(expanded_resources(&list), expanded_resources(&map));
    assert_eq!(
        expanded_resource_entries(&map)[0].1,
        PathBuf::from("licenses/MAGPIE_LICENSE")
    );
}

#[test]
fn every_platform_maps_the_assistant_runtime_to_its_exact_bundle_root() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let entry = root
        .join("../assistant-runtime/resources/runtime/src/main.mjs")
        .canonicalize()
        .expect("assistant production entry prepared before Cargo");
    for config in ["tauri.conf.json", "tauri.windows.conf.json"] {
        let config_value = resource_config(config);
        let resources = config_value["bundle"]["resources"]
            .as_object()
            .expect("assistant resources require an explicit source-to-target map");
        assert_eq!(
            resources
                .get("../assistant-runtime/resources/")
                .and_then(Value::as_str),
            Some("assistant-runtime/"),
            "{config} must preserve the runtime's resource root"
        );
        let entries = expanded_resource_entries(&config_value);
        assert!(
            entries.iter().any(|(source, target)| source == &entry
                && target == &PathBuf::from("assistant-runtime/runtime/src/main.mjs")),
            "{config} must ship the sidecar at its host-resolved entry path"
        );
        let node_name = if cfg!(windows) { "node.exe" } else { "node" };
        let node = root
            .join("../assistant-runtime/resources/node")
            .join(node_name)
            .canonicalize()
            .expect("bundled Node prepared before Cargo");
        assert!(
            entries.iter().any(|(source, target)| source == &node
                && target == &PathBuf::from(format!("assistant-runtime/node/{node_name}"))),
            "{config} must ship the actual platform's fixed Node at its host-resolved path"
        );
        // The sidecar starts the web search server next to `runtime/`.
        let search = root
            .join("../assistant-runtime/resources/search-mcp/ofox-search-mcp.mjs")
            .canonicalize()
            .expect("search server bundle prepared before Cargo");
        assert!(
            entries.iter().any(|(source, target)| source == &search
                && target == &PathBuf::from("assistant-runtime/search-mcp/ofox-search-mcp.mjs")),
            "{config} must ship the search server where the sidecar resolves it"
        );
        // Pi's demo extensions (one is a GPL-licensed doom.wasm) and source maps
        // are pruned by the prepare script and must never reach the bundle; a
        // doom.wasm is rejected wherever a dependency might carry one.
        let examples = root
            .join("../assistant-runtime/resources/runtime/node_modules/@earendil-works/pi-coding-agent")
            .canonicalize()
            .expect("Pi SDK prepared before Cargo")
            .join("examples");
        assert!(
            entries
                .iter()
                .all(|(source, _)| !source.starts_with(&examples)
                    && source.file_name().is_none_or(|name| name != "doom.wasm")
                    && source
                        .extension()
                        .is_none_or(|extension| extension != "map")),
            "{config} must not ship Pi examples, doom.wasm or source maps"
        );
        // Only the legacy PDF.js build runs on the bundled Node; the runtime
        // never loads PDF.js viewers, standalone decoders or standard fonts.
        let pdfjs = root
            .join("../assistant-runtime/resources/runtime/node_modules/pdfjs-dist")
            .canonicalize()
            .expect("PDF.js prepared before Cargo");
        assert!(
            entries.iter().all(|(source, _)| [
                "build",
                "web",
                "image_decoders",
                "standard_fonts",
                "legacy/web",
                "legacy/image_decoders",
            ]
            .iter()
            .all(|unused| !source.starts_with(pdfjs.join(unused)))),
            "{config} must not ship the modern PDF.js build, viewers, decoders or standard fonts"
        );
    }
}

#[test]
fn every_platform_bundles_the_assistant_runtime_licenses() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let node_license = root
        .join("../assistant-runtime/resources/node/LICENSE")
        .canonicalize()
        .expect("Node license prepared before Cargo");
    let pi_license = root
        .join("../assistant-runtime/PI_LICENSE")
        .canonicalize()
        .unwrap();
    // Notices for the native canvas addon, PDF.js and the packages bundled
    // into the search server, copied by the prepare script into the runtime
    // resources.
    let notices = root.join("../assistant-runtime/resources/notices");
    let canvas_notice = notices
        .join("napi-rs-canvas.txt")
        .canonicalize()
        .expect("canvas notice prepared before Cargo");
    let pdfjs_notice = notices
        .join("pdfjs-dist.txt")
        .canonicalize()
        .expect("PDF.js notice prepared before Cargo");
    let search_notice = notices
        .join("ofox-search-mcp.txt")
        .canonicalize()
        .expect("search server notice prepared before Cargo");
    for config in ["tauri.conf.json", "tauri.windows.conf.json"] {
        let entries = expanded_resource_entries(&resource_config(config));
        for (license, target) in [
            (&node_license, "assistant-runtime/node/LICENSE"),
            (&pi_license, "assistant-runtime/PI_LICENSE"),
            (
                &canvas_notice,
                "assistant-runtime/notices/napi-rs-canvas.txt",
            ),
            (&pdfjs_notice, "assistant-runtime/notices/pdfjs-dist.txt"),
            (
                &search_notice,
                "assistant-runtime/notices/ofox-search-mcp.txt",
            ),
        ] {
            assert!(
                entries
                    .iter()
                    .any(|(source, bundled)| source == license && bundled == &PathBuf::from(target)),
                "{config} must ship {target}"
            );
        }
    }
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
        let resources = expanded_resources(&resource_config(config));
        let library_logos = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../src/assets/library-logos")
            .canonicalize()
            .unwrap();
        assert_eq!(
            resources
                .iter()
                .filter(|resource| resource.starts_with(&library_logos))
                .collect::<Vec<_>>(),
            vec![&library_logos.join("NOTICE.md")],
            "{config} must ship the library logo notice without duplicate images"
        );
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

#[test]
fn every_platform_bundles_the_license_of_each_ported_magpie_module() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for config in ["tauri.conf.json", "tauri.windows.conf.json"] {
        let resources = expanded_resources(&resource_config(config));
        for module in ["services", "session_manager", "native_tools"] {
            let license = root
                .join("src")
                .join(module)
                .join("MAGPIE_LICENSE")
                .canonicalize()
                .unwrap();
            assert!(
                resources.contains(&license),
                "{config} must ship src/{module}/MAGPIE_LICENSE"
            );
        }
    }
}
