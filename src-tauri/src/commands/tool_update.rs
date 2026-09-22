//! Source-anchored CLI updates, following cc-switch's lifecycle design.
//! Never fall back to another package manager when the active install is unknown.
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
static BUSY: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// Shared with the installer so two entry points cannot mutate one tool together.
pub(crate) struct ToolOperationGuard(String);
impl ToolOperationGuard {
    pub(crate) fn acquire(tool: &str) -> Result<Self, String> {
        let mut busy = BUSY.lock().map_err(|e| e.to_string())?;
        if !busy.insert(tool.to_string()) {
            return Err("An installation or update is already running for this tool".into());
        }
        Ok(Self(tool.to_string()))
    }
}
impl Drop for ToolOperationGuard {
    fn drop(&mut self) {
        if let Ok(mut busy) = BUSY.lock() {
            busy.remove(&self.0);
        }
    }
}

pub(crate) fn npm_package(tool: &str) -> Option<&'static str> {
    match tool {
        "claude" => Some("@anthropic-ai/claude-code"),
        "codex" => Some("@openai/codex"),
        "gemini" => Some("@google/gemini-cli"),
        "opencode" => Some("opencode-ai"),
        "openclaw" => Some("openclaw"),
        _ => None,
    }
}

pub(crate) fn version_status(current: Option<&str>, latest: Option<&str>) -> &'static str {
    let Some(current) = current else {
        return "notInstalled";
    };
    let Some(latest) = latest else {
        return "failed";
    };
    match (parse_version(current), parse_version(latest)) {
        (Ok(current), Ok(latest)) if latest.cmp_precedence(&current).is_gt() => "available",
        (Ok(_), Ok(_)) => "current",
        _ => "unknown",
    }
}

fn parse_version(value: &str) -> Result<semver::Version, semver::Error> {
    semver::Version::parse(value.trim().trim_start_matches('v'))
}

#[derive(Debug, Clone)]
pub(crate) struct Installation {
    pub path: PathBuf,
    pub real: PathBuf,
    pub version: String,
    pub search_path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct UpdatePlan {
    pub source: &'static str,
    program: PathBuf,
    args: Vec<String>,
    path: String,
}

/// Match the login + interactive shell used by launch_tool. Markers discard rc output.
pub(crate) async fn probe(tool: &str) -> Result<Installation, String> {
    if npm_package(tool).is_none() && tool != "hermes" {
        return Err("Unsupported CLI".into());
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut command = Command::new(shell);
    command.args(["-l", "-i", "-c", &format!(
        "printf '\\n__OFOX_BIN__%s\\n' \"$(command -v {tool})\"; printf '__OFOX_PATH__%s\\n' \"$PATH\""
    )]);
    let output = bounded_output(command).await?;
    let text = String::from_utf8_lossy(&output.stdout);
    let path = text
        .lines()
        .find_map(|s| s.strip_prefix("__OFOX_BIN__"))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
        .ok_or("No executable in the launch shell PATH (aliases/functions are not auto-updated)")?;
    let search_path = text
        .lines()
        .find_map(|s| s.strip_prefix("__OFOX_PATH__"))
        .ok_or("Could not read launch shell PATH")?
        .to_string();
    let real = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
    let mut command = Command::new(&path);
    command.arg("--version").env("PATH", &search_path);
    let output = bounded_output(command).await?;
    if !output.status.success() {
        return Err("Active executable failed its version check".into());
    }
    let text = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let regex =
        regex::Regex::new(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?").unwrap();
    let version = regex
        .find(&text)
        .ok_or("Unrecognized version output")?
        .as_str()
        .to_string();
    Ok(Installation {
        path,
        real,
        version,
        search_path,
    })
}

async fn bounded_output(mut command: Command) -> Result<std::process::Output, String> {
    command
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    tokio::time::timeout(PROBE_TIMEOUT, command.output())
        .await
        .map_err(|_| "Version probe timed out".to_string())?
        .map_err(|e| e.to_string())
}

/// Pure source classification. Callers validate the resulting executable before use.
pub(crate) fn plan(tool: &str, install: &Installation) -> Result<UpdatePlan, String> {
    let make = |source, program: PathBuf, args: Vec<String>| {
        let directory = program.parent().unwrap_or(Path::new("/usr/bin"));
        UpdatePlan {
            source,
            path: format!("{}:{}", directory.display(), install.search_path),
            program,
            args,
        }
    };
    // Canonical Homebrew ownership takes precedence over native self-update.
    for ancestor in install.real.ancestors() {
        if matches!(
            ancestor.file_name().and_then(|s| s.to_str()),
            Some("Cellar" | "Caskroom")
        ) {
            let relative = install
                .real
                .strip_prefix(ancestor)
                .map_err(|e| e.to_string())?;
            let formula = relative
                .components()
                .next()
                .ok_or("Missing brew formula")?
                .as_os_str()
                .to_string_lossy();
            let expected = match tool {
                "claude" => "claude-code",
                "gemini" => "gemini-cli",
                other => other,
            };
            if formula != expected {
                break;
            }
            let program = ancestor
                .parent()
                .ok_or("Missing brew prefix")?
                .join("bin/brew");
            let kind = if ancestor.ends_with("Caskroom") {
                "--cask"
            } else {
                "--formula"
            };
            return Ok(make(
                "homebrew",
                program,
                vec!["upgrade".into(), kind.into(), formula.into_owned()],
            ));
        }
    }
    if let Some(package) = npm_package(tool) {
        for ancestor in install.real.ancestors() {
            if ancestor.ends_with(Path::new("lib/node_modules").join(package)) {
                let mut prefix = ancestor;
                for _ in 0..(2 + package.split('/').count()) {
                    prefix = prefix.parent().ok_or("Missing npm prefix")?;
                }
                // Explicit --prefix prevents user npmrc from redirecting the write.
                return Ok(make(
                    "npm",
                    prefix.join("bin/npm"),
                    vec![
                        "install".into(),
                        "--global".into(),
                        "--prefix".into(),
                        prefix.to_string_lossy().into_owned(),
                        format!("{package}@latest"),
                    ],
                ));
            }
        }
    }
    let real = install.real.to_string_lossy();
    let native = match tool {
        "claude" => real.contains("/.local/share/claude/versions/"),
        "opencode" => real.contains("/.opencode/bin/"),
        // Hermes owns its Python environment and has an official update command.
        "hermes" => true,
        _ => false,
    };
    if native {
        let argument = if tool == "opencode" {
            "upgrade"
        } else {
            "update"
        };
        return Ok(make("native", install.path.clone(), vec![argument.into()]));
    }
    Err("Unknown installation source; update with the original installer/package manager".into())
}

pub(crate) fn verified_plan(tool: &str, install: &Installation) -> Result<UpdatePlan, String> {
    let plan = plan(tool, install)?;
    if !plan.program.is_file() {
        return Err("Original package manager is missing; update manually".into());
    }
    if plan.source == "npm" && !plan.program.with_file_name("node").is_file() {
        return Err("Original Node.js runtime is missing; update manually".into());
    }
    Ok(plan)
}

/// pnpm shims may outlive the pnpm version that created them. Confirm both
/// global bin and package root before selecting a manager (never migrate copies).
pub(crate) async fn resolve_plan(tool: &str, install: &Installation) -> Result<UpdatePlan, String> {
    tokio::time::timeout(PROBE_TIMEOUT, resolve_plan_inner(tool, install))
        .await
        .map_err(|_| {
            "Installation-source detection timed out; retry or update manually".to_string()
        })?
}

async fn resolve_plan_inner(tool: &str, install: &Installation) -> Result<UpdatePlan, String> {
    let original = verified_plan(tool, install);
    if original.is_ok() {
        return original;
    }
    let Some(package) = npm_package(tool) else {
        return original;
    };
    let Some(bin_dir) = install.path.parent() else {
        return original;
    };
    use std::io::Read;
    let mut shim = String::new();
    if let Ok(file) = std::fs::File::open(&install.path) {
        let _ = file.take(65536).read_to_string(&mut shim);
    }
    let target = shim
        .lines()
        .find_map(|line| line.strip_prefix("# cmd-shim-target="))
        .map(PathBuf::from)
        .unwrap_or_else(|| install.real.clone());
    if !target.is_absolute() || !target.is_file() || !target.starts_with(bin_dir.join("global")) {
        return original;
    }
    let mut candidates = vec![bin_dir.join("pnpm")];
    candidates.extend(std::env::split_paths(&install.search_path).map(|path| path.join("pnpm")));
    // pnpm 11 keeps downloaded package-manager versions in its local tools store.
    // Read only this bounded directory, not arbitrary user package contents.
    let versions = bin_dir.join("store/v11/links/@/pnpm");
    if let Ok(entries) = std::fs::read_dir(versions) {
        for entry in entries.flatten().take(16) {
            if let Ok(builds) = std::fs::read_dir(entry.path()) {
                candidates.extend(
                    builds
                        .flatten()
                        .take(4)
                        .map(|entry| entry.path().join("bin/pnpm")),
                );
            }
        }
    }
    let mut seen = HashSet::new();
    for candidate in candidates {
        let Ok(program) = std::fs::canonicalize(candidate) else {
            continue;
        };
        if !seen.insert(program.clone()) {
            continue;
        }
        let query = |argument| {
            let mut command = Command::new(&program);
            command
                .args([argument, "--global"])
                .env("PATH", &install.search_path)
                .current_dir(bin_dir);
            bounded_output(command)
        };
        let (root, bin) = tokio::join!(query("root"), query("bin"));
        let (Ok(root), Ok(bin)) = (root, bin) else {
            continue;
        };
        if !root.status.success() || !bin.status.success() {
            continue;
        }
        let root = PathBuf::from(String::from_utf8_lossy(&root.stdout).trim());
        let bin = PathBuf::from(String::from_utf8_lossy(&bin.stdout).trim());
        if std::fs::canonicalize(&bin).ok() != std::fs::canonicalize(bin_dir).ok() {
            continue;
        }
        let Some(global_dir) = pnpm_owned_root(&root, &target, package) else {
            continue;
        };
        return Ok(UpdatePlan {
            source: "pnpm",
            program,
            args: vec![
                "--dir".into(),
                bin_dir.to_string_lossy().into_owned(),
                "update".into(),
                "--global".into(),
                "--latest".into(),
                format!("--config.global-dir={}", global_dir.display()),
                format!("--config.global-bin-dir={}", bin_dir.display()),
                package.into(),
            ],
            path: install.search_path.clone(),
        });
    }
    Err("Could not find the pnpm version owning this installation; update using the original pnpm version".into())
}

fn pnpm_owned_root(root: &Path, target: &Path, package: &str) -> Option<PathBuf> {
    if target
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    if root.ends_with("node_modules") {
        target.strip_prefix(root.join(package)).ok()?;
        return root.parent()?.parent().map(Path::to_path_buf);
    }
    if root.ends_with("v11") {
        let relative = target.strip_prefix(root).ok()?;
        let group = relative.components().next()?.as_os_str();
        target
            .strip_prefix(root.join(group).join("node_modules").join(package))
            .ok()?;
        return root.parent().map(Path::to_path_buf);
    }
    None
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    tool: &'a str,
    operation_id: &'a str,
    stage: &'a str,
    detail: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    pub status: String,
    pub before: String,
    pub after: String,
}

fn verify_result(before: &str, after: &str, latest: &str) -> UpdateResult {
    let status = match (
        parse_version(before),
        parse_version(after),
        parse_version(latest),
    ) {
        (Ok(before), Ok(after), Ok(latest))
            if after.cmp_precedence(&before).is_gt() && !after.cmp_precedence(&latest).is_lt() =>
        {
            "updated"
        }
        _ => "unchanged",
    };
    UpdateResult {
        status: status.into(),
        before: before.into(),
        after: after.into(),
    }
}

#[tauri::command]
pub async fn update_tool(
    app: AppHandle,
    tool: String,
    operation_id: String,
) -> Result<UpdateResult, String> {
    if !cfg!(target_os = "macos") {
        return Err("Automatic updates are currently supported on macOS only".into());
    }
    if operation_id.is_empty() || operation_id.len() > 128 {
        return Err("Invalid operation ID".into());
    }
    let _guard = ToolOperationGuard::acquire(&tool)?;
    let emit = |stage: &str, detail: &str| {
        let _ = app.emit(
            "tool-update-progress",
            Progress {
                tool: &tool,
                operation_id: &operation_id,
                stage,
                detail,
            },
        );
    };
    let result = perform_update(&tool, &emit).await;
    match &result {
        Ok(_) => emit("done", ""),
        Err(error) => emit("failed", error),
    }
    result
}

async fn perform_update<F: Fn(&str, &str)>(tool: &str, emit: F) -> Result<UpdateResult, String> {
    emit("checking", "");
    let install = probe(tool).await?;
    let latest = super::misc::latest_tool_version(tool, Some(&install.version))
        .await
        .ok_or("Could not check latest stable version; retry later")?;
    if version_status(Some(&install.version), Some(&latest)) != "available" {
        return Ok(UpdateResult {
            status: "current".into(),
            before: install.version.clone(),
            after: install.version,
        });
    }
    let plan = resolve_plan(tool, &install).await?;
    emit("updating", plan.source);
    let mut command = Command::new(&plan.program);
    command
        .args(&plan.args)
        .env("PATH", &plan.path)
        .env("CI", "1");
    run_update_process(command, |line| emit("log", line), UPDATE_TIMEOUT).await?;
    emit("verifying", "");
    let after = probe(tool).await?;
    let after_plan = resolve_plan(tool, &after).await?;
    // fnm creates a fresh multishell symlink on every login. Compare the
    // canonical package-manager target, not that ephemeral launcher path.
    if after_plan.source != plan.source
        || after_plan.program != plan.program
        || after_plan.args != plan.args
    {
        return Err(
            "The launch PATH changed during update; inspect the active installation".into(),
        );
    }
    Ok(verify_result(&install.version, &after.version, &latest))
}

/// Stream both pipes before waiting, so a verbose package manager cannot deadlock.
async fn run_update_process<F: Fn(&str)>(
    mut command: Command,
    log: F,
    timeout: Duration,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    command
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let _group = ProcessGroup(child.id().ok_or("Missing process ID")?);
    let stdout = BufReader::new(child.stdout.take().ok_or("Missing stdout")?).lines();
    let stderr = BufReader::new(child.stderr.take().ok_or("Missing stderr")?).lines();
    let stdout_task = async {
        let mut lines = stdout;
        while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            log(&line);
        }
        Ok::<(), String>(())
    };
    let stderr_task = async {
        let mut lines = stderr;
        while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            log(&line);
        }
        Ok::<(), String>(())
    };
    let work = async {
        let (out, err, status) = tokio::join!(stdout_task, stderr_task, child.wait());
        out?;
        err?;
        let status = status.map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "Update process exited with {status}; see update log"
            ))
        }
    };
    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| "Update timed out; inspect the installation before retrying".to_string())?
}

#[cfg(unix)]
struct ProcessGroup(u32);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // SAFETY: this child leads a dedicated group. Stop descendants before unlocking.
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn codex_desktop_version() -> Option<(String, String)> {
    for directory in [
        PathBuf::from("/Applications"),
        crate::config::get_home_dir().join("Applications"),
    ] {
        for name in ["ChatGPT.app", "Codex.app"] {
            let path = directory.join(name);
            let plist = path.join("Contents/Info.plist");
            if !plist.is_file() {
                continue;
            }
            let mut command = Command::new("/usr/libexec/PlistBuddy");
            command
                .args(["-c", "Print :CFBundleIdentifier"])
                .arg(&plist);
            let Ok(output) = bounded_output(command).await else {
                continue;
            };
            if String::from_utf8_lossy(&output.stdout).trim() != "com.openai.codex" {
                continue;
            }
            let mut command = Command::new("/usr/libexec/PlistBuddy");
            command
                .args(["-c", "Print :CFBundleShortVersionString"])
                .arg(&plist);
            let Ok(output) = bounded_output(command).await else {
                continue;
            };
            if !output.status.success() {
                continue;
            }
            return Some((
                path.to_string_lossy().into_owned(),
                String::from_utf8_lossy(&output.stdout).trim().into(),
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pnpm_roots_reject_other_versions_packages_and_traversal() {
        let root = Path::new("/pnpm/global/v11");
        assert_eq!(
            pnpm_owned_root(
                root,
                Path::new("/pnpm/global/v11/group/node_modules/opencode-ai/bin/opencode"),
                "opencode-ai"
            ),
            Some(PathBuf::from("/pnpm/global"))
        );
        for path in [
            "/pnpm/global/5/node_modules/opencode-ai/bin/opencode",
            "/pnpm/global/v11/group/node_modules/other/bin/opencode",
            "/pnpm/global/v11/../group/node_modules/opencode-ai/bin/opencode",
        ] {
            assert!(pnpm_owned_root(root, Path::new(path), "opencode-ai").is_none());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pnpm_uses_matching_cached_manager_not_another_global_install() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let target = home.join("global/v11/group/node_modules/opencode-ai/bin/opencode");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "binary").unwrap();
        let wrapper = home.join("opencode");
        std::fs::write(
            &wrapper,
            format!("#!/bin/sh\n# cmd-shim-target={}\n", target.display()),
        )
        .unwrap();
        let cached = home.join("store/v11/links/@/pnpm/11.6.0/build/bin/pnpm");
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        for (manager, root) in [
            (home.join("pnpm"), home.join("global/5/node_modules")),
            (cached.clone(), home.join("global/v11")),
        ] {
            std::fs::write(
                &manager,
                format!(
                    "#!/bin/sh\ncase \"$1\" in root) echo '{}';; bin) echo '{}';; esac\n",
                    root.display(),
                    home.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(manager, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let installation = install(wrapper.to_str().unwrap(), wrapper.to_str().unwrap());
        let plan = resolve_plan("opencode", &installation).await.unwrap();
        assert_eq!(plan.source, "pnpm");
        assert_eq!(plan.program, std::fs::canonicalize(cached).unwrap());
        assert!(plan.args.contains(&format!(
            "--config.global-dir={}",
            home.join("global").display()
        )));
        assert_eq!(plan.args.last().unwrap(), "opencode-ai");
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "Manual installed-tool and network probe"]
    async fn manual_probe_installed_tools() {
        for tool in [
            "claude", "codex", "gemini", "opencode", "openclaw", "hermes",
        ] {
            match probe(tool).await {
                Ok(install) => {
                    let latest =
                        super::super::misc::latest_tool_version(tool, Some(&install.version)).await;
                    println!(
                        "{tool}: current={} latest={latest:?} path={} real={} plan={:?}",
                        install.version,
                        install.path.display(),
                        install.real.display(),
                        resolve_plan(tool, &install).await
                    );
                }
                Err(error) => println!("{tool}: {error}"),
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "Explicit manual acceptance: upgrades one actual npm installation"]
    async fn manual_update_npm_tool() {
        let tool =
            std::env::var("OFOX_TEST_UPDATE_TOOL").expect("Set OFOX_TEST_UPDATE_TOOL explicitly");
        let _guard = ToolOperationGuard::acquire(&tool).unwrap();
        let install = probe(&tool).await.unwrap();
        assert_eq!(verified_plan(&tool, &install).unwrap().source, "npm");
        let result = perform_update(&tool, |stage, line| println!("{stage}: {line}"))
            .await
            .unwrap();
        println!("{result:?}");
        assert_eq!(
            result.status, "updated",
            "Acceptance requires an actual version change"
        );
    }
    fn install(path: &str, real: &str) -> Installation {
        Installation {
            path: path.into(),
            real: real.into(),
            version: "1.0.0".into(),
            search_path: "/usr/bin:/bin".into(),
        }
    }
    #[test]
    fn semantic_versions_and_failed_queries() {
        assert_eq!(version_status(Some("1.9.0"), Some("1.10.0")), "available");
        assert_eq!(
            version_status(Some("2.0.0-beta.1"), Some("2.0.0")),
            "available"
        );
        assert_eq!(version_status(Some("2.0.0"), Some("1.0.0")), "current");
        assert_eq!(
            version_status(Some("2.0.0+build"), Some("2.0.0+other")),
            "current"
        );
        assert_eq!(version_status(Some("1.0.0"), None), "failed");
        assert_eq!(version_status(Some("installed"), Some("1.0.0")), "unknown");
    }
    #[test]
    fn update_is_anchored_to_actual_npm_prefix() {
        let plan = plan(
            "codex",
            &install(
                "/other/bin/codex",
                "/Users/O'Brien/node/lib/node_modules/@openai/codex/bin/codex.js",
            ),
        )
        .unwrap();
        assert_eq!(plan.program, PathBuf::from("/Users/O'Brien/node/bin/npm"));
        assert_eq!(plan.args[3], "/Users/O'Brien/node");
        assert!(plan.path.starts_with("/Users/O'Brien/node/bin:"));
    }
    #[test]
    fn fnm_multishell_changes_keep_the_same_update_target() {
        let real = "/home/user/.local/share/fnm/node-versions/v24/installation/lib/node_modules/opencode-ai/bin/opencode";
        let first = plan(
            "opencode",
            &install(
                "/home/user/.local/state/fnm_multishells/1/bin/opencode",
                real,
            ),
        )
        .unwrap();
        let next = plan(
            "opencode",
            &install(
                "/home/user/.local/state/fnm_multishells/2/bin/opencode",
                real,
            ),
        )
        .unwrap();
        assert_eq!(first.program, next.program);
        assert_eq!(first.args, next.args);
        let other = plan(
            "opencode",
            &install(
                "/other/bin/opencode",
                "/other/lib/node_modules/opencode-ai/bin/opencode",
            ),
        )
        .unwrap();
        assert_ne!(first.program, other.program);
    }

    #[test]
    fn missing_original_package_manager_cannot_fall_back_to_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("lib/node_modules/@openai/codex/bin/codex.js");
        let install = install("/unrelated/bin/codex", path.to_str().unwrap());
        assert!(verified_plan("codex", &install)
            .unwrap_err()
            .contains("package manager is missing"));
    }
    #[test]
    fn brew_precedes_self_update_and_unknown_sources_stop() {
        let brew = plan(
            "claude",
            &install(
                "/opt/homebrew/bin/claude",
                "/opt/homebrew/Caskroom/claude-code/1.0/claude",
            ),
        )
        .unwrap();
        assert_eq!(brew.source, "homebrew");
        assert!(plan("gemini", &install("/custom/gemini", "/custom/gemini")).is_err());
        assert_eq!(
            plan(
                "claude",
                &install(
                    "/home/me/.local/bin/claude",
                    "/home/me/.local/share/claude/versions/1.0.0"
                )
            )
            .unwrap()
            .source,
            "native"
        );
    }
    #[test]
    fn duplicate_operation_lock_releases_on_drop() {
        let first = ToolOperationGuard::acquire("test-guard").unwrap();
        assert!(ToolOperationGuard::acquire("test-guard").is_err());
        drop(first);
        assert!(ToolOperationGuard::acquire("test-guard").is_ok());
    }
    #[test]
    fn successful_exit_does_not_prove_update() {
        assert_eq!(verify_result("1.0.0", "1.0.0", "1.1.0").status, "unchanged");
        assert_eq!(verify_result("1.0.0", "1.0.1", "1.1.0").status, "unchanged");
        assert_eq!(verify_result("1.0.0", "1.1.0", "1.1.0").status, "updated");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn process_streams_and_waits_for_exit_or_timeout() {
        let lines = Mutex::new(Vec::new());
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "echo stdout; echo stderr >&2; exit 7"]);
        let error = run_update_process(
            command,
            |s| lines.lock().unwrap().push(s.to_string()),
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
        assert!(error.contains('7'));
        assert_eq!(lines.lock().unwrap().len(), 2);
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec sleep 5"]);
        assert!(
            run_update_process(command, |_| {}, Duration::from_millis(20))
                .await
                .unwrap_err()
                .contains("timed out")
        );
    }
}
