//! Uninstall a tool: unbind it from Ofox (restore its configuration), retire its
//! Ofox key, then remove the program the way it was installed. The agent's own
//! data (`~/.codex`, sessions, ...) is never touched.
//!
//! This file has two halves. The planner (`uninstall_plan`, `app_plan`) is pure:
//! it only reads the file system, so it is tested against temp directory trees.
//! The orchestration below it takes every side effect through [`Effects`], so
//! tests can check the order of the steps without touching a real tool.

use serde::Serialize;
use std::path::{Path, PathBuf};

use super::tool_update::{installed_npm_package, DesktopInstallation, Installation, UpdatePlan};

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
    },
    /// Delete these files and directories (a native installer's own layout).
    RemovePaths(Vec<PathBuf>),
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

/// What the user could run themselves when the source is not known. A guess at
/// the usual package manager is only offered when nothing is known about it.
fn suggested_command(tool: &str, install: &Installation) -> Option<String> {
    if tool == "hermes" {
        return Some("hermes uninstall".into());
    }
    installed_npm_package(tool, &install.path, &install.real)
        .map(|package| format!("npm uninstall --global {package}"))
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
            if verb != "upgrade" {
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
    Ok(UninstallPlan {
        source,
        command: Some(render_command(&update.program, &args)),
        action: Action::Run {
            program: update.program.clone(),
            args,
            path: update.path.clone(),
        },
        removes: vec![removes],
        keeps: keeps_shown(facts),
    })
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
    let paths = match tool {
        "claude" => {
            let share = home.join(".local/share/claude");
            let versions = share.join("versions");
            if !within(&install.real, &versions) {
                return Err(unrecognised());
            }
            // The launcher is ours only while it points into the versions.
            let launcher = home.join(".local/bin/claude");
            let mut paths = Vec::new();
            if links_into(&launcher, &versions) {
                paths.push(launcher);
            }
            paths.push(share);
            paths
        }
        "opencode" => {
            let bin = home.join(".opencode/bin");
            if !within(&install.real, &bin) {
                return Err(unrecognised());
            }
            vec![bin]
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
                },
                removes: vec![shown(&install.path, home)],
                keeps: keeps_shown(facts),
            });
        }
        _ => return Err(unrecognised()),
    };
    if paths.iter().any(|path| reaches_data(path, &facts.keeps)) {
        return Err(unrecognised());
    }
    Ok(UninstallPlan {
        source: UninstallSource::Native,
        command: None,
        removes: paths.iter().map(|path| shown(path, home)).collect(),
        action: Action::RemovePaths(paths),
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

/// The arguments that remove Hermes but keep its data, if this Hermes can do it
/// without asking questions. Hermes keeps configs and data unless told `--full`,
/// so the option is only trusted when the help also documents `--full`; `--yes`
/// is what spares it the prompts it cannot get here.
pub(crate) fn hermes_uninstall_args(help: &str) -> Option<Vec<String>> {
    let offers = |flag: &str| {
        help.split(|c: char| c.is_whitespace() || "[]|,".contains(c))
            .any(|word| word == flag)
    };
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

/// Whether any process runs from inside the app bundle (`ps -axo comm=` output).
pub(crate) fn bundle_in_use(ps_output: &str, bundle: &Path) -> bool {
    let inside = format!(
        "{}/Contents/",
        bundle.to_string_lossy().trim_end_matches('/')
    );
    ps_output
        .lines()
        .any(|line| line.trim_start().starts_with(&inside))
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
        assert_eq!(
            plan.action,
            Action::RemovePaths(vec![
                home.join(".local/bin/claude"),
                home.join(".local/share/claude"),
            ])
        );
        assert_eq!(plan.command, None);
        assert_eq!(plan.keeps, ["~/.claude", "~/.claude.json"]);
        assert_eq!(
            plan.removes,
            ["~/.local/bin/claude", "~/.local/share/claude"]
        );
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
            Action::RemovePaths(vec![home.join(".local/share/claude")])
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
        let Action::RemovePaths(paths) = plan.action else {
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
    fn native_opencode_removes_its_bin_directory() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let real = home.join(".opencode/bin/opencode");
        touch(&real, "binary");
        let active = install(&real, &real);
        let plan =
            uninstall_plan("opencode", &active, &facts(home, native_owner(&active))).unwrap();
        assert_eq!(
            plan.action,
            Action::RemovePaths(vec![home.join(".opencode/bin")])
        );
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
}
