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
    pub error: Option<String>,
    pub search_path: String,
}

/// Missing is only emitted after a successful shell lookup. Other probe errors
/// must not be presented as an uninstall, or cause Codex to change identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProbeError {
    NotFound,
    Failed(String),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("No executable in the launch shell PATH"),
            Self::Failed(error) => formatter.write_str(error),
        }
    }
}

impl From<String> for ProbeError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl From<&str> for ProbeError {
    fn from(error: &str) -> Self {
        Self::Failed(error.to_string())
    }
}

impl From<ProbeError> for String {
    fn from(error: ProbeError) -> Self {
        error.to_string()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DesktopInstallation {
    pub path: PathBuf,
    pub version: Option<String>,
    pub error: Option<String>,
}

pub(crate) fn candidate_exists(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Could not inspect {}: {error}", path.display())),
    }
}

pub(crate) fn shell_resolution(
    output: &std::process::Output,
) -> Result<(String, String), ProbeError> {
    if !output.status.success() {
        return Err(format!(
            "Launch shell lookup failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2048)
                .collect::<String>()
        )
        .into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let path = text
        .lines()
        .find_map(|line| line.strip_prefix("__OFOX_BIN__"))
        .ok_or("Launch shell did not return an executable lookup result")?;
    let search_path = text
        .lines()
        .find_map(|line| line.strip_prefix("__OFOX_PATH__"))
        .ok_or("Could not read launch shell PATH")?;
    if path.is_empty() {
        // Interactive rc files routinely write to stderr (e.g. fnm's "Using Node ..."),
        // so it is not evidence against the lookup. On macOS, a PATH left incomplete
        // by a broken profile is still covered by the scan that follows NotFound.
        if !output.stderr.is_empty() {
            log::debug!(
                "Launch shell stderr during executable lookup: {}",
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(2048)
                    .collect::<String>()
            );
        }
        return Err(ProbeError::NotFound);
    }
    if !path.starts_with('/') {
        return Err("Launch shell resolved an alias or function instead of an executable".into());
    }
    Ok((path.to_string(), search_path.to_string()))
}

#[derive(Debug, Clone)]
pub(crate) struct UpdatePlan {
    pub source: &'static str,
    program: PathBuf,
    args: Vec<String>,
    path: String,
}

/// Match the login + interactive shell used by launch_tool. Markers discard rc output.
pub(crate) async fn probe(tool: &str) -> Result<Installation, ProbeError> {
    if npm_package(tool).is_none() && tool != "hermes" {
        return Err("Unsupported CLI".into());
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut command = Command::new(shell);
    command.args(["-l", "-i", "-c", &format!(
        "printf '\\n__OFOX_BIN__%s\\n' \"$(command -v {tool})\"; printf '__OFOX_PATH__%s\\n' \"$PATH\""
    )]);
    let output = bounded_output(command).await?;
    let (path, search_path) = shell_resolution(&output)?;
    let path = PathBuf::from(path);
    if !candidate_exists(&path)? {
        return Err("Executable disappeared after the launch shell lookup".into());
    }
    let real = match std::fs::canonicalize(&path) {
        Ok(real) => real,
        Err(error) => {
            return Ok(Installation {
                real: path.clone(),
                path,
                version: String::new(),
                error: Some(format!("Active executable failed its path check: {error}")),
                search_path,
            });
        }
    };
    let mut command = Command::new(&path);
    command.arg("--version").env("PATH", &search_path);
    let output = bounded_output(command).await;
    let (version, error) = executable_version(output);
    Ok(Installation {
        path,
        real,
        version,
        error,
        search_path,
    })
}

pub(crate) fn executable_version(
    output: Result<std::process::Output, String>,
) -> (String, Option<String>) {
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return (
                String::new(),
                Some(format!(
                    "Active executable failed its version check: {error}"
                )),
            )
        }
    };
    let text = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let regex =
        regex::Regex::new(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?").unwrap();
    if output.status.success() {
        if let Some(version) = regex.find(&text) {
            return (version.as_str().to_string(), None);
        }
    }
    (
        String::new(),
        Some(format!(
            "Active executable failed its version check: {}",
            text.chars().take(2048).collect::<String>()
        )),
    )
}

pub(crate) async fn bounded_output(command: Command) -> Result<std::process::Output, String> {
    bounded_output_with_timeout(command, PROBE_TIMEOUT).await
}

pub(crate) async fn bounded_output_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    command
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| "Version probe timed out".to_string())?
        .map_err(|e| e.to_string())
}

#[cfg(test)]
pub(crate) fn probe_fixture_output(stdout: &str, stderr: &str, code: i32) -> std::process::Output {
    #[cfg(unix)]
    let status = {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    };
    #[cfg(windows)]
    let status = {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    };
    std::process::Output {
        status,
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
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
        // The scoped --allow-build lifecycle is supported by pnpm 11. Do not
        // silently use an older manager or globally disable build protections.
        if !root.ends_with("v11") {
            continue;
        }
        return Ok(UpdatePlan {
            source: "pnpm",
            program,
            args: vec![
                "--dir".into(),
                bin_dir.to_string_lossy().into_owned(),
                "add".into(),
                "--global".into(),
                format!("--allow-build={package}"),
                format!("--config.global-dir={}", global_dir.display()),
                format!("--config.global-bin-dir={}", bin_dir.display()),
                format!("{package}@latest"),
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

fn execution_args(plan: &UpdatePlan, install: &Installation, latest: &str) -> Vec<String> {
    let mut args = plan.args.clone();
    // A release-age policy may select an older version than dist-tags.latest.
    // Constrain a healthy installation so that this cannot downgrade it.
    if plan.source == "pnpm" && install.error.is_none() {
        if let Some(specifier) = args.last_mut() {
            if let Some(package) = specifier.strip_suffix("@latest") {
                *specifier = format!("{package}@>={} <={latest}", install.version);
            }
        }
    }
    args
}

#[tauri::command]
pub async fn update_tool(
    app: AppHandle,
    tool: String,
    operation_id: String,
) -> Result<UpdateResult, String> {
    if operation_id.is_empty() || operation_id.len() > 128 {
        return Err("Invalid operation ID".into());
    }
    if tool != "chatgpt" && !cfg!(target_os = "macos") {
        return Err("Automatic updates are currently supported on macOS only".into());
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
    let result = if tool == "chatgpt" {
        chatgpt_update(&emit).await
    } else {
        perform_update(&tool, &emit).await
    };
    match &result {
        Ok(_) => emit("done", ""),
        Err(error) => emit("failed", error),
    }
    result
}

/// ChatGPT 桌面 App：Windows 由 Ofox 走 winget/Store 升级；macOS 交给 App 内置 Sparkle。
#[cfg(target_os = "windows")]
async fn chatgpt_update<F: Fn(&str, &str) + Sync>(emit: &F) -> Result<UpdateResult, String> {
    super::windows_chatgpt::upgrade_chatgpt_desktop_app_with(emit).await
}

#[cfg(not(target_os = "windows"))]
async fn chatgpt_update<F: Fn(&str, &str) + Sync>(_emit: &F) -> Result<UpdateResult, String> {
    Err(
        "ChatGPT updates itself on this platform; open ChatGPT and choose Check for Updates…"
            .into(),
    )
}

/// 升级前确认桌面 App 是否在运行，好让用户先确认再关闭它。
#[tauri::command]
pub async fn is_tool_app_running(tool: String) -> Result<bool, String> {
    if tool != "chatgpt" {
        return Err(format!("Unsupported desktop app: {tool}"));
    }
    chatgpt_running().await
}

#[cfg(target_os = "windows")]
async fn chatgpt_running() -> Result<bool, String> {
    tokio::task::spawn_blocking(super::windows_chatgpt::chatgpt_process_ids)
        .await
        .map_err(|err| err.to_string())?
        .map(|ids| !ids.is_empty())
}

#[cfg(not(target_os = "windows"))]
async fn chatgpt_running() -> Result<bool, String> {
    Ok(false)
}

async fn perform_update<F: Fn(&str, &str)>(tool: &str, emit: F) -> Result<UpdateResult, String> {
    emit("checking", "");
    let install = probe(tool).await?;
    let latest = super::misc::latest_tool_version(tool, Some(&install.version))
        .await
        .ok_or("Could not check latest stable version; retry later")?;
    if install.error.is_none()
        && version_status(Some(&install.version), Some(&latest)) != "available"
    {
        return Ok(UpdateResult {
            status: "current".into(),
            before: install.version.clone(),
            after: install.version,
        });
    }
    let plan = resolve_plan(tool, &install).await?;
    if install.error.is_some() && plan.source != "pnpm" {
        return Err("Repair is currently supported only for verified pnpm 11 installations; use the original installer".into());
    }
    emit("updating", plan.source);
    let mut command = Command::new(&plan.program);
    command
        .args(execution_args(&plan, &install, &latest))
        .env("PATH", &plan.path)
        .env("CI", "1");
    run_update_process(command, |line| emit("log", line), UPDATE_TIMEOUT).await?;
    emit("verifying", "");
    let after = probe(tool).await?;
    if let Some(error) = &after.error {
        return Err(format!(
            "Installation is present but cannot run. Retry repair. {error}"
        ));
    }
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
    if install.error.is_some() {
        return Ok(UpdateResult {
            status: "repaired".into(),
            before: install.version,
            after: after.version,
        });
    }
    if version_status(Some(&after.version), Some(&latest)) == "available" {
        emit("log", "The runnable version is below the registry latest. Check the package manager's release-age policy and registry; safety policies were not disabled.");
    }
    Ok(verify_result(&install.version, &after.version, &latest))
}

async fn run_update_process<F: Fn(&str)>(
    command: Command,
    log: F,
    timeout: Duration,
) -> Result<(), String> {
    let status = run_logged_process(command, log, timeout).await?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Update process exited with {status}; see update log"
        ))
    }
}

/// Stream both pipes before waiting, so a verbose package manager cannot deadlock.
/// Lines are decoded lossily: localized winget/npm output must not abort the update.
pub(crate) async fn run_logged_process<F: Fn(&str)>(
    mut command: Command,
    log: F,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
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
    let stdout = BufReader::new(child.stdout.take().ok_or("Missing stdout")?);
    let stderr = BufReader::new(child.stderr.take().ok_or("Missing stderr")?);
    let work = async {
        let (out, err, status) = tokio::join!(
            forward_lines(stdout, &log),
            forward_lines(stderr, &log),
            child.wait()
        );
        out?;
        err?;
        status.map_err(|e| e.to_string())
    };
    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| "Update timed out; inspect the installation before retrying".to_string())?
}

async fn forward_lines<R, F>(mut reader: BufReader<R>, log: &F) -> Result<(), String>
where
    R: tokio::io::AsyncRead + Unpin,
    F: Fn(&str),
{
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        if reader
            .read_until(b'\n', &mut buffer)
            .await
            .map_err(|e| e.to_string())?
            == 0
        {
            return Ok(());
        }
        let line = String::from_utf8_lossy(&buffer);
        let line = line.trim_end_matches(['\r', '\n']);
        // Progress bars redraw with a bare \r; keep only the final frame.
        log(line.rsplit('\r').next().unwrap_or(line));
    }
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
pub(crate) async fn codex_desktop_version() -> Result<Option<DesktopInstallation>, String> {
    let candidates = [
        PathBuf::from("/Applications"),
        crate::config::get_home_dir().join("Applications"),
    ]
    .into_iter()
    .flat_map(|directory| [directory.join("ChatGPT.app"), directory.join("Codex.app")])
    .collect::<Vec<_>>();
    detect_macos_apps(&candidates, "com.openai.codex").await
}

#[cfg(target_os = "macos")]
pub(crate) async fn detect_macos_apps(
    candidates: &[PathBuf],
    bundle_id: &str,
) -> Result<Option<DesktopInstallation>, String> {
    let mut first_error = None;
    for path in candidates {
        let detected = async {
            if !candidate_exists(path)? {
                return Ok(None);
            }
            let plist = path.join("Contents/Info.plist");
            let mut command = Command::new("/usr/bin/plutil");
            command.args(["-convert", "json", "-o", "-"]).arg(&plist);
            let output = bounded_output(command).await?;
            if !output.status.success() {
                return Err(format!(
                    "Could not read application metadata: {}",
                    plist.display()
                ));
            }
            let json: serde_json::Value =
                serde_json::from_slice(&output.stdout).map_err(|error| {
                    format!("Invalid application metadata {}: {error}", plist.display())
                })?;
            desktop_from_metadata(path, bundle_id, &json)
        }
        .await;
        match detected {
            Ok(Some(app)) => return Ok(Some(app)),
            Ok(None) => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(None),
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn desktop_from_metadata(
    path: &Path,
    bundle_id: &str,
    json: &serde_json::Value,
) -> Result<Option<DesktopInstallation>, String> {
    let detected_id = json
        .get("CFBundleIdentifier")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("Invalid bundle identifier in {}", path.display()))?;
    if detected_id != bundle_id {
        return Ok(None);
    }
    let version = ["CFBundleShortVersionString", "CFBundleVersion"]
        .into_iter()
        .filter_map(|key| json.get(key).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string);
    let error = version
        .is_none()
        .then(|| "Installed application has no version metadata".to_string());
    Ok(Some(DesktopInstallation {
        path: path.to_path_buf(),
        version,
        error,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_output(stdout: &str, code: i32) -> std::process::Output {
        probe_fixture_output(stdout, "", code)
    }

    #[test]
    fn shell_lookup_only_reports_missing_after_a_complete_successful_lookup() {
        let absent = "__OFOX_BIN__\n__OFOX_PATH__/usr/bin:/bin\n";
        assert_eq!(
            shell_resolution(&shell_output(absent, 0)),
            Err(ProbeError::NotFound)
        );
        for output in [
            shell_output(absent, 1),
            shell_output("shell startup failed", 0),
            shell_output("__OFOX_BIN__\n", 0),
            shell_output("__OFOX_BIN__alias tool=other\n__OFOX_PATH__/bin\n", 0),
        ] {
            assert!(matches!(
                shell_resolution(&output),
                Err(ProbeError::Failed(_))
            ));
        }
        assert_eq!(
            shell_resolution(&shell_output(
                "startup noise\n__OFOX_BIN__/opt/bin/codex\n__OFOX_PATH__/opt/bin:/bin\n",
                0
            )),
            Ok(("/opt/bin/codex".into(), "/opt/bin:/bin".into()))
        );
    }

    #[test]
    fn shell_startup_stderr_does_not_turn_a_complete_lookup_into_a_failure() {
        let absent = "__OFOX_BIN__\n__OFOX_PATH__/usr/bin:/bin\n";
        for stderr in ["Using Node v22.12.0\n", "profile: Permission denied"] {
            assert_eq!(
                shell_resolution(&probe_fixture_output(absent, stderr, 0)),
                Err(ProbeError::NotFound)
            );
        }
    }

    #[test]
    fn failed_version_process_does_not_accept_a_version_from_its_error_message() {
        let (version, error) = executable_version(Ok(probe_fixture_output(
            "",
            "node 22.1.0 could not start: Permission denied",
            1,
        )));
        assert!(version.is_empty());
        assert!(error.as_deref().unwrap().contains("Permission denied"));
        let (version, error) = executable_version(Err("Access is denied. (os error 5)".into()));
        assert!(version.is_empty());
        assert!(error.as_deref().unwrap().contains("os error 5"));
    }

    #[test]
    #[ignore = "Subprocess fixture; invoked only by bounded_probe_times_out_for_an_isolated_slow_child"]
    fn probe_timeout_fixture_child() {
        if std::env::var_os("OFOX_PROBE_TIMEOUT_FIXTURE_CHILD").is_some() {
            std::thread::sleep(Duration::from_secs(3));
        }
    }

    #[tokio::test]
    async fn bounded_probe_times_out_for_an_isolated_slow_child() {
        // Use this test executable instead of a shell, WSL, or an installed
        // agent. The child is safe on every host and receives no user config.
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "commands::tool_update::tests::probe_timeout_fixture_child",
                "--ignored",
                "--nocapture",
            ])
            .env("OFOX_PROBE_TIMEOUT_FIXTURE_CHILD", "1");
        let result = bounded_output_with_timeout(command, Duration::from_millis(100)).await;
        assert!(result.unwrap_err().contains("timed out"));
    }

    #[tokio::test]
    async fn a_missing_probe_program_is_a_query_failure() {
        let temp = tempfile::tempdir().unwrap();
        let command = Command::new(temp.path().join("missing-query.exe"));
        let error = bounded_output_with_timeout(command, Duration::from_secs(1))
            .await
            .unwrap_err();
        let (version, error) = executable_version(Err(error));
        assert!(version.is_empty());
        assert!(error.is_some());
    }

    #[test]
    fn desktop_presence_does_not_depend_on_a_version_string() {
        let path = Path::new("/Applications/Tool.app");
        let app = desktop_from_metadata(
            path,
            "com.example.tool",
            &serde_json::json!({
                "CFBundleIdentifier": "com.example.tool"
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(app.path, path);
        assert!(app.version.is_none());
        assert!(app.error.is_some());
        assert!(desktop_from_metadata(path, "com.example.tool", &serde_json::json!({})).is_err());
        assert!(desktop_from_metadata(
            path,
            "com.example.tool",
            &serde_json::json!({
                "CFBundleIdentifier": "com.example.other"
            })
        )
        .unwrap()
        .is_none());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn desktop_probe_distinguishes_absence_from_invalid_metadata_and_accepts_another_valid_copy(
    ) {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("Missing.app");
        assert!(detect_macos_apps(&[missing], "com.example.tool")
            .await
            .unwrap()
            .is_none());
        let broken = temp.path().join("Broken.app");
        std::fs::create_dir_all(broken.join("Contents")).unwrap();
        std::fs::write(broken.join("Contents/Info.plist"), "broken plist").unwrap();
        assert!(
            detect_macos_apps(std::slice::from_ref(&broken), "com.example.tool")
                .await
                .is_err()
        );
        let valid = temp.path().join("Valid.app");
        std::fs::create_dir_all(valid.join("Contents")).unwrap();
        std::fs::write(valid.join("Contents/Info.plist"), r#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.example.tool</string><key>CFBundleVersion</key><string>1.2.3</string></dict></plist>"#).unwrap();
        let app = detect_macos_apps(&[broken, valid.clone()], "com.example.tool")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(app.path, valid);
        assert_eq!(app.version.as_deref(), Some("1.2.3"));
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_cli_shim_is_evidence_of_a_broken_installation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tool");
        assert!(!candidate_exists(&path).unwrap());
        std::os::unix::fs::symlink(temp.path().join("missing-target"), &path).unwrap();
        assert!(candidate_exists(&path).unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn broken_installation_preserves_diagnostic_not_a_version_from_error() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "echo '1.18.32 postinstall script was not run' >&2; exit 1",
        ]);
        let (version, error) = executable_version(bounded_output(command).await);
        assert!(version.is_empty());
        assert!(error.unwrap().contains("postinstall script was not run"));
        let (version, error) = executable_version(Err("Version probe timed out".into()));
        assert!(version.is_empty());
        assert!(error.unwrap().starts_with("Active executable failed"));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "Explicit manual acceptance: repairs/upgrades the actual pnpm OpenCode installation"]
    async fn manual_repair_pnpm_opencode() {
        assert_eq!(std::env::var("OFOX_TEST_UPDATE_TOOL").unwrap(), "opencode");
        let _guard = ToolOperationGuard::acquire("opencode").unwrap();
        let install = probe("opencode").await.unwrap();
        assert_eq!(
            resolve_plan("opencode", &install).await.unwrap().source,
            "pnpm"
        );
        let result = perform_update("opencode", |stage, line| println!("{stage}: {line}"))
            .await
            .unwrap();
        println!("{result:?}");
        let after = probe("opencode").await.unwrap();
        assert!(after.error.is_none(), "{:?}", after.error);
        assert!(!after.version.is_empty());
    }
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
        let selected = resolve_plan("opencode", &installation).await.unwrap();
        assert!(selected
            .args
            .contains(&"--allow-build=opencode-ai".to_string()));
        assert!(selected.args.contains(&"opencode-ai@latest".to_string()));
        assert_eq!(
            execution_args(&selected, &installation, "1.1.0")
                .last()
                .unwrap(),
            "opencode-ai@>=1.0.0 <=1.1.0"
        );
        assert!(!selected
            .args
            .iter()
            .any(|arg| arg.contains("dangerously") || arg.contains("minimum-release-age")));
        let plan = resolve_plan("opencode", &installation).await.unwrap();
        assert_eq!(plan.source, "pnpm");
        assert_eq!(plan.program, std::fs::canonicalize(cached).unwrap());
        assert!(plan.args.contains(&format!(
            "--config.global-dir={}",
            home.join("global").display()
        )));
        assert_eq!(plan.args.last().unwrap(), "opencode-ai@latest");
        // A manager returning exit 0 is insufficient: simulate a package whose
        // runnable entry is created only by its explicitly allowed postinstall.
        std::fs::write(&plan.program, format!(
            "#!/bin/sh\ncase \" $* \" in *' --allow-build=opencode-ai '*) printf '#!/bin/sh\\necho 1.1.0\\n' > '{}'; chmod +x '{}';; *) exit 9;; esac\n",
            target.display(), target.display()
        )).unwrap();
        let mut command = Command::new(&plan.program);
        command.args(&plan.args);
        run_update_process(command, |_| {}, Duration::from_secs(2))
            .await
            .unwrap();
        let mut command = Command::new(&target);
        command.arg("--version");
        let (version, error) = executable_version(bounded_output(command).await);
        assert_eq!(version, "1.1.0");
        assert!(error.is_none());
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
            error: None,
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
        let prefix = PathBuf::from("/Users/O'Brien/node");
        let directory = prefix.join("bin");
        let plan = plan(
            "codex",
            &install(
                "/other/bin/codex",
                "/Users/O'Brien/node/lib/node_modules/@openai/codex/bin/codex.js",
            ),
        )
        .unwrap();
        assert_eq!(plan.program, directory.join("npm"));
        assert_eq!(plan.args[3], prefix.display().to_string());
        assert!(plan.path.starts_with(&format!("{}:", directory.display())));
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

    #[cfg(unix)]
    #[tokio::test]
    async fn logged_process_reports_exit_status_and_survives_invalid_utf8() {
        let lines = Mutex::new(Vec::new());
        let mut command = Command::new("/bin/sh");
        command.args(["-c", r"printf 'ok\r\n\377\n'; exit 3"]);
        let status = run_logged_process(
            command,
            |s| lines.lock().unwrap().push(s.to_string()),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(status.code(), Some(3));
        assert_eq!(
            *lines.lock().unwrap(),
            vec!["ok".to_string(), "\u{FFFD}".to_string()]
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn chatgpt_update_is_left_to_the_app_off_windows() {
        let error = chatgpt_update(&|_: &str, _: &str| {}).await.unwrap_err();
        assert!(error.contains("Check for Updates"), "{error}");
    }

    #[tokio::test]
    async fn only_chatgpt_reports_a_running_app() {
        assert!(is_tool_app_running("claude".into()).await.is_err());
        #[cfg(not(target_os = "windows"))]
        assert!(!is_tool_app_running("chatgpt".into()).await.unwrap());
    }
}
