#![allow(non_snake_case)]

use crate::app_config::AppType;
use crate::init_status::{InitErrorPayload, SkillsMigrationPayload};
use crate::services::ProviderService;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tauri::AppHandle;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use super::tool_update::{DesktopInstallation, ProbeError};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 打开外部链接
#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<bool, String> {
    let url = if url.starts_with("http://") || url.starts_with("https://") {
        url
    } else {
        format!("https://{url}")
    };

    app.opener()
        .open_url(&url, None::<String>)
        .map_err(|e| format!("打开链接失败: {e}"))?;

    Ok(true)
}

#[tauri::command]
pub async fn copy_text_to_clipboard(text: String) -> Result<bool, String> {
    // Use spawn_blocking to avoid blocking the async runtime
    // Clipboard access can block on some platforms and may have thread/loop constraints
    tokio::task::spawn_blocking(move || {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("访问系统剪贴板失败: {e}"))?;
        clipboard
            .set_text(text)
            .map_err(|e| format!("写入系统剪贴板失败: {e}"))?;
        Ok(true)
    })
    .await
    .map_err(|e| format!("剪贴板任务执行失败: {e}"))?
}

/// Ofox Desktop 自托管更新清单地址（Cloudflare R2 + 自定义域）。
/// 结构见 `scripts/release.sh` 与 `tool-release` skill。
const OFOX_UPDATE_MANIFEST_URL: &str = "https://desktop.ofox.ai/latest.json";

/// `latest.json` 的反序列化结构。`downloads` 的 key 形如 `darwin-aarch64`，
/// 与 `current_platform_key()` 对齐。
#[derive(Debug, serde::Deserialize)]
struct UpdateManifest {
    version: String,
    #[serde(default)]
    pub_date: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    downloads: HashMap<String, String>,
    #[serde(default)]
    download_page: Option<String>,
}

/// 返回给前端的检查结果。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    /// 是否有比当前更高的版本
    has_update: bool,
    current_version: String,
    latest_version: String,
    /// 当前平台对应的安装包直链；选不到时回退到 download_page
    download_url: Option<String>,
    notes: Option<String>,
    pub_date: Option<String>,
}

/// `latest.json` 用的是 Tauri/通用三元组命名（darwin/windows + aarch64/x86_64），
/// 而 `std::env::consts::OS/ARCH` 是 Rust 命名（macos/windows + aarch64/x86_64）。
/// 这里把 Rust 命名映射成清单命名，避免两边各写一套。
fn manifest_platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other, // windows / linux 原样
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// 比较两个 `x.y.z[-pre]` 版本号，返回 `latest > current`。
/// 只比较主三段数字；预发布后缀（`-beta` 等）忽略，足够发布场景使用。
fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(v: &str) -> [u64; 3] {
        let core = v.trim().trim_start_matches('v');
        let core = core.split('-').next().unwrap_or(core);
        let mut out = [0u64; 3];
        for (i, part) in core.split('.').take(3).enumerate() {
            out[i] = part.parse().unwrap_or(0);
        }
        out
    }
    parse(latest) > parse(current)
}

/// 检查更新：拉取 R2 上的 `latest.json`，与当前版本比对，返回结构化结果。
/// 前端据此弹"前往下载"提示——本应用不做自动安装，引导用户手动下载。
#[tauri::command]
pub async fn check_for_updates() -> Result<UpdateCheckResult, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    let client = crate::proxy::http_client::get();
    let manifest: UpdateManifest = client
        .get(OFOX_UPDATE_MANIFEST_URL)
        .header("User-Agent", "ofox-desktop")
        .send()
        .await
        .map_err(|e| format!("获取更新清单失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("解析更新清单失败: {e}"))?;

    let has_update = is_newer(&manifest.version, &current_version);

    // 命中当前平台的直链；否则回退到通用下载页。
    let download_url = manifest
        .downloads
        .get(&manifest_platform_key())
        .cloned()
        .or(manifest.download_page.clone());

    Ok(UpdateCheckResult {
        has_update,
        current_version,
        latest_version: manifest.version,
        download_url,
        notes: manifest.notes,
        pub_date: manifest.pub_date,
    })
}

/// 判断是否为便携版（绿色版）运行
#[tauri::command]
pub async fn is_portable_mode() -> Result<bool, String> {
    let exe_path = std::env::current_exe().map_err(|e| format!("获取可执行路径失败: {e}"))?;
    if let Some(dir) = exe_path.parent() {
        Ok(dir.join("portable.ini").is_file())
    } else {
        Ok(false)
    }
}

/// 获取应用启动阶段的初始化错误（若有）。
/// 用于前端在早期主动拉取，避免事件订阅竞态导致的提示缺失。
#[tauri::command]
pub async fn get_init_error() -> Result<Option<InitErrorPayload>, String> {
    Ok(crate::init_status::get_init_error())
}

/// 获取 JSON→SQLite 迁移结果（若有）。
/// 只返回一次 true，之后返回 false，用于前端显示一次性 Toast 通知。
#[tauri::command]
pub async fn get_migration_result() -> Result<bool, String> {
    Ok(crate::init_status::take_migration_success())
}

/// 获取 Skills 自动导入（SSOT）迁移结果（若有）。
/// 只返回一次 Some({count})，之后返回 None，用于前端显示一次性 Toast 通知。
#[tauri::command]
pub async fn get_skills_migration_result() -> Result<Option<SkillsMigrationPayload>, String> {
    Ok(crate::init_status::take_skills_migration_result())
}

#[derive(serde::Serialize)]
pub struct ToolVersion {
    name: String,
    version: Option<String>,
    latest_version: Option<String>, // 新增字段：最新版本
    error: Option<String>,
    /// 工具运行环境: "windows", "wsl", "macos", "linux", "unknown"
    env_type: String,
    /// 当 env_type 为 "wsl" 时，返回该工具绑定的 WSL distro（用于按 distro 探测 shells）
    wsl_distro: Option<String>,
    #[serde(rename = "installationKind")]
    installation_kind: String,
    #[serde(rename = "installationStatus")]
    installation_status: InstallationStatus,
    update_status: String,
    update_source: Option<String>,
    update_supported: bool,
    update_reason: Option<String>,
    executable_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
enum InstallationStatus {
    Installed,
    NotInstalled,
    Unknown,
}

#[derive(Debug)]
struct LocalDetection {
    status: InstallationStatus,
    version: Option<String>,
    error: Option<String>,
    path: Option<PathBuf>,
}

impl LocalDetection {
    fn missing() -> Self {
        Self {
            status: InstallationStatus::NotInstalled,
            version: None,
            error: None,
            path: None,
        }
    }

    fn failed(error: impl Into<String>) -> Self {
        Self {
            status: InstallationStatus::Unknown,
            version: None,
            error: Some(error.into()),
            path: None,
        }
    }

    fn found(path: PathBuf, version: Option<String>, error: Option<String>) -> Self {
        Self {
            status: InstallationStatus::Installed,
            version,
            error,
            path: Some(path),
        }
    }
}

const VALID_TOOLS: [&str; 8] = [
    "claude",
    "codex",
    "chatgpt",
    "gemini",
    "opencode",
    "openclaw",
    "hermes",
    "workbuddy",
];

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WslShellPreferenceInput {
    #[serde(default)]
    pub wsl_shell: Option<String>,
    #[serde(default)]
    pub wsl_shell_flag: Option<String>,
}

// Keep platform-specific env detection in one place to avoid repeating cfg blocks.
#[cfg(target_os = "windows")]
fn tool_env_type_and_wsl_distro(tool: &str) -> (String, Option<String>) {
    if let Some(distro) = wsl_distro_for_tool(tool) {
        ("wsl".to_string(), Some(distro))
    } else {
        ("windows".to_string(), None)
    }
}

#[cfg(target_os = "macos")]
fn tool_env_type_and_wsl_distro(_tool: &str) -> (String, Option<String>) {
    ("macos".to_string(), None)
}

#[cfg(target_os = "linux")]
fn tool_env_type_and_wsl_distro(_tool: &str) -> (String, Option<String>) {
    ("linux".to_string(), None)
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn tool_env_type_and_wsl_distro(_tool: &str) -> (String, Option<String>) {
    ("unknown".to_string(), None)
}

#[cfg(target_os = "macos")]
const WORKBUDDY_MACOS_BUNDLE_ID: &str = "com.tencent.workbuddy.mac";

#[cfg(target_os = "macos")]
pub(crate) async fn find_workbuddy_app() -> Result<Option<DesktopInstallation>, String> {
    let candidates = [
        PathBuf::from("/Applications/WorkBuddy.app"),
        crate::config::get_home_dir()
            .join("Applications")
            .join("WorkBuddy.app"),
    ];
    super::tool_update::detect_macos_apps(&candidates, WORKBUDDY_MACOS_BUNDLE_ID).await
}

#[cfg(target_os = "windows")]
pub(crate) async fn find_workbuddy_app() -> Result<Option<DesktopInstallation>, String> {
    let (candidates, first_error) = workbuddy_discovery_sources(
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        workbuddy_registry_candidates(),
    );
    let Some(path) = select_workbuddy_candidate(&candidates, first_error)? else {
        return Ok(None);
    };
    let escaped = path.to_string_lossy().replace('\'', "''");
    let mut command = tokio::process::Command::new("powershell");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("(Get-Item -LiteralPath '{escaped}').VersionInfo.ProductVersion"),
        ])
        .creation_flags(CREATE_NO_WINDOW);
    Ok(Some(workbuddy_version_output(
        path,
        super::tool_update::bounded_output(command).await,
    )))
}

#[cfg(any(target_os = "windows", test))]
fn workbuddy_discovery_sources(
    local_app_data: Option<PathBuf>,
    registry: Result<Vec<PathBuf>, String>,
) -> (Vec<PathBuf>, Option<String>) {
    let mut candidates = Vec::new();
    let mut first_error = None;
    if let Some(local) = local_app_data {
        candidates.push(
            local
                .join("Programs")
                .join("WorkBuddy")
                .join("WorkBuddy.exe"),
        );
    } else {
        first_error = Some("Could not read LOCALAPPDATA for application discovery".into());
    }
    // A failed registry read must not conceal an installation found at the
    // usual path, but it prevents a conclusive missing result.
    match registry {
        Ok(paths) => candidates.extend(paths),
        Err(error) => {
            first_error.get_or_insert(error);
        }
    }
    (candidates, first_error)
}

#[cfg(any(target_os = "windows", test))]
fn select_workbuddy_candidate(
    candidates: &[PathBuf],
    mut first_error: Option<String>,
) -> Result<Option<PathBuf>, String> {
    for path in candidates {
        match super::tool_update::candidate_exists(path) {
            Ok(false) => continue,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
            Ok(true) => {}
        }
        return Ok(Some(path.clone()));
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(None),
    }
}

#[cfg(any(target_os = "windows", test))]
fn workbuddy_version_output(
    path: PathBuf,
    output: Result<std::process::Output, String>,
) -> DesktopInstallation {
    let (version, error) = match output {
        Ok(output) if output.status.success() => {
            let version = Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
                .filter(|version| !version.is_empty());
            let error = version
                .is_none()
                .then(|| "Installed WorkBuddy has no version metadata".into());
            (version, error)
        }
        Ok(_) => (
            None,
            Some("WorkBuddy version metadata could not be read".into()),
        ),
        Err(error) => (None, Some(error)),
    };
    DesktopInstallation {
        path,
        version,
        error,
    }
}

#[cfg(any(target_os = "windows", test))]
fn workbuddy_registry_candidate(field: &str, value: &str) -> Option<PathBuf> {
    let value = value.trim();
    // DisplayIcon may end in an icon resource index; commas in a directory
    // name or InstallLocation are part of the path.
    let value = if field == "DisplayIcon" {
        value
            .rsplit_once(',')
            .filter(|(_, index)| index.trim().parse::<i32>().is_ok())
            .map_or(value, |(path, _)| path.trim())
    } else {
        value
    };
    let value = value.trim_matches('"');
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    Some(if field == "InstallLocation" {
        path.join("WorkBuddy.exe")
    } else {
        path
    })
}

#[cfg(target_os = "windows")]
fn workbuddy_registry_candidates() -> Result<Vec<PathBuf>, String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let root = match RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall")
    {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not query installed applications: {error}")),
    };
    let mut candidates = Vec::new();
    for name in root.enum_keys() {
        let name = name.map_err(|error| error.to_string())?;
        let key = root.open_subkey(name).map_err(|error| error.to_string())?;
        let display_name: String = match key.get_value("DisplayName") {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        if !display_name.to_ascii_lowercase().contains("workbuddy") {
            continue;
        }
        for field in ["DisplayIcon", "InstallLocation"] {
            let value: String = match key.get_value(field) {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
            };
            if let Some(path) = workbuddy_registry_candidate(field, &value) {
                candidates.push(path);
            }
        }
    }
    Ok(candidates)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) async fn find_workbuddy_app() -> Result<Option<DesktopInstallation>, String> {
    Err("WorkBuddy 首版仅支持 macOS 和 Windows".to_string())
}

/// Detect installed AI-tool CLIs and (optionally) their latest published
/// versions.
///
/// `include_latest`: when `Some(false)`, skip the npm / GitHub round-trips
/// that look up each tool's latest published version. Callers that only
/// need `version`/`error` for "is the CLI installed?" should pass `false` —
/// those network fetches run serially per tool and otherwise dominate the
/// command latency (several seconds in the worst case). Defaults to `true`
/// to preserve the AboutSection callsite, which renders an "update
/// available" hint.
#[tauri::command]
pub async fn get_tool_versions(
    tools: Option<Vec<String>>,
    wsl_shell_by_tool: Option<HashMap<String, WslShellPreferenceInput>>,
    include_latest: Option<bool>,
) -> Result<Vec<ToolVersion>, String> {
    let include_latest = include_latest.unwrap_or(true);
    let requested: Vec<&str> = if let Some(tools) = tools.as_ref() {
        let set: std::collections::HashSet<&str> = tools.iter().map(|s| s.as_str()).collect();
        VALID_TOOLS
            .iter()
            .copied()
            .filter(|t| set.contains(t))
            .collect()
    } else {
        VALID_TOOLS.to_vec()
    };

    // Run all tools concurrently — each `get_single_tool_version_impl`
    // spawns a child process for `--version` and (when include_latest)
    // an HTTP request, both of which idle on I/O. Awaiting them serially
    // serialized all of that for no reason. `futures::future::join_all`
    // preserves ordering so the returned `Vec<ToolVersion>` is still in
    // VALID_TOOLS order.
    let futs = requested.into_iter().map(|tool| {
        let pref = wsl_shell_by_tool.as_ref().and_then(|m| m.get(tool));
        let tool_wsl_shell = pref.and_then(|p| p.wsl_shell.as_deref());
        let tool_wsl_shell_flag = pref.and_then(|p| p.wsl_shell_flag.as_deref());
        get_single_tool_version_impl(tool, tool_wsl_shell, tool_wsl_shell_flag, include_latest)
    });

    let results = futures::future::join_all(futs).await;
    Ok(results)
}

/// ChatGPT 桌面 App 的更新字段；`include_latest = false` 时不联网。
#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn chatgpt_update_fields(
    source: &str,
    installed: &str,
    include_latest: bool,
    one_click: bool,
) -> super::chatgpt_updates::DesktopUpdateFields {
    let latest = if include_latest {
        let client = crate::proxy::http_client::get();
        Some(super::chatgpt_updates::fetch_latest_for_host(&client).await)
    } else {
        None
    };
    super::chatgpt_updates::desktop_update_fields(source, installed, latest, one_click)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn desktop_tool_version(
    tool: &str,
    version: Option<String>,
    error: Option<String>,
    update: super::chatgpt_updates::DesktopUpdateFields,
    env_type: String,
    wsl_distro: Option<String>,
    executable_path: String,
) -> ToolVersion {
    ToolVersion {
        name: tool.to_string(),
        version,
        latest_version: update.latest_version,
        error,
        env_type,
        wsl_distro,
        installation_kind: "desktopApp".into(),
        installation_status: InstallationStatus::Installed,
        update_status: update.update_status.into(),
        update_source: update.update_source,
        update_supported: update.update_supported,
        update_reason: update.update_reason,
        executable_path: Some(executable_path),
    }
}

/// 获取单个工具的版本信息（内部实现）
///
/// `include_latest = false` skips the remote npm/GitHub lookup, returning
/// `latest_version = None`. Used by the bound-tool list on the Console page,
/// which only needs to know whether the CLI is installed locally.
async fn get_single_tool_version_impl(
    tool: &str,
    wsl_shell: Option<&str>,
    wsl_shell_flag: Option<&str>,
    include_latest: bool,
) -> ToolVersion {
    debug_assert!(
        VALID_TOOLS.contains(&tool),
        "unexpected tool name in get_single_tool_version_impl: {tool}"
    );

    if tool == "workbuddy" {
        let (env_type, wsl_distro) = tool_env_type_and_wsl_distro(tool);
        return match find_workbuddy_app().await {
            Ok(Some(app)) => ToolVersion {
                name: tool.to_string(),
                version: app.version,
                latest_version: None,
                error: app.error,
                env_type,
                wsl_distro,
                installation_kind: "desktopApp".into(),
                installation_status: InstallationStatus::Installed,
                update_status: "appManaged".into(),
                update_source: None,
                update_supported: false,
                update_reason: None,
                executable_path: Some(app.path.to_string_lossy().into_owned()),
            },
            Ok(None) => unavailable_tool_version(
                tool,
                "desktopApp",
                LocalDetection::missing(),
                env_type,
                wsl_distro,
            ),
            Err(error) => unavailable_tool_version(
                tool,
                "desktopApp",
                LocalDetection::failed(error),
                env_type,
                wsl_distro,
            ),
        };
    }

    if tool == "chatgpt" {
        let (env_type, wsl_distro) = tool_env_type_and_wsl_distro(tool);
        #[cfg(target_os = "macos")]
        {
            return match super::tool_update::codex_desktop_version().await {
                Ok(Some(app)) => {
                    let update = chatgpt_update_fields(
                        "sparkle",
                        app.version.as_deref().unwrap_or(""),
                        include_latest,
                        false,
                    )
                    .await;
                    desktop_tool_version(
                        tool,
                        app.version,
                        app.error,
                        update,
                        env_type,
                        wsl_distro,
                        app.path.to_string_lossy().into_owned(),
                    )
                }
                Ok(None) => unavailable_tool_version(
                    tool,
                    "desktopApp",
                    LocalDetection::missing(),
                    env_type,
                    wsl_distro,
                ),
                Err(error) => unavailable_tool_version(
                    tool,
                    "desktopApp",
                    LocalDetection::failed(error),
                    env_type,
                    wsl_distro,
                ),
            };
        }
        #[cfg(target_os = "windows")]
        {
            return match super::windows_chatgpt::detect_chatgpt_installation().await {
                Ok(Some(app)) => {
                    let update = chatgpt_update_fields(
                        "msstore",
                        app.version.as_deref().unwrap_or(""),
                        include_latest,
                        true,
                    )
                    .await;
                    desktop_tool_version(
                        tool,
                        app.version,
                        app.error,
                        update,
                        env_type,
                        wsl_distro,
                        app.path.to_string_lossy().into_owned(),
                    )
                }
                Ok(None) => unavailable_tool_version(
                    tool,
                    "desktopApp",
                    LocalDetection::missing(),
                    env_type,
                    wsl_distro,
                ),
                Err(error) => unavailable_tool_version(
                    tool,
                    "desktopApp",
                    LocalDetection::failed(error),
                    env_type,
                    wsl_distro,
                ),
            };
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        return unavailable_tool_version(
            tool,
            "desktopApp",
            LocalDetection::failed("ChatGPT App is not supported on this platform"),
            env_type,
            wsl_distro,
        );
    }

    let (env_type, wsl_distro) = tool_env_type_and_wsl_distro(tool);
    // A CLI bound to WSL is detected inside WSL; everything else the way a
    // terminal opened now would find it.
    let active_installation = match wsl_distro {
        Some(_) => Err(ProbeError::Failed(
            "WSL installations are detected inside WSL".into(),
        )),
        None => super::tool_update::probe(tool).await,
    };
    let local = match (wsl_distro.as_deref(), &active_installation) {
        #[cfg(not(unix))]
        (Some(distro), _) => try_get_version_wsl(tool, distro, wsl_shell, wsl_shell_flag).await,
        (_, Ok(installation)) => LocalDetection::found(
            installation.path.clone(),
            (!installation.version.is_empty()).then(|| installation.version.clone()),
            installation.error.clone(),
        ),
        (_, Err(ProbeError::NotFound)) => scan_cli_version(tool).await,
        (_, Err(error)) => {
            after_failed_shell_lookup(error.to_string(), scan_cli_version(tool).await)
        }
    };

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if tool == "codex" && wsl_distro.is_none() && local.status == InstallationStatus::NotInstalled {
        match codex_desktop_app().await {
            Ok(Some(app)) => {
                return ToolVersion {
                    name: tool.into(),
                    version: app.version,
                    latest_version: None,
                    error: app.error,
                    env_type,
                    wsl_distro,
                    installation_kind: "desktopApp".into(),
                    installation_status: InstallationStatus::Installed,
                    update_status: "appManaged".into(),
                    update_source: None,
                    update_supported: false,
                    update_reason: None,
                    executable_path: Some(app.path.to_string_lossy().into_owned()),
                }
            }
            Err(error) => {
                return unavailable_tool_version(
                    tool,
                    "cli",
                    LocalDetection::failed(error),
                    env_type,
                    wsl_distro,
                )
            }
            Ok(None) => {}
        }
    }

    let latest_version = if include_latest {
        latest_tool_version(tool, local.version.as_deref()).await
    } else {
        None
    };
    let update_status =
        cli_update_status(&local, latest_version.as_deref(), include_latest).to_string();
    #[cfg(target_os = "macos")]
    let (update_source, update_supported, update_reason) = match &active_installation {
        Ok(installation) => {
            let plan = if include_latest {
                super::tool_update::resolve_plan(tool, installation).await
            } else {
                super::tool_update::verified_plan(tool, installation)
            };
            match plan {
                Ok(plan) => (
                    Some(plan.source.to_string()),
                    installation.error.is_none() || plan.source == "pnpm",
                    installation.error.clone(),
                ),
                Err(reason) => (None, false, Some(reason)),
            }
        }
        Err(reason) => (None, false, Some(reason.to_string())),
    };
    #[cfg(not(target_os = "macos"))]
    let (update_source, update_supported, update_reason) = (
        None,
        false,
        Some("Automatic updates are currently supported on macOS only".into()),
    );
    #[cfg(unix)]
    let _ = (wsl_shell, wsl_shell_flag);
    ToolVersion {
        name: tool.into(),
        version: local.version,
        latest_version,
        error: local.error,
        env_type,
        wsl_distro,
        installation_kind: "cli".into(),
        installation_status: local.status,
        update_status,
        update_source,
        update_supported,
        update_reason,
        executable_path: local.path.map(|path| path.to_string_lossy().into_owned()),
    }
}

/// The Codex desktop app, counted as Codex when its CLI is absent:
/// ChatGPT.app on macOS, the Microsoft Store package on Windows.
#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn codex_desktop_app() -> Result<Option<DesktopInstallation>, String> {
    #[cfg(target_os = "macos")]
    return super::tool_update::codex_desktop_version().await;
    #[cfg(target_os = "windows")]
    return super::windows_chatgpt::detect_chatgpt_installation().await;
}

/// A failed shell lookup is not uninstall evidence, but an executable found by
/// the path scan (e.g. behind an alias or a slow rc file) still proves installation.
fn after_failed_shell_lookup(probe_error: String, scanned: LocalDetection) -> LocalDetection {
    if scanned.status == InstallationStatus::Installed {
        scanned
    } else {
        LocalDetection::failed(probe_error)
    }
}

fn unavailable_tool_version(
    tool: &str,
    kind: &str,
    local: LocalDetection,
    env_type: String,
    wsl_distro: Option<String>,
) -> ToolVersion {
    ToolVersion {
        name: tool.into(),
        version: None,
        latest_version: None,
        error: local.error,
        env_type,
        wsl_distro,
        installation_kind: kind.into(),
        installation_status: local.status,
        update_status: if local.status == InstallationStatus::NotInstalled {
            "notInstalled"
        } else {
            "failed"
        }
        .into(),
        update_source: None,
        update_supported: false,
        update_reason: None,
        executable_path: None,
    }
}

fn cli_update_status(
    local: &LocalDetection,
    latest: Option<&str>,
    include_latest: bool,
) -> &'static str {
    match local.status {
        InstallationStatus::Unknown => "failed",
        InstallationStatus::Installed if local.error.is_some() => "broken",
        _ if !include_latest => "unchecked",
        InstallationStatus::Installed if local.version.is_none() => "unknown",
        _ => super::tool_update::version_status(local.version.as_deref(), latest),
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn cli_is_missing(tool: &str) -> Result<bool, String> {
    match super::tool_update::probe(tool).await {
        Ok(_) => Ok(false),
        Err(ProbeError::NotFound) => {
            let local = scan_cli_version(tool).await;
            match local.status {
                InstallationStatus::Installed => Ok(false),
                InstallationStatus::NotInstalled => Ok(true),
                InstallationStatus::Unknown => {
                    Err(local.error.unwrap_or_else(|| "CLI detection failed".into()))
                }
            }
        }
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) async fn latest_tool_version(tool: &str, local: Option<&str>) -> Option<String> {
    let client = crate::proxy::http_client::get();
    let query = async {
        if let Some(package) = super::tool_update::npm_package(tool) {
            let version = fetch_npm_latest_version(&client, package).await;
            if version.is_some() || tool != "opencode" {
                return version;
            }
            return fetch_github_latest_version(&client, "anomalyco/opencode").await;
        }
        if tool == "hermes" {
            if let Some(version) =
                fetch_github_latest_version(&client, "NousResearch/hermes-agent").await
            {
                return Some(version);
            }
            let fallback = client
                .get("https://pypi.org/pypi/hermes-agent/json")
                .timeout(std::time::Duration::from_secs(7))
                .send()
                .await
                .ok()?
                .error_for_status()
                .ok()?
                .json::<serde_json::Value>()
                .await
                .ok()?
                .get("info")?
                .get("version")?
                .as_str()
                .map(str::to_string);
            // PyPI can lag official git releases; don't claim it is current.
            return fallback.filter(|latest| {
                match (
                    local.and_then(|value| semver::Version::parse(value).ok()),
                    semver::Version::parse(latest).ok(),
                ) {
                    (Some(local), Some(latest)) => !local.cmp_precedence(&latest).is_gt(),
                    _ => true,
                }
            });
        }
        None
    };
    tokio::time::timeout(std::time::Duration::from_secs(15), query)
        .await
        .ok()
        .flatten()
        .filter(|v| semver::Version::parse(v).is_ok_and(|v| v.pre.is_empty()))
}

/// One bounded request, including body read. Failures remain unknown, never "current".
async fn fetch_version_json(
    client: &reqwest::Client,
    url: &str,
    timeout: std::time::Duration,
) -> Option<serde_json::Value> {
    client
        .get(url)
        .timeout(timeout)
        .header("User-Agent", "ofox-desktop")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()
}

/// Fetch the small dist-tags document, not the full package history.
async fn fetch_npm_latest_version(client: &reqwest::Client, package: &str) -> Option<String> {
    let package = package.replace('/', "%2f");
    let url = format!("https://registry.npmjs.org/-/package/{package}/dist-tags");
    fetch_version_json(client, &url, std::time::Duration::from_secs(7))
        .await?
        .get("latest")?
        .as_str()
        .map(str::to_string)
}

async fn fetch_github_latest_version(client: &reqwest::Client, repo: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let json = fetch_version_json(client, &url, std::time::Duration::from_secs(7)).await?;
    if json.get("prerelease").and_then(|value| value.as_bool()) == Some(true) {
        return None;
    }
    release_version(&json)
}

fn release_version(json: &serde_json::Value) -> Option<String> {
    // Hermes release tags are dates; the release title contains the CLI version.
    [json.get("name"), json.get("tag_name")]
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(extract_version)
        .find(|value| {
            semver::Version::parse(value).is_ok_and(|v| v.major < 1000 && v.pre.is_empty())
        })
}

#[cfg(test)]
mod latest_version_tests {
    use super::*;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    #[test]
    fn release_calendar_tag_is_not_a_cli_version() {
        assert_eq!(
            release_version(
                &serde_json::json!({"name": "Hermes Agent v0.21.4", "tag_name": "v2026.9.21"})
            ),
            Some("0.21.4".into())
        );
        assert_eq!(
            release_version(&serde_json::json!({"tag_name": "v2026.9.21"})),
            None
        );
        assert_eq!(
            release_version(&serde_json::json!({"tag_name": "v1.2.3"})),
            Some("1.2.3".into())
        );
    }
    #[tokio::test]
    async fn version_lookup_rejects_http_errors_bad_json_and_timeout() {
        let server = MockServer::start().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for (route, response) in [
            (
                "/ok",
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"latest": "1.2.3"})),
            ),
            (
                "/error",
                ResponseTemplate::new(503).set_body_json(serde_json::json!({"latest": "9.9.9"})),
            ),
            (
                "/bad",
                ResponseTemplate::new(200).set_body_string("not json"),
            ),
            (
                "/slow",
                ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(1)),
            ),
        ] {
            Mock::given(path(route))
                .respond_with(response)
                .mount(&server)
                .await;
        }
        let timeout = std::time::Duration::from_millis(100);
        let value = fetch_version_json(&client, &format!("{}/ok", server.uri()), timeout)
            .await
            .unwrap();
        assert_eq!(value["latest"], "1.2.3");
        for route in ["error", "bad", "slow"] {
            assert!(
                fetch_version_json(&client, &format!("{}/{route}", server.uri()), timeout)
                    .await
                    .is_none()
            );
        }
    }
}

/// 预编译的版本号正则表达式
static VERSION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\d+\.\d+\.\d+(-[\w.]+)?").expect("Invalid version regex"));

/// 从版本输出中提取纯版本号
fn extract_version(raw: &str) -> String {
    VERSION_RE
        .find(raw)
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| raw.to_string())
}

async fn cli_at_path(path: &Path, search_path: &str) -> LocalDetection {
    if path.is_dir() {
        return LocalDetection::failed(format!("Expected a CLI executable at {}", path.display()));
    }
    if let Err(error) = std::fs::canonicalize(path) {
        return LocalDetection::found(
            path.to_path_buf(),
            None,
            Some(format!("Active executable failed its path check: {error}")),
        );
    }
    let mut command = version_command(path);
    command.env("PATH", search_path);
    cli_version_output(path, super::tool_update::bounded_output(command).await)
}

/// `<path> --version`, with a Windows `.cmd`/`.bat` shim run through cmd.exe
/// without a console window.
pub(crate) fn version_command(path: &Path) -> tokio::process::Command {
    #[cfg(target_os = "windows")]
    {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let mut command =
            if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat") {
                let mut command = tokio::process::Command::new("cmd");
                // cmd.exe has different quoting rules than native argv. Expand a
                // child-only variable once, so spaces and literal % in the path
                // cannot alter the command used for the version check.
                command
                    .args(["/D", "/V:OFF", "/S", "/C"])
                    .raw_arg("\"\"%OFOX_VERSION_EXECUTABLE%\" --version\"")
                    .env("OFOX_VERSION_EXECUTABLE", path);
                command
            } else {
                let mut command = tokio::process::Command::new(path);
                command.arg("--version");
                command
            };
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut command = tokio::process::Command::new(path);
        command.arg("--version");
        command
    }
}

fn cli_version_output(path: &Path, output: Result<std::process::Output, String>) -> LocalDetection {
    let (version, error) = super::tool_update::executable_version(output);
    LocalDetection::found(
        path.to_path_buf(),
        (!version.is_empty()).then_some(version),
        error,
    )
}

/// 校验 WSL 发行版名称是否合法
/// WSL 发行版名称只允许字母、数字、连字符和下划线
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_valid_wsl_distro_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Validate that the given shell name is one of the allowed shells.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_valid_shell(shell: &str) -> bool {
    matches!(
        shell.rsplit('/').next().unwrap_or(shell),
        "sh" | "bash" | "zsh" | "fish" | "dash"
    )
}

/// Validate that the given shell flag is one of the allowed flags.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_valid_shell_flag(flag: &str) -> bool {
    matches!(flag, "-c" | "-lc" | "-lic")
}

/// Return the default invocation flag for the given shell.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn default_flag_for_shell(shell: &str) -> &'static str {
    match shell.rsplit('/').next().unwrap_or(shell) {
        "dash" | "sh" => "-c",
        "fish" => "-lc",
        _ => "-lic",
    }
}

#[cfg(target_os = "windows")]
async fn try_get_version_wsl(
    tool: &str,
    distro: &str,
    force_shell: Option<&str>,
    force_shell_flag: Option<&str>,
) -> LocalDetection {
    if !is_valid_wsl_distro_name(distro) {
        return LocalDetection::failed(format!("[WSL:{distro}] invalid distro name"));
    }
    if force_shell.is_some_and(|shell| !is_valid_shell(shell))
        || force_shell_flag.is_some_and(|flag| !is_valid_shell_flag(flag))
    {
        return LocalDetection::failed(format!("[WSL:{distro}] invalid shell preference"));
    }
    let lookup = format!("printf '\\n__OFOX_BIN__%s\\n' \"$(command -v {tool})\"; printf '__OFOX_PATH__%s\\n' \"$PATH\"");
    let (shell, flag, script) = if let Some(shell) = force_shell {
        let shell = shell.rsplit('/').next().unwrap_or(shell);
        (
            shell.to_string(),
            force_shell_flag
                .unwrap_or_else(|| default_flag_for_shell(shell))
                .to_string(),
            lookup,
        )
    } else {
        let flag = force_shell_flag.unwrap_or("-lic");
        (
            "sh".into(),
            "-c".into(),
            format!(
                "\"${{SHELL:-sh}}\" {flag} '{}'",
                lookup.replace('\'', "'\\''")
            ),
        )
    };
    let mut command = tokio::process::Command::new("wsl.exe");
    command
        .args(["-d", distro, "--", &shell, &flag, &script])
        .creation_flags(CREATE_NO_WINDOW);
    let (path, search_path) =
        match wsl_lookup_result(distro, super::tool_update::bounded_output(command).await) {
            Ok(resolution) => resolution,
            Err(detection) => return detection,
        };
    let mut command = tokio::process::Command::new("wsl.exe");
    command
        .args([
            "-d",
            distro,
            "--",
            "env",
            &format!("PATH={search_path}"),
            &path,
            "--version",
        ])
        .creation_flags(CREATE_NO_WINDOW);
    wsl_version_result(
        &path,
        distro,
        super::tool_update::bounded_output(command).await,
    )
}

#[cfg(any(target_os = "windows", test))]
fn wsl_lookup_result(
    distro: &str,
    output: Result<std::process::Output, String>,
) -> Result<(String, String), LocalDetection> {
    let output =
        output.map_err(|error| LocalDetection::failed(format!("[WSL:{distro}] {error}")))?;
    match super::tool_update::shell_resolution(&output) {
        Ok(resolution) => Ok(resolution),
        Err(ProbeError::NotFound) => Err(LocalDetection::missing()),
        Err(error) => Err(LocalDetection::failed(format!("[WSL:{distro}] {error}"))),
    }
}

#[cfg(any(target_os = "windows", test))]
fn wsl_version_result(
    path: &str,
    distro: &str,
    output: Result<std::process::Output, String>,
) -> LocalDetection {
    let mut local = cli_version_output(Path::new(path), output);
    local.error = local.error.map(|error| format!("[WSL:{distro}] {error}"));
    local
}

/// 用户目录下常见的 CLI 安装位置（登录 shell 的 PATH 里不一定有）。
fn home_bin_search_paths(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    if home.as_os_str().is_empty() {
        return Vec::new();
    }
    [
        ".local/bin",
        ".npm-global/bin",
        "n/bin",
        ".volta/bin",
        // Hermes 的安装脚本把启动器放在这里。
        ".hermes/bin",
    ]
    .iter()
    .map(|rel| home.join(rel))
    .collect()
}

fn push_unique_path(paths: &mut Vec<std::path::PathBuf>, path: std::path::PathBuf) {
    if path.as_os_str().is_empty() {
        return;
    }

    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn push_env_single_dir(paths: &mut Vec<std::path::PathBuf>, value: Option<std::ffi::OsString>) {
    if let Some(raw) = value {
        push_unique_path(paths, std::path::PathBuf::from(raw));
    }
}

fn extend_from_path_list(
    paths: &mut Vec<std::path::PathBuf>,
    value: Option<std::ffi::OsString>,
    suffix: Option<&str>,
) {
    if let Some(raw) = value {
        for p in std::env::split_paths(&raw) {
            let dir = match suffix {
                Some(s) => p.join(s),
                None => p,
            };
            push_unique_path(paths, dir);
        }
    }
}

/// OpenCode install.sh 路径优先级（见 https://github.com/anomalyco/opencode README）:
///   $OPENCODE_INSTALL_DIR > $XDG_BIN_DIR > $HOME/bin > $HOME/.opencode/bin
/// 额外扫描 Bun 默认全局安装路径（~/.bun/bin）
/// 和 Go 安装路径（~/go/bin、$GOPATH/*/bin）。
fn opencode_extra_search_paths(
    home: &Path,
    opencode_install_dir: Option<std::ffi::OsString>,
    xdg_bin_dir: Option<std::ffi::OsString>,
    gopath: Option<std::ffi::OsString>,
) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();

    push_env_single_dir(&mut paths, opencode_install_dir);
    push_env_single_dir(&mut paths, xdg_bin_dir);

    if !home.as_os_str().is_empty() {
        push_unique_path(&mut paths, home.join("bin"));
        push_unique_path(&mut paths, home.join(".opencode").join("bin"));
        push_unique_path(&mut paths, home.join(".bun").join("bin"));
        push_unique_path(&mut paths, home.join("go").join("bin"));
    }

    extend_from_path_list(&mut paths, gopath, Some("bin"));

    paths
}

fn tool_executable_candidates(tool: &str, dir: &Path) -> Vec<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        // Same order as `windows_tools::tool_in`: an installer's binary, then an npm shim.
        vec![
            dir.join(format!("{tool}.exe")),
            dir.join(format!("{tool}.cmd")),
            dir.join(tool),
        ]
    }

    #[cfg(not(target_os = "windows"))]
    {
        vec![dir.join(tool)]
    }
}

/// 扫描常见路径查找 CLI
async fn scan_cli_version(tool: &str) -> LocalDetection {
    let home = dirs::home_dir().unwrap_or_default();

    // 常见的安装路径（原生安装优先）
    let mut search_paths: Vec<std::path::PathBuf> = Vec::new();
    let mut first_error = home
        .as_os_str()
        .is_empty()
        .then(|| "Could not read the home directory for CLI discovery".to_string());
    let current_path = match std::env::var("PATH") {
        Ok(path) => path,
        Err(error) => {
            first_error = Some(format!("Could not read PATH: {error}"));
            String::new()
        }
    };
    #[cfg(target_os = "windows")]
    for path in std::env::split_paths(&current_path) {
        push_unique_path(&mut search_paths, path);
    }
    for path in home_bin_search_paths(&home) {
        push_unique_path(&mut search_paths, path);
    }

    #[cfg(target_os = "macos")]
    {
        push_unique_path(
            &mut search_paths,
            std::path::PathBuf::from("/opt/homebrew/bin"),
        );
        push_unique_path(
            &mut search_paths,
            std::path::PathBuf::from("/usr/local/bin"),
        );
    }

    #[cfg(target_os = "linux")]
    {
        push_unique_path(
            &mut search_paths,
            std::path::PathBuf::from("/usr/local/bin"),
        );
        push_unique_path(&mut search_paths, std::path::PathBuf::from("/usr/bin"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = dirs::data_dir() {
            push_unique_path(&mut search_paths, appdata.join("npm"));
        }
        push_unique_path(
            &mut search_paths,
            std::path::PathBuf::from("C:\\Program Files\\nodejs"),
        );
        // Hermes' install.ps1 puts its launcher in <HermesHome>\bin.
        for home in [
            std::env::var_os("HERMES_HOME").map(PathBuf::from),
            std::env::var_os("LOCALAPPDATA").map(|local| PathBuf::from(local).join("hermes")),
        ]
        .into_iter()
        .flatten()
        {
            push_unique_path(&mut search_paths, home.join("bin"));
        }
    }

    for base in [
        home.join(".local/state/fnm_multishells"),
        home.join(".nvm/versions/node"),
    ] {
        match std::fs::read_dir(&base) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => push_unique_path(&mut search_paths, entry.path().join("bin")),
                        Err(error) => {
                            first_error.get_or_insert(error.to_string());
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                first_error.get_or_insert(format!("Could not inspect {}: {error}", base.display()));
            }
        }
    }
    if tool == "opencode" {
        let extra_paths = opencode_extra_search_paths(
            &home,
            std::env::var_os("OPENCODE_INSTALL_DIR"),
            std::env::var_os("XDG_BIN_DIR"),
            std::env::var_os("GOPATH"),
        );

        for path in extra_paths {
            push_unique_path(&mut search_paths, path);
        }
    }

    #[cfg(not(target_os = "windows"))]
    for path in std::env::split_paths(&current_path) {
        push_unique_path(&mut search_paths, path);
    }
    scan_cli_paths(tool, &search_paths, &current_path, first_error).await
}

async fn scan_cli_paths(
    tool: &str,
    search_paths: &[PathBuf],
    current_path: &str,
    first_error: Option<String>,
) -> LocalDetection {
    scan_cli_paths_with(
        tool,
        search_paths,
        current_path,
        first_error,
        super::tool_update::candidate_exists,
    )
    .await
}

async fn scan_cli_paths_with<F>(
    tool: &str,
    search_paths: &[PathBuf],
    current_path: &str,
    mut first_error: Option<String>,
    mut inspect: F,
) -> LocalDetection
where
    F: FnMut(&Path) -> Result<bool, String>,
{
    for directory in search_paths {
        #[cfg(target_os = "windows")]
        let new_path = format!("{};{}", directory.display(), current_path);
        #[cfg(not(target_os = "windows"))]
        let new_path = format!("{}:{}", directory.display(), current_path);
        for path in tool_executable_candidates(tool, directory) {
            match inspect(&path) {
                Ok(true) => return cli_at_path(&path, &new_path).await,
                Ok(false) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
    }
    match first_error {
        Some(error) => LocalDetection::failed(error),
        None => LocalDetection::missing(),
    }
}

#[cfg(target_os = "windows")]
fn wsl_distro_for_tool(tool: &str) -> Option<String> {
    let override_dir = match tool {
        "claude" => crate::settings::get_claude_override_dir(),
        "codex" => crate::settings::get_codex_override_dir(),
        "gemini" => crate::settings::get_gemini_override_dir(),
        "opencode" => crate::settings::get_opencode_override_dir(),
        _ => None,
    }?;

    wsl_distro_from_path(&override_dir)
}

/// 从 UNC 路径中提取 WSL 发行版名称
/// 支持 `\\wsl$\Ubuntu\...` 和 `\\wsl.localhost\Ubuntu\...` 两种格式
#[cfg(target_os = "windows")]
fn wsl_distro_from_path(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    match prefix.kind() {
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            let server_name = server.to_string_lossy();
            if server_name.eq_ignore_ascii_case("wsl$")
                || server_name.eq_ignore_ascii_case("wsl.localhost")
            {
                let distro = share.to_string_lossy().to_string();
                if !distro.is_empty() {
                    return Some(distro);
                }
            }
            None
        }
        _ => None,
    }
}

/// 打开指定提供商的终端
///
/// 根据提供商配置的环境变量启动一个带有该提供商特定设置的终端
/// 无需检查是否为当前激活的提供商，任何提供商都可以打开终端
#[allow(non_snake_case)]
#[tauri::command]
pub async fn open_provider_terminal(
    state: State<'_, crate::store::AppState>,
    app: String,
    #[allow(non_snake_case)] providerId: String,
    cwd: Option<String>,
) -> Result<bool, String> {
    let app_type = AppType::from_str(&app).map_err(|e| e.to_string())?;
    let launch_cwd = resolve_launch_cwd(cwd)?;

    // 获取提供商配置
    let providers = ProviderService::list(state.inner(), app_type)
        .map_err(|e| format!("获取提供商列表失败: {e}"))?;

    let provider = providers
        .get(&providerId)
        .ok_or_else(|| format!("提供商 {providerId} 不存在"))?;

    // 从提供商配置中提取环境变量
    let config = &provider.settings_config;
    let env_vars = extract_env_vars_from_config(config, &app_type);

    // 根据平台启动终端，传入提供商ID用于生成唯一的配置文件名
    launch_terminal_with_env(env_vars, &providerId, launch_cwd.as_deref())
        .map_err(|e| format!("启动终端失败: {e}"))?;

    Ok(true)
}

/// 从提供商配置中提取环境变量
fn extract_env_vars_from_config(
    config: &serde_json::Value,
    app_type: &AppType,
) -> Vec<(String, String)> {
    let mut env_vars = Vec::new();

    let Some(obj) = config.as_object() else {
        return env_vars;
    };

    // 处理 env 字段（Claude/Gemini 通用）
    if let Some(env) = obj.get("env").and_then(|v| v.as_object()) {
        for (key, value) in env {
            if let Some(str_val) = value.as_str() {
                env_vars.push((key.clone(), str_val.to_string()));
            }
        }

        // 处理 base_url: 根据应用类型添加对应的环境变量
        let base_url_key = match app_type {
            AppType::Claude => Some("ANTHROPIC_BASE_URL"),
            AppType::Gemini => Some("GOOGLE_GEMINI_BASE_URL"),
            _ => None,
        };

        if let Some(key) = base_url_key {
            if let Some(url_str) = env.get(key).and_then(|v| v.as_str()) {
                env_vars.push((key.to_string(), url_str.to_string()));
            }
        }
    }

    // Codex 使用 auth 字段转换为 OPENAI_API_KEY
    if *app_type == AppType::Codex {
        if let Some(auth) = obj.get("auth").and_then(|v| v.as_str()) {
            env_vars.push(("OPENAI_API_KEY".to_string(), auth.to_string()));
        }
    }

    // Gemini 使用 api_key 字段转换为 GEMINI_API_KEY
    if *app_type == AppType::Gemini {
        if let Some(api_key) = obj.get("api_key").and_then(|v| v.as_str()) {
            env_vars.push(("GEMINI_API_KEY".to_string(), api_key.to_string()));
        }
    }

    env_vars
}

fn resolve_launch_cwd(cwd: Option<String>) -> Result<Option<PathBuf>, String> {
    let Some(raw_path) = cwd.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };

    if raw_path.contains('\n') || raw_path.contains('\r') {
        return Err("目录路径包含非法换行符".to_string());
    }

    let path = Path::new(&raw_path);
    if !path.exists() {
        return Err(format!("目录不存在: {raw_path}"));
    }

    let resolved = std::fs::canonicalize(path).map_err(|e| format!("解析目录失败: {e}"))?;
    if !resolved.is_dir() {
        return Err(format!("选择的路径不是文件夹: {}", resolved.display()));
    }

    // Strip Windows extended-length prefix that canonicalize produces,
    // as it can break batch scripts and other shell commands.
    // Special-case \\?\UNC\server\share -> \\server\share for network/WSL paths.
    #[cfg(target_os = "windows")]
    let resolved = {
        let s = resolved.to_string_lossy();
        if let Some(unc) = s.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{unc}"))
        } else if let Some(stripped) = s.strip_prefix(r"\\?\") {
            PathBuf::from(stripped)
        } else {
            resolved
        }
    };

    Ok(Some(resolved))
}

/// 创建临时配置文件并启动 claude 终端
/// 使用 --settings 参数传入提供商特定的 API 配置
fn launch_terminal_with_env(
    env_vars: Vec<(String, String)>,
    provider_id: &str,
    cwd: Option<&Path>,
) -> Result<(), String> {
    let temp_dir = std::env::temp_dir();
    let config_file = temp_dir.join(format!(
        "claude_{}_{}.json",
        provider_id,
        std::process::id()
    ));

    // 创建并写入配置文件
    write_claude_config(&config_file, &env_vars)?;

    #[cfg(target_os = "macos")]
    {
        launch_macos_terminal(&config_file, cwd)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        launch_linux_terminal(&config_file, cwd)?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        launch_windows_terminal(&temp_dir, &config_file, cwd)?;
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("不支持的操作系统".to_string())
}

/// 写入 claude 配置文件
fn write_claude_config(
    config_file: &std::path::Path,
    env_vars: &[(String, String)],
) -> Result<(), String> {
    let mut config_obj = serde_json::Map::new();
    let mut env_obj = serde_json::Map::new();

    for (key, value) in env_vars {
        env_obj.insert(key.clone(), serde_json::Value::String(value.clone()));
    }

    config_obj.insert("env".to_string(), serde_json::Value::Object(env_obj));

    let config_json =
        serde_json::to_string_pretty(&config_obj).map_err(|e| format!("序列化配置失败: {e}"))?;

    std::fs::write(config_file, config_json).map_err(|e| format!("写入配置文件失败: {e}"))
}

/// macOS: 根据用户首选终端启动
#[cfg(target_os = "macos")]
fn launch_macos_terminal(config_file: &std::path::Path, cwd: Option<&Path>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let preferred = crate::settings::get_preferred_terminal();
    let terminal = preferred.as_deref().unwrap_or("terminal");

    let temp_dir = std::env::temp_dir();
    let script_file = temp_dir.join(format!("cc_switch_launcher_{}.sh", std::process::id()));
    let config_path = config_file.to_string_lossy();
    let cd_command = build_shell_cd_command(cwd);

    // Write the shell script to a temp file
    let script_content = format!(
        r#"#!/bin/bash
trap 'rm -f "{config_path}" "{script_file}"' EXIT
{cd_command}
echo "Using provider-specific claude config:"
echo "{config_path}"
claude --settings "{config_path}"
exec bash --norc --noprofile
"#,
        config_path = config_path,
        script_file = script_file.display(),
        cd_command = cd_command,
    );

    std::fs::write(&script_file, &script_content).map_err(|e| format!("写入启动脚本失败: {e}"))?;

    // Make script executable
    std::fs::set_permissions(&script_file, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("设置脚本权限失败: {e}"))?;

    // Try the preferred terminal first, fall back to Terminal.app if it fails
    // Note: Kitty doesn't need the -e flag, others do
    let result = match terminal {
        "iterm2" => launch_macos_iterm2(&script_file),
        "alacritty" => launch_macos_open_app("Alacritty", &script_file, true),
        "kitty" => launch_macos_open_app("kitty", &script_file, false),
        "ghostty" => launch_macos_open_app("Ghostty", &script_file, true),
        "wezterm" => launch_macos_open_app("WezTerm", &script_file, true),
        "kaku" => launch_macos_open_app("Kaku", &script_file, true),
        _ => launch_macos_terminal_app(&script_file), // "terminal" or default
    };

    // If preferred terminal fails and it's not the default, try Terminal.app as fallback
    if result.is_err() && terminal != "terminal" {
        log::warn!(
            "首选终端 {} 启动失败，回退到 Terminal.app: {:?}",
            terminal,
            result.as_ref().err()
        );
        return launch_macos_terminal_app(&script_file);
    }

    result
}

/// macOS: Terminal.app
///
/// 冷启动坑：`tell application "Terminal" → activate` 会先启动 Terminal.app，
/// 而 Terminal.app 自己 launch 时默认开一个空窗口；紧接着的 `do script "..."`
/// 又开第二个窗口跑脚本——用户看到两个窗口，其中一个是空的孤儿。
///
/// 修法：如果 Terminal 没在跑，就用 `in window 1` 复用它 launch 时开的第一个
/// 空窗口（`do script` 支持指定 target window）。如果已经在跑，走原来的
/// 无参 `do script`——那时候不能给 `in window 1`，否则会把用户正在用的窗口
/// 接管掉。
#[cfg(target_os = "macos")]
fn launch_macos_terminal_app(script_file: &std::path::Path) -> Result<(), String> {
    use std::process::Command;

    // 先探测 Terminal 是否在跑——用 pgrep 比 osascript 的 `is running` 快
    // 且无 UI 副作用。跑不跑决定 do script 后面接不接 `in window 1`。
    let terminal_running = Command::new("pgrep")
        .arg("-x")
        .arg("Terminal")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let do_script_target = if terminal_running {
        // 已在跑：新开窗口。不指定 target。
        String::new()
    } else {
        // 冷启动：复用 activate 触发生成的第一个窗口，避免孤儿空窗口。
        " in window 1".to_string()
    };

    let applescript = format!(
        r#"tell application "Terminal"
    activate
    do script "bash '{}'"{}
end tell"#,
        script_file.display(),
        do_script_target,
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&applescript)
        .output()
        .map_err(|e| format!("执行 osascript 失败: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Terminal.app 执行失败 (exit code: {:?}): {}",
            output.status.code(),
            stderr
        ));
    }

    Ok(())
}

/// macOS: iTerm2
#[cfg(target_os = "macos")]
fn launch_macos_iterm2(script_file: &std::path::Path) -> Result<(), String> {
    use std::process::Command;

    let applescript = format!(
        r#"tell application "iTerm"
    activate
    tell current window
        create tab with default profile
        tell current session
            write text "bash '{}'"
        end tell
    end tell
end tell"#,
        script_file.display()
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&applescript)
        .output()
        .map_err(|e| format!("执行 osascript 失败: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "iTerm2 执行失败 (exit code: {:?}): {}",
            output.status.code(),
            stderr
        ));
    }

    Ok(())
}

/// macOS: 使用 open -a 启动支持 --args 参数的终端（Alacritty/Kitty/Ghostty）
#[cfg(target_os = "macos")]
fn launch_macos_open_app(
    app_name: &str,
    script_file: &std::path::Path,
    use_e_flag: bool,
) -> Result<(), String> {
    use std::process::Command;

    let mut cmd = Command::new("open");
    cmd.arg("-a").arg(app_name).arg("--args");

    if use_e_flag {
        cmd.arg("-e");
    }
    cmd.arg("bash").arg(script_file);

    let output = cmd
        .output()
        .map_err(|e| format!("启动 {app_name} 失败: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{} 启动失败 (exit code: {:?}): {}",
            app_name,
            output.status.code(),
            stderr
        ));
    }

    Ok(())
}

/// Linux: 根据用户首选终端启动
#[cfg(target_os = "linux")]
fn launch_linux_terminal(config_file: &std::path::Path, cwd: Option<&Path>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let preferred = crate::settings::get_preferred_terminal();

    // Default terminal list with their arguments
    let default_terminals = [
        ("gnome-terminal", vec!["--"]),
        ("konsole", vec!["-e"]),
        ("xfce4-terminal", vec!["-e"]),
        ("mate-terminal", vec!["--"]),
        ("lxterminal", vec!["-e"]),
        ("alacritty", vec!["-e"]),
        ("kitty", vec!["-e"]),
        ("ghostty", vec!["-e"]),
    ];

    // Create temp script file
    let temp_dir = std::env::temp_dir();
    let script_file = temp_dir.join(format!("cc_switch_launcher_{}.sh", std::process::id()));
    let config_path = config_file.to_string_lossy();
    let cd_command = build_shell_cd_command(cwd);

    let script_content = format!(
        r#"#!/bin/bash
trap 'rm -f "{config_path}" "{script_file}"' EXIT
{cd_command}
echo "Using provider-specific claude config:"
echo "{config_path}"
claude --settings "{config_path}"
exec bash --norc --noprofile
"#,
        config_path = config_path,
        script_file = script_file.display(),
        cd_command = cd_command,
    );

    std::fs::write(&script_file, &script_content).map_err(|e| format!("写入启动脚本失败: {e}"))?;

    std::fs::set_permissions(&script_file, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("设置脚本权限失败: {e}"))?;

    // Build terminal list: preferred terminal first (if specified), then defaults
    let terminals_to_try: Vec<(&str, Vec<&str>)> = if let Some(ref pref) = preferred {
        // Find the preferred terminal's args from default list
        let pref_args = default_terminals
            .iter()
            .find(|(name, _)| *name == pref.as_str())
            .map(|(_, args)| args.to_vec())
            .unwrap_or_else(|| vec!["-e"]); // Default args for unknown terminals

        let mut list = vec![(pref.as_str(), pref_args)];
        // Add remaining terminals as fallbacks
        for (name, args) in &default_terminals {
            if *name != pref.as_str() {
                list.push((*name, args.to_vec()));
            }
        }
        list
    } else {
        default_terminals
            .iter()
            .map(|(name, args)| (*name, args.to_vec()))
            .collect()
    };

    let mut last_error = String::from("未找到可用的终端");

    for (terminal, args) in terminals_to_try {
        // Check if terminal exists in common paths
        let terminal_exists = std::path::Path::new(&format!("/usr/bin/{}", terminal)).exists()
            || std::path::Path::new(&format!("/bin/{}", terminal)).exists()
            || std::path::Path::new(&format!("/usr/local/bin/{}", terminal)).exists()
            || which_command(terminal);

        if terminal_exists {
            let result = Command::new(terminal)
                .args(&args)
                .arg("bash")
                .arg(script_file.to_string_lossy().as_ref())
                .spawn();

            match result {
                Ok(_) => return Ok(()),
                Err(e) => {
                    last_error = format!("执行 {} 失败: {}", terminal, e);
                }
            }
        }
    }

    // Clean up on failure
    let _ = std::fs::remove_file(&script_file);
    let _ = std::fs::remove_file(config_file);
    Err(last_error)
}

/// Check if a command exists using `which`
#[cfg(target_os = "linux")]
fn which_command(cmd: &str) -> bool {
    use std::process::Command;
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Windows: 根据用户首选终端启动
#[cfg(target_os = "windows")]
fn launch_windows_terminal(
    temp_dir: &std::path::Path,
    config_file: &std::path::Path,
    cwd: Option<&Path>,
) -> Result<(), String> {
    let preferred = crate::settings::get_preferred_terminal();
    let terminal = preferred.as_deref().unwrap_or("cmd");

    let bat_file = temp_dir.join(format!("cc_switch_claude_{}.bat", std::process::id()));
    let config_path_for_batch = escape_windows_batch_value(&config_file.to_string_lossy());
    let cwd_command = build_windows_cwd_command(cwd);

    let content = format!(
        "@echo off
{cwd_command}
echo Using provider-specific claude config:
echo {}
rem `call` returns from npm's claude.cmd, so the API-key settings file is deleted.
call claude --settings \"{}\"
del \"{}\" >nul 2>&1
(goto) 2>nul & del \"%~f0\"
",
        config_path_for_batch,
        config_path_for_batch,
        config_path_for_batch,
        cwd_command = cwd_command,
    );

    std::fs::write(&bat_file, &content).map_err(|e| format!("写入批处理文件失败: {e}"))?;

    let bat_path = bat_file.to_string_lossy();
    let ps_cmd = format!("& '{}'", bat_path);

    // Try the preferred terminal first
    let result = match terminal {
        "powershell" => run_windows_start_command(
            &["powershell", "-NoExit", "-Command", &ps_cmd],
            "PowerShell",
        ),
        "wt" => run_windows_start_command(&["wt", "cmd", "/K", &bat_path], "Windows Terminal"),
        _ => run_windows_start_command(&["cmd", "/K", &bat_path], "cmd"), // "cmd" or default
    };

    // If preferred terminal fails and it's not the default, try cmd as fallback
    if result.is_err() && terminal != "cmd" {
        log::warn!(
            "首选终端 {} 启动失败，回退到 cmd: {:?}",
            terminal,
            result.as_ref().err()
        );
        return run_windows_start_command(&["cmd", "/K", &bat_path], "cmd");
    }

    result
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn build_shell_cd_command(cwd: Option<&Path>) -> String {
    cwd.map(|dir| {
        format!(
            "cd {} || exit 1\n",
            shell_single_quote(&dir.to_string_lossy())
        )
    })
    .unwrap_or_default()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_windows_unc_path(path: &str) -> bool {
    path.starts_with(r"\\")
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn build_windows_cwd_command_str(path: &str) -> String {
    let escaped = escape_windows_batch_value(path);

    if is_windows_unc_path(path) {
        // `cmd.exe` cannot make a UNC path current via `cd`; `pushd` maps it first.
        format!("pushd \"{escaped}\" || exit /b 1\r\n")
    } else {
        format!("cd /d \"{escaped}\" || exit /b 1\r\n")
    }
}

#[cfg(target_os = "windows")]
fn build_windows_cwd_command(cwd: Option<&Path>) -> String {
    cwd.map(|dir| build_windows_cwd_command_str(&dir.to_string_lossy()))
        .unwrap_or_default()
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn escape_windows_batch_value(value: &str) -> String {
    value
        .replace('^', "^^")
        .replace('%', "%%")
        .replace('&', "^&")
        .replace('|', "^|")
        .replace('<', "^<")
        .replace('>', "^>")
        .replace('(', "^(")
        .replace(')', "^)")
}
/// Windows: Run a start command with common error handling
#[cfg(target_os = "windows")]
fn run_windows_start_command(args: &[&str], terminal_name: &str) -> Result<(), String> {
    use std::process::Command;

    let mut full_args = vec!["/C", "start"];
    full_args.extend(args);

    let output = Command::new("cmd")
        .args(&full_args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("启动 {} 失败: {e}", terminal_name))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{} 启动失败 (exit code: {:?}): {}",
            terminal_name,
            output.status.code(),
            stderr
        ));
    }

    Ok(())
}

/// 打开用户首选终端并在其中执行一条命令行。脚本尾部 `read -n 1` / `pause`
/// 是刻意设计的——让命令退出后窗口不要瞬间关闭，用户才看得到 `command
/// not found` / `ModuleNotFoundError` 这类诊断信息。
///
/// **Security**：`command_line` 会被原样拼进 shell/batch 脚本，调用方必须
/// 保证它是可信字符串（当前只由后端硬编码调用）。
pub(crate) fn launch_terminal_running(command_line: &str, label: &str) -> Result<(), String> {
    launch_terminal_running_with_env(command_line, label, &[])
}

fn write_terminal_launcher(
    label: &str,
    suffix: &str,
    content: &str,
) -> Result<std::path::PathBuf, String> {
    use std::io::Write;

    let mut file = tempfile::Builder::new()
        .prefix(&format!("cc_switch_{label}_"))
        .suffix(suffix)
        .tempfile()
        .map_err(|e| format!("创建终端启动脚本失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("设置终端启动脚本权限失败: {e}"))?;
    }
    file.write_all(content.as_bytes())
        .map_err(|e| format!("写入终端启动脚本失败: {e}"))?;
    file.into_temp_path()
        .keep()
        .map_err(|e| format!("保存终端启动脚本失败: {e}"))
}

fn launcher_env_name(name: &str) -> bool {
    matches!(
        name,
        "HTTP_PROXY"
            | "http_proxy"
            | "HTTPS_PROXY"
            | "https_proxy"
            | "ALL_PROXY"
            | "all_proxy"
            | "NO_PROXY"
            | "no_proxy"
    )
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn unix_launcher_env_lines(env_vars: &[(String, String)]) -> String {
    env_vars
        .iter()
        .filter(|(name, _)| launcher_env_name(name))
        .map(|(name, value)| {
            let quoted = shell_single_quote(value);
            // The login shell may source a profile that changes proxy vars.
            // Keep a private copy and restore it after profile loading.
            format!("export {name}={quoted}\nexport OFOX_LAUNCH_{name}={quoted}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_launcher_env_line(name: &str, value: &str) -> Result<String, String> {
    if !launcher_env_name(name) || value.contains(['"', '\r', '\n', '^']) {
        return Err(format!("无法安全传递代理变量 {name} 到 Windows 终端"));
    }
    Ok(format!("set \"{name}={}\"", value.replace('%', "%%")))
}

/// Start a terminal command with selected proxy variables from Ofox's process
/// or system settings. New Terminal.app windows do not inherit Ofox's process
/// environment, so the variables must be written into the terminal script.
pub(crate) fn launch_terminal_running_with_env(
    command_line: &str,
    label: &str,
    env_vars: &[(String, String)],
) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let script_file = {
        let env_lines = unix_launcher_env_lines(env_vars);
        let content = format!(
            r#"#!/bin/bash
trap 'rm -f -- "$0"' EXIT
echo "[ofox-switch] Starting: {label}"
echo ""
{env_lines}
{cmd}
echo ""
echo "[ofox-switch] Command exited. Press any key to close."
read -n 1 -s
"#,
            cmd = command_line,
        );
        write_terminal_launcher(label, ".sh", &content)?
    };

    #[cfg(target_os = "macos")]
    {
        let preferred = crate::settings::get_preferred_terminal();
        let terminal = preferred.as_deref().unwrap_or("terminal");

        let result = match terminal {
            "iterm2" => launch_macos_iterm2(&script_file),
            "alacritty" => launch_macos_open_app("Alacritty", &script_file, true),
            "kitty" => launch_macos_open_app("kitty", &script_file, false),
            "ghostty" => launch_macos_open_app("Ghostty", &script_file, true),
            "wezterm" => launch_macos_open_app("WezTerm", &script_file, true),
            "kaku" => launch_macos_open_app("Kaku", &script_file, true),
            _ => launch_macos_terminal_app(&script_file),
        };

        let final_result = if result.is_err() && terminal != "terminal" {
            log::warn!(
                "首选终端 {} 启动失败，回退到 Terminal.app: {:?}",
                terminal,
                result.as_ref().err()
            );
            launch_macos_terminal_app(&script_file)
        } else {
            result
        };
        if final_result.is_err() {
            let _ = std::fs::remove_file(&script_file);
        }
        final_result
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        let preferred = crate::settings::get_preferred_terminal();
        let default_terminals = [
            ("gnome-terminal", vec!["--"]),
            ("konsole", vec!["-e"]),
            ("xfce4-terminal", vec!["-e"]),
            ("mate-terminal", vec!["--"]),
            ("lxterminal", vec!["-e"]),
            ("alacritty", vec!["-e"]),
            ("kitty", vec!["-e"]),
            ("ghostty", vec!["-e"]),
        ];

        let terminals_to_try: Vec<(&str, Vec<&str>)> = if let Some(ref pref) = preferred {
            let pref_args = default_terminals
                .iter()
                .find(|(name, _)| *name == pref.as_str())
                .map(|(_, args)| args.to_vec())
                .unwrap_or_else(|| vec!["-e"]);
            let mut list = vec![(pref.as_str(), pref_args)];
            for (name, args) in &default_terminals {
                if *name != pref.as_str() {
                    list.push((*name, args.to_vec()));
                }
            }
            list
        } else {
            default_terminals
                .iter()
                .map(|(name, args)| (*name, args.to_vec()))
                .collect()
        };

        let mut last_error = String::from("未找到可用的终端");

        for (terminal, args) in terminals_to_try {
            let terminal_exists = which_command(terminal)
                || ["/usr/bin", "/bin", "/usr/local/bin"]
                    .iter()
                    .any(|dir| std::path::Path::new(&format!("{}/{}", dir, terminal)).exists());

            if terminal_exists {
                let spawn_result = Command::new(terminal)
                    .args(&args)
                    .arg("bash")
                    .arg(script_file.to_string_lossy().as_ref())
                    .spawn();
                match spawn_result {
                    Ok(_) => return Ok(()),
                    Err(e) => {
                        last_error = format!("执行 {} 失败: {}", terminal, e);
                    }
                }
            }
        }

        let _ = std::fs::remove_file(&script_file);
        Err(last_error)
    }

    #[cfg(target_os = "windows")]
    {
        let env_lines = env_vars
            .iter()
            .map(|(name, value)| windows_launcher_env_line(name, value))
            .collect::<Result<Vec<_>, _>>()?
            .join("\r\n");
        let preferred = crate::settings::get_preferred_terminal();
        let terminal = preferred.as_deref().unwrap_or("cmd");

        let content = format!(
            "@echo off\r\nsetlocal DisableDelayedExpansion\r\necho [ofox-switch] Starting: {cmd}\r\necho.\r\n{env_lines}\r\n{cmd}\r\necho.\r\necho [ofox-switch] Command exited. Press any key to close.\r\npause >nul\r\ndel \"%~f0\" >nul 2>&1\r\n",
            cmd = command_line,
        );
        let bat_file = write_terminal_launcher(label, ".bat", &content)?;

        let bat_path = bat_file.to_string_lossy();
        let ps_cmd = format!("& '{}'", bat_path);

        let result = match terminal {
            "powershell" => run_windows_start_command(
                &["powershell", "-NoExit", "-Command", &ps_cmd],
                "PowerShell",
            ),
            "wt" => run_windows_start_command(&["wt", "cmd", "/K", &bat_path], "Windows Terminal"),
            _ => run_windows_start_command(&["cmd", "/K", &bat_path], "cmd"),
        };

        let final_result = if result.is_err() && terminal != "cmd" {
            log::warn!(
                "首选终端 {} 启动失败，回退到 cmd: {:?}",
                terminal,
                result.as_ref().err()
            );
            run_windows_start_command(&["cmd", "/K", &bat_path], "cmd")
        } else {
            result
        };

        // The .bat self-deletes (`del "%~f0"`) after it runs, but that only
        // fires if *some* terminal actually launched it. If every attempt
        // failed, sweep the temp file ourselves to avoid pollution.
        if final_result.is_err() {
            let _ = std::fs::remove_file(&bat_file);
        }
        final_result
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (command_line, label, env_vars);
        Err("不支持的操作系统".to_string())
    }
}

/// 设置窗口主题（Windows/macOS 标题栏颜色）
/// theme: "dark" | "light" | "system"
#[tauri::command]
pub async fn set_window_theme(window: tauri::Window, theme: String) -> Result<(), String> {
    use tauri::Theme;

    let tauri_theme = match theme.as_str() {
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        _ => None, // system default
    };

    window.set_theme(tauri_theme).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tool_update::probe_fixture_output;

    #[tokio::test]
    async fn cli_scan_confirms_missing_binary_in_an_isolated_directory() {
        let temp = tempfile::tempdir().unwrap();
        let result = scan_cli_paths("fixture-agent", &[temp.path().into()], "", None).await;
        assert_eq!(result.status, InstallationStatus::NotInstalled);
        assert!(result.version.is_none());
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn cli_scan_retains_a_found_executable_that_cannot_start() {
        let temp = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let path = temp.path().join("fixture-agent.exe");
        #[cfg(not(windows))]
        let path = temp.path().join("fixture-agent");
        // Deliberately invalid native executable, requiring no agent install
        // or executable permission changes in the user's environment.
        std::fs::write(&path, "invalid executable fixture").unwrap();
        let result = scan_cli_paths("fixture-agent", &[temp.path().into()], "", None).await;
        assert_eq!(result.status, InstallationStatus::Installed);
        assert_eq!(result.path.as_deref(), Some(path.as_path()));
        assert!(result.version.is_none());
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn cli_candidate_permission_failure_cannot_be_reported_as_uninstalled() {
        let temp = tempfile::tempdir().unwrap();
        let missing = tempfile::tempdir().unwrap();
        let result = scan_cli_paths_with(
            "fixture-agent",
            &[temp.path().into(), missing.path().into()],
            "",
            None,
            |path| {
                if path.starts_with(temp.path()) {
                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).to_string())
                } else {
                    super::super::tool_update::candidate_exists(path)
                }
            },
        )
        .await;
        assert_eq!(result.status, InstallationStatus::Unknown);
        assert!(result.error.is_some());
        assert!(result.path.is_none());
    }

    #[test]
    fn failed_shell_lookup_accepts_scan_evidence_but_not_scan_absence() {
        let probe_error = "Launch shell resolved an alias or function instead of an executable";
        let path = PathBuf::from("/opt/fixture/bin/agent");
        let found = after_failed_shell_lookup(
            probe_error.into(),
            LocalDetection::found(path.clone(), Some("1.2.3".into()), None),
        );
        assert_eq!(found.status, InstallationStatus::Installed);
        assert_eq!(found.path, Some(path));
        assert_eq!(found.version.as_deref(), Some("1.2.3"));

        for scanned in [
            LocalDetection::missing(),
            LocalDetection::failed("Could not inspect /opt/fixture/bin"),
        ] {
            let local = after_failed_shell_lookup(probe_error.into(), scanned);
            assert_eq!(local.status, InstallationStatus::Unknown);
            assert_eq!(local.error.as_deref(), Some(probe_error));
        }
    }

    #[test]
    fn cli_version_failures_keep_the_installed_identity() {
        let path = Path::new("fixture-agent.exe");
        for output in [
            Err("Version probe timed out".into()),
            Err("Access is denied. (os error 5)".into()),
            Ok(probe_fixture_output("1.2.3", "shell startup failed", 1)),
            Ok(probe_fixture_output("not a version", "", 0)),
        ] {
            let result = cli_version_output(path, output);
            assert_eq!(result.status, InstallationStatus::Installed);
            assert_eq!(result.path.as_deref(), Some(path));
            assert!(result.version.is_none());
            assert!(result.error.is_some());
        }
    }

    #[test]
    fn workbuddy_stale_registry_paths_do_not_prove_installation() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("Removed WorkBuddy");
        std::fs::create_dir_all(&old).unwrap();
        let exe = old.join("WorkBuddy.exe");
        std::fs::write(&exe, "fixture").unwrap();
        let icon = workbuddy_registry_candidate("DisplayIcon", &format!("\"{}\",0", exe.display()))
            .unwrap();
        let location =
            workbuddy_registry_candidate("InstallLocation", &old.to_string_lossy()).unwrap();
        std::fs::remove_file(&exe).unwrap();
        let (paths, error) =
            workbuddy_discovery_sources(Some(temp.path().into()), Ok(vec![icon, location]));
        assert!(select_workbuddy_candidate(&paths, error).unwrap().is_none());
    }

    #[test]
    fn workbuddy_registry_failure_is_unknown_unless_an_executable_is_found() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, error) = workbuddy_discovery_sources(
            Some(temp.path().into()),
            Err("registry access denied".into()),
        );
        assert!(select_workbuddy_candidate(&paths, error)
            .unwrap_err()
            .contains("registry access denied"));
        let exe = temp
            .path()
            .join("Programs")
            .join("WorkBuddy")
            .join("WorkBuddy.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "fixture").unwrap();
        let (paths, error) = workbuddy_discovery_sources(
            Some(temp.path().into()),
            Err("registry access denied".into()),
        );
        assert_eq!(
            select_workbuddy_candidate(&paths, error).unwrap(),
            Some(exe.clone())
        );
        let installed = workbuddy_version_output(exe, Err("Version probe timed out".into()));
        assert!(installed.version.is_none());
        assert!(installed.error.as_deref().unwrap().contains("timed out"));
        let (paths, error) = workbuddy_discovery_sources(None, Ok(vec![installed.path.clone()]));
        assert_eq!(
            select_workbuddy_candidate(&paths, error).unwrap(),
            Some(installed.path)
        );
    }

    #[test]
    fn workbuddy_registry_paths_preserve_commas_spaces_and_icon_indexes() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("User, Name").join("WorkBuddy");
        let exe = directory.join("WorkBuddy.exe");
        for value in [
            format!("\"{}\",0", exe.display()),
            format!("\"{}\",-12", exe.display()),
            exe.to_string_lossy().into_owned(),
        ] {
            assert_eq!(
                workbuddy_registry_candidate("DisplayIcon", &value),
                Some(exe.clone())
            );
        }
        assert_eq!(
            workbuddy_registry_candidate(
                "InstallLocation",
                &format!("\"{}\"", directory.display())
            ),
            Some(exe),
        );
        assert!(workbuddy_registry_candidate("DisplayIcon", " \"\" ").is_none());
    }

    #[test]
    fn workbuddy_version_failures_preserve_the_found_application() {
        let path = PathBuf::from("WorkBuddy.exe");
        for output in [
            Err("Version probe timed out".into()),
            Err("Access is denied".into()),
            Ok(probe_fixture_output("5.6.2", "Get-Item failed", 1)),
            Ok(probe_fixture_output("", "", 0)),
        ] {
            let installed = workbuddy_version_output(path.clone(), output);
            assert_eq!(installed.path, path);
            assert!(installed.version.is_none());
            assert!(installed.error.is_some());
        }
    }

    #[test]
    fn wsl_unreachable_or_failed_shell_is_unknown_rather_than_not_installed() {
        let missing = "__OFOX_BIN__\n__OFOX_PATH__/usr/bin:/bin\n";
        for output in [
            Err("wsl.exe was not found".into()),
            Err("Version probe timed out".into()),
            Ok(probe_fixture_output(
                "There is no distribution with the supplied name.",
                "",
                1,
            )),
            Ok(probe_fixture_output(missing, "shell startup failed", 1)),
            Ok(probe_fixture_output("", "", 0)),
            Ok(probe_fixture_output(
                "__OFOX_BIN__alias tool=other\n__OFOX_PATH__/bin\n",
                "",
                0,
            )),
        ] {
            let local = wsl_lookup_result("Fixture-Distro", output).unwrap_err();
            assert_eq!(local.status, InstallationStatus::Unknown);
            assert!(local
                .error
                .as_deref()
                .unwrap()
                .contains("[WSL:Fixture-Distro]"));
        }
        for stderr in ["", "Now using node v22.12.0 (npm v10.9.0)\n"] {
            let local = wsl_lookup_result(
                "Fixture-Distro",
                Ok(probe_fixture_output(missing, stderr, 0)),
            )
            .unwrap_err();
            assert_eq!(local.status, InstallationStatus::NotInstalled);
            assert!(local.error.is_none());
        }
    }

    #[test]
    fn wsl_found_cli_preserves_its_identity_when_the_version_query_fails() {
        let path = "/home/fixture user/.local/bin/agent";
        let stdout =
            format!("__OFOX_BIN__{path}\n__OFOX_PATH__/home/fixture user/.local/bin:/usr/bin\n");
        let (detected, search_path) =
            wsl_lookup_result("Fixture-Distro", Ok(probe_fixture_output(&stdout, "", 0))).unwrap();
        assert_eq!(detected, path);
        assert!(search_path.contains("fixture user"));
        for output in [
            Err("Version probe timed out".into()),
            Ok(probe_fixture_output(
                "",
                "permission denied; 2.1.3 cannot start",
                126,
            )),
        ] {
            let result = wsl_version_result(&detected, "Fixture-Distro", output);
            assert_eq!(result.status, InstallationStatus::Installed);
            assert_eq!(result.path.as_deref(), Some(Path::new(path)));
            assert!(result.version.is_none());
            assert!(result
                .error
                .as_deref()
                .unwrap()
                .contains("[WSL:Fixture-Distro]"));
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_cmd_version_probe_quotes_space_percent_and_ampersand_paths() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("space %PATH% & fixture");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("fixture-agent.cmd");
        std::fs::write(&path, "@echo off\r\necho 2.1.3\r\nexit /b 0\r\n").unwrap();
        let system_path = std::env::var("PATH").unwrap();
        let result = scan_cli_paths("fixture-agent", &[directory], &system_path, None).await;
        assert_eq!(result.status, InstallationStatus::Installed);
        assert_eq!(result.path, Some(path));
        assert_eq!(result.version.as_deref(), Some("2.1.3"));
        assert!(result.error.is_none());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_broken_cmd_installation_is_not_hidden_by_an_older_copy() {
        let temp = tempfile::tempdir().unwrap();
        let older = tempfile::tempdir().unwrap();
        let path = temp.path().join("fixture-agent.cmd");
        std::fs::write(
            &path,
            "@echo off\r\necho node 22.1.0 could not start 1>&2\r\nexit /b 1\r\n",
        )
        .unwrap();
        std::fs::write(
            older.path().join("fixture-agent.cmd"),
            "@echo off\r\necho 1.0.0\r\n",
        )
        .unwrap();
        let result = scan_cli_paths(
            "fixture-agent",
            &[temp.path().into(), older.path().into()],
            &std::env::var("PATH").unwrap(),
            None,
        )
        .await;
        assert_eq!(result.status, InstallationStatus::Installed);
        assert_eq!(result.path, Some(path));
        assert!(result.version.is_none());
        assert!(result.error.as_deref().unwrap().contains("could not start"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn invalid_wsl_preferences_fail_before_running_any_distribution() {
        for (distro, shell, flag) in [
            ("bad;name", None, None),
            ("Fixture-Distro", Some("bash;echo"), None),
            ("Fixture-Distro", Some("bash"), Some("-c;echo")),
        ] {
            let local = try_get_version_wsl("codex", distro, shell, flag).await;
            assert_eq!(local.status, InstallationStatus::Unknown);
            assert!(local.error.is_some());
        }
    }

    #[test]
    fn installation_detection_serializes_missing_and_failure_separately() {
        for (detection, expected) in [
            (LocalDetection::missing(), "notInstalled"),
            (LocalDetection::failed("permission denied"), "unknown"),
        ] {
            let result = unavailable_tool_version(
                "workbuddy",
                "desktopApp",
                detection,
                "macos".into(),
                None,
            );
            let json = serde_json::to_value(result).unwrap();
            assert_eq!(json["installationStatus"], expected);
            assert_eq!(json["installationKind"], "desktopApp");
        }
    }

    #[test]
    fn remote_update_results_cannot_change_local_installation_evidence() {
        let found = LocalDetection::found("/opt/bin/tool".into(), Some("1.0.0".into()), None);
        assert_eq!(cli_update_status(&found, None, true), "failed");
        assert_eq!(cli_update_status(&found, Some("2.0.0"), true), "available");
        assert_eq!(cli_update_status(&found, None, false), "unchecked");
        assert_eq!(found.status, InstallationStatus::Installed);
        let no_version = LocalDetection::found("/opt/bin/tool".into(), None, None);
        assert_eq!(
            cli_update_status(&no_version, Some("2.0.0"), true),
            "unknown"
        );
        let failure = LocalDetection::failed("lookup timed out");
        assert_eq!(cli_update_status(&failure, Some("2.0.0"), true), "failed");
        assert_eq!(failure.status, InstallationStatus::Unknown);
    }

    #[cfg(unix)]
    fn write_cli(path: &Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_scan_preserves_found_but_broken_installations_and_does_not_mask_with_an_older_copy(
    ) {
        let temp = tempfile::tempdir().unwrap();
        let older = tempfile::tempdir().unwrap();
        write_cli(
            &temp.path().join("tool"),
            "echo 'node: not found' >&2; exit 127",
        );
        write_cli(&older.path().join("tool"), "echo 1.0.0");
        let result = scan_cli_paths(
            "tool",
            &[temp.path().into(), older.path().into()],
            "/usr/bin:/bin",
            None,
        )
        .await;
        assert_eq!(result.status, InstallationStatus::Installed);
        assert!(result.version.is_none());
        assert!(result.error.as_deref().unwrap().contains("node: not found"));
        assert_eq!(cli_update_status(&result, Some("2.0.0"), true), "broken");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_scan_only_reports_absent_when_every_supported_candidate_was_checked() {
        let temp = tempfile::tempdir().unwrap();
        let paths = [temp.path().to_path_buf()];
        assert_eq!(
            scan_cli_paths("tool", &paths, "/bin", None).await.status,
            InstallationStatus::NotInstalled
        );
        assert_eq!(
            scan_cli_paths("tool", &paths, "/bin", Some("permission denied".into()))
                .await
                .status,
            InstallationStatus::Unknown
        );
        write_cli(&temp.path().join("tool"), "echo 2.1.3");
        let installed = scan_cli_paths(
            "tool",
            &paths,
            "/bin",
            Some("another path unreadable".into()),
        )
        .await;
        assert_eq!(installed.status, InstallationStatus::Installed);
        assert_eq!(installed.version.as_deref(), Some("2.1.3"));
        assert!(installed.error.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_scan_keeps_a_broken_symlink_installed() {
        let temp = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(temp.path().join("missing-target"), temp.path().join("tool"))
            .unwrap();
        let installed = scan_cli_paths("tool", &[temp.path().into()], "/bin", None).await;
        assert_eq!(installed.status, InstallationStatus::Installed);
        assert!(installed.version.is_none());
        assert!(installed.error.is_some());
    }

    #[test]
    fn home_bin_search_paths_include_hermes_bin() {
        let home = std::path::Path::new("/home/me");
        let paths = home_bin_search_paths(home);
        assert!(paths.contains(&home.join(".local/bin")));
        assert!(paths.contains(&home.join(".hermes/bin")));
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len());
        assert!(home_bin_search_paths(std::path::Path::new("")).is_empty());
    }
    use std::path::PathBuf;

    #[cfg(unix)]
    #[test]
    fn terminal_proxy_exports_are_quoted_and_launcher_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let env = vec![(
            "HTTPS_PROXY".to_string(),
            "http://user:pa'ss@127.0.0.1:7890".to_string(),
        )];
        let lines = unix_launcher_env_lines(&env);
        assert_eq!(
            lines,
            "export HTTPS_PROXY='http://user:pa'\"'\"'ss@127.0.0.1:7890'\nexport OFOX_LAUNCH_HTTPS_PROXY='http://user:pa'\"'\"'ss@127.0.0.1:7890'"
        );

        let path = write_terminal_launcher("proxy_test", ".sh", &lines).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn launcher_can_clear_stale_proxy_vars_on_unix_and_windows() {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let vars = [
                ("HTTPS_PROXY".to_string(), String::new()),
                ("https_proxy".to_string(), String::new()),
            ];
            let lines = unix_launcher_env_lines(&vars);
            assert!(lines.contains("export OFOX_LAUNCH_HTTPS_PROXY=''"));
            assert!(lines.contains("export OFOX_LAUNCH_https_proxy=''"));
        }
        assert_eq!(
            windows_launcher_env_line("HTTPS_PROXY", "").unwrap(),
            "set \"HTTPS_PROXY=\""
        );
        assert_eq!(
            windows_launcher_env_line("https_proxy", "").unwrap(),
            "set \"https_proxy=\""
        );
    }

    #[test]
    fn windows_proxy_assignment_preserves_percent_encoded_credentials() {
        assert_eq!(
            windows_launcher_env_line("HTTPS_PROXY", "http://user:p%40ss@host:7890").unwrap(),
            "set \"HTTPS_PROXY=http://user:p%%40ss@host:7890\""
        );
        assert!(windows_launcher_env_line("HTTPS_PROXY", "http://bad\"host").is_err());
        assert!(windows_launcher_env_line("OTHER", "http://host:7890").is_err());
    }

    #[test]
    fn test_extract_version() {
        assert_eq!(extract_version("claude 1.0.20"), "1.0.20");
        assert_eq!(extract_version("v2.3.4-beta.1"), "2.3.4-beta.1");
        assert_eq!(extract_version("no version here"), "no version here");
    }

    #[cfg(target_os = "macos")]
    mod workbuddy_macos {
        use super::super::*;
        use crate::commands::tool_update::desktop_from_metadata;

        #[test]
        fn accepts_the_official_bundle_identifier_and_reads_version() {
            let path = Path::new("/Applications/WorkBuddy.app");
            let app = desktop_from_metadata(
                path,
                WORKBUDDY_MACOS_BUNDLE_ID,
                &serde_json::json!({
                    "CFBundleIdentifier": "com.tencent.workbuddy.mac",
                    "CFBundleShortVersionString": "5.5.6"
                }),
            )
            .unwrap()
            .unwrap();
            assert_eq!(app.path, path);
            assert_eq!(app.version.as_deref(), Some("5.5.6"));
        }

        #[test]
        fn rejects_the_previous_incorrect_bundle_identifier() {
            assert!(desktop_from_metadata(
                Path::new("/Applications/WorkBuddy.app"),
                WORKBUDDY_MACOS_BUNDLE_ID,
                &serde_json::json!({
                    "CFBundleIdentifier": "com.workbuddy.workbuddy"
                })
            )
            .unwrap()
            .is_none());
        }

        #[test]
        fn falls_back_to_bundle_version() {
            let app = desktop_from_metadata(
                Path::new("/Users/tester/Applications/WorkBuddy.app"),
                WORKBUDDY_MACOS_BUNDLE_ID,
                &serde_json::json!({
                    "CFBundleIdentifier": WORKBUDDY_MACOS_BUNDLE_ID,
                    "CFBundleVersion": "42"
                }),
            )
            .unwrap()
            .unwrap();
            assert_eq!(app.version.as_deref(), Some("42"));
        }
    }

    mod wsl_helpers {
        use super::super::*;

        #[test]
        fn test_is_valid_shell() {
            assert!(is_valid_shell("bash"));
            assert!(is_valid_shell("zsh"));
            assert!(is_valid_shell("sh"));
            assert!(is_valid_shell("fish"));
            assert!(is_valid_shell("dash"));
            assert!(is_valid_shell("/usr/bin/bash"));
            assert!(is_valid_shell("/bin/zsh"));
            assert!(!is_valid_shell("powershell"));
            assert!(!is_valid_shell("cmd"));
            assert!(!is_valid_shell(""));
        }

        #[test]
        fn test_is_valid_shell_flag() {
            assert!(is_valid_shell_flag("-c"));
            assert!(is_valid_shell_flag("-lc"));
            assert!(is_valid_shell_flag("-lic"));
            assert!(!is_valid_shell_flag("-x"));
            assert!(!is_valid_shell_flag(""));
            assert!(!is_valid_shell_flag("--login"));
        }

        #[test]
        fn test_default_flag_for_shell() {
            assert_eq!(default_flag_for_shell("sh"), "-c");
            assert_eq!(default_flag_for_shell("dash"), "-c");
            assert_eq!(default_flag_for_shell("/bin/dash"), "-c");
            assert_eq!(default_flag_for_shell("fish"), "-lc");
            assert_eq!(default_flag_for_shell("bash"), "-lic");
            assert_eq!(default_flag_for_shell("zsh"), "-lic");
            assert_eq!(default_flag_for_shell("/usr/bin/zsh"), "-lic");
        }

        #[test]
        fn test_is_valid_wsl_distro_name() {
            assert!(is_valid_wsl_distro_name("Ubuntu"));
            assert!(is_valid_wsl_distro_name("Ubuntu-22.04"));
            assert!(is_valid_wsl_distro_name("my_distro"));
            assert!(!is_valid_wsl_distro_name(""));
            assert!(!is_valid_wsl_distro_name("distro with spaces"));
            assert!(!is_valid_wsl_distro_name(&"a".repeat(65)));
        }
    }

    #[test]
    fn opencode_extra_search_paths_includes_install_and_fallback_dirs() {
        let home = PathBuf::from("/home/tester");
        let install_dir = Some(std::ffi::OsString::from("/custom/opencode/bin"));
        let xdg_bin_dir = Some(std::ffi::OsString::from("/xdg/bin"));
        let gopath =
            std::env::join_paths([PathBuf::from("/go/path1"), PathBuf::from("/go/path2")]).ok();

        let paths = opencode_extra_search_paths(&home, install_dir, xdg_bin_dir, gopath);

        assert_eq!(paths[0], PathBuf::from("/custom/opencode/bin"));
        assert_eq!(paths[1], PathBuf::from("/xdg/bin"));
        assert!(paths.contains(&PathBuf::from("/home/tester/bin")));
        assert!(paths.contains(&PathBuf::from("/home/tester/.opencode/bin")));
        assert!(paths.contains(&PathBuf::from("/home/tester/.bun/bin")));
        assert!(paths.contains(&PathBuf::from("/home/tester/go/bin")));
        assert!(paths.contains(&PathBuf::from("/go/path1/bin")));
        assert!(paths.contains(&PathBuf::from("/go/path2/bin")));
    }

    #[test]
    fn opencode_extra_search_paths_deduplicates_repeated_entries() {
        let home = PathBuf::from("/home/tester");
        let same_dir = Some(std::ffi::OsString::from("/same/path"));

        let paths = opencode_extra_search_paths(&home, same_dir.clone(), same_dir, None);

        let count = paths
            .iter()
            .filter(|path| path.as_path() == Path::new("/same/path"))
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn opencode_extra_search_paths_deduplicates_bun_default_dir() {
        let home = PathBuf::from("/home/tester");
        let paths = opencode_extra_search_paths(&home, None, None, None);

        let count = paths
            .iter()
            .filter(|path| path.as_path() == Path::new("/home/tester/.bun/bin"))
            .count();
        assert_eq!(count, 1);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn tool_executable_candidates_non_windows_uses_plain_binary_name() {
        let dir = PathBuf::from("/usr/local/bin");
        let candidates = tool_executable_candidates("opencode", &dir);

        assert_eq!(candidates, vec![PathBuf::from("/usr/local/bin/opencode")]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn tool_executable_candidates_windows_prefers_exe_then_cmd_then_plain_name() {
        let dir = PathBuf::from("C:\\tools");
        let candidates = tool_executable_candidates("opencode", &dir);

        assert_eq!(
            candidates,
            vec![
                PathBuf::from("C:\\tools\\opencode.exe"),
                PathBuf::from("C:\\tools\\opencode.cmd"),
                PathBuf::from("C:\\tools\\opencode"),
            ]
        );
    }

    #[test]
    fn resolve_launch_cwd_accepts_existing_directory() {
        let resolved =
            resolve_launch_cwd(Some(std::env::temp_dir().to_string_lossy().into_owned()))
                .expect("temp dir should resolve")
                .expect("temp dir should be present");

        assert!(resolved.is_dir());
    }

    #[test]
    fn resolve_launch_cwd_rejects_missing_directory() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let missing = std::env::temp_dir().join(format!("ofox-switch-missing-{unique}"));

        let error = resolve_launch_cwd(Some(missing.to_string_lossy().into_owned()))
            .expect_err("missing directory should fail");

        assert!(error.contains("目录不存在"));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn build_shell_cd_command_quotes_spaces_and_single_quotes() {
        let command = build_shell_cd_command(Some(Path::new("/tmp/project O'Brien")));

        assert_eq!(command, "cd '/tmp/project O'\"'\"'Brien' || exit 1\n");
    }

    #[test]
    fn build_windows_cwd_command_str_uses_cd_for_drive_paths() {
        let command = build_windows_cwd_command_str(r"C:\work\repo");

        assert_eq!(command, "cd /d \"C:\\work\\repo\" || exit /b 1\r\n");
    }

    #[test]
    fn build_windows_cwd_command_str_uses_pushd_for_unc_paths() {
        let command = build_windows_cwd_command_str(r"\\wsl$\Ubuntu\home\coder\repo");

        assert_eq!(
            command,
            "pushd \"\\\\wsl$\\Ubuntu\\home\\coder\\repo\" || exit /b 1\r\n"
        );
    }

    #[test]
    fn build_windows_cwd_command_str_escapes_batch_metacharacters() {
        let command = build_windows_cwd_command_str(r"\\server\share\100%&(test)");

        assert_eq!(
            command,
            "pushd \"\\\\server\\share\\100%%^&^(test^)\" || exit /b 1\r\n"
        );
    }
}
