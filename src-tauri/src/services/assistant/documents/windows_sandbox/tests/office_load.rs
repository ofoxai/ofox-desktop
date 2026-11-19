//! Small, fixed-input comparison; this never admits an engine or relaxes LPAC.
use super::*;
use crate::services::assistant::documents::runner::run_engine_limited;
use serde::Serialize;
use std::os::windows::fs::MetadataExt;

const OWNER_FILE: &str = ".office-load-diagnostic-owner";
const ERROR_CODES: &[&str] = &[
    "file_not_found",
    "io_error",
    "access_denied",
    "permission_denied",
    "validation_error",
    "invalid_argument",
    "invalid_operation",
    "unsupported_format",
    "internal_error",
    "unknown_error",
];

/// Only fixed tokens can leave the source output. In particular, never retain
/// exception messages, file paths, suggestions, XML or arbitrary JSON keys.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OfficeReport {
    status: Option<i32>,
    timed_out: bool,
    stdout_bytes: usize,
    stderr_bytes: usize,
    json: bool,
    success: Option<bool>,
    valid: Option<bool>,
    error_code: Option<&'static str>,
    errors_count: Option<usize>,
    message_class: &'static str,
    message_terms: Vec<&'static str>,
    path_components: Vec<&'static str>,
    quoted_path: Option<QuotedPath>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotedPath {
    kind: &'static str,
    component_count: usize,
    trailing_separator: bool,
    ends_with_temp: bool,
    ends_with_ac_temp: bool,
    has_ac_temp_pair: bool,
    components_after_temp: Option<usize>,
    extension: &'static str,
    leaf_class: &'static str,
    profile_class: &'static str,
}

fn quoted_path(message: &str) -> Option<QuotedPath> {
    let path = message
        .strip_prefix("could not find a part of the path '")?
        .strip_suffix("'.")?;
    let segments = path
        .split(['\\', '/'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let leaf = *segments.last()?;
    let extension = Path::new(leaf)
        .extension()
        .and_then(OsStr::to_str)
        .map(|value| match value {
            "pptx" => "pptx",
            "docx" => "docx",
            "xlsx" => "xlsx",
            "json" => "json",
            "tmp" => "tmp",
            "dll" => "dll",
            "exe" => "exe",
            "xml" => "xml",
            "rels" => "rels",
            "lock" => "lock",
            "dirty" => "dirty",
            "node" => "node",
            _ => "other",
        })
        .unwrap_or("none");
    let fixed_leaf = [
        "packages",
        "localcache",
        "localstate",
        "roamingstate",
        "ac",
        "temp",
        "officecli",
        "isolatedstorage",
        ".dotnet",
    ];
    let leaf_class = fixed_leaf
        .into_iter()
        .find(|known| *known == leaf)
        .unwrap_or_else(|| {
            if leaf.starts_with("ocli_ro_") && extension == "pptx" {
                "office-read-only-repair-pptx"
            } else if leaf.starts_with("officecli_batch_") && extension == "json" {
                "office-batch-json"
            } else {
                "other"
            }
        });
    let profile_class = segments
        .iter()
        .position(|part| *part == "packages")
        .and_then(|index| segments.get(index + 1))
        .map(|profile| {
            if profile.strip_prefix("ofox.document.").is_some_and(|id| {
                id.len() == 32 && id.bytes().all(|value| value.is_ascii_hexdigit())
            }) {
                "ofox-document-uuid"
            } else if profile.starts_with("s-1-15-2-") {
                "app-container-sid"
            } else {
                "other"
            }
        })
        .unwrap_or("absent");
    Some(QuotedPath {
        kind: if path.starts_with(r"\\?\") {
            "namespaced"
        } else if path.starts_with(r"\\") {
            "unc"
        } else if path.as_bytes().get(1) == Some(&b':') {
            "drive"
        } else {
            "relative-or-other"
        },
        component_count: segments.len().min(128),
        trailing_separator: path.ends_with(['\\', '/']),
        ends_with_temp: leaf == "temp",
        ends_with_ac_temp: segments.ends_with(&["ac", "temp"]),
        has_ac_temp_pair: segments.windows(2).any(|pair| pair == ["ac", "temp"]),
        components_after_temp: segments
            .iter()
            .rposition(|part| *part == "temp")
            .map(|index| (segments.len() - index - 1).min(128)),
        extension,
        leaf_class,
        profile_class,
    })
}

fn message_class(message: &str) -> &'static str {
    // Prefixes are the fixed English .NET/Win32 exception templates. Their
    // path-bearing substitutions are deliberately never copied or formatted.
    let templates = [
        ("could not find a part of the path ", "directory-not-found"),
        (
            "the system cannot find the path specified",
            "path-not-found",
        ),
        (
            "the system cannot find the file specified",
            "file-not-found",
        ),
        ("could not find file ", "file-not-found"),
        ("access to the path ", "path-access-denied"),
        ("access is denied", "access-denied"),
        (
            "the specified path, file name, or both are too long",
            "path-too-long",
        ),
        ("the filename or extension is too long", "path-too-long"),
        ("the path is empty", "empty-path"),
        ("path cannot be the empty string", "empty-path"),
        ("illegal characters in path", "invalid-path"),
        ("the path is not of a legal form", "invalid-path"),
        (
            "the filename, directory name, or volume label syntax is incorrect",
            "invalid-path",
        ),
        (
            "the given path's format is not supported",
            "unsupported-path-format",
        ),
        ("the network path was not found", "network-path-not-found"),
        ("the process cannot access the file ", "file-in-use"),
        ("the directory name is invalid", "invalid-directory"),
        (
            "unable to determine the identity of domain",
            "identity-unavailable",
        ),
        (
            "unable to determine the identity of the user",
            "identity-unavailable",
        ),
        (
            "unable to determine the application identity",
            "identity-unavailable",
        ),
        (
            "unable to create the store directory",
            "isolated-store-directory",
        ),
        (
            "package should contain a content type part",
            "package-content-type-missing",
        ),
        ("package is not open", "package-not-open"),
        ("package is in read-only mode", "package-read-only"),
        ("file contains corrupted data", "package-corrupted"),
        ("the specified package is invalid", "package-invalid"),
        ("central directory corrupt", "zip-directory-corrupted"),
        (
            "end of central directory record could not be found",
            "zip-directory-missing",
        ),
        (
            "the archive entry was compressed using an unsupported compression method",
            "zip-compression-unsupported",
        ),
    ];
    templates
        .into_iter()
        .find_map(|(prefix, class)| message.starts_with(prefix).then_some(class))
        .unwrap_or("other")
}

fn path_components(message: &str) -> Vec<&'static str> {
    let components = [
        "packages",
        "localcache",
        "localstate",
        "roamingstate",
        "ac",
        "temp",
        "officecli",
        "isolatedstorage",
        ".dotnet",
        "appdata",
        "local",
        "roaming",
        "microsoft",
        "windows",
        "inetcache",
        "inetsessions",
        "systemprofile",
    ];
    let segments = message
        .split(['\\', '/'])
        .map(|segment| {
            segment
                .trim_start_matches(|c: char| c.is_ascii_whitespace() || matches!(c, '\'' | '"'))
                .trim_end_matches(|c: char| {
                    c.is_ascii_whitespace() || matches!(c, '\'' | '"' | '.' | ':' | ';' | ')' | ']')
                })
        })
        .collect::<Vec<_>>();
    components
        .into_iter()
        .filter(|component| segments.contains(component))
        .collect()
}

impl OfficeReport {
    fn from_output(output: &EngineOutput) -> Self {
        let parsed = serde_json::from_str::<serde_json::Value>(&output.stdout).ok();
        let value = parsed.as_ref();
        let error = value.and_then(|value| value.get("error"));
        let raw_code = error
            .and_then(|error| error.get("code"))
            .and_then(serde_json::Value::as_str);
        let error_code = raw_code.map(|code| {
            ERROR_CODES
                .iter()
                .copied()
                .find(|known| *known == code)
                .unwrap_or("other")
        });
        let message = error
            .and_then(|error| error.get("error"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| error.and_then(serde_json::Value::as_str))
            .unwrap_or_default()
            .to_ascii_lowercase();
        let terms = [
            "access",
            "denied",
            "unauthorized",
            "permission",
            "path",
            "file",
            "directory",
            "root",
            "registry",
            "font",
            "null",
            "network",
            "invalid",
            "not found",
            "validation",
            "exception",
            "argument",
            "xml",
            "package",
            "zip",
            "open",
            "create",
            "read",
            "write",
            "format",
            "missing",
            "configuration",
            "could not load",
            "not supported",
            "0x80070005",
        ];
        Self {
            status: output.status,
            timed_out: output.timed_out,
            stdout_bytes: output.stdout.len(),
            stderr_bytes: output.stderr.len(),
            json: parsed.is_some(),
            success: value
                .and_then(|value| value.get("success"))
                .and_then(serde_json::Value::as_bool),
            valid: value
                .and_then(|value| value.get("data"))
                .and_then(|data| data.get("valid"))
                .and_then(serde_json::Value::as_bool),
            error_code,
            errors_count: value
                .and_then(|value| value.get("data"))
                .and_then(|data| data.get("errors"))
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            message_class: message_class(&message),
            message_terms: terms
                .into_iter()
                .filter(|term| message.contains(term))
                .collect(),
            path_components: path_components(&message),
            quoted_path: quoted_path(&message),
        }
    }

    fn passed(&self) -> bool {
        self.status == Some(0)
            && !self.timed_out
            && self.json
            && self.success != Some(false)
            && self.valid != Some(false)
    }
}

fn copy_office(source: &Path, fixture: &Path, owner: &str) {
    let relative = "engines/officecli/officecli.exe";
    let mut bytes = Vec::new();
    open_regular(source, &source.join(relative), MAX_TREE_BYTES)
        .expect("open fixed OfficeCLI binary")
        .read_to_end(&mut bytes)
        .expect("read fixed OfficeCLI binary");
    let target = fixture.join(relative);
    fs::create_dir_all(target.parent().unwrap()).expect("create OfficeCLI fixture parent");
    fs::write(&target, &bytes).expect("copy fixed OfficeCLI binary");
    assert!(
        fs::read(&target).is_ok_and(|copied| copied == bytes),
        "fixed OfficeCLI binary copy differs"
    );
    fs::write(fixture.join(OWNER_FILE), owner).expect("mark owned OfficeCLI cache");
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
        "unexpected OfficeCLI fixture cache identity"
    );
    let expected_parent = expected_parent
        .canonicalize()
        .expect("resolve OfficeCLI cache parent");
    let resolved_root = cached
        .root
        .canonicalize()
        .expect("resolve owned OfficeCLI cache");
    assert!(
        resolved_root == expected_parent.join(&cached.digest),
        "OfficeCLI fixture cache left its owned parent"
    );
    let metadata = fs::symlink_metadata(&cached.root).expect("inspect OfficeCLI cache");
    assert!(
        metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
        "OfficeCLI fixture cache was replaced"
    );
    let _parents = pin_ancestors(cached.root.parent().unwrap())
        .expect("pin OfficeCLI fixture cache ancestors");
    assert!(
        fs::read(cached.root.join(OWNER_FILE)).is_ok_and(|bytes| bytes == owner.as_bytes()),
        "OfficeCLI cache ownership marker changed"
    );
    fs::remove_dir_all(&cached.root).expect("remove completed OfficeCLI fixture cache");
}

#[test]
fn office_diagnostic_discards_arbitrary_errors_and_paths() {
    let output = EngineOutput {
        status: Some(1),
        timed_out: false,
        stdout: serde_json::json!({
            "success": false,
            "privateKey": "private-output",
            "error": {
                "code": "private-output",
                "error": "Access denied to C:\\private-output\\document.pptx",
                "suggestion": "private-output"
            }
        })
        .to_string(),
        stderr: "private-output".into(),
    };
    let report = OfficeReport::from_output(&output);
    assert_eq!(report.error_code, Some("other"));
    assert_eq!(report.message_terms, ["access", "denied"]);
    assert_eq!(report.message_class, "other");
    assert!(report.path_components.is_empty());
    assert!(report.quoted_path.is_none());
    assert!(!report.passed());
    assert!(!serde_json::to_string(&report)
        .unwrap()
        .contains("private-output"));
}

#[test]
fn office_diagnostic_classifies_fixed_exception_templates_and_whole_components() {
    let message = "could not find a part of the path 'c:\\private-user\\packages\\private-identity\\localcache\\localstate\\roamingstate\\ac\\temp\\officecli\\isolatedstorage\\.dotnet'.";
    assert_eq!(message_class(message), "directory-not-found");
    assert_eq!(
        path_components(message),
        [
            "packages",
            "localcache",
            "localstate",
            "roamingstate",
            "ac",
            "temp",
            "officecli",
            "isolatedstorage",
            ".dotnet"
        ]
    );
    assert!(
        path_components("c:\\private-packages\\localcache-suffix\\temp-file\\.dotnet-tool")
            .is_empty()
    );
    assert_eq!(message_class("private-error-directory-not-found"), "other");
    assert_eq!(
        message_class("the given path's format is not supported. c:\\private"),
        "unsupported-path-format"
    );
    assert_eq!(
        message_class("unable to determine the application identity"),
        "identity-unavailable"
    );
    assert_eq!(
        message_class("package should contain a content type part"),
        "package-content-type-missing"
    );
    let path = quoted_path("could not find a part of the path 'c:\\private-user\\packages\\ofox.document.0123456789abcdef0123456789abcdef\\ac\\temp\\ocli_ro_private-suffix.pptx'.").unwrap();
    assert_eq!(path.profile_class, "ofox-document-uuid");
    assert_eq!(path.leaf_class, "office-read-only-repair-pptx");
    assert_eq!(path.extension, "pptx");
    assert_eq!(path.components_after_temp, Some(1));
    assert!(path.has_ac_temp_pair && !path.ends_with_temp);
    assert!(!serde_json::to_string(&path).unwrap().contains("private"));
    let path = quoted_path("could not find a part of the path 'c:\\private-user\\packages\\private-identity\\ac\\temp\\'.").unwrap();
    assert!(path.ends_with_ac_temp && path.ends_with_temp && path.trailing_separator);
    assert_eq!(path.profile_class, "other");
    assert_eq!(path.components_after_temp, Some(0));
    assert_eq!(path.extension, "none");
}

#[tokio::test]
#[serial_test::serial]
async fn document_lpac_office_validate_diagnostic() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = runtime();
    let cwd = sandbox.home().join("office-task");
    let home = cwd.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(cwd.join("code.py"), PYTHON_PROBE).unwrap();
    let runner = root.join("runtime/src/python/runner.mjs");
    let assets = root.join("engines/python");
    // This ordinary process generates only our fixed, trusted test fixture.
    // It is not a fallback for any production or model-generated execution.
    let generated = run_engine_limited(
        &root.join("node/node.exe"),
        &[
            &runner.to_string_lossy(),
            "--assets",
            &assets.to_string_lossy(),
            "--code",
            "code.py",
            "--out",
            "out.pptx",
            "--lint",
            "lint.json",
        ],
        &cwd,
        &home,
        Duration::from_secs(90),
        16 * 1024,
    )
    .await
    .expect("generate fixed OfficeCLI fixture");
    assert!(
        generated.status == Some(0) && !generated.timed_out,
        "fixed Python fixture generation failed"
    );
    assert!(
        fs::read(cwd.join("out.pptx")).is_ok_and(|bytes| bytes.starts_with(b"PK")),
        "fixed Python fixture is not a ZIP package"
    );
    let fixture = sandbox.home().join("office-runtime");
    fs::create_dir(&fixture).unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    copy_office(&root.canonicalize().unwrap(), &fixture, &owner);
    let fixture = fixture.canonicalize().unwrap();
    let office = fixture.join("engines/officecli/officecli.exe");
    let args = ["validate", "out.pptx", "--json"];
    let original = fs::read(cwd.join("out.pptx")).unwrap();
    let ordinary = run_engine_limited(
        &office,
        &args,
        &cwd,
        &home,
        Duration::from_secs(60),
        16 * 1024,
    )
    .await
    .expect("ordinary fixed OfficeCLI validation");
    let ordinary = OfficeReport::from_output(&ordinary);
    eprintln!(
        "OFOX_OFFICE_ORDINARY_V1 {}",
        serde_json::to_string(&ordinary).unwrap()
    );
    assert!(ordinary.passed(), "ordinary fixed deck must validate");
    assert!(
        fs::read(cwd.join("out.pptx")).is_ok_and(|bytes| bytes == original),
        "ordinary OfficeCLI changed the fixed input"
    );
    let mut trace = ProbeTrace::new();
    trace.engine(ProbeStage::OfficeValidate);
    let restricted = run_candidate_observed(
        &office,
        &args,
        &cwd,
        &home,
        &fixture,
        Duration::from_secs(60),
        16 * 1024,
        true,
        trace.observation.clone(),
    )
    .await;
    let admitted = windows_document_engine_proven(&fixture);
    if restricted.is_ok() {
        // Only the nonce-owned, completed, tiny copied cache is removed.
        remove_fixture_cache(&fixture, &owner);
    }
    let restricted = restricted.unwrap_or_else(|error| {
        panic!(
            "fixed OfficeCLI fixture failed: {}",
            trace.failure(&error, &[])
        )
    });
    let restricted = OfficeReport::from_output(&restricted);
    eprintln!(
        "OFOX_OFFICE_LPAC_V1 {}",
        serde_json::to_string(&restricted).unwrap()
    );
    assert!(!admitted, "OfficeCLI diagnostic admitted an engine");
    assert!(
        fs::read(cwd.join("out.pptx")).is_ok_and(|bytes| bytes == original),
        "restricted OfficeCLI changed the fixed input"
    );
    assert!(restricted.passed(), "restricted fixed deck must validate");
}

#[test]
#[serial_test::serial]
fn office_private_home_bootstrap_prepares_existing_temp_and_cleans_it() {
    let _sandbox = crate::test_sandbox::sandbox();
    let capability = RuntimeCapability::new().unwrap();
    let mut profile = Profile::new(&capability).unwrap();
    profile.make_low_profile().unwrap();
    let mut task = task_directory(&current_user().unwrap(), profile.sid.0).unwrap();
    let root = task.0.clone();
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    let profile_name = String::from_utf16(&profile.name[..profile.name.len() - 1]).unwrap();
    let expected = home
        .join("Packages")
        .join(profile_name)
        .join("AC")
        .join("Temp");
    assert!(!expected.exists());
    profile.prepare_private_temp(&home).unwrap();
    profile.prepare_private_temp(&home).unwrap();
    let mut cursor = home.clone();
    for component in expected.strip_prefix(&home).unwrap().components() {
        cursor.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&cursor).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0,
            0
        );
    }
    task.dispose().unwrap();
    assert!(!root.exists(), "private task scratch survived cleanup");
    profile.dispose().unwrap();
}

#[test]
#[serial_test::serial]
fn office_private_home_bootstrap_refuses_junction_and_file_collisions() {
    use std::os::windows::process::CommandExt;
    let sandbox = crate::test_sandbox::sandbox();
    let capability = RuntimeCapability::new().unwrap();
    let mut profile = Profile::new(&capability).unwrap();
    profile.make_low_profile().unwrap();
    let outside = sandbox.home().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), b"preserved").unwrap();
    let profile_name = String::from_utf16(&profile.name[..profile.name.len() - 1]).unwrap();
    let components = ["Packages", profile_name.as_str(), "AC", "Temp"];
    for depth in 1..=components.len() {
        for reparse in [false, true] {
            let mut task = task_directory(&current_user().unwrap(), profile.sid.0).unwrap();
            let root = task.0.clone();
            let home = root.join("home");
            fs::create_dir(&home).unwrap();
            let mut collision = home.clone();
            for component in &components[..depth] {
                collision.push(component);
            }
            fs::create_dir_all(collision.parent().unwrap()).unwrap();
            if reparse {
                // Relative arguments avoid cmd's handling of verbatim paths.
                let target =
                    PathBuf::from("..").join(outside.strip_prefix(root.parent().unwrap()).unwrap());
                let output = std::process::Command::new("cmd.exe")
                    .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
                    .current_dir(&root)
                    .args(["/D", "/C", "mklink", "/J"])
                    .arg(collision.strip_prefix(&root).unwrap())
                    .arg(target)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "create owned junction refusal control"
                );
                assert!(fs::symlink_metadata(&collision)
                    .is_ok_and(|metadata| metadata.file_attributes()
                        & FILE_ATTRIBUTE_REPARSE_POINT.0
                        != 0));
            } else {
                fs::write(&collision, b"collision preserved").unwrap();
            }
            assert!(
                profile.prepare_private_temp(&home).is_err(),
                "private temp preparation accepted a colliding object"
            );
            if !reparse {
                assert!(fs::read(&collision).is_ok_and(|bytes| bytes == b"collision preserved"));
            }
            assert!(
                fs::read(outside.join("sentinel")).is_ok_and(|bytes| bytes == b"preserved")
                    && fs::read_dir(&outside).unwrap().count() == 1,
                "private temp preparation changed a junction target"
            );
            task.dispose().unwrap();
            assert!(!root.exists(), "collision fixture scratch survived cleanup");
            assert!(fs::read(outside.join("sentinel")).is_ok_and(|bytes| bytes == b"preserved"));
        }
    }
    profile.dispose().unwrap();
}
