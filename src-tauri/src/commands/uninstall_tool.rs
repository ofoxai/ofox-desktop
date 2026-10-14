//! Uninstall a tool: unbind it from Ofox (restore its configuration), retire its
//! Ofox key, then remove the program the way it was installed. The agent's own
//! data (`~/.codex`, sessions, ...) is never touched.
//!
//! This file has two halves. The planner (`uninstall_plan`, `app_plan`) is pure:
//! it only reads the file system, so it is tested against temp directory trees.
//! The orchestration below it takes every side effect through [`Effects`], so
//! tests can check the order of the steps without touching a real tool.

use serde::Serialize;
use serde_json::json;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::tool_update::{
    installed_npm_package, DesktopInstallation, Installation, ToolOperationGuard, UpdatePlan,
};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::RwLock;

use super::ofox_auth::OfoxAuthState;
use crate::app_config::BindableTool;
use crate::ofox_api_keys::ApiKeyError;
use crate::ofox_auth::OfoxAuthManager;
use crate::ofox_secret::SecretStore;
use crate::services::ofox_bind::report::UnbindReport;
use crate::store::AppState;

/// How the program was installed, as the frontend sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UninstallSource {
    Homebrew,
    Npm,
    Pnpm,
    Bun,
    Native,
    App,
    Unknown,
}

/// Why an uninstall cannot be run from here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UninstallBlock {
    UnknownSource,
    AppRunning,
    Busy,
    UnsupportedPlatform,
}

/// What an uninstall does to the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    /// Run the package manager (or the tool's own uninstaller).
    Run {
        program: PathBuf,
        args: Vec<String>,
        path: String,
        /// Environment variables set for this run only.
        env: Vec<(String, String)>,
    },
    /// Delete these files and directories (a native installer's own layout),
    /// then each of `empty_dirs` only if nothing else is left in it.
    Remove {
        paths: Vec<PathBuf>,
        empty_dirs: Vec<PathBuf>,
    },
    /// Move a macOS app bundle to the Trash.
    Trash(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UninstallPlan {
    pub source: UninstallSource,
    pub action: Action,
    /// The command line as the user would read it; `None` when no command runs.
    pub command: Option<String>,
    pub removes: Vec<String>,
    pub keeps: Vec<String>,
}

/// The uninstall cannot be done automatically; the user does it themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualOnly {
    pub source: UninstallSource,
    pub command: Option<String>,
    pub reason: UninstallBlock,
}

/// Why an installed desktop app cannot be removed right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Blocked {
    Manual(ManualOnly),
    AppRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Os {
    Mac,
    Windows,
    Other,
}

impl Os {
    pub(crate) const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// Everything the planner may know about the machine.
pub(crate) struct Facts {
    pub home: PathBuf,
    /// Agent data directories that must never be touched.
    pub keeps: Vec<PathBuf>,
    /// The package manager owning the active install, as `tool_update` found it.
    pub owner: Result<UpdatePlan, String>,
    /// Output of `hermes uninstall --help`, when it could be read.
    pub hermes_help: Option<String>,
    /// Output of `hermes uninstall --dry-run`, when this Hermes has one.
    pub hermes_dry_run: Option<String>,
}

/// `~/...` for anything under `home`, the plain path otherwise.
fn shown(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Whether `path` is `dir` or below it, following links where both exist. A path
/// that climbs out with `..` only counts once the links are resolved.
fn within(path: &Path, dir: &Path) -> bool {
    let climbs = path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir));
    if !climbs && path.starts_with(dir) {
        return true;
    }
    match (std::fs::canonicalize(path), std::fs::canonicalize(dir)) {
        (Ok(path), Ok(dir)) => path.starts_with(dir),
        _ => false,
    }
}

/// Whether removing `path` would take agent data with it, or be taken from it.
fn reaches_data(path: &Path, keeps: &[PathBuf]) -> bool {
    keeps
        .iter()
        .any(|keep| within(path, keep) || within(keep, path))
}

/// Whether `link` is a symbolic link that points into `dir`.
fn links_into(link: &Path, dir: &Path) -> bool {
    let Ok(target) = std::fs::read_link(link) else {
        return false;
    };
    let target = match link.parent() {
        Some(parent) if target.is_relative() => parent.join(target),
        _ => target,
    };
    within(&target, dir)
}

/// Whether `dir` can be read and holds nothing but the entries named.
fn holds_only(dir: &Path, names: &[&str]) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.into_iter().all(|entry| {
        entry.is_ok_and(|entry| {
            let name = entry.file_name();
            names.iter().any(|allowed| name == *allowed)
        })
    })
}

fn quote(argument: &str) -> String {
    let plain = !argument.is_empty()
        && argument
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain {
        argument.to_string()
    } else {
        format!("'{}'", argument.replace('\'', "'\\''"))
    }
}

/// The command line as the user would read it: `brew uninstall --cask x`.
fn render_command(program: &Path, args: &[String]) -> String {
    let name = program
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.display().to_string());
    std::iter::once(quote(&name))
        .chain(args.iter().map(|arg| quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `PATH` with the program's own directory first, so `npm` finds its `node`.
fn path_with(dir: Option<&Path>, search_path: &str) -> String {
    let mut entries: Vec<PathBuf> = dir.map(Path::to_path_buf).into_iter().collect();
    entries.extend(std::env::split_paths(search_path));
    std::env::join_paths(entries)
        .map(|joined| joined.to_string_lossy().into_owned())
        .unwrap_or_else(|_| search_path.to_string())
}

fn keeps_shown(facts: &Facts) -> Vec<String> {
    facts
        .keeps
        .iter()
        .map(|keep| shown(keep, &facts.home))
        .collect()
}

fn manual(source: UninstallSource, command: Option<String>) -> ManualOnly {
    ManualOnly {
        source,
        command,
        reason: UninstallBlock::UnknownSource,
    }
}

/// What the user could run themselves when the source is not known: the manager
/// the location points to, npm when it says nothing.
fn suggested_command(tool: &str, install: &Installation) -> Option<String> {
    if tool == "hermes" {
        return Some("hermes uninstall".into());
    }
    let package = installed_npm_package(tool, &install.path, &install.real)?;
    let located = format!(
        "{}|{}",
        install.real.to_string_lossy(),
        install.path.to_string_lossy()
    )
    .replace('\\', "/")
    .to_lowercase();
    let in_any = |markers: &[&str]| markers.iter().any(|marker| located.contains(marker));
    let remove = if in_any(&["/pnpm/", "/.pnpm/"]) {
        "pnpm remove --global"
    } else if in_any(&["/.bun/"]) {
        "bun remove --global"
    } else if in_any(&["/.volta/", "/volta/"]) {
        "volta uninstall"
    } else if in_any(&["/yarn/", "/.yarn/"]) {
        "yarn global remove"
    } else {
        "npm uninstall --global"
    };
    Some(format!("{remove} {package}"))
}

pub(crate) fn uninstall_plan(
    tool: &str,
    install: &Installation,
    facts: &Facts,
) -> Result<UninstallPlan, ManualOnly> {
    let Ok(update) = &facts.owner else {
        return Err(manual(
            UninstallSource::Unknown,
            suggested_command(tool, install),
        ));
    };
    match update.source {
        "native" => native_plan(tool, install, facts),
        "homebrew" => manager_plan(UninstallSource::Homebrew, tool, install, update, facts),
        "npm" => manager_plan(UninstallSource::Npm, tool, install, update, facts),
        "pnpm" => manager_plan(UninstallSource::Pnpm, tool, install, update, facts),
        "bun" => manager_plan(UninstallSource::Bun, tool, install, update, facts),
        _ => Err(manual(UninstallSource::Unknown, None)),
    }
}

/// The update command with the install verb turned into the remove verb and the
/// `@latest` package spec into the bare package: the same manager, the same
/// prefix or global directory.
fn invert(args: &[String], install: &str, remove: &str, package: &str) -> Option<Vec<String>> {
    let (spec, rest) = args.split_last()?;
    if *spec != format!("{package}@latest") {
        return None;
    }
    let mut inverted = Vec::new();
    let mut swapped = false;
    for argument in rest {
        if !swapped && argument == install {
            inverted.push(remove.to_string());
            swapped = true;
        // Removing a package runs no build scripts.
        } else if !argument.starts_with("--allow-build=") {
            inverted.push(argument.clone());
        }
    }
    inverted.push(package.to_string());
    swapped.then_some(inverted)
}

/// `<Cellar|Caskroom>/<formula>` holding the active install.
fn keg_dir(install: &Installation, formula: &str) -> Option<PathBuf> {
    install
        .real
        .ancestors()
        .find(|dir| {
            matches!(
                dir.file_name().and_then(|n| n.to_str()),
                Some("Cellar" | "Caskroom")
            )
        })
        .map(|dir| dir.join(formula))
}

/// `.../node_modules/<package>` holding the active install.
fn package_dir(install: &Installation, package: &str) -> Option<PathBuf> {
    let real = install.real.to_string_lossy();
    let marker = format!("/node_modules/{package}");
    // The same byte offsets: only the separators differ.
    let index = real.replace('\\', "/").find(&marker)?;
    Some(PathBuf::from(&real[..index + marker.len()]))
}

/// The Homebrew formulas and casks that really are this agent. `tool_update`
/// matches a package by its name alone, but a package that only shares the name
/// is not ours to remove: Homebrew's `dsh` is a distributed shell, not DeepSeek's.
fn is_homebrew_agent(tool: &str, name: &str) -> bool {
    let known: &[&str] = match tool {
        "claude" => &["claude-code"],
        "codex" => &["codex"],
        "gemini" => &["gemini-cli"],
        "opencode" => &["opencode"],
        "openclaw" => &["openclaw"],
        _ => &[],
    };
    known.contains(&name)
}

fn manager_plan(
    source: UninstallSource,
    tool: &str,
    install: &Installation,
    update: &UpdatePlan,
    facts: &Facts,
) -> Result<UninstallPlan, ManualOnly> {
    let unsupported = || manual(source, None);
    let package = installed_npm_package(tool, &install.path, &install.real);
    let (args, removes) = match source {
        UninstallSource::Homebrew => {
            let [verb, kind, formula] = update.args.as_slice() else {
                return Err(unsupported());
            };
            if verb != "upgrade" || !is_homebrew_agent(tool, formula) {
                return Err(unsupported());
            }
            let kept = keg_dir(install, formula)
                .map_or_else(|| formula.clone(), |dir| shown(&dir, &facts.home));
            (
                vec!["uninstall".to_string(), kind.clone(), formula.clone()],
                kept,
            )
        }
        UninstallSource::Npm => {
            let package = package.ok_or_else(unsupported)?;
            let args =
                invert(&update.args, "install", "uninstall", package).ok_or_else(unsupported)?;
            let kept = package_dir(install, package)
                .map_or_else(|| package.to_string(), |dir| shown(&dir, &facts.home));
            (args, kept)
        }
        _ => {
            let package = package.ok_or_else(unsupported)?;
            let args = invert(&update.args, "add", "remove", package).ok_or_else(unsupported)?;
            (args, package.to_string())
        }
    };
    // `brew uninstall` also removes dependencies nothing else needs any more;
    // Node.js can be one of them, and every npm-installed agent runs on it.
    let env = if source == UninstallSource::Homebrew {
        vec![("HOMEBREW_NO_AUTOREMOVE".to_string(), "1".to_string())]
    } else {
        vec![]
    };
    Ok(UninstallPlan {
        source,
        command: Some(render_command(&update.program, &args)),
        action: Action::Run {
            program: update.program.clone(),
            args,
            path: update.path.clone(),
            env,
        },
        removes: vec![removes],
        keeps: keeps_shown(facts),
    })
}

/// What `hermes uninstall --yes` does even though it keeps the user's data: it
/// deletes its code checkout (inside `~/.hermes`), edits shell rc files, removes
/// the desktop app, the gateway service and caches. Its own dry run says exactly
/// what; when it has none, this is what its uninstaller is known to do.
fn hermes_removes(install: &Installation, facts: &Facts) -> Vec<String> {
    let listed = facts
        .hermes_dry_run
        .as_deref()
        .map(|output| hermes_dry_run_actions(output, &facts.home))
        .unwrap_or_default();
    if !listed.is_empty() {
        return listed;
    }
    let checkout = install
        .real
        .ancestors()
        .find(|dir| dir.file_name().is_some_and(|name| name == "hermes-agent"))
        .map_or_else(
            || "the Hermes code checkout".to_string(),
            |dir| shown(dir, &facts.home),
        );
    vec![
        format!("Code checkout: {checkout}"),
        "Hermes PATH entries in shell configs".to_string(),
        "Hermes wrapper scripts and Hermes-managed node/npm/npx symlinks".to_string(),
        "Desktop Chat GUI artifacts and caches".to_string(),
        "Gateway services and standalone gateway processes".to_string(),
    ]
}

fn native_plan(
    tool: &str,
    install: &Installation,
    facts: &Facts,
) -> Result<UninstallPlan, ManualOnly> {
    let home = &facts.home;
    let unrecognised = || {
        manual(
            UninstallSource::Native,
            (tool == "hermes").then(|| "hermes uninstall".to_string()),
        )
    };
    let (paths, empty_dirs) = match tool {
        "claude" => {
            let share = home.join(".local/share/claude");
            let versions = share.join("versions");
            if !within(&install.real, &versions) {
                return Err(unrecognised());
            }
            // Only the installer's own layout is removed. Anything else in
            // there is not ours, and deleting a directory is not undoable.
            if !holds_only(&share, &["versions"]) {
                return Err(unrecognised());
            }
            // The launcher is ours only while it points into the versions.
            let launcher = home.join(".local/bin/claude");
            let mut paths = Vec::new();
            if links_into(&launcher, &versions) {
                paths.push(launcher);
            }
            paths.push(versions);
            (paths, vec![share])
        }
        "opencode" => {
            let bin = home.join(".opencode/bin");
            let binary = bin.join("opencode");
            if !within(&install.real, &binary) {
                return Err(unrecognised());
            }
            (vec![binary], vec![bin])
        }
        "hermes" => {
            let args = facts
                .hermes_help
                .as_deref()
                .and_then(hermes_uninstall_args)
                .ok_or_else(unrecognised)?;
            return Ok(UninstallPlan {
                source: UninstallSource::Native,
                command: Some(render_command(&install.path, &args)),
                action: Action::Run {
                    program: install.path.clone(),
                    args,
                    path: path_with(install.path.parent(), &install.search_path),
                    env: vec![],
                },
                removes: hermes_removes(install, facts),
                keeps: keeps_shown(facts),
            });
        }
        _ => return Err(unrecognised()),
    };
    if paths
        .iter()
        .chain(&empty_dirs)
        .any(|path| reaches_data(path, &facts.keeps))
    {
        return Err(unrecognised());
    }
    Ok(UninstallPlan {
        source: UninstallSource::Native,
        command: None,
        removes: paths
            .iter()
            .chain(&empty_dirs)
            .map(|path| shown(path, home))
            .collect(),
        action: Action::Remove { paths, empty_dirs },
        keeps: keeps_shown(facts),
    })
}

const WINDOWS_CHATGPT_UNINSTALL: &str = "winget uninstall --id 9PLM9XGG6VKS --source msstore";

pub(crate) fn app_plan(
    tool: &str,
    app: &DesktopInstallation,
    running: bool,
    os: Os,
    facts: &Facts,
) -> Result<UninstallPlan, Blocked> {
    let manual_app = |command: Option<&str>, reason| {
        Blocked::Manual(ManualOnly {
            source: UninstallSource::App,
            command: command.map(str::to_string),
            reason,
        })
    };
    match (tool, os) {
        ("chatgpt", Os::Mac) => {
            if running {
                return Err(Blocked::AppRunning);
            }
            let bundle = app.path.clone();
            let is_bundle = bundle.extension().is_some_and(|ext| ext == "app");
            if !is_bundle || reaches_data(&bundle, &facts.keeps) {
                return Err(manual_app(None, UninstallBlock::UnknownSource));
            }
            Ok(UninstallPlan {
                source: UninstallSource::App,
                command: None,
                removes: vec![shown(&bundle, &facts.home)],
                action: Action::Trash(bundle),
                keeps: keeps_shown(facts),
            })
        }
        ("chatgpt", Os::Windows) => Err(manual_app(
            Some(WINDOWS_CHATGPT_UNINSTALL),
            UninstallBlock::UnsupportedPlatform,
        )),
        ("chatgpt", Os::Other) => Err(manual_app(None, UninstallBlock::UnsupportedPlatform)),
        _ => Err(manual_app(None, UninstallBlock::UnknownSource)),
    }
}

/// Whether `hermes uninstall --help` advertises `flag`.
pub(crate) fn hermes_offers(help: &str, flag: &str) -> bool {
    help.split(|c: char| c.is_whitespace() || "[]|,".contains(c))
        .any(|word| word == flag)
}

/// The arguments that remove Hermes but keep its data, if this Hermes can do it
/// without asking questions. Hermes keeps configs and data unless told `--full`,
/// so the option is only trusted when the help also documents `--full`; `--yes`
/// is what spares it the prompts it cannot get here.
pub(crate) fn hermes_uninstall_args(help: &str) -> Option<Vec<String>> {
    let offers = |flag: &str| hermes_offers(help, flag);
    if !(offers("--yes") || offers("-y")) {
        return None;
    }
    let args = |flags: &[&str]| {
        ["uninstall"]
            .into_iter()
            .chain(flags.iter().copied())
            .map(String::from)
            .collect::<Vec<_>>()
    };
    if let Some(keep) = ["--keep-data", "--keep-config", "--keep"]
        .into_iter()
        .find(|flag| offers(flag))
    {
        return Some(args(&[keep, "--yes"]));
    }
    offers("--full").then(|| args(&["--yes"]))
}

/// What Hermes lists under "Would inspect/remove:" in `uninstall --dry-run`,
/// minus the line that claims its whole home is kept (its code checkout lives
/// inside that home and does go).
pub(crate) fn hermes_dry_run_actions(output: &str, home: &Path) -> Vec<String> {
    let home = home.to_string_lossy();
    let mut listed = false;
    let mut actions = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("Would inspect/remove") {
            listed = true;
            continue;
        }
        if !listed {
            continue;
        }
        let text = line.trim_start_matches(|c: char| !c.is_alphanumeric());
        if text.is_empty() || text.starts_with("Keep ") {
            continue;
        }
        let text = if home.is_empty() {
            text.to_string()
        } else {
            text.replace(&*home, "~")
        };
        actions.push(present(&text));
        if actions.len() == MAX_LISTED_ACTIONS {
            break;
        }
    }
    actions
}

const MAX_LISTED_ACTIONS: usize = 20;
const MAX_LINE_CHARS: usize = 300;

/// A line of someone else's output, made safe to show. Every such line passes
/// through here (progress lines through `emit_line`, the Hermes listing through
/// `hermes_dry_run_actions`), so this is the one place to filter it further.
fn present(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_LINE_CHARS)
        .collect()
}

/// Whether any process runs from inside the app bundle (`ps -axo comm=` output).
#[cfg(any(target_os = "macos", test))]
pub(crate) fn bundle_in_use(ps_output: &str, bundle: &Path) -> bool {
    let inside = format!(
        "{}/Contents/",
        bundle.to_string_lossy().trim_end_matches('/')
    );
    ps_output
        .lines()
        .any(|line| line.trim_start().starts_with(&inside))
}

// ---- what the frontend sees ---------------------------------------------

/// What the key step will do (preview).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UninstallKey {
    Revoke,
    KeepShared,
    None,
}

/// What the key step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UninstallKeyOutcome {
    Revoked,
    Queued,
    KeptShared,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UninstallStatus {
    Removed,
    StillPresent,
}

/// A dry run of an uninstall: nothing is changed to produce it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallPreview {
    pub tool: String,
    pub installed: bool,
    pub source: UninstallSource,
    pub command: Option<String>,
    pub removes: Vec<String>,
    pub keeps: Vec<String>,
    pub manual_command: Option<String>,
    pub bound: bool,
    pub shared_with: Vec<String>,
    pub key: UninstallKey,
    pub blocked_reason: Option<UninstallBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallResult {
    pub unbind: Option<UnbindReport>,
    pub key: UninstallKeyOutcome,
    pub status: UninstallStatus,
    pub remaining_path: Option<String>,
    pub kept: Vec<String>,
}

// ---- side effects -------------------------------------------------------

/// What the machine looks like for one tool.
#[derive(Debug, Clone)]
pub(crate) enum Inspection {
    NotInstalled,
    Cli {
        install: Installation,
        owner: Result<UpdatePlan, String>,
        hermes_help: Option<String>,
        hermes_dry_run: Option<String>,
    },
    App {
        app: DesktopInstallation,
        running: bool,
    },
}

/// Everything an uninstall does to the outside world, so tests can stand in.
pub(crate) trait Effects {
    fn os(&self) -> Os;
    async fn inspect(&self, tool: &str) -> Result<Inspection, String>;
    async fn unbind(
        &self,
        tool: &str,
        still_bound: &[String],
        dry_run: bool,
    ) -> Result<UnbindReport, String>;
    /// Revoke the tool's Ofox key (or queue its deletion) and clear the local copy.
    async fn retire_key(&self, tool: &str) -> UninstallKeyOutcome;
    fn forget_bound(&self, tool: &str) -> Result<(), String>;
    /// Run the removal; every line it prints goes to `log`.
    async fn execute(&self, action: &Action, log: &(dyn Fn(&str) + Sync)) -> Result<(), String>;
}

/// Receives `(step, line)`.
pub(crate) type Sink<'a> = &'a (dyn Fn(u32, &str) + Sync);

pub(crate) const TOTAL_STEPS: u32 = 3;
const STEP_UNBIND: u32 = 1;
const STEP_KEY: u32 = 2;
const STEP_REMOVE: u32 = 3;
pub(crate) const UNINSTALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

const UNINSTALLABLE: [&str; 10] = [
    "claude",
    "codex",
    "chatgpt",
    "gemini",
    "opencode",
    "openclaw",
    "hermes",
    "workbuddy",
    "dsh",
    "pi",
];

/// Every line shown to the user, ours and the process's, reaches the sink here
/// and nowhere else.
fn emit_line(sink: Sink<'_>, step: u32, line: &str) {
    sink(step, &present(line));
}

// ---- orchestration ------------------------------------------------------

impl UninstallBlock {
    fn code(self) -> &'static str {
        match self {
            Self::UnknownSource => "unknownSource",
            Self::AppRunning => "appRunning",
            Self::Busy => "busy",
            Self::UnsupportedPlatform => "unsupportedPlatform",
        }
    }
}

fn check_tool(tool: &str) -> Result<(), String> {
    if UNINSTALLABLE.contains(&tool) {
        Ok(())
    } else {
        Err(format!("Unsupported tool: {tool}"))
    }
}

/// The agent's own data that exists on this machine. Nothing here is ever removed.
fn kept_data(tool: &str) -> Vec<PathBuf> {
    use crate::{
        codex_config, config, gemini_config, hermes_config, native_tools, openclaw_config,
        opencode_config,
    };
    let home = config::get_home_dir();
    let native = |tool: BindableTool| native_tools::config_dir(tool).ok();
    let candidates: Vec<PathBuf> = match tool {
        "claude" => vec![
            config::get_claude_config_dir(),
            config::get_claude_mcp_path(),
        ],
        "codex" | "chatgpt" => vec![codex_config::get_codex_config_dir()],
        "gemini" => vec![gemini_config::get_gemini_dir()],
        "opencode" => vec![
            opencode_config::get_opencode_dir(),
            home.join(".local/share/opencode"),
        ],
        "openclaw" => vec![openclaw_config::get_openclaw_dir()],
        // Its home also holds the code checkout, which its uninstaller deletes:
        // only the user's own files are kept.
        "hermes" => {
            let hermes = hermes_config::get_hermes_dir();
            ["config.yaml", ".env", "cron", "sessions", "logs"]
                .map(|name| hermes.join(name))
                .to_vec()
        }
        "pi" => native(BindableTool::Pi).into_iter().collect(),
        "dsh" => native(BindableTool::Dsh).into_iter().collect(),
        "workbuddy" => vec![home.join(".workbuddy")],
        _ => vec![],
    };
    candidates
        .into_iter()
        .filter(|path| std::fs::symlink_metadata(path).is_ok())
        .collect()
}

fn bound_registry() -> Vec<String> {
    crate::settings::get_settings()
        .bound_tools
        .unwrap_or_default()
}

/// The registry without `tool`: what the unbind step is told is still bound.
fn others(registry: &[String], tool: &str) -> Vec<String> {
    registry
        .iter()
        .filter(|other| other.as_str() != tool)
        .cloned()
        .collect()
}

fn has_recorded_key(tool: &str) -> bool {
    tool.parse::<BindableTool>()
        .ok()
        .and_then(crate::settings::get_api_key_meta)
        .is_some()
}

enum Decision {
    NotInstalled,
    Plan(UninstallPlan),
    Manual(ManualOnly),
    AppRunning,
}

fn decide(
    tool: &str,
    inspection: &Inspection,
    home: &Path,
    keeps: Vec<PathBuf>,
    os: Os,
) -> Decision {
    match inspection {
        Inspection::NotInstalled => Decision::NotInstalled,
        Inspection::Cli {
            install,
            owner,
            hermes_help,
            hermes_dry_run,
        } => {
            let facts = Facts {
                home: home.to_path_buf(),
                keeps,
                owner: owner.clone(),
                hermes_help: hermes_help.clone(),
                hermes_dry_run: hermes_dry_run.clone(),
            };
            match uninstall_plan(tool, install, &facts) {
                Ok(plan) => Decision::Plan(plan),
                Err(manual) => Decision::Manual(manual),
            }
        }
        Inspection::App { app, running } => {
            let facts = Facts {
                home: home.to_path_buf(),
                keeps,
                owner: Err("a desktop app has no package manager".into()),
                hermes_help: None,
                hermes_dry_run: None,
            };
            match app_plan(tool, app, *running, os, &facts) {
                Ok(plan) => Decision::Plan(plan),
                Err(Blocked::Manual(manual)) => Decision::Manual(manual),
                Err(Blocked::AppRunning) => Decision::AppRunning,
            }
        }
    }
}

pub(crate) async fn preview<E: Effects>(
    effects: &E,
    tool: &str,
) -> Result<UninstallPreview, String> {
    check_tool(tool)?;
    let home = crate::config::get_home_dir();
    let keeps = kept_data(tool);
    let inspection = effects.inspect(tool).await?;
    let decision = decide(tool, &inspection, &home, keeps.clone(), effects.os());

    let registry = bound_registry();
    let report = effects.unbind(tool, &others(&registry, tool), true).await?;
    let key = if !report.shared_kept_by.is_empty() {
        UninstallKey::KeepShared
    } else if has_recorded_key(tool) {
        UninstallKey::Revoke
    } else {
        UninstallKey::None
    };

    let mut preview = UninstallPreview {
        tool: tool.to_string(),
        installed: !matches!(inspection, Inspection::NotInstalled),
        source: UninstallSource::Unknown,
        command: None,
        removes: vec![],
        keeps: keeps.iter().map(|keep| shown(keep, &home)).collect(),
        manual_command: None,
        bound: registry.iter().any(|bound| bound == tool),
        shared_with: report.shared_kept_by,
        key,
        blocked_reason: None,
    };
    match decision {
        Decision::NotInstalled => {}
        Decision::Plan(plan) => {
            preview.source = plan.source;
            preview.command = plan.command;
            preview.removes = plan.removes;
            preview.keeps = plan.keeps;
        }
        Decision::Manual(manual) => {
            preview.source = manual.source;
            preview.manual_command = manual.command;
            preview.blocked_reason = Some(manual.reason);
        }
        Decision::AppRunning => {
            preview.source = UninstallSource::App;
            preview.blocked_reason = Some(UninstallBlock::AppRunning);
        }
    }
    if ToolOperationGuard::acquire(tool).is_err() {
        preview.blocked_reason = Some(UninstallBlock::Busy);
    }
    Ok(preview)
}

fn remaining_path(inspection: &Inspection, home: &Path) -> Option<String> {
    match inspection {
        Inspection::NotInstalled => None,
        Inspection::Cli { install, .. } => Some(shown(&install.path, home)),
        Inspection::App { app, .. } => Some(shown(&app.path, home)),
    }
}

/// Unbind, retire the key, remove the program. The unbind comes first and
/// stops everything if it fails: a tool must never be left configured for a key
/// that is gone, or for a program that is gone.
pub(crate) async fn uninstall<E: Effects>(
    effects: &E,
    tool: &str,
    sink: Sink<'_>,
) -> Result<UninstallResult, String> {
    check_tool(tool)?;
    let _guard = ToolOperationGuard::acquire(tool)?;
    let home = crate::config::get_home_dir();
    let inspection = effects.inspect(tool).await?;
    let plan = match decide(tool, &inspection, &home, kept_data(tool), effects.os()) {
        Decision::Plan(plan) => plan,
        Decision::NotInstalled => return Err("Tool is not installed".into()),
        Decision::Manual(manual) => {
            return Err(format!("Uninstall blocked: {}", manual.reason.code()))
        }
        Decision::AppRunning => {
            return Err(format!(
                "Uninstall blocked: {}",
                UninstallBlock::AppRunning.code()
            ))
        }
    };

    emit_line(sink, STEP_UNBIND, "Restoring the tool's configuration");
    let report = effects
        .unbind(tool, &others(&bound_registry(), tool), false)
        .await?;

    emit_line(sink, STEP_KEY, "Revoking the Ofox key");
    let key = if report.shared_kept_by.is_empty() {
        effects.retire_key(tool).await
    } else {
        UninstallKeyOutcome::KeptShared
    };
    if let Err(error) = effects.forget_bound(tool) {
        log::warn!("[uninstall] could not update the bound-tools registry for {tool}: {error}");
        emit_line(
            sink,
            STEP_KEY,
            &format!("Could not update the bound tools: {error}"),
        );
    }

    emit_line(sink, STEP_REMOVE, "Removing the program");
    let log = |line: &str| emit_line(sink, STEP_REMOVE, line);
    if let Err(error) = effects.execute(&plan.action, &log).await {
        emit_line(sink, STEP_REMOVE, &error);
    }
    // Ask the machine, not the exit code: a second copy elsewhere still counts.
    let (status, remaining_path) = match effects.inspect(tool).await {
        Ok(Inspection::NotInstalled) => (UninstallStatus::Removed, None),
        Ok(found) => (UninstallStatus::StillPresent, remaining_path(&found, &home)),
        Err(_) => (UninstallStatus::StillPresent, None),
    };
    Ok(UninstallResult {
        unbind: Some(report),
        key,
        status,
        remaining_path,
        kept: plan.keeps,
    })
}

/// Remove this tool from the registry the frontend mirrors into the settings.
pub(crate) fn forget_bound_tool(tool: &str) -> Result<(), String> {
    crate::settings::mutate_settings(|settings| {
        if let Some(bound) = settings.bound_tools.as_mut() {
            bound.retain(|other| other != tool);
        }
    })
    .map_err(|error| error.to_string())
}

/// Delete the tool's key on the server (`delete` gets its id), or queue the
/// deletion for when its account next signs in; either way the local copy goes.
pub(crate) async fn retire_key_with<F, Fut>(
    tool: BindableTool,
    store: Arc<dyn SecretStore>,
    delete: F,
) -> UninstallKeyOutcome
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<(), ApiKeyError>>,
{
    let Some(meta) = crate::settings::get_api_key_meta(tool) else {
        return UninstallKeyOutcome::None;
    };
    // A key stopped at sign-out was already deleted, or already queued then.
    let outcome = if meta.revoked {
        UninstallKeyOutcome::None
    } else {
        match delete(meta.key_id.clone()).await {
            Ok(()) => UninstallKeyOutcome::Revoked,
            Err(error) => {
                log::warn!("[uninstall] could not delete the {tool:?} key now: {error}");
                crate::ofox_api_keys::queue_deletion_unless_stopped(&meta);
                UninstallKeyOutcome::Queued
            }
        }
    };
    if let Err(error) = crate::ofox_api_keys::revoke_local_with_store(tool, store).await {
        log::warn!("[uninstall] could not clear the local {tool:?} key: {error}");
    }
    outcome
}

fn remove_paths(paths: &[PathBuf], log: &(dyn Fn(&str) + Sync)) -> Result<(), String> {
    for path in paths {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Could not inspect {}: {error}", path.display())),
        };
        // A link is removed as itself, never what it points to.
        let removed = if meta.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        removed.map_err(|error| format!("Could not remove {}: {error}", path.display()))?;
        log(&format!("Removed {}", path.display()));
    }
    Ok(())
}

/// Remove each directory only if nothing is left in it.
fn remove_empty_dirs(dirs: &[PathBuf], log: &(dyn Fn(&str) + Sync)) -> Result<(), String> {
    for dir in dirs {
        match std::fs::remove_dir(dir) {
            Ok(()) => log(&format!("Removed {}", dir.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                log(&format!("Kept {}: it holds other files", dir.display()));
            }
            Err(error) => return Err(format!("Could not remove {}: {error}", dir.display())),
        }
    }
    Ok(())
}

async fn run_command(
    command: tokio::process::Command,
    log: &(dyn Fn(&str) + Sync),
    timeout: Duration,
) -> Result<(), String> {
    let status = super::tool_update::run_logged_process(command, |line| log(line), timeout)
        .await
        .map_err(|error| {
            if error.contains("timed out") {
                "Uninstall timed out; check whether the program is still installed".to_string()
            } else {
                error
            }
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Uninstall process exited with {status}"))
    }
}

/// Run one removal. Agent data is guarded when the plan is made, not here.
pub(crate) async fn perform(
    action: &Action,
    log: &(dyn Fn(&str) + Sync),
    timeout: Duration,
) -> Result<(), String> {
    match action {
        Action::Remove { paths, empty_dirs } => {
            remove_paths(paths, log)?;
            remove_empty_dirs(empty_dirs, log)
        }
        Action::Run {
            program,
            args,
            path,
            env,
        } => {
            let mut command = tokio::process::Command::new(program);
            command.args(args).env("PATH", path).env("CI", "1");
            command.envs(env.iter().map(|(name, value)| (name, value)));
            run_command(command, log, timeout).await
        }
        Action::Trash(bundle) => {
            trash_bundle(bundle)?;
            log(&format!("Moved {} to the Trash", bundle.display()));
            Ok(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn trash_bundle(_bundle: &Path) -> Result<(), String> {
    Err("Moving an app to the Trash is only supported on macOS".into())
}

/// `NSFileManager trashItemAtURL:`: the same call Finder's Move to Trash makes. A
/// plain rename into `~/.Trash` is refused by macOS for apps without Full Disk
/// Access, and Finder scripting asks the user for Automation permission.
#[cfg(target_os = "macos")]
fn trash_bundle(bundle: &Path) -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send, msg_send_id};
    use std::ffi::{c_char, CStr, CString};
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(bundle.as_os_str().as_bytes())
        .map_err(|_| format!("Invalid app path: {}", bundle.display()))?;
    // SAFETY: Foundation calls on objects created here and used only on this
    // thread; every pointer passed is either one of those objects or a valid
    // out-parameter, and the selectors are the documented ones.
    unsafe {
        let ns_path: Option<Retained<AnyObject>> =
            msg_send_id![class!(NSString), stringWithUTF8String: path.as_ptr()];
        let ns_path = ns_path.ok_or("Could not read the app path")?;
        let url: Option<Retained<AnyObject>> =
            msg_send_id![class!(NSURL), fileURLWithPath: &*ns_path];
        let url = url.ok_or("Could not read the app path")?;
        let manager: Retained<AnyObject> = msg_send_id![class!(NSFileManager), defaultManager];
        let mut error: *mut AnyObject = std::ptr::null_mut();
        let moved: Bool = msg_send![
            &*manager,
            trashItemAtURL: &*url,
            resultingItemURL: std::ptr::null_mut::<*mut AnyObject>(),
            error: &mut error
        ];
        if moved.as_bool() {
            return Ok(());
        }
        let reason = if error.is_null() {
            "unknown error".to_string()
        } else {
            let text: Option<Retained<AnyObject>> = msg_send_id![&*error, localizedDescription];
            let utf8: *const c_char = match &text {
                Some(text) => msg_send![&**text, UTF8String],
                None => std::ptr::null(),
            };
            if utf8.is_null() {
                "unknown error".to_string()
            } else {
                CStr::from_ptr(utf8).to_string_lossy().into_owned()
            }
        };
        Err(format!(
            "Could not move {} to the Trash: {reason}",
            bundle.display()
        ))
    }
}

// ---- the real machine ---------------------------------------------------

/// Whether any process runs from inside `bundle`.
#[cfg(target_os = "macos")]
async fn bundle_running(bundle: &Path) -> Result<bool, String> {
    let mut command = tokio::process::Command::new("/bin/ps");
    command.args(["-axo", "comm="]);
    let output = super::tool_update::bounded_output(command).await?;
    if !output.status.success() {
        return Err("Could not list the running processes".into());
    }
    Ok(bundle_in_use(
        &String::from_utf8_lossy(&output.stdout),
        bundle,
    ))
}

#[cfg(target_os = "macos")]
async fn detect_chatgpt() -> Result<Option<DesktopInstallation>, String> {
    super::tool_update::codex_desktop_version().await
}

#[cfg(target_os = "windows")]
async fn detect_chatgpt() -> Result<Option<DesktopInstallation>, String> {
    super::windows_chatgpt::detect_chatgpt_installation().await
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn detect_chatgpt() -> Result<Option<DesktopInstallation>, String> {
    Ok(None)
}

async fn inspect_app(tool: &str) -> Result<Inspection, String> {
    let found = if tool == "chatgpt" {
        detect_chatgpt().await?
    } else {
        super::misc::find_workbuddy_app().await?
    };
    let Some(app) = found else {
        return Ok(Inspection::NotInstalled);
    };
    #[cfg(target_os = "macos")]
    let running = bundle_running(&app.path).await?;
    // Windows removes it by hand, so whether it runs does not matter.
    #[cfg(not(target_os = "macos"))]
    let running = false;
    Ok(Inspection::App { app, running })
}

/// `hermes uninstall --help` (can this Hermes keep its data without asking
/// questions?) and, when it advertises one, `hermes uninstall --dry-run` (what
/// would it remove?). Both only print. Not asked on Windows, where Hermes is left
/// to the user.
#[cfg(not(target_os = "windows"))]
async fn hermes_uninstall_output(install: &Installation, flag: &str) -> Option<String> {
    let mut command = tokio::process::Command::new(&install.path);
    command
        .args(["uninstall", flag])
        .env("PATH", &install.search_path)
        .env("PYTHONIOENCODING", "utf-8");
    let output = super::tool_update::bounded_output(command).await.ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "windows")]
async fn hermes_uninstall_output(_install: &Installation, _flag: &str) -> Option<String> {
    None
}

async fn hermes_probe(install: &Installation) -> (Option<String>, Option<String>) {
    let help = hermes_uninstall_output(install, "--help").await;
    let dry_run = match &help {
        Some(help) if hermes_offers(help, "--dry-run") => {
            hermes_uninstall_output(install, "--dry-run").await
        }
        _ => None,
    };
    (help, dry_run)
}

async fn inspect_cli(tool: &str) -> Result<Inspection, String> {
    use super::tool_update::{probe, resolve_plan, ProbeError};
    let install = match probe(tool).await {
        Ok(install) => install,
        Err(ProbeError::NotFound) => return Ok(Inspection::NotInstalled),
        Err(error) => return Err(error.to_string()),
    };
    let owner = resolve_plan(tool, &install).await;
    let (hermes_help, hermes_dry_run) = if tool == "hermes" {
        hermes_probe(&install).await
    } else {
        (None, None)
    };
    Ok(Inspection::Cli {
        install,
        owner,
        hermes_help,
        hermes_dry_run,
    })
}

struct Live {
    app: AppHandle,
    db: Arc<crate::database::Database>,
    manager: Arc<RwLock<OfoxAuthManager>>,
}

impl Live {
    fn new(app: AppHandle, state: &State<'_, AppState>, auth: &State<'_, OfoxAuthState>) -> Self {
        Self {
            app,
            db: state.db.clone(),
            manager: auth.0.clone(),
        }
    }
}

async fn inspect_machine(tool: &str) -> Result<Inspection, String> {
    match tool {
        "chatgpt" | "workbuddy" => inspect_app(tool).await,
        _ => inspect_cli(tool).await,
    }
}

impl Effects for Live {
    fn os(&self) -> Os {
        Os::current()
    }

    async fn inspect(&self, tool: &str) -> Result<Inspection, String> {
        inspect_machine(tool).await
    }

    async fn unbind(
        &self,
        tool: &str,
        still_bound: &[String],
        dry_run: bool,
    ) -> Result<UnbindReport, String> {
        let report =
            super::ofox_auth::unbind_tool_from_ofox_internal(&self.db, tool, still_bound, dry_run)
                .await?;
        if !dry_run {
            super::model_selection::notify(
                &self.app,
                vec![crate::model_selection::canonical_tool(tool)?],
            );
        }
        Ok(report)
    }

    async fn retire_key(&self, tool: &str) -> UninstallKeyOutcome {
        let Ok(bindable) = tool.parse::<BindableTool>() else {
            return UninstallKeyOutcome::None;
        };
        let manager = self.manager.clone();
        let outcome = retire_key_with(
            bindable,
            crate::ofox_secret::default_store(),
            |key_id| async move {
                let bearer = manager
                    .read()
                    .await
                    .get_valid_access_token()
                    .await
                    .map_err(|error| ApiKeyError::Unauthorized(error.to_string()))?;
                crate::ofox_api_keys::delete_remote(&key_id, &bearer).await
            },
        )
        .await;
        super::model_selection::notify(&self.app, vec![bindable.as_str().into()]);
        outcome
    }

    fn forget_bound(&self, tool: &str) -> Result<(), String> {
        forget_bound_tool(tool)?;
        // The tray popover and the health loop re-read the bound tools on this.
        let _ = self.app.emit("ofox-prefs-updated", ());
        Ok(())
    }

    async fn execute(&self, action: &Action, log: &(dyn Fn(&str) + Sync)) -> Result<(), String> {
        perform(action, log, UNINSTALL_TIMEOUT).await
    }
}

/// What an uninstall would do to `tool`, without doing any of it.
#[tauri::command(rename_all = "camelCase")]
pub async fn uninstall_tool_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
    tool: String,
) -> Result<UninstallPreview, String> {
    preview(&Live::new(app, &state, &ofox_state), &tool).await
}

/// Unbind `tool`, revoke its Ofox key, and remove the program by how it was
/// installed. The agent's own data is kept. Progress goes out as
/// `tool-uninstall-progress` events.
#[tauri::command(rename_all = "camelCase")]
pub async fn uninstall_tool(
    app: AppHandle,
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
    tool: String,
    operation_id: String,
) -> Result<UninstallResult, String> {
    if operation_id.is_empty() || operation_id.len() > 128 {
        return Err("Invalid operation ID".into());
    }
    let live = Live::new(app.clone(), &state, &ofox_state);
    let sink = |step: u32, line: &str| {
        let _ = app.emit(
            "tool-uninstall-progress",
            json!({
                "operationId": operation_id,
                "line": line,
                "step": step,
                "totalSteps": TOTAL_STEPS,
            }),
        );
    };
    uninstall(&live, &tool, &sink).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::how_installed::{how_installed, Updater};
    use std::fs;

    fn touch(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn install(path: &Path, real: &Path) -> Installation {
        Installation {
            path: path.into(),
            real: real.into(),
            version: "1.0.0".into(),
            error: None,
            search_path: "/usr/bin:/bin".into(),
        }
    }

    fn facts(home: &Path, owner: Result<UpdatePlan, String>) -> Facts {
        Facts {
            home: home.into(),
            keeps: vec![home.join(".codex")],
            owner,
            hermes_help: None,
            hermes_dry_run: None,
        }
    }

    fn unknown() -> Result<UpdatePlan, String> {
        Err("Unknown installation source".into())
    }

    /// What `tool_update::plan` returns on Windows, built from `how_installed`.
    fn windows_owner(
        tool: &str,
        bin: &Path,
        real: &Path,
        npm: &Path,
    ) -> Result<UpdatePlan, String> {
        let lookup = |name: &str| (name == "npm").then(|| npm.to_path_buf());
        let Updater {
            source,
            program,
            args,
            ..
        } = how_installed(tool, bin, real, &lookup).ok_or("unknown")?;
        Ok(UpdatePlan {
            source,
            program,
            args,
            path: "C:\\Windows".into(),
        })
    }

    fn env_of(plan: &UninstallPlan) -> &[(String, String)] {
        match &plan.action {
            Action::Run { env, .. } => env,
            other => panic!("expected a command, got {other:?}"),
        }
    }

    fn run_of(plan: &UninstallPlan) -> (&Path, &[String]) {
        match &plan.action {
            Action::Run { program, args, .. } => (program, args),
            other => panic!("expected a command, got {other:?}"),
        }
    }

    // ---- package managers -------------------------------------------------

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn homebrew_formulas_and_casks_are_removed_by_brew() {
        let temp = tempfile::tempdir().unwrap();
        let cases = [
            (
                "codex",
                "/opt/homebrew/Cellar/codex/1.0/bin/codex",
                "/opt/homebrew/Cellar/codex",
                ["uninstall", "--formula", "codex"],
            ),
            (
                "claude",
                "/opt/homebrew/Caskroom/claude-code/1.0/claude",
                "/opt/homebrew/Caskroom/claude-code",
                ["uninstall", "--cask", "claude-code"],
            ),
        ];
        for (tool, real, keg, expected) in cases {
            let active = install(Path::new("/opt/homebrew/bin/x"), Path::new(real));
            let owner = super::super::tool_update::plan(tool, &active);
            let plan = uninstall_plan(tool, &active, &facts(temp.path(), owner)).unwrap();
            assert_eq!(plan.source, UninstallSource::Homebrew);
            let (program, args) = run_of(&plan);
            assert_eq!(program, Path::new("/opt/homebrew/bin/brew"));
            assert_eq!(args, expected);
            assert_eq!(
                plan.command.as_deref(),
                Some(format!("brew {}", expected.join(" ")).as_str())
            );
            assert_eq!(plan.removes, [keg]);
            // brew would otherwise also remove dependencies nothing else needs
            // any more, and Node.js may be one of them.
            assert_eq!(
                env_of(&plan),
                [("HOMEBREW_NO_AUTOREMOVE".to_string(), "1".to_string())]
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn only_homebrew_packages_that_really_are_the_agent_are_removed() {
        let temp = tempfile::tempdir().unwrap();
        // The agents' real Homebrew names (checked with `brew info`).
        for (tool, real) in [
            ("claude", "/opt/homebrew/Caskroom/claude-code/1.0/claude"),
            ("claude", "/opt/homebrew/Cellar/claude-code/1.0/bin/claude"),
            ("codex", "/opt/homebrew/Cellar/codex/1.0/bin/codex"),
            ("gemini", "/opt/homebrew/Cellar/gemini-cli/1.0/bin/gemini"),
            ("opencode", "/opt/homebrew/Cellar/opencode/1.0/bin/opencode"),
            ("openclaw", "/opt/homebrew/Caskroom/openclaw/1.0/openclaw"),
        ] {
            let active = install(Path::new("/opt/homebrew/bin/x"), Path::new(real));
            let owner = super::super::tool_update::plan(tool, &active);
            let plan = uninstall_plan(tool, &active, &facts(temp.path(), owner));
            assert!(plan.is_ok(), "{tool} {real}");
        }
        // A package that only shares the name is not ours: Homebrew's `dsh` is
        // a distributed shell.
        for (tool, real) in [
            ("dsh", "/opt/homebrew/Cellar/dsh/0.2.4/bin/dsh"),
            ("pi", "/opt/homebrew/Cellar/pi/3.0/bin/pi"),
            ("hermes", "/opt/homebrew/Cellar/hermes/1.0/bin/hermes"),
        ] {
            let active = install(Path::new("/opt/homebrew/bin/x"), Path::new(real));
            let owner = super::super::tool_update::plan(tool, &active);
            assert_eq!(owner.as_ref().unwrap().source, "homebrew");
            let manual = uninstall_plan(tool, &active, &facts(temp.path(), owner)).unwrap_err();
            assert_eq!(manual.source, UninstallSource::Homebrew, "{tool}");
            assert_eq!(manual.command, None, "{tool}");
            assert_eq!(manual.reason, UninstallBlock::UnknownSource, "{tool}");
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn an_npm_global_is_removed_by_the_npm_that_owns_its_prefix() {
        let temp = tempfile::tempdir().unwrap();
        for (tool, package) in [
            ("codex", "@openai/codex"),
            ("opencode", "opencode-ai"),
            ("pi", "@mariozechner/pi-coding-agent"),
        ] {
            let prefix = temp.path().join("node 中文");
            let real = prefix.join(format!("lib/node_modules/{package}/bin/cli.js"));
            let active = install(&prefix.join("bin").join(tool), &real);
            let owner = super::super::tool_update::plan(tool, &active);
            let plan = uninstall_plan(tool, &active, &facts(temp.path(), owner)).unwrap();
            assert_eq!(plan.source, UninstallSource::Npm);
            assert!(env_of(&plan).is_empty());
            let (program, args) = run_of(&plan);
            assert_eq!(program, prefix.join("bin/npm"));
            assert_eq!(
                args,
                [
                    "uninstall",
                    "--global",
                    "--prefix",
                    prefix.to_str().unwrap(),
                    package
                ]
            );
            assert_eq!(
                plan.removes,
                [format!("~/node 中文/lib/node_modules/{package}")]
            );
        }
    }

    #[test]
    fn a_pnpm_global_is_removed_from_the_global_dir_pnpm_discovered() {
        let temp = tempfile::tempdir().unwrap();
        let pnpm = PathBuf::from("/Users/me/Library/pnpm/store/v11/links/@/pnpm/11.0.0/h/bin/pnpm");
        // The shape `tool_update::resolve_plan` returns for a pnpm 11 install.
        let owner = Ok(UpdatePlan {
            source: "pnpm",
            program: pnpm.clone(),
            args: [
                "--dir",
                "/Users/me/Library/pnpm",
                "add",
                "--global",
                "--allow-build=@google/gemini-cli",
                "--config.global-dir=/Users/me/Library/pnpm/global",
                "--config.global-bin-dir=/Users/me/Library/pnpm",
                "@google/gemini-cli@latest",
            ]
            .map(String::from)
            .to_vec(),
            path: "/usr/bin".into(),
        });
        let active = install(
            Path::new("/Users/me/Library/pnpm/gemini"),
            Path::new(
                "/Users/me/Library/pnpm/global/5/.pnpm/x/node_modules/@google/gemini-cli/a.js",
            ),
        );
        let plan = uninstall_plan("gemini", &active, &facts(temp.path(), owner)).unwrap();
        assert_eq!(plan.source, UninstallSource::Pnpm);
        let (program, args) = run_of(&plan);
        assert_eq!(program, pnpm);
        assert_eq!(
            args,
            [
                "--dir",
                "/Users/me/Library/pnpm",
                "remove",
                "--global",
                "--config.global-dir=/Users/me/Library/pnpm/global",
                "--config.global-bin-dir=/Users/me/Library/pnpm",
                "@google/gemini-cli",
            ]
        );
    }

    #[test]
    fn windows_pnpm_bun_and_npm_installs_go_back_to_their_manager() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        let pnpm_home = root.join("Local/pnpm");
        let shim = pnpm_home.join("gemini.cmd");
        touch(
            &shim,
            "@\"%~dp0\\global\\5\\.pnpm\\@google+gemini-cli@0.63.0\\node_modules\\@google\\gemini-cli\\dist\\index.js\" %*",
        );
        let pnpm_exe = pnpm_home.join("pnpm.exe");
        let lookup = |name: &str| (name == "pnpm").then(|| pnpm_exe.clone());
        let Updater {
            source,
            program,
            args,
            ..
        } = how_installed("gemini", &shim, &shim, &lookup).unwrap();
        let owner = Ok(UpdatePlan {
            source,
            program,
            args,
            path: String::new(),
        });
        let plan = uninstall_plan("gemini", &install(&shim, &shim), &facts(root, owner)).unwrap();
        assert_eq!(plan.source, UninstallSource::Pnpm);
        assert_eq!(run_of(&plan).1, ["remove", "-g", "@google/gemini-cli"]);

        let bun_bin = root.join(".bun/bin");
        touch(&bun_bin.join("bun.exe"), "");
        let bin = bun_bin.join("opencode.exe");
        let real = root.join(".bun/install/global/node_modules/opencode-ai/bin/opencode.exe");
        let owner = windows_owner("opencode", &bin, &real, Path::new("npm.cmd"));
        let plan = uninstall_plan("opencode", &install(&bin, &real), &facts(root, owner)).unwrap();
        assert_eq!(plan.source, UninstallSource::Bun);
        let (program, args) = run_of(&plan);
        assert_eq!(program, bun_bin.join("bun.exe"));
        assert_eq!(args, ["remove", "-g", "opencode-ai"]);

        let prefix = root.join("Roaming/npm");
        touch(&prefix.join("node_modules/@openai/codex/bin/codex.js"), "");
        let bin = prefix.join("codex.cmd");
        touch(
            &bin,
            "\"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*",
        );
        touch(&prefix.join("npm.cmd"), "");
        let owner = windows_owner("codex", &bin, &bin, Path::new("npm.cmd"));
        let plan = uninstall_plan("codex", &install(&bin, &bin), &facts(root, owner)).unwrap();
        assert_eq!(plan.source, UninstallSource::Npm);
        let (program, args) = run_of(&plan);
        assert_eq!(program, prefix.join("npm.cmd"));
        assert_eq!(
            args,
            [
                "uninstall",
                "-g",
                "--prefix",
                prefix.to_str().unwrap(),
                "@openai/codex"
            ]
        );
    }

    #[test]
    fn a_manager_plan_whose_arguments_changed_shape_is_left_to_the_user() {
        let temp = tempfile::tempdir().unwrap();
        let owner = Ok(UpdatePlan {
            source: "npm",
            program: PathBuf::from("/n/bin/npm"),
            args: ["i", "-g", "@openai/codex@next"].map(String::from).to_vec(),
            path: String::new(),
        });
        let active = install(
            Path::new("/n/bin/codex"),
            Path::new("/n/lib/node_modules/@openai/codex/bin/codex.js"),
        );
        let manual = uninstall_plan("codex", &active, &facts(temp.path(), owner)).unwrap_err();
        assert_eq!(manual.reason, UninstallBlock::UnknownSource);
    }

    // ---- native installers ------------------------------------------------

    /// `~/.local/bin/claude` -> `~/.local/share/claude/versions/<v>`.
    fn native_claude(home: &Path) -> Installation {
        let version = home.join(".local/share/claude/versions/2.0.5");
        touch(&version, "binary");
        let link = home.join(".local/bin/claude");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&version, &link).unwrap();
        install(&link, &version)
    }

    fn native_owner(active: &Installation) -> Result<UpdatePlan, String> {
        Ok(UpdatePlan {
            source: "native",
            program: active.path.clone(),
            args: vec!["update".into()],
            path: String::new(),
        })
    }

    #[cfg(unix)]
    #[test]
    fn native_claude_removes_its_launcher_and_its_versions_but_not_its_data() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let active = native_claude(home);
        let mut facts = facts(home, native_owner(&active));
        facts.keeps = vec![home.join(".claude"), home.join(".claude.json")];
        let plan = uninstall_plan("claude", &active, &facts).unwrap();
        assert_eq!(plan.source, UninstallSource::Native);
        // Only what the installer makes; the share directory goes only if it is
        // empty by then.
        assert_eq!(
            plan.action,
            Action::Remove {
                paths: vec![
                    home.join(".local/bin/claude"),
                    home.join(".local/share/claude/versions"),
                ],
                empty_dirs: vec![home.join(".local/share/claude")],
            }
        );
        assert_eq!(plan.command, None);
        assert_eq!(plan.keeps, ["~/.claude", "~/.claude.json"]);
        assert_eq!(
            plan.removes,
            [
                "~/.local/bin/claude",
                "~/.local/share/claude/versions",
                "~/.local/share/claude"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_claude_share_directory_with_anything_but_versions_is_left_to_the_user() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let active = native_claude(home);
        for extra in ["config.json", "cache/entry", "versions.bak"] {
            let extra = home.join(".local/share/claude").join(extra);
            touch(&extra, "not the installer's");
            let manual =
                uninstall_plan("claude", &active, &facts(home, native_owner(&active))).unwrap_err();
            assert_eq!(manual.source, UninstallSource::Native);
            assert_eq!(manual.command, None);
            let top = extra
                .strip_prefix(home.join(".local/share/claude"))
                .unwrap();
            let top = home
                .join(".local/share/claude")
                .join(top.components().next().unwrap());
            if top.is_dir() {
                fs::remove_dir_all(top).unwrap();
            } else {
                fs::remove_file(top).unwrap();
            }
        }
        assert!(uninstall_plan("claude", &active, &facts(home, native_owner(&active))).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_claude_launcher_that_does_not_link_into_the_versions_is_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let active = native_claude(home);
        let elsewhere = home.join("tools/claude");
        touch(&elsewhere, "mine");
        fs::remove_file(home.join(".local/bin/claude")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.join(".local/bin/claude")).unwrap();
        let plan = uninstall_plan("claude", &active, &facts(home, native_owner(&active))).unwrap();
        assert_eq!(
            plan.action,
            Action::Remove {
                paths: vec![home.join(".local/share/claude/versions")],
                empty_dirs: vec![home.join(".local/share/claude")],
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_relative_claude_launcher_into_the_versions_is_still_ours() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let active = native_claude(home);
        fs::remove_file(home.join(".local/bin/claude")).unwrap();
        std::os::unix::fs::symlink(
            "../share/claude/versions/2.0.5",
            home.join(".local/bin/claude"),
        )
        .unwrap();
        let plan = uninstall_plan("claude", &active, &facts(home, native_owner(&active))).unwrap();
        let Action::Remove { paths, .. } = plan.action else {
            panic!("expected removal");
        };
        assert_eq!(paths[0], home.join(".local/bin/claude"));
    }

    #[test]
    fn native_claude_outside_the_home_it_was_installed_into_is_left_to_the_user() {
        let temp = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let active = install(
            &other.path().join(".local/bin/claude"),
            &other.path().join(".local/share/claude/versions/2.0.5"),
        );
        let owner = native_owner(&active);
        let manual = uninstall_plan("claude", &active, &facts(temp.path(), owner)).unwrap_err();
        assert_eq!(manual.reason, UninstallBlock::UnknownSource);
    }

    #[test]
    fn native_opencode_removes_its_binary_and_then_the_bin_directory_if_empty() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let real = home.join(".opencode/bin/opencode");
        touch(&real, "binary");
        let active = install(&real, &real);
        let plan =
            uninstall_plan("opencode", &active, &facts(home, native_owner(&active))).unwrap();
        assert_eq!(
            plan.action,
            Action::Remove {
                paths: vec![home.join(".opencode/bin/opencode")],
                empty_dirs: vec![home.join(".opencode/bin")],
            }
        );
        assert_eq!(
            plan.removes,
            ["~/.opencode/bin/opencode", "~/.opencode/bin"]
        );

        // Something other than the installer's binary is not ours to remove.
        let stray = home.join(".opencode/bin/notes");
        touch(&stray, "mine");
        let active = install(&stray, &stray);
        assert!(uninstall_plan("opencode", &active, &facts(home, native_owner(&active))).is_err());
    }

    #[test]
    fn a_removal_that_would_reach_agent_data_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let real = home.join(".opencode/bin/opencode");
        touch(&real, "binary");
        let active = install(&real, &real);
        let mut facts = facts(home, native_owner(&active));
        for keep in [home.join(".opencode"), home.join(".opencode/bin/state")] {
            facts.keeps = vec![keep];
            assert!(uninstall_plan("opencode", &active, &facts).is_err());
        }
    }

    // ---- Hermes -----------------------------------------------------------

    /// `hermes uninstall --help` of Hermes 0.21.7.
    const HERMES_HELP: &str =
        "usage: hermes uninstall [-h] [--full | --gui | --data] [--gui-summary] [--yes]
                        [--dry-run]

Remove Hermes Agent from your system. Can keep configs/data for reinstall.

options:
  -h, --help     show this help message and exit
  --full         Full uninstall - remove everything including configs and data
  --gui          Uninstall only the desktop Chat GUI, leaving the agent intact
  --data         Remove only the active Hermes home's user data
  --gui-summary  Print a JSON summary
  --yes, -y      Skip confirmation prompts
  --dry-run      Print what uninstall would remove without changing anything
";

    #[test]
    fn hermes_uses_its_own_uninstaller_in_the_mode_that_keeps_data() {
        let args = hermes_uninstall_args(HERMES_HELP).unwrap();
        assert_eq!(args, ["uninstall", "--yes"]);
        assert!(!args.contains(&"--full".to_string()));
    }

    /// `hermes uninstall --dry-run` of Hermes 0.21.7.
    const HERMES_DRY_RUN: &str = "
Dry run: no files, services, or environment entries will be changed.

Would inspect/remove:
  \u{2022} Gateway services and standalone gateway processes
  \u{2022} Hermes PATH entries from shell configs / Windows User PATH
  \u{2022} Hermes wrapper scripts and Hermes-managed node/npm/npx symlinks
  \u{2022} Desktop Chat GUI artifacts
  \u{2022} Code checkout: /home/me/.hermes/hermes-agent
  \u{2022} Keep Hermes config/data: /home/me/.hermes
";

    #[test]
    fn the_hermes_dry_run_lists_what_goes_but_not_the_line_that_keeps_its_whole_home() {
        let expected = [
            "Gateway services and standalone gateway processes",
            "Hermes PATH entries from shell configs / Windows User PATH",
            "Hermes wrapper scripts and Hermes-managed node/npm/npx symlinks",
            "Desktop Chat GUI artifacts",
            "Code checkout: ~/.hermes/hermes-agent",
        ];
        assert_eq!(
            hermes_dry_run_actions(HERMES_DRY_RUN, Path::new("/home/me")),
            expected
        );
        // Output that lost its bullets to a console encoding still parses.
        let garbled = HERMES_DRY_RUN.replace('\u{2022}', "?");
        assert_eq!(
            hermes_dry_run_actions(&garbled, Path::new("/home/me")),
            expected
        );
        assert!(hermes_dry_run_actions("usage: hermes", Path::new("/home/me")).is_empty());
    }

    #[test]
    fn output_shown_to_the_user_has_no_control_characters_and_a_bounded_length() {
        assert_eq!(present("  done\u{1b}[31m red\r\n"), "done[31m red");
        assert_eq!(present(&"x".repeat(1000)).chars().count(), 300);
    }

    #[test]
    fn hermes_is_only_asked_for_a_dry_run_when_it_advertises_one() {
        assert!(hermes_offers(HERMES_HELP, "--dry-run"));
        let old = HERMES_HELP.replace("--dry-run", "").replace("[]", "");
        assert!(!hermes_offers(&old, "--dry-run"));
    }

    #[test]
    fn the_hermes_preview_lists_everything_its_uninstaller_removes() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let bin = home.join(".local/bin/hermes");
        let real = home.join(".hermes/hermes-agent/venv/bin/hermes");
        let active = install(&bin, &real);
        let mut facts = facts(home, native_owner(&active));
        facts.hermes_help = Some(HERMES_HELP.into());

        // Its own dry run, when it has one.
        let listing = HERMES_DRY_RUN.replace("/home/me", home.to_str().unwrap());
        facts.hermes_dry_run = Some(listing);
        let plan = uninstall_plan("hermes", &active, &facts).unwrap();
        assert_eq!(plan.removes.len(), 5);
        assert!(plan
            .removes
            .contains(&"Code checkout: ~/.hermes/hermes-agent".to_string()));
        assert!(plan.removes.iter().all(|line| !line.starts_with("Keep")));

        // Otherwise what it is known to do, never just the launcher.
        facts.hermes_dry_run = None;
        let plan = uninstall_plan("hermes", &active, &facts).unwrap();
        assert!(plan
            .removes
            .contains(&"Code checkout: ~/.hermes/hermes-agent".to_string()));
        assert!(plan
            .removes
            .iter()
            .any(|line| line.contains("PATH entries")));
        assert!(plan
            .removes
            .iter()
            .any(|line| line.contains("Desktop Chat GUI")));
        assert!(plan.removes.iter().any(|line| line.contains("Gateway")));
    }

    #[test]
    fn hermes_data_is_listed_by_what_it_keeps_not_as_its_whole_home() {
        let home = sandbox();
        let hermes = home.home().join(".hermes");
        touch(&hermes.join("config.yaml"), "model: x");
        touch(&hermes.join(".env"), "KEY=1");
        touch(&hermes.join("sessions/one.json"), "{}");
        touch(&hermes.join("hermes-agent/venv/bin/hermes"), "code");
        let kept: Vec<String> = kept_data("hermes")
            .iter()
            .map(|path| shown(path, home.home()))
            .collect();
        assert_eq!(
            kept,
            [
                "~/.hermes/config.yaml",
                "~/.hermes/.env",
                "~/.hermes/sessions"
            ]
        );
    }

    #[test]
    fn a_hermes_that_would_ask_questions_or_cannot_keep_data_is_left_to_the_user() {
        let interactive_only = HERMES_HELP.replace("--yes, -y      Skip confirmation prompts", "");
        let interactive_only = interactive_only.replace("[--yes]", "");
        assert_eq!(hermes_uninstall_args(&interactive_only), None);
        assert_eq!(hermes_uninstall_args("usage: hermes uninstall [-h]"), None);
        assert_eq!(hermes_uninstall_args(""), None);
    }

    #[test]
    fn an_explicit_keep_data_option_wins() {
        let help = format!("{HERMES_HELP}  --keep-data    Keep configs\n");
        assert_eq!(
            hermes_uninstall_args(&help).unwrap(),
            ["uninstall", "--keep-data", "--yes"]
        );
    }

    #[test]
    fn hermes_runs_its_uninstaller_from_where_it_was_found() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let bin = home.join(".local/bin/hermes");
        let real = home.join(".hermes/hermes-agent/venv/bin/hermes");
        let active = install(&bin, &real);
        let mut facts = facts(home, native_owner(&active));
        facts.hermes_help = Some(HERMES_HELP.into());
        let plan = uninstall_plan("hermes", &active, &facts).unwrap();
        assert_eq!(plan.source, UninstallSource::Native);
        let (program, args) = run_of(&plan);
        assert_eq!(program, bin);
        assert_eq!(args, ["uninstall", "--yes"]);
        assert_eq!(plan.command.as_deref(), Some("hermes uninstall --yes"));

        facts.hermes_help = None;
        let manual = uninstall_plan("hermes", &active, &facts).unwrap_err();
        assert_eq!(manual.command.as_deref(), Some("hermes uninstall"));
        assert_eq!(manual.reason, UninstallBlock::UnknownSource);
    }

    // ---- unknown sources --------------------------------------------------

    #[test]
    fn an_unknown_source_is_manual_with_the_usual_package_command() {
        let temp = tempfile::tempdir().unwrap();
        let active = install(Path::new("/custom/gemini"), Path::new("/custom/gemini"));
        let manual = uninstall_plan("gemini", &active, &facts(temp.path(), unknown())).unwrap_err();
        assert_eq!(manual.source, UninstallSource::Unknown);
        assert_eq!(manual.reason, UninstallBlock::UnknownSource);
        assert_eq!(
            manual.command.as_deref(),
            Some("npm uninstall --global @google/gemini-cli")
        );
    }

    #[test]
    fn the_suggested_command_follows_the_manager_the_path_points_to() {
        let temp = tempfile::tempdir().unwrap();
        let facts = facts(temp.path(), unknown());
        for (path, expected) in [
            (
                "/Users/me/.local/share/pnpm/gemini",
                "pnpm remove --global @google/gemini-cli",
            ),
            (
                "/Users/me/.bun/bin/gemini",
                "bun remove --global @google/gemini-cli",
            ),
            (
                "/Users/me/.volta/bin/gemini",
                "volta uninstall @google/gemini-cli",
            ),
            (
                "/Users/me/.yarn/bin/gemini",
                "yarn global remove @google/gemini-cli",
            ),
        ] {
            let active = install(Path::new(path), Path::new(path));
            let manual = uninstall_plan("gemini", &active, &facts).unwrap_err();
            assert_eq!(manual.command.as_deref(), Some(expected), "{path}");
        }
    }

    #[test]
    fn a_native_install_of_an_unrecognised_layout_has_no_guessed_command() {
        let temp = tempfile::tempdir().unwrap();
        let active = install(
            Path::new("/Programs/OpenAI/Codex/bin/codex.exe"),
            Path::new("/Users/me/.codex/packages/standalone/releases/1/bin/codex.exe"),
        );
        let owner = native_owner(&active);
        let manual = uninstall_plan("codex", &active, &facts(temp.path(), owner)).unwrap_err();
        assert_eq!(manual.source, UninstallSource::Native);
        assert_eq!(manual.command, None);
    }

    // ---- desktop app ------------------------------------------------------

    fn app(path: &Path) -> DesktopInstallation {
        DesktopInstallation {
            path: path.into(),
            version: Some("26.1".into()),
            error: None,
        }
    }

    #[test]
    fn the_macos_chatgpt_app_is_moved_to_the_trash() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("Applications/ChatGPT.app");
        let plan = app_plan(
            "chatgpt",
            &app(&bundle),
            false,
            Os::Mac,
            &facts(temp.path(), unknown()),
        )
        .unwrap();
        assert_eq!(plan.source, UninstallSource::App);
        assert_eq!(plan.action, Action::Trash(bundle));
        assert_eq!(plan.command, None);
        assert_eq!(plan.removes, ["~/Applications/ChatGPT.app"]);
        assert_eq!(plan.keeps, ["~/.codex"]);
    }

    #[test]
    fn a_running_chatgpt_app_is_not_removed() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("ChatGPT.app");
        let blocked = app_plan(
            "chatgpt",
            &app(&bundle),
            true,
            Os::Mac,
            &facts(temp.path(), unknown()),
        )
        .unwrap_err();
        assert_eq!(blocked, Blocked::AppRunning);
    }

    #[test]
    fn windows_chatgpt_and_other_desktop_apps_are_manual() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("ChatGPT.app");
        let facts = facts(temp.path(), unknown());
        let Err(Blocked::Manual(windows)) =
            app_plan("chatgpt", &app(&bundle), false, Os::Windows, &facts)
        else {
            panic!("expected a manual uninstall");
        };
        assert_eq!(windows.source, UninstallSource::App);
        assert_eq!(windows.reason, UninstallBlock::UnsupportedPlatform);
        assert_eq!(
            windows.command.as_deref(),
            Some("winget uninstall --id 9PLM9XGG6VKS --source msstore")
        );
        let Err(Blocked::Manual(linux)) =
            app_plan("chatgpt", &app(&bundle), false, Os::Other, &facts)
        else {
            panic!("expected a manual uninstall");
        };
        assert_eq!(
            (linux.reason, linux.command),
            (UninstallBlock::UnsupportedPlatform, None)
        );
        let Err(Blocked::Manual(other)) =
            app_plan("workbuddy", &app(&bundle), false, Os::Mac, &facts)
        else {
            panic!("expected a manual uninstall");
        };
        assert_eq!(other.reason, UninstallBlock::UnknownSource);
    }

    #[test]
    fn a_process_inside_the_bundle_means_the_app_is_running() {
        let bundle = Path::new("/Applications/ChatGPT.app");
        let ps = "/sbin/launchd\n/Applications/Safari.app/Contents/MacOS/Safari\n\
                  /Applications/ChatGPT.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper\n";
        assert!(bundle_in_use(ps, bundle));
        let ps = "/sbin/launchd\n/Applications/ChatGPT.app.backup/Contents/MacOS/x\n/usr/bin/ChatGPT.app\n";
        assert!(!bundle_in_use(ps, bundle));
        assert!(!bundle_in_use("", bundle));
    }

    // ---- orchestration ----------------------------------------------------

    use crate::ofox_secret::{InMemoryStore, Slot};
    use crate::settings::{self, ApiKeyMeta, KeyToDelete};
    use crate::test_sandbox::sandbox;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Records every effect, in order, and answers from what the test set up.
    struct Fake {
        calls: Mutex<Vec<String>>,
        inspections: Mutex<VecDeque<Result<Inspection, String>>>,
        unbind: Result<UnbindReport, String>,
        key: UninstallKeyOutcome,
        execute: Result<(), String>,
        output: Vec<&'static str>,
    }

    impl Fake {
        fn new(inspections: impl IntoIterator<Item = Result<Inspection, String>>) -> Self {
            Self {
                calls: Mutex::default(),
                inspections: Mutex::new(inspections.into_iter().collect()),
                unbind: Ok(UnbindReport::new("codex", false)),
                key: UninstallKeyOutcome::Revoked,
                execute: Ok(()),
                output: vec![],
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn record(&self, call: impl Into<String>) {
            self.calls.lock().unwrap().push(call.into());
        }
    }

    impl Effects for Fake {
        fn os(&self) -> Os {
            Os::Mac
        }

        async fn inspect(&self, _tool: &str) -> Result<Inspection, String> {
            self.record("inspect");
            let mut queue = self.inspections.lock().unwrap();
            if queue.len() > 1 {
                queue.pop_front().unwrap()
            } else {
                queue.front().cloned().unwrap()
            }
        }

        async fn unbind(
            &self,
            tool: &str,
            still_bound: &[String],
            dry_run: bool,
        ) -> Result<UnbindReport, String> {
            self.record(format!("unbind:{tool}:{dry_run}:{}", still_bound.join(",")));
            self.unbind.clone()
        }

        async fn retire_key(&self, tool: &str) -> UninstallKeyOutcome {
            self.record(format!("retire_key:{tool}"));
            self.key
        }

        fn forget_bound(&self, tool: &str) -> Result<(), String> {
            self.record(format!("forget_bound:{tool}"));
            Ok(())
        }

        async fn execute(
            &self,
            _action: &Action,
            log: &(dyn Fn(&str) + Sync),
        ) -> Result<(), String> {
            self.record("execute");
            self.output.iter().for_each(|line| log(line));
            self.execute.clone()
        }
    }

    /// A Codex installed by npm under `/n`.
    fn codex_by_npm() -> Inspection {
        Inspection::Cli {
            install: install(
                Path::new("/n/bin/codex"),
                Path::new("/n/lib/node_modules/@openai/codex/bin/codex.js"),
            ),
            owner: Ok(UpdatePlan {
                source: "npm",
                program: PathBuf::from("/n/bin/npm"),
                args: [
                    "install",
                    "--global",
                    "--prefix",
                    "/n",
                    "@openai/codex@latest",
                ]
                .map(String::from)
                .to_vec(),
                path: "/n/bin:/usr/bin".into(),
            }),
            hermes_help: None,
            hermes_dry_run: None,
        }
    }

    fn bind(tools: &[&str]) {
        let tools: Vec<String> = tools.iter().map(|tool| tool.to_string()).collect();
        settings::mutate_settings(|s| s.bound_tools = Some(tools)).unwrap();
    }

    fn key_of(tool: BindableTool, key_id: &str) -> ApiKeyMeta {
        ApiKeyMeta {
            tool,
            key_id: key_id.into(),
            name: None,
            alias: None,
            key_start: None,
            created_at: 1,
            last_used_at: None,
            owner: None,
            revoked: false,
        }
    }

    #[tokio::test]
    async fn unbind_comes_before_the_key_is_revoked_before_the_program_is_removed() {
        let _home = sandbox();
        bind(&["claude", "codex"]);
        let fake = Fake::new([Ok(codex_by_npm()), Ok(Inspection::NotInstalled)]);

        let result = uninstall(&fake, "codex", &|_, _| {}).await.unwrap();

        assert_eq!(
            fake.calls(),
            [
                "inspect",
                "unbind:codex:false:claude",
                "retire_key:codex",
                "forget_bound:codex",
                "execute",
                "inspect",
            ]
        );
        assert_eq!(result.status, UninstallStatus::Removed);
        assert_eq!(result.key, UninstallKeyOutcome::Revoked);
        assert_eq!(result.remaining_path, None);
        assert!(result.unbind.is_some());
    }

    #[tokio::test]
    async fn a_failed_unbind_stops_before_anything_else_is_done() {
        let _home = sandbox();
        let mut fake = Fake::new([Ok(codex_by_npm())]);
        fake.unbind = Err("config is locked".into());

        let error = uninstall(&fake, "codex", &|_, _| {}).await.unwrap_err();

        assert!(error.contains("config is locked"), "{error}");
        assert_eq!(fake.calls(), ["inspect", "unbind:codex:false:"]);
    }

    #[tokio::test]
    async fn a_key_still_used_by_another_bound_tool_is_kept() {
        let _home = sandbox();
        let mut fake = Fake::new([Ok(codex_by_npm()), Ok(Inspection::NotInstalled)]);
        let mut report = UnbindReport::new("codex", false);
        report.shared_kept_by = vec!["chatgpt".into()];
        fake.unbind = Ok(report);

        let result = uninstall(&fake, "codex", &|_, _| {}).await.unwrap();

        assert_eq!(result.key, UninstallKeyOutcome::KeptShared);
        assert!(!fake
            .calls()
            .iter()
            .any(|call| call.starts_with("retire_key")));
    }

    #[tokio::test]
    async fn a_key_that_could_not_be_deleted_remotely_is_reported_queued() {
        let _home = sandbox();
        let mut fake = Fake::new([Ok(codex_by_npm()), Ok(Inspection::NotInstalled)]);
        fake.key = UninstallKeyOutcome::Queued;
        let result = uninstall(&fake, "codex", &|_, _| {}).await.unwrap();
        assert_eq!(result.key, UninstallKeyOutcome::Queued);
    }

    #[tokio::test]
    async fn a_program_still_found_after_the_command_is_reported_with_its_path() {
        let _home = sandbox();
        let mut fake = Fake::new([Ok(codex_by_npm()), Ok(codex_by_npm())]);
        fake.execute = Err("Uninstall process exited with exit status: 1".into());

        let result = uninstall(&fake, "codex", &|_, _| {}).await.unwrap();

        assert_eq!(result.status, UninstallStatus::StillPresent);
        assert_eq!(result.remaining_path.as_deref(), Some("/n/bin/codex"));
    }

    #[tokio::test]
    async fn a_program_that_is_gone_counts_as_removed_even_if_the_command_complained() {
        let _home = sandbox();
        let mut fake = Fake::new([Ok(codex_by_npm()), Ok(Inspection::NotInstalled)]);
        fake.execute = Err("exit status: 1".into());
        let result = uninstall(&fake, "codex", &|_, _| {}).await.unwrap();
        assert_eq!(result.status, UninstallStatus::Removed);
    }

    #[tokio::test]
    async fn a_failed_recheck_is_not_reported_as_removed() {
        let _home = sandbox();
        let fake = Fake::new([Ok(codex_by_npm()), Err("shell lookup failed".into())]);
        let result = uninstall(&fake, "codex", &|_, _| {}).await.unwrap();
        assert_eq!(result.status, UninstallStatus::StillPresent);
        assert_eq!(result.remaining_path, None);
    }

    #[tokio::test]
    async fn nothing_is_changed_when_the_tool_cannot_be_uninstalled_from_here() {
        let _home = sandbox();
        let unknown = Inspection::Cli {
            install: install(Path::new("/custom/codex"), Path::new("/custom/codex")),
            owner: unknown(),
            hermes_help: None,
            hermes_dry_run: None,
        };
        let running = Inspection::App {
            app: app(Path::new("/Applications/ChatGPT.app")),
            running: true,
        };
        let cases = [
            (Inspection::NotInstalled, "codex", "not installed"),
            (unknown, "codex", "unknownSource"),
            (running, "chatgpt", "appRunning"),
        ];
        for (inspection, tool, reason) in cases {
            let fake = Fake::new([Ok(inspection)]);
            let error = uninstall(&fake, tool, &|_, _| {}).await.unwrap_err();
            assert!(error.contains(reason), "{error}");
            assert_eq!(fake.calls(), ["inspect"]);
        }
    }

    #[tokio::test]
    async fn an_unsupported_tool_or_a_busy_one_is_refused_before_any_probe() {
        let _home = sandbox();
        let fake = Fake::new([Ok(codex_by_npm())]);
        assert!(uninstall(&fake, "vim", &|_, _| {}).await.is_err());
        let _running = ToolOperationGuard::acquire("codex").unwrap();
        let error = uninstall(&fake, "codex", &|_, _| {}).await.unwrap_err();
        assert!(error.contains("already running"), "{error}");
        assert!(fake.calls().is_empty());
    }

    #[tokio::test]
    async fn progress_lines_are_numbered_by_step_and_all_go_through_the_sink() {
        let _home = sandbox();
        let mut fake = Fake::new([Ok(codex_by_npm()), Ok(Inspection::NotInstalled)]);
        fake.output = vec!["removed 12 packages", "done"];
        let lines = Mutex::new(Vec::new());

        uninstall(&fake, "codex", &|step, line| {
            lines.lock().unwrap().push((step, line.to_string()));
        })
        .await
        .unwrap();

        let lines = lines.into_inner().unwrap();
        let steps: Vec<u32> = lines.iter().map(|(step, _)| *step).collect();
        assert_eq!(steps.first(), Some(&1));
        assert!(steps.contains(&2));
        assert_eq!(steps.last(), Some(&3));
        assert!(steps.windows(2).all(|pair| pair[0] <= pair[1]));
        let process: Vec<&str> = lines
            .iter()
            .filter(|(step, _)| *step == 3)
            .map(|(_, line)| line.as_str())
            .collect();
        assert!(process.contains(&"removed 12 packages") && process.contains(&"done"));
        assert_eq!(TOTAL_STEPS, 3);
    }

    #[tokio::test]
    async fn the_result_lists_the_data_that_was_kept() {
        let home = sandbox();
        fs::create_dir_all(home.home().join(".codex")).unwrap();
        let fake = Fake::new([Ok(codex_by_npm()), Ok(Inspection::NotInstalled)]);
        let result = uninstall(&fake, "codex", &|_, _| {}).await.unwrap();
        assert_eq!(result.kept, ["~/.codex"]);
    }

    // ---- preview ----------------------------------------------------------

    /// Every file below `dir` with its content, to prove nothing changed.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, String)> {
        let mut found = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(path) = pending.pop() {
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                pending.extend(fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            } else {
                let text = fs::read_to_string(&path).unwrap_or_else(|_| "link".into());
                found.push((path, text));
            }
        }
        found.sort();
        found
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_preview_does_nothing_but_look() {
        let home = sandbox();
        let active = native_claude(home.home());
        fs::create_dir_all(home.home().join(".claude")).unwrap();
        bind(&["claude"]);
        settings::upsert_api_key_meta(key_of(BindableTool::Claude, "key-1")).unwrap();
        let owner = native_owner(&active);
        let fake = Fake::new([Ok(Inspection::Cli {
            install: active,
            owner,
            hermes_help: None,
            hermes_dry_run: None,
        })]);
        let before = snapshot(home.home());
        let registry = settings::get_settings().bound_tools;

        let preview = preview(&fake, "claude").await.unwrap();

        assert_eq!(fake.calls(), ["inspect", "unbind:claude:true:"]);
        assert_eq!(snapshot(home.home()), before);
        assert_eq!(settings::get_settings().bound_tools, registry);
        assert!(settings::get_api_key_meta(BindableTool::Claude).is_some());
        assert!(settings::list_keys_to_delete().is_empty());
        assert!(preview.installed && preview.bound);
        assert_eq!(preview.source, UninstallSource::Native);
        assert_eq!(preview.command, None);
        assert_eq!(preview.key, UninstallKey::Revoke);
        assert_eq!(
            preview.removes,
            [
                "~/.local/bin/claude",
                "~/.local/share/claude/versions",
                "~/.local/share/claude"
            ]
        );
        assert_eq!(preview.keeps, ["~/.claude"]);
        assert_eq!(preview.blocked_reason, None);
    }

    #[tokio::test]
    async fn a_preview_says_which_tool_shares_the_config_and_keeps_the_key() {
        let _home = sandbox();
        bind(&["codex", "chatgpt"]);
        settings::upsert_api_key_meta(key_of(BindableTool::Codex, "key-1")).unwrap();
        let mut fake = Fake::new([Ok(codex_by_npm())]);
        let mut report = UnbindReport::new("codex", true);
        report.shared_kept_by = vec!["chatgpt".into()];
        fake.unbind = Ok(report);

        let preview = preview(&fake, "codex").await.unwrap();

        assert_eq!(preview.shared_with, ["chatgpt"]);
        assert_eq!(preview.key, UninstallKey::KeepShared);
        assert_eq!(preview.source, UninstallSource::Npm);
        assert_eq!(
            preview.command.as_deref(),
            Some("npm uninstall --global --prefix /n @openai/codex")
        );
        assert_eq!(fake.calls(), ["inspect", "unbind:codex:true:chatgpt"]);
    }

    #[tokio::test]
    async fn a_preview_without_a_key_or_binding_has_nothing_to_revoke() {
        let _home = sandbox();
        let fake = Fake::new([Ok(codex_by_npm())]);
        let preview = preview(&fake, "codex").await.unwrap();
        assert!(!preview.bound);
        assert_eq!(preview.key, UninstallKey::None);
        assert!(preview.shared_with.is_empty());
    }

    #[tokio::test]
    async fn a_preview_names_why_an_uninstall_is_blocked() {
        let _home = sandbox();
        let bundle = Path::new("/Applications/ChatGPT.app");
        let unknown = Inspection::Cli {
            install: install(Path::new("/custom/gemini"), Path::new("/custom/gemini")),
            owner: unknown(),
            hermes_help: None,
            hermes_dry_run: None,
        };
        let cases = [
            (unknown, "gemini", Some(UninstallBlock::UnknownSource)),
            (
                Inspection::App {
                    app: app(bundle),
                    running: true,
                },
                "chatgpt",
                Some(UninstallBlock::AppRunning),
            ),
            (Inspection::NotInstalled, "gemini", None),
        ];
        for (inspection, tool, reason) in cases {
            let fake = Fake::new([Ok(inspection)]);
            let preview = preview(&fake, tool).await.unwrap();
            assert_eq!(preview.blocked_reason, reason, "{tool}");
            assert_eq!(preview.installed, reason.is_some());
        }
        let fake = Fake::new([Ok(codex_by_npm())]);
        let _running = ToolOperationGuard::acquire("codex").unwrap();
        let preview = preview(&fake, "codex").await.unwrap();
        assert_eq!(preview.blocked_reason, Some(UninstallBlock::Busy));
    }

    #[tokio::test]
    async fn a_blocked_preview_offers_the_manual_command() {
        let _home = sandbox();
        let fake = Fake::new([Ok(Inspection::Cli {
            install: install(Path::new("/custom/gemini"), Path::new("/custom/gemini")),
            owner: unknown(),
            hermes_help: None,
            hermes_dry_run: None,
        })]);
        let preview = preview(&fake, "gemini").await.unwrap();
        assert_eq!(preview.source, UninstallSource::Unknown);
        assert_eq!(preview.command, None);
        assert_eq!(
            preview.manual_command.as_deref(),
            Some("npm uninstall --global @google/gemini-cli")
        );
        assert!(preview.removes.is_empty());
    }

    // ---- bound registry ---------------------------------------------------

    #[test]
    fn forgetting_a_tool_leaves_the_others_bound() {
        let _home = sandbox();
        bind(&["claude", "codex", "chatgpt"]);
        forget_bound_tool("codex").unwrap();
        assert_eq!(
            settings::get_settings().bound_tools,
            Some(vec!["claude".to_string(), "chatgpt".to_string()])
        );
        // A registry that was never written stays unwritten.
        settings::mutate_settings(|s| s.bound_tools = None).unwrap();
        forget_bound_tool("codex").unwrap();
        assert_eq!(settings::get_settings().bound_tools, None);
    }

    // ---- key --------------------------------------------------------------

    fn secrets(tool: BindableTool) -> Arc<dyn SecretStore> {
        let store: Arc<dyn SecretStore> = Arc::new(InMemoryStore::new());
        store.save(Slot::ApiKey { tool }, "sk-of-secret").unwrap();
        store
    }

    #[tokio::test]
    async fn a_key_deleted_on_the_server_is_gone_locally_too() {
        let _home = sandbox();
        let tool = BindableTool::Claude;
        let store = secrets(tool);
        settings::upsert_api_key_meta(key_of(tool, "key-1")).unwrap();
        let asked = Mutex::new(Vec::new());

        let outcome = retire_key_with(tool, store.clone(), |key_id| {
            asked.lock().unwrap().push(key_id);
            async { Ok::<(), ApiKeyError>(()) }
        })
        .await;

        assert_eq!(outcome, UninstallKeyOutcome::Revoked);
        assert_eq!(*asked.lock().unwrap(), ["key-1"]);
        assert_eq!(store.load(Slot::ApiKey { tool }).unwrap(), None);
        assert!(settings::get_api_key_meta(tool).is_none());
        assert!(settings::list_keys_to_delete().is_empty());
    }

    #[tokio::test]
    async fn a_key_the_server_could_not_delete_is_queued_and_dropped_locally() {
        let _home = sandbox();
        let tool = BindableTool::Claude;
        let store = secrets(tool);
        settings::upsert_api_key_meta(key_of(tool, "key-1")).unwrap();

        let outcome = retire_key_with(tool, store.clone(), |_| async {
            Err::<(), _>(ApiKeyError::RemoteRejected("network: offline".into()))
        })
        .await;

        assert_eq!(outcome, UninstallKeyOutcome::Queued);
        assert_eq!(
            settings::list_keys_to_delete(),
            [KeyToDelete {
                key_id: "key-1".into(),
                owner: None
            }]
        );
        assert_eq!(store.load(Slot::ApiKey { tool }).unwrap(), None);
        assert!(settings::get_api_key_meta(tool).is_none());
    }

    #[tokio::test]
    async fn there_is_nothing_to_revoke_without_a_recorded_key_or_for_a_stopped_one() {
        let _home = sandbox();
        let tool = BindableTool::Claude;
        let store = secrets(tool);
        let untouched = |_: String| async { panic!("no request expected") };
        let outcome = retire_key_with(tool, store.clone(), untouched).await;
        assert_eq!(outcome, UninstallKeyOutcome::None);
        assert_eq!(
            store.load(Slot::ApiKey { tool }).unwrap().as_deref(),
            Some("sk-of-secret")
        );

        // Stopped at sign-out: already deleted or already queued then.
        let stopped = ApiKeyMeta {
            revoked: true,
            ..key_of(tool, "key-2")
        };
        settings::upsert_api_key_meta(stopped).unwrap();
        let outcome = retire_key_with(tool, store.clone(), untouched).await;
        assert_eq!(outcome, UninstallKeyOutcome::None);
        assert!(settings::get_api_key_meta(tool).is_none());
        assert_eq!(store.load(Slot::ApiKey { tool }).unwrap(), None);
        assert!(settings::list_keys_to_delete().is_empty());
    }

    // ---- running the removal ----------------------------------------------

    fn lines_of(lines: &Mutex<Vec<String>>) -> Vec<String> {
        lines.lock().unwrap().clone()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn removing_paths_removes_a_link_not_what_it_points_to() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("keep/data");
        touch(&outside, "mine");
        let link = temp.path().join("bin/claude");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let tree = temp.path().join("share/claude");
        touch(&tree.join("versions/1/claude"), "binary");
        let missing = temp.path().join("never/there");
        let lines = Mutex::new(Vec::new());

        perform(
            &Action::Remove {
                paths: vec![link.clone(), tree.clone(), missing],
                empty_dirs: vec![],
            },
            &|line| lines.lock().unwrap().push(line.to_string()),
            UNINSTALL_TIMEOUT,
        )
        .await
        .unwrap();

        assert!(fs::symlink_metadata(&link).is_err());
        assert!(!tree.exists());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "mine");
        assert_eq!(lines_of(&lines).len(), 2);
    }

    #[tokio::test]
    async fn an_emptied_directory_is_removed_but_one_with_other_files_stays() {
        let temp = tempfile::tempdir().unwrap();
        let emptied = temp.path().join("claude");
        touch(&emptied.join("versions/1/claude"), "binary");
        let shared = temp.path().join("bin");
        touch(&shared.join("opencode"), "binary");
        touch(&shared.join("notes"), "mine");
        let lines = Mutex::new(Vec::new());

        perform(
            &Action::Remove {
                paths: vec![emptied.join("versions"), shared.join("opencode")],
                empty_dirs: vec![emptied.clone(), shared.clone(), temp.path().join("gone")],
            },
            &|line| lines.lock().unwrap().push(line.to_string()),
            UNINSTALL_TIMEOUT,
        )
        .await
        .unwrap();

        assert!(!emptied.exists());
        assert_eq!(fs::read_to_string(shared.join("notes")).unwrap(), "mine");
        assert!(!shared.join("opencode").exists());
        assert!(lines_of(&lines)
            .iter()
            .any(|line| line.starts_with("Kept") && line.contains("bin")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_streams_its_output_and_a_failure_is_an_error() {
        let run = |script: &str| Action::Run {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            path: "/usr/bin:/bin".into(),
            env: vec![],
        };
        let lines = Mutex::new(Vec::new());
        let log = |line: &str| lines.lock().unwrap().push(line.to_string());

        perform(&run("echo out; echo err >&2"), &log, UNINSTALL_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(lines_of(&lines).len(), 2);

        let error = perform(&run("exit 3"), &log, UNINSTALL_TIMEOUT)
            .await
            .unwrap_err();
        assert!(error.contains("exited"), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_runs_with_the_environment_its_plan_carries() {
        let action = Action::Run {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "echo \"$OFOX_PROBE_VALUE\"".into()],
            path: "/usr/bin:/bin".into(),
            env: vec![("OFOX_PROBE_VALUE".into(), "carried".into())],
        };
        let lines = Mutex::new(Vec::new());
        perform(
            &action,
            &|line| lines.lock().unwrap().push(line.to_string()),
            UNINSTALL_TIMEOUT,
        )
        .await
        .unwrap();
        assert_eq!(lines_of(&lines), ["carried"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_that_hangs_is_stopped_at_the_timeout() {
        let action = Action::Run {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "exec sleep 5".into()],
            path: "/usr/bin:/bin".into(),
            env: vec![],
        };
        let error = perform(&action, &|_| {}, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(error.contains("Uninstall timed out"), "{error}");
        assert_eq!(UNINSTALL_TIMEOUT, Duration::from_secs(300));
    }

    /// Moves a throwaway bundle into the REAL Trash, so `--ignored` alone must not
    /// run it: the variable has to be set on purpose, like `OFOX_TEST_UPDATE_TOOL`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Manual acceptance: needs OFOX_TEST_TRASH_PROBE=1; moves a throwaway app bundle to the real Trash"]
    fn manual_trash_moves_a_bundle_into_the_users_trash() {
        assert_eq!(
            std::env::var("OFOX_TEST_TRASH_PROBE").as_deref(),
            Ok("1"),
            "Set OFOX_TEST_TRASH_PROBE=1 explicitly: this puts a throwaway app bundle in your real Trash"
        );
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("Ofox Uninstall Probe.app");
        touch(&bundle.join("Contents/Info.plist"), "probe");
        trash_bundle(&bundle).unwrap();
        assert!(!bundle.exists());
        assert!(
            trash_bundle(&bundle).is_err(),
            "a bundle that is gone is an error"
        );
    }

    /// Read-only: shows what an uninstall would do to each tool installed here.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "Manual probe of the installed tools; changes nothing"]
    async fn manual_plan_installed_tools() {
        let home = crate::config::get_home_dir();
        for tool in UNINSTALLABLE {
            let decision = match inspect_machine(tool).await {
                Ok(inspection) => decide(tool, &inspection, &home, kept_data(tool), Os::current()),
                Err(error) => {
                    println!("{tool}: {error}");
                    continue;
                }
            };
            match decision {
                Decision::NotInstalled => println!("{tool}: not installed"),
                Decision::Plan(plan) => println!("{tool}: {plan:#?}"),
                Decision::Manual(manual) => println!("{tool}: manual {manual:?}"),
                Decision::AppRunning => println!("{tool}: app running"),
            }
        }
    }
}
