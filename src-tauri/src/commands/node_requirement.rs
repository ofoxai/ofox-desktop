//! 装 npm 工具前，看本机的 Node.js 够不够新。
//!
//! 工具的最新版会在 `engines.node` 里写它要的 Node（例如 openclaw 2026.9.9 要
//! `>=24.16.0 <25 || >=26.1.0`），有的还会在 preinstall 里卡住旧 Node。以前只在
//! 终端里报错，界面上只剩一句「请手动运行 npm install」。
//!
//! 规则：没有 Node 由安装器自动装（不问）；有 Node 但不够新，不悄悄升级——它是
//! 整台电脑共用的，版本管理器切了默认版本，之前装在旧版本下的 CLI 还会「消失」。
//! 先告诉用户原因，用户同意后再升级，并把受影响的工具重新装回来。

use std::path::{Path, PathBuf};

use semver::{Op, Version, VersionReq};
use serde::Serialize;

/// 安装前的检查结果，交给前端决定是直接装、还是先问用户。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeRequirement {
    /// `ok`、`missing`（安装器会先装 Node）、`tooOld`、`unknown`（查不到，照常装）、
    /// `notApplicable`（这个工具不走 npm）。
    pub status: &'static str,
    /// 工具最新版的 `engines.node`。
    pub required: Option<String>,
    /// 当前 Node 版本，不带 `v`。
    pub current: Option<String>,
    /// 谁在管这个 Node：`fnm`、`nvm`、`nvm-windows`、`volta`、`homebrew`、
    /// `nodejs`（官方安装包 / winget）、`asdf`、`mise`、`unknown`。
    pub manager: Option<&'static str>,
    /// Ofox 能否替用户升级。
    pub can_upgrade: bool,
    /// 升级会弹管理员授权（UAC）。
    pub needs_admin: bool,
    /// 升级后要重新安装的工具：版本管理器按 Node 版本分开放全局包。
    pub reinstall: Vec<String>,
    /// 用户自己升级时可以运行的命令或做法。
    pub manual: Option<String>,
}

impl NodeRequirement {
    fn plain(status: &'static str) -> Self {
        Self {
            status,
            required: None,
            current: None,
            manager: None,
            can_upgrade: false,
            needs_admin: false,
            reinstall: Vec::new(),
            manual: None,
        }
    }
}

/// `node --version` 的输出满足 npm 的版本范围吗？范围看不懂时返回 `None`。
pub(crate) fn engine_satisfied(version: &str, range: &str) -> Option<bool> {
    let version = Version::parse(version.trim().trim_start_matches('v')).ok()?;
    let alternatives = npm_range(range)?;
    Some(alternatives.iter().any(|req| req.matches(&version)))
}

/// npm 的范围 → semver：`||` 分开的每一段各是一个 `VersionReq`。
fn npm_range(range: &str) -> Option<Vec<VersionReq>> {
    let range = range.trim();
    if range.is_empty() || range == "*" {
        return Some(vec![VersionReq::STAR]);
    }
    range
        .split("||")
        .map(|alternative| VersionReq::parse(&npm_alternative(alternative)?).ok())
        .collect()
}

/// 一段 npm 范围（空格分隔、可能带 `a - b`）→ semver 的逗号分隔写法。
fn npm_alternative(alternative: &str) -> Option<String> {
    let tokens: Vec<&str> = alternative.split_whitespace().collect();
    if let [low, "-", high] = tokens.as_slice() {
        return Some(format!(">={}, <={}", bare(low), bare(high)));
    }
    let mut comparators = Vec::new();
    let mut pending_op = "";
    for token in tokens {
        // `>= 24` 这种运算符和版本号之间带空格的写法
        if token.chars().all(|c| "<>=~^".contains(c)) {
            pending_op = token;
            continue;
        }
        let token = format!("{pending_op}{token}");
        pending_op = "";
        comparators.push(comparator(&token));
    }
    (!comparators.is_empty()).then(|| comparators.join(", "))
}

/// npm 的裸版本号：`24.16.0` 是恰好这个版本，`24.16`、`24` 是这一行的任意版本；
/// semver 会把它们都当成 `^`，所以改写成 `=` 或通配。
fn comparator(token: &str) -> String {
    let op_len = token
        .find(|c: char| !"<>=~^".contains(c))
        .unwrap_or(token.len());
    let (op, version) = token.split_at(op_len);
    let version = bare(version);
    if !op.is_empty() {
        return format!("{op}{version}");
    }
    match version.split('.').count() {
        3 if !version.contains(['x', 'X', '*']) => format!("={version}"),
        2 => format!("{version}.*"),
        1 if version.chars().all(|c| c.is_ascii_digit()) => format!("{version}.*"),
        _ => version.to_string(),
    }
}

fn bare(version: &str) -> &str {
    version.trim().trim_start_matches(['v', '='])
}

/// 升级到哪个大版本：范围里有和当前同一大版本的段（例如 24.15 → `>=24.16 <25`），
/// 就留在这个大版本里升到最新，动静最小；否则取范围里最低的大版本。
pub(crate) fn upgrade_target(range: &str, current: &str) -> Option<u64> {
    let current = Version::parse(current.trim().trim_start_matches('v')).ok()?;
    let lows: Vec<u64> = npm_range(range)?
        .iter()
        .filter_map(|req| {
            req.comparators
                .iter()
                .filter(|c| !matches!(c.op, Op::Less | Op::LessEq))
                .map(|c| c.major)
                .max()
        })
        .collect();
    if lows.contains(&current.major) {
        return Some(current.major);
    }
    lows.into_iter()
        .filter(|major| *major > current.major)
        .min()
}

/// 这个 Node 是谁装的、谁在管。`real` 是跟随链接后的路径。
pub(crate) fn node_manager(path: &Path, real: &Path) -> &'static str {
    let both = format!(
        "{}|{}",
        path.to_string_lossy().replace('\\', "/"),
        real.to_string_lossy().replace('\\', "/")
    )
    .to_lowercase();
    if both.contains("/fnm/node-versions/") || both.contains("fnm_multishells") {
        "fnm"
    } else if both.contains("/.nvm/versions/node/") {
        "nvm"
    } else if both.contains("/nvm/v") || both.contains("/nvm4w/") {
        "nvm-windows"
    } else if both.contains("/volta/") || both.contains("/.volta/") {
        "volta"
    } else if both.contains("/cellar/node") {
        "homebrew"
    } else if both.contains("/.asdf/") {
        "asdf"
    } else if both.contains("/mise/") {
        "mise"
    } else if both.contains("/program files/nodejs/") || both.contains("/usr/local/bin/node") {
        "nodejs"
    } else {
        "unknown"
    }
}

/// Homebrew 公式名：`/opt/homebrew/Cellar/node@22/22.1.0/bin/node` → `node@22`。
fn homebrew_formula(real: &Path) -> Option<String> {
    let real = real.to_string_lossy().replace('\\', "/");
    let lower = real.to_lowercase();
    let start = lower.find("/cellar/")? + "/cellar/".len();
    let formula = real[start..].split('/').next()?;
    formula
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "@.-+_".contains(c))
        .then(|| formula.to_string())
}

/// 怎么升级：命令（Ofox 能替用户跑时）、要不要管理员、会不会让其它全局 CLI 消失。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpgradePlan {
    pub command: Option<String>,
    pub needs_admin: bool,
    /// 版本管理器切了默认版本后，旧版本下的全局包不在新版本里，要重新装。
    pub reinstalls: bool,
    pub manual: String,
}

/// `target` 是要装的大版本；`windows` 时生成 PowerShell 命令，否则 bash。
pub(crate) fn upgrade_plan(
    manager: &str,
    real: &Path,
    target: Option<u64>,
    windows: bool,
    winget: bool,
) -> UpgradePlan {
    let version = target.map_or_else(|| "lts".to_string(), |major| major.to_string());
    let download =
        "从 https://nodejs.org/zh-cn/download 下载并安装 Node.js 最新的长期支持版".to_string();
    let manual_only = |manual: String| UpgradePlan {
        command: None,
        needs_admin: false,
        reinstalls: false,
        manual,
    };
    let join = |steps: &[String]| {
        if windows {
            steps
                .iter()
                .map(|step| {
                    format!(
                        "{step}; if ($LASTEXITCODE -ne 0) {{ throw '升级 Node.js 失败: {step}' }}"
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        } else {
            steps.join(" && ")
        }
    };
    let (steps, needs_admin, reinstalls): (Vec<String>, bool, bool) = match manager {
        "fnm" => {
            let install = if target.is_some() {
                format!("fnm install {version}")
            } else {
                "fnm install --lts".to_string()
            };
            let default = if target.is_some() {
                format!("fnm default {version}")
            } else {
                "fnm default lts-latest".to_string()
            };
            // 安装窗口里的 PowerShell 不加载配置文件，`fnm use` 要先有 fnm 的环境。
            let mut steps = Vec::new();
            if windows {
                steps.push("fnm env --shell powershell | Out-String | Invoke-Expression".to_string());
            }
            steps.extend([install, default]);
            (steps, false, true)
        }
        "nvm" => {
            let line = if target.is_some() { version.clone() } else { "--lts".to_string() };
            let alias = if target.is_some() { version.clone() } else { "'lts/*'".to_string() };
            (
                vec![
                    r#"export NVM_DIR="${NVM_DIR:-$HOME/.nvm}""#.to_string(),
                    r#". "$NVM_DIR/nvm.sh""#.to_string(),
                    // nvm 自己把旧版本的全局包装进新版本
                    format!("nvm install {line} --reinstall-packages-from=current"),
                    format!("nvm alias default {alias}"),
                ],
                false,
                false,
            )
        }
        "nvm-windows" => {
            let line = if target.is_some() { version.clone() } else { "lts".to_string() };
            (
                vec![format!("nvm install {line}"), format!("nvm use {line}")],
                true,
                true,
            )
        }
        "volta" => (
            // Volta 给每个工具固定自己的 Node，换默认版本不影响它们。
            vec![format!(
                "volta install node@{}",
                if target.is_some() { version.as_str() } else { "lts" }
            )],
            false,
            false,
        ),
        "homebrew" => match homebrew_formula(real) {
            Some(formula) => (vec![format!("brew upgrade {formula}")], false, false),
            None => return manual_only(download),
        },
        "nodejs" if windows && winget => (
            vec!["winget install -e --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements".to_string()],
            true,
            false,
        ),
        _ => return manual_only(download),
    };
    let command = join(&steps);
    UpgradePlan {
        manual: command.clone(),
        command: Some(command),
        needs_admin,
        reinstalls,
    }
}

/// `npm ls -g --depth=0 --json` 里 Ofox 认识的工具（不含 `except`）。
pub(crate) fn global_tools(npm_ls_json: &str, except: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(npm_ls_json) else {
        return Vec::new();
    };
    let Some(dependencies) = value.get("dependencies").and_then(|d| d.as_object()) else {
        return Vec::new();
    };
    ["claude", "codex", "gemini", "opencode", "openclaw"]
        .into_iter()
        .filter(|tool| *tool != except)
        .filter(|tool| {
            super::tool_update::npm_package(tool)
                .is_some_and(|package| dependencies.contains_key(package))
        })
        .map(str::to_string)
        .collect()
}

/// 这台机器上装 `tool` 走不走 npm：macOS 上除了 Hermes 都走；Windows 上 Gemini、
/// OpenCode、OpenClaw 走，Claude Code、Codex 只在国内区域走（见 `windows_install`）。
pub(crate) fn uses_npm(tool: &str, windows: bool, mainland: bool) -> bool {
    if super::tool_update::npm_package(tool).is_none() {
        return false;
    }
    !windows || matches!(tool, "gemini" | "opencode" | "openclaw") || mainland
}

/// 范围里选中的那一段的下限和（排他）上限，安装器升级后据此确认版本够了。
pub(crate) fn target_bounds(range: &str, target: Option<u64>) -> Option<(Version, Option<u64>)> {
    let target = target?;
    npm_range(range)?.into_iter().find_map(|req| {
        let low = req
            .comparators
            .iter()
            .filter(|c| {
                matches!(
                    c.op,
                    Op::GreaterEq | Op::Greater | Op::Exact | Op::Caret | Op::Tilde | Op::Wildcard
                )
            })
            .find(|c| c.major == target)?;
        let below = req
            .comparators
            .iter()
            .find(|c| matches!(c.op, Op::Less))
            .map(|c| c.major);
        Some((
            Version::new(low.major, low.minor.unwrap_or(0), low.patch.unwrap_or(0)),
            below,
        ))
    })
}

/// 本机正在用的 Node（新开终端里那个）。
#[derive(Debug, Clone)]
pub(crate) struct NodeInstall {
    pub path: PathBuf,
    pub real: PathBuf,
    pub version: String,
    pub search_path: String,
}

/// 工具最新版的 `engines.node`：`Some(None)` 是没写要求，`None` 是查不到。
async fn latest_engine(package: &str) -> Option<Option<String>> {
    let client = crate::proxy::http_client::get();
    let package = package.replace('/', "%2f");
    for registry in [
        "https://registry.npmjs.org",
        "https://registry.npmmirror.com",
    ] {
        let Ok(response) = client
            .get(format!("{registry}/{package}/latest"))
            .timeout(std::time::Duration::from_secs(7))
            .header("User-Agent", "ofox-desktop")
            .send()
            .await
        else {
            continue;
        };
        let Ok(manifest) = response
            .error_for_status()
            .map(|r| r.json::<serde_json::Value>())
        else {
            continue;
        };
        let Ok(manifest) = manifest.await else {
            continue;
        };
        return Some(
            manifest
                .get("engines")
                .and_then(|engines| engines.get("node"))
                .and_then(|node| node.as_str())
                .map(str::to_string),
        );
    }
    None
}

/// 新开终端会用的 Node：登录 shell 在用户主目录里解析，避开项目目录里固定的版本。
#[cfg(not(target_os = "windows"))]
pub(crate) async fn current_node() -> Option<NodeInstall> {
    use super::tool_update::{bounded_output, shell_resolution};
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut command = tokio::process::Command::new(shell);
    command
        .args([
            "-l",
            "-i",
            "-c",
            r#"printf '\n__OFOX_BIN__%s\n' "$(command -v node)"; printf '__OFOX_PATH__%s\n' "$PATH""#,
        ])
        .current_dir(crate::config::get_home_dir());
    let (path, search_path) = shell_resolution(&bounded_output(command).await.ok()?).ok()?;
    let path = PathBuf::from(path);
    let real = std::fs::canonicalize(&path).ok()?;
    let version = node_version(&path, &search_path).await?;
    Some(NodeInstall {
        path,
        real,
        version,
        search_path,
    })
}

/// 新开终端会用的 Node：注册表里当前的 PATH。
#[cfg(target_os = "windows")]
pub(crate) async fn current_node() -> Option<NodeInstall> {
    use super::windows_tools::{effective_path, find_tool, real_path};
    let path = find_tool("node")?;
    let real = real_path(&path).unwrap_or_else(|_| path.clone());
    let search_path = effective_path().to_string_lossy().into_owned();
    let version = node_version(&path, &search_path).await?;
    Some(NodeInstall {
        path,
        real,
        version,
        search_path,
    })
}

async fn node_version(node: &Path, search_path: &str) -> Option<String> {
    let mut command = tokio::process::Command::new(node);
    command.arg("--version").env("PATH", search_path);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = super::tool_update::bounded_output(command).await.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.trim().trim_start_matches('v');
    (output.status.success() && Version::parse(version).is_ok()).then(|| version.to_string())
}

/// 和这个 Node 装在一起的 Ofox 工具（不含 `except`）。
async fn tools_beside(node: &NodeInstall, except: &str) -> Vec<String> {
    let npm = node
        .path
        .with_file_name(if cfg!(windows) { "npm.cmd" } else { "npm" });
    let mut command = tokio::process::Command::new(&npm);
    command
        .args(["ls", "-g", "--depth=0", "--json"])
        .env("PATH", &node.search_path);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    match super::tool_update::bounded_output(command).await {
        Ok(output) => global_tools(&String::from_utf8_lossy(&output.stdout), except),
        Err(_) => Vec::new(),
    }
}

/// 升级 Node 要用到的全部信息：命令、升级后要重装的包、升级后至少要到的版本。
#[derive(Debug, Clone)]
pub(crate) struct NodeUpgrade {
    /// bash（macOS）或 PowerShell（Windows）命令，已经包含重装受影响的工具。
    pub command: String,
    pub minimum: Version,
    /// 版本号要小于这个大版本（范围里有上限时）。
    pub below: Option<u64>,
    pub manual: String,
}

fn windows_mainland() -> bool {
    crate::ofox_apex::current_apex() == "ofox.io"
}

/// 安装前检查。查不到的一律当作不拦，照常安装。
pub(crate) async fn check(tool: &str) -> NodeRequirement {
    let windows = cfg!(target_os = "windows");
    if !uses_npm(tool, windows, windows && windows_mainland()) {
        return NodeRequirement::plain("notApplicable");
    }
    let Some(node) = current_node().await else {
        return NodeRequirement::plain("missing");
    };
    let mut report = NodeRequirement::plain("unknown");
    report.current = Some(node.version.clone());
    let Some(package) = super::tool_update::npm_package(tool) else {
        return report;
    };
    let Some(engine) = latest_engine(package).await else {
        return report;
    };
    let Some(range) = engine else {
        report.status = "ok";
        return report;
    };
    report.required = Some(range.clone());
    match engine_satisfied(&node.version, &range) {
        Some(true) => report.status = "ok",
        None => {}
        Some(false) => {
            report.status = "tooOld";
            let manager = node_manager(&node.path, &node.real);
            let plan = upgrade_plan(
                manager,
                &node.real,
                upgrade_target(&range, &node.version),
                windows,
                winget_available(),
            );
            report.manager = Some(manager);
            report.can_upgrade = plan.command.is_some();
            report.needs_admin = plan.needs_admin;
            report.manual = Some(plan.manual);
            if plan.reinstalls {
                report.reinstall = tools_beside(&node, tool).await;
            }
        }
    }
    report
}

/// 用户在 Ofox 里同意升级后，安装器要执行的升级。重新检查一遍，不用前端传来的内容。
pub(crate) async fn upgrade_for_install(tool: &str) -> Result<NodeUpgrade, String> {
    let report = check(tool).await;
    if report.status != "tooOld" || !report.can_upgrade {
        return Err("当前不需要、或无法自动升级 Node.js，请刷新后重试。".into());
    }
    let node = current_node().await.ok_or("找不到 Node.js")?;
    let range = report.required.clone().unwrap_or_default();
    let target = upgrade_target(&range, &node.version);
    let windows = cfg!(target_os = "windows");
    let plan = upgrade_plan(
        report.manager.unwrap_or("unknown"),
        &node.real,
        target,
        windows,
        winget_available(),
    );
    let mut command = plan.command.ok_or("无法自动升级 Node.js")?;
    // fnm 切了默认版本后，这个终端里还是旧的；先切过来再重装旧版本下的工具。
    if report.manager == Some("fnm") {
        if let Some(target) = target {
            command.push_str(&format!(
                "{}fnm use {target}",
                if windows { "; " } else { " && " }
            ));
        }
    }
    let packages: Vec<&str> = report
        .reinstall
        .iter()
        .filter_map(|tool| super::tool_update::npm_package(tool))
        .collect();
    if !packages.is_empty() {
        let list = packages
            .iter()
            .map(|package| format!("{package}@latest"))
            .collect::<Vec<_>>()
            .join(" ");
        if windows {
            command.push_str(&format!(
                "; npm.cmd install -g {list}; if ($LASTEXITCODE -ne 0) {{ throw '重新安装 {list} 失败' }}"
            ));
        } else {
            command.push_str(&format!(" && npm install -g {list}"));
        }
    }
    let (minimum, below) = target_bounds(&range, target).ok_or("看不懂工具要求的 Node.js 版本")?;
    Ok(NodeUpgrade {
        command,
        minimum,
        below,
        manual: plan.manual,
    })
}

#[cfg(target_os = "windows")]
fn winget_available() -> bool {
    super::windows_tools::find_tool("winget").is_some()
}

#[cfg(not(target_os = "windows"))]
fn winget_available() -> bool {
    false
}

/// 安装前检查本机 Node.js 是否满足 `tool` 最新版的要求。
#[tauri::command]
pub async fn check_tool_node_requirement(tool: String) -> Result<NodeRequirement, String> {
    Ok(check(&tool).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENCLAW: &str = ">=24.16.0 <25 || >=26.1.0";

    /// `cargo test -- --ignored this_machine --nocapture`：查本机的 Node 和 npm 仓库。
    #[tokio::test]
    #[ignore = "probes this machine's Node.js and the npm registry"]
    async fn this_machine() {
        for tool in ["openclaw", "gemini", "hermes"] {
            println!("{tool}: {:#?}", check(tool).await);
        }
    }

    #[test]
    fn npm_ranges_are_read_the_way_npm_reads_them() {
        assert_eq!(engine_satisfied("v24.15.0", OPENCLAW), Some(false));
        assert_eq!(engine_satisfied("24.16.0", OPENCLAW), Some(true));
        assert_eq!(engine_satisfied("25.1.0", OPENCLAW), Some(false));
        assert_eq!(engine_satisfied("26.2.0", OPENCLAW), Some(true));
        assert_eq!(engine_satisfied("22.12.0", ">=20"), Some(true));
        assert_eq!(
            engine_satisfied("18.20.0", "^18.17.0 || >=20.5.0"),
            Some(true)
        );
        assert_eq!(
            engine_satisfied("20.1.0", "^18.17.0 || >=20.5.0"),
            Some(false)
        );
        assert_eq!(engine_satisfied("20.9.0", "20.x"), Some(true));
        // A bare version is that exact version, not a caret range.
        assert_eq!(engine_satisfied("24.16.1", "24.16.0"), Some(false));
        assert_eq!(engine_satisfied("24.16.9", "24.16"), Some(true));
        assert_eq!(engine_satisfied("24.17.0", "24.16"), Some(false));
        assert_eq!(engine_satisfied("20.0.0", ">= 18"), Some(true));
        assert_eq!(engine_satisfied("19.0.0", "18 - 20"), Some(true));
        assert_eq!(engine_satisfied("21.0.0", "18 - 20"), Some(false));
        assert_eq!(engine_satisfied("1.0.0", "*"), Some(true));
        assert_eq!(engine_satisfied("1.0.0", "not a range"), None);
        assert_eq!(engine_satisfied("garbage", ">=20"), None);
    }

    #[test]
    fn upgrades_stay_on_the_current_line_when_the_range_allows_it() {
        assert_eq!(upgrade_target(OPENCLAW, "24.15.0"), Some(24));
        assert_eq!(upgrade_target(OPENCLAW, "22.12.0"), Some(24));
        assert_eq!(upgrade_target(OPENCLAW, "25.0.0"), Some(26));
        assert_eq!(upgrade_target(">=20.5.0", "18.0.0"), Some(20));
        assert_eq!(upgrade_target("not a range", "18.0.0"), None);
        assert_eq!(
            target_bounds(OPENCLAW, Some(24)),
            Some((Version::new(24, 16, 0), Some(25)))
        );
        assert_eq!(
            target_bounds(OPENCLAW, Some(26)),
            Some((Version::new(26, 1, 0), None))
        );
    }

    #[test]
    fn node_managers_are_told_apart_by_where_node_lives() {
        let manager = |path: &str, real: &str| node_manager(Path::new(path), Path::new(real));
        assert_eq!(
            manager(
                "/Users/u/.local/state/fnm_multishells/1_2/bin/node",
                "/Users/u/.local/share/fnm/node-versions/v24.15.0/installation/bin/node"
            ),
            "fnm"
        );
        assert_eq!(
            manager(
                "/Users/u/.nvm/versions/node/v20.1.0/bin/node",
                "/Users/u/.nvm/versions/node/v20.1.0/bin/node"
            ),
            "nvm"
        );
        assert_eq!(
            manager(
                r"C:\nvm4w\nodejs\node.exe",
                r"C:\Users\u\AppData\Roaming\nvm\v20.1.0\node.exe"
            ),
            "nvm-windows"
        );
        assert_eq!(
            manager(
                "/Users/u/.volta/bin/node",
                "/Users/u/.volta/tools/image/node/20.1.0/bin/node"
            ),
            "volta"
        );
        assert_eq!(
            manager(
                "/opt/homebrew/bin/node",
                "/opt/homebrew/Cellar/node/24.1.0/bin/node"
            ),
            "homebrew"
        );
        assert_eq!(
            manager(
                r"C:\Program Files\nodejs\node.exe",
                r"C:\Program Files\nodejs\node.exe"
            ),
            "nodejs"
        );
        assert_eq!(
            manager("/usr/local/bin/node", "/usr/local/bin/node"),
            "nodejs"
        );
        assert_eq!(manager("/somewhere/node", "/somewhere/node"), "unknown");
    }

    #[test]
    fn upgrade_plans_say_what_changes_for_the_rest_of_the_machine() {
        let none = Path::new("");
        let fnm = upgrade_plan("fnm", none, Some(24), false, false);
        assert_eq!(
            fnm.command.as_deref(),
            Some("fnm install 24 && fnm default 24")
        );
        assert!(fnm.reinstalls && !fnm.needs_admin);

        let nvm = upgrade_plan("nvm", none, Some(24), false, false);
        assert!(nvm
            .command
            .as_deref()
            .unwrap()
            .contains("nvm install 24 --reinstall-packages-from=current"));
        assert!(!nvm.reinstalls);

        let brew = upgrade_plan(
            "homebrew",
            Path::new("/opt/homebrew/Cellar/node@22/22.1.0/bin/node"),
            Some(22),
            false,
            false,
        );
        assert_eq!(brew.command.as_deref(), Some("brew upgrade node@22"));

        let winget = upgrade_plan("nodejs", none, Some(24), true, true);
        assert!(winget
            .command
            .as_deref()
            .unwrap()
            .starts_with("winget install -e --id OpenJS.NodeJS.LTS"));
        assert!(winget.needs_admin);
        assert!(winget.command.as_deref().unwrap().contains("$LASTEXITCODE"));

        let nvm_windows = upgrade_plan("nvm-windows", none, Some(24), true, true);
        assert!(nvm_windows.reinstalls && nvm_windows.needs_admin);

        for (manager, windows, winget) in [
            ("nodejs", false, false),
            ("nodejs", true, false),
            ("unknown", false, false),
            ("asdf", false, false),
        ] {
            let plan = upgrade_plan(manager, none, Some(24), windows, winget);
            assert_eq!(plan.command, None, "{manager}");
            assert!(plan.manual.contains("nodejs.org"));
        }
    }

    #[test]
    fn only_tools_installed_beside_this_node_need_reinstalling() {
        let json = r#"{"dependencies":{"@google/gemini-cli":{"version":"0.63.0"},"opencode-ai":{"version":"1.18.33"},"typescript":{"version":"5.0.0"}}}"#;
        assert_eq!(global_tools(json, "openclaw"), ["gemini", "opencode"]);
        assert_eq!(global_tools(json, "gemini"), ["opencode"]);
        assert!(global_tools("not json", "gemini").is_empty());
    }

    #[test]
    fn only_npm_installs_are_checked() {
        assert!(uses_npm("openclaw", false, false));
        assert!(uses_npm("claude", false, false));
        assert!(!uses_npm("hermes", false, false));
        assert!(!uses_npm("claude", true, false));
        assert!(uses_npm("claude", true, true));
        assert!(uses_npm("gemini", true, false));
    }
}
