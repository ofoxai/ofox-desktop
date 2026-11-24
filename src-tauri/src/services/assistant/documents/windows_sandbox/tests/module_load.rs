//! A small, production-token diagnosis. It never proves engine admission.
use super::*;
use serde::{Deserialize, Serialize};
use std::os::windows::fs::MetadataExt;

const OWNER_FILE: &str = ".module-load-diagnostic-owner";
const SCRIPT: &str = r#"
const fs = require('node:fs');
const path = require('node:path');
const codes = new Set(['EPERM','EACCES','ENOENT','ENOTDIR','EISDIR','ELOOP','ERR_DLOPEN_FAILED','MODULE_NOT_FOUND']);
const errorCode = error => codes.has(error?.code) ? error.code : 'OTHER';
const inspect = file => {
  const state = {};
  for (const method of ['stat','lstat']) {
    try {
      const value = fs[method + 'Sync'](file);
      state[method] = value.isFile() ? 'file' : value.isDirectory() ? 'directory' : 'other';
    } catch (error) { state[method] = errorCode(error); }
  }
  try { fs.realpathSync(file); state.realpath = 'resolved'; }
  catch (error) { state.realpath = errorCode(error); }
  return state;
};
const prefixes = LOADER_PREFIXES;
const loaderCode = (error,file) => {
  if (error?.code !== 'ERR_DLOPEN_FAILED') return null;
  const message = typeof error.message === 'string' ? error.message : '';
  const matches = prefixes.filter(([code,prefix]) => message.startsWith(
    code === 193 ? prefix.replace('%1', () => path.toNamespacedPath(file)) : prefix));
  return matches.length === 1 ? String(matches[0][0]) : 'unknown';
};
const load = (label, file) => {
  const record = {label, state: inspect(file), require: 'loaded', loader: null,
    direct: 'not-attempted', directLoader: null};
  try { require(file); }
  catch (error) {
    record.require = errorCode(error);
    record.loader = loaderCode(error,file);
    try { process.dlopen({exports:{}},path.toNamespacedPath(file)); record.direct = 'loaded'; }
    catch (directError) { record.direct = errorCode(directError); record.directLoader = loaderCode(directError,file); }
  }
  return record;
};
const probe = process.argv[1];
const digest = path.dirname(path.dirname(probe));
const parent = path.dirname(digest);
const temp = path.dirname(parent);
const ancestors = [
  ['drive-root',path.parse(probe).root],['temp-root',temp],
  ['cache-parent',parent],['digest-root',digest],['module-dir',path.dirname(probe)],
].map(([label,file]) => ({label,state:inspect(file)}));
console.log(JSON.stringify({
  nodeVersion:process.versions.node, namespaced:probe.startsWith('\\\\?\\'),
  modules:[load('probe',probe),load('skia',process.argv[2])], ancestors,
}));
"#;

const ERROR_CODES: &[&str] = &[
    "EPERM",
    "EACCES",
    "ENOENT",
    "ENOTDIR",
    "EISDIR",
    "ELOOP",
    "ERR_DLOPEN_FAILED",
    "MODULE_NOT_FOUND",
    "OTHER",
];

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PathState {
    stat: String,
    lstat: String,
    realpath: String,
}

impl PathState {
    fn valid(&self) -> bool {
        [&self.stat, &self.lstat].iter().all(|value| {
            ["file", "directory", "other"].contains(&value.as_str())
                || ERROR_CODES.contains(&value.as_str())
        }) && (self.realpath == "resolved" || ERROR_CODES.contains(&self.realpath.as_str()))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ModuleState {
    label: String,
    state: PathState,
    require: String,
    loader: Option<String>,
    direct: String,
    direct_loader: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AncestorState {
    label: String,
    state: PathState,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Report {
    node_version: String,
    namespaced: bool,
    modules: [ModuleState; 2],
    ancestors: [AncestorState; 5],
}

impl Report {
    fn parse(stdout: &str) -> Option<Self> {
        let report: Self = serde_json::from_str(stdout).ok()?;
        let valid = report.node_version == "22.23.4"
            && report
                .modules
                .iter()
                .zip(["probe", "skia"])
                .all(|(item, label)| {
                    item.label == label
                        && item.state.valid()
                        && (item.require == "loaded"
                            || ERROR_CODES.contains(&item.require.as_str()))
                        && valid_loader(&item.require, item.loader.as_deref())
                        && if item.require == "loaded" {
                            item.direct == "not-attempted" && item.direct_loader.is_none()
                        } else {
                            (item.direct == "loaded" || ERROR_CODES.contains(&item.direct.as_str()))
                                && valid_loader(&item.direct, item.direct_loader.as_deref())
                        }
                })
            && report
                .ancestors
                .iter()
                .zip([
                    "drive-root",
                    "temp-root",
                    "cache-parent",
                    "digest-root",
                    "module-dir",
                ])
                .all(|(item, label)| item.label == label && item.state.valid());
        valid.then_some(report)
    }
}

fn valid_loader(result: &str, code: Option<&str>) -> bool {
    match code {
        None => result != "ERR_DLOPEN_FAILED",
        Some("unknown") => result == "ERR_DLOPEN_FAILED",
        Some(code) => {
            result == "ERR_DLOPEN_FAILED"
                && CHILD_LOADER_WIN32_CODES
                    .iter()
                    .any(|known| known.to_string() == code)
        }
    }
}

fn copy_fixed_binaries(source: &Path, fixture: &Path, owner: &str) {
    for relative in [
        "node/node.exe",
        "probes/windows-child-probe.node",
        "runtime/node_modules/@napi-rs/canvas-win32-x64-msvc/skia.win32-x64-msvc.node",
    ] {
        let mut bytes = Vec::new();
        open_regular(source, &source.join(relative), MAX_TREE_BYTES)
            .expect("open fixed bundled binary")
            .read_to_end(&mut bytes)
            .expect("read fixed bundled binary");
        let target = fixture.join(relative);
        fs::create_dir_all(target.parent().unwrap()).expect("create fixed fixture parent");
        fs::write(&target, &bytes).expect("copy fixed fixture binary");
        assert!(
            fs::read(&target).is_ok_and(|copied| copied == bytes),
            "fixed binary copy differs"
        );
    }
    fs::write(fixture.join(OWNER_FILE), owner).expect("mark owned fixture cache");
}

fn remove_fixture_cache(source: &Path, owner: &str) {
    let cached = CACHE
        .get()
        .and_then(|cache| cache.lock().ok()?.remove(source));
    let Some(cached) = cached else { return };
    let user = current_user().expect("identify cache owner");
    let expected_parent = std::env::temp_dir().join(format!(
        "ofox-document-runtime-{}",
        &format!("{:x}", Sha256::digest(user.as_bytes()))[..16]
    ));
    assert!(
        cached.source == source
            && cached
                .root
                .file_name()
                .is_some_and(|name| name == cached.digest.as_str()),
        "unexpected fixture cache identity"
    );
    let expected_parent = expected_parent
        .canonicalize()
        .expect("resolve fixture cache parent");
    let resolved_root = cached
        .root
        .canonicalize()
        .expect("resolve owned fixture cache");
    assert!(
        resolved_root == expected_parent.join(&cached.digest),
        "fixture cache left its owned parent"
    );
    let metadata = fs::symlink_metadata(&cached.root).expect("inspect owned fixture cache");
    assert!(
        metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
        "fixture cache was replaced"
    );
    let _parents =
        pin_ancestors(cached.root.parent().unwrap()).expect("pin fixture cache ancestors");
    assert!(
        fs::read(cached.root.join(OWNER_FILE)).is_ok_and(|bytes| bytes == owner.as_bytes()),
        "fixture cache ownership marker changed"
    );
    fs::remove_dir_all(&cached.root).expect("remove completed fixture binary cache");
}

#[test]
fn module_load_diagnostic_rejects_unbounded_receipts() {
    let state = serde_json::json!({"stat":"file","lstat":"file","realpath":"resolved"});
    let modules = ["probe", "skia"].map(|label| {
        serde_json::json!({
            "label":label,"state":state,"require":"loaded","loader":null,
            "direct":"not-attempted","directLoader":null,
        })
    });
    let ancestors = [
        "drive-root",
        "temp-root",
        "cache-parent",
        "digest-root",
        "module-dir",
    ]
    .map(|label| serde_json::json!({"label":label,"state":state}));
    let valid = serde_json::json!({"nodeVersion":"22.23.4","namespaced":false,"modules":modules,"ancestors":ancestors});
    let parse = |value: &serde_json::Value| Report::parse(&value.to_string()).is_some();
    assert!(parse(&valid));
    let mut extra = valid.clone();
    extra["privatePath"] = serde_json::json!("private output");
    assert!(!parse(&extra));
    for field in ["require", "direct", "loader", "directLoader"] {
        let mut unknown = valid.clone();
        unknown["modules"][0][field] = serde_json::json!("private output");
        assert!(!parse(&unknown));
    }
    let duplicated = valid.to_string().replacen(
        "\"namespaced\":false",
        "\"namespaced\":false,\"namespaced\":false",
        1,
    );
    assert!(Report::parse(&duplicated).is_none());
}

#[tokio::test]
#[serial_test::serial]
async fn document_lpac_native_module_load_diagnostic() {
    let sandbox = crate::test_sandbox::sandbox();
    let fixture = sandbox.home().join("module-runtime");
    fs::create_dir(&fixture).unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    copy_fixed_binaries(&runtime().canonicalize().unwrap(), &fixture, &owner);
    let fixture = fixture.canonicalize().unwrap();
    let cwd = sandbox.home().join("module-task");
    let home = cwd.join("home");
    fs::create_dir_all(&home).unwrap();
    let node = fixture.join("node/node.exe");
    let probe = fixture.join("probes/windows-child-probe.node");
    let skia = fixture
        .join("runtime/node_modules/@napi-rs/canvas-win32-x64-msvc/skia.win32-x64-msvc.node");
    let prefixes = CHILD_LOADER_WIN32_CODES
        .iter()
        .filter_map(|&code| child_loader_prefix(code).map(|prefix| (code, prefix)))
        .collect::<Vec<_>>();
    let script = SCRIPT.replace(
        "LOADER_PREFIXES",
        &serde_json::to_string(&prefixes).unwrap(),
    );
    let mut trace = ProbeTrace::new();
    trace.engine(ProbeStage::NodeIsolation);
    let result = run_candidate_observed(
        &node,
        &[
            "-e",
            &script,
            &probe.to_string_lossy(),
            &skia.to_string_lossy(),
        ],
        &cwd,
        &home,
        &fixture,
        Duration::from_secs(15),
        16 * 1024,
        true,
        trace.observation.clone(),
    )
    .await;
    // The cache contains three copied public binaries and a per-run marker.
    // No other engine can own this nonce-derived digest; the Job has returned.
    let admitted = windows_document_engine_proven(&fixture);
    if result.is_ok() {
        remove_fixture_cache(&fixture, &owner);
    }
    let output = result.unwrap_or_else(|error| {
        panic!(
            "minimal module fixture failed: {}",
            trace.failure(&error, &[])
        )
    });
    assert!(
        output.status == Some(0) && !output.timed_out,
        "fixed module diagnostic did not complete"
    );
    assert!(
        output.stderr.is_empty(),
        "fixed module diagnostic wrote unexpected stderr"
    );
    let report = Report::parse(&output.stdout)
        .expect("fixed module diagnostic output is not a closed receipt");
    assert!(!admitted, "a module diagnostic admitted an engine");
    eprintln!(
        "OFOX_MODULE_LOAD_V1 {}",
        serde_json::to_string(&report).unwrap()
    );
    assert!(
        report
            .modules
            .iter()
            .all(|module| module.require == "loaded"),
        "verified native modules must load through ordinary require"
    );
}

#[test]
fn node_path_options_apply_only_to_the_exact_cached_node_image() {
    let runtime = CachedRuntime {
        source: PathBuf::from(r"C:\source"),
        root: PathBuf::from(r"C:\verified-runtime"),
        digest: "fixture".into(),
    };
    let original = vec![
        "python/runner.mjs".into(),
        "--code".into(),
        "code.py".into(),
    ];
    let node = runtime.root.join("node/node.exe");
    assert_eq!(
        engine_arguments(&node, original.clone(), &runtime),
        [
            "--preserve-symlinks",
            "--preserve-symlinks-main",
            "python/runner.mjs",
            "--code",
            "code.py"
        ]
    );
    for other in [
        runtime.root.join("engines/officecli/officecli.exe"),
        runtime.root.join("other/node.exe"),
        runtime.source.join("node/node.exe"),
    ] {
        assert_eq!(
            engine_arguments(&other, original.clone(), &runtime),
            original
        );
    }
}
