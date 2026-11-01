//! npm 工具的启动脚本（fizzy #1351）。
//!
//! fnm / nvm / mise 按 Node 版本分开放全局包。Ofox 用新开终端在主目录里解析到的
//! Node 装工具，工具就只在那一个版本里；进入锁定了别的 Node 版本的目录
//! （`.node-version`、`.nvmrc`）后，PATH 换成那个版本的 bin，命令就「消失」了。
//!
//! 做法：工具装在这类版本管理器里时，在 `~/.ofox-desktop/bin/<tool>` 写一个启动脚本，
//! 固定用安装时的 Node 运行；这个目录追加在 PATH 的**最末尾**，只在别处都找不到
//! 命令时才用到。主目录里解析到的仍然是真正的 npm 安装，检测、升级、卸载照旧。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::tool_update::{Installation, ProbeError, ToolOperationGuard};

/// 启动脚本目录，相对用户主目录。
const DIR: &str = ".ofox-desktop/bin";
/// 启动脚本里记录它转发到哪个可执行文件。
const TARGET_MARKER: &str = "# ofox-launcher-target=";
/// 写过 PATH 的 rc 文件；用户之后把那几行删掉，就不再加回去。
const PATH_RECORD: &str = ".path-entries";
/// 与 `tool_update::npm_package` 对齐：Ofox 用 npm 装的工具。
const NPM_TOOLS: &[&str] = &[
    "claude", "codex", "gemini", "opencode", "openclaw", "dsh", "pi",
];

static LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn launcher_dir(home: &Path) -> PathBuf {
    home.join(DIR)
}

/// 装在按 Node 版本分开放全局包的版本管理器里时，返回这次安装的 npm 前缀。
fn pinned_prefix(tool: &str, install: &Installation) -> Option<PathBuf> {
    let manager = super::node_requirement::node_manager(&install.path, &install.real);
    if !matches!(manager, "fnm" | "nvm" | "mise" | "asdf") {
        return None;
    }
    let plan = super::tool_update::plan(tool, install).ok()?;
    if plan.source != "npm" {
        return None;
    }
    plan.program.parent()?.parent().map(Path::to_path_buf)
}

fn sh_quote(path: &Path) -> Result<String, String> {
    let text = path.to_string_lossy();
    if text.contains('\n') {
        return Err(format!(
            "Unsupported path for a launcher: {}",
            path.display()
        ));
    }
    Ok(format!("'{}'", text.replace('\'', r"'\''")))
}

/// 入口是 Node 脚本时用安装时的 node 跑；原生二进制（claude、opencode 的 npm 包）
/// 直接运行。运行时才判断：工具自己升级后可能从 JS 换成原生二进制。
/// 不改 PATH，工具里执行的命令仍用项目目录选中的 Node。
fn script(tool: &str, target: &Path, node: &Path) -> Result<String, String> {
    let quoted_target = sh_quote(target)?;
    let quoted_node = sh_quote(node)?;
    let bin_dir = sh_quote(node.parent().unwrap_or(Path::new("/")))?;
    Ok(format!(
        r#"#!/bin/sh
# Ofox Desktop: runs {tool} with the Node.js it was installed with, so the command
# also works in folders that switch to another Node.js version (fnm, nvm, mise).
{TARGET_MARKER}{target}
target={quoted_target}
node={quoted_node}
if [ ! -e "$target" ]; then
  echo "{tool}: $target no longer exists. Reinstall {tool} from Ofox Desktop." >&2
  exit 127
fi
if [ "$(/usr/bin/head -c 2 "$target")" = '#!' ]; then
  IFS= read -r shebang < "$target"
  case $shebang in
    '#!/usr/bin/env node' | '#!'*/node) exec "$node" "$target" "$@" ;;
    *node*) PATH={bin_dir}:$PATH exec "$target" "$@" ;;
  esac
fi
exec "$target" "$@"
"#,
        target = target.display(),
    ))
}

fn read_head(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(4096)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

fn marker_target(path: &Path) -> Option<PathBuf> {
    read_head(path)?
        .lines()
        .find_map(|line| line.strip_prefix(TARGET_MARKER))
        .map(PathBuf::from)
}

/// `path` 是 Ofox 的启动脚本时，返回它转发到的可执行文件。
pub(crate) fn launcher_target(path: &Path, home: &Path) -> Option<PathBuf> {
    if path.parent() != Some(launcher_dir(home).as_path()) {
        return None;
    }
    marker_target(path)
}

/// 新开终端解析到的是启动脚本时（默认 Node 里已经没有这个工具），按它转发到的
/// 安装来判断安装来源，升级、卸载才找得到原来的 npm。
pub(crate) fn resolve_real(path: &Path, real: PathBuf, home: &Path) -> PathBuf {
    launcher_target(path, home)
        .and_then(|target| std::fs::canonicalize(target).ok())
        .unwrap_or(real)
}

/// 需要时写（或更新）`tool` 的启动脚本，并让新开的终端能找到它。
/// 不需要时只清掉指向已不存在的安装的旧脚本。
pub(crate) fn ensure(
    tool: &str,
    install: &Installation,
    home: &Path,
    shell: &str,
) -> Result<(), String> {
    let _lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let launcher = launcher_dir(home).join(tool);
    let Some(prefix) = pinned_prefix(tool, install) else {
        return prune_locked(&launcher);
    };
    let target = prefix.join("bin").join(tool);
    let node = prefix.join("bin/node");
    if !target.exists() || !node.is_file() {
        return Err(format!(
            "No {tool} or node in {}; launcher not written",
            prefix.display()
        ));
    }
    let exists = std::fs::symlink_metadata(&launcher).is_ok();
    if exists && marker_target(&launcher).is_none() {
        // 不是 Ofox 写的文件，不动。
        return Ok(());
    }
    let text = script(tool, &target, &node)?;
    if read_head(&launcher).as_deref() != Some(text.as_str()) {
        crate::config::atomic_write_with(
            &launcher,
            text.as_bytes(),
            crate::config::AtomicOpts { mode: Some(0o755) },
        )
        .map_err(|error| error.to_string())?;
        log::info!(
            "[node-launcher] {tool} now runs with {} outside its Node.js version",
            node.display()
        );
    }
    add_to_path(home, shell)
}

/// 删掉 `tool` 指向已不存在的安装的启动脚本（卸载之后、Node 版本被删之后）。
pub(crate) fn prune(tool: &str, home: &Path) -> Result<(), String> {
    let _lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    prune_locked(&launcher_dir(home).join(tool))
}

fn prune_locked(launcher: &Path) -> Result<(), String> {
    match marker_target(launcher) {
        Some(target) if !target.exists() => std::fs::remove_file(launcher)
            .map_err(|error| format!("Could not remove {}: {error}", launcher.display())),
        _ => Ok(()),
    }
}

/// 交互式终端会读的 rc 文件。
fn rc_files(home: &Path, shell: &str) -> Vec<PathBuf> {
    match Path::new(shell).file_name().and_then(|name| name.to_str()) {
        Some("zsh") => vec![home.join(".zshrc")],
        Some("bash") => {
            // 登录 shell 只读第一个存在的；不要新建 .bash_profile 把用户的 .profile 挡住。
            let login = [".bash_profile", ".bash_login", ".profile"]
                .iter()
                .map(|name| home.join(name))
                .find(|path| path.exists())
                .unwrap_or_else(|| home.join(".bash_profile"));
            let mut files = vec![login];
            let rc = home.join(".bashrc");
            if rc.exists() {
                files.push(rc);
            }
            files
        }
        Some("fish") => vec![home.join(".config/fish/conf.d/ofox-desktop.fish")],
        _ => Vec::new(),
    }
}

fn path_block(rc: &Path) -> &'static str {
    if rc.extension().is_some_and(|ext| ext == "fish") {
        "\n# Added by Ofox Desktop: commands it installed keep working in folders that pin another Node.js version.\nif not contains -- $HOME/.ofox-desktop/bin $PATH\n    set -gx PATH $PATH $HOME/.ofox-desktop/bin\nend\n"
    } else {
        "\n# Added by Ofox Desktop: commands it installed keep working in folders that pin another Node.js version.\ncase \":$PATH:\" in *\":$HOME/.ofox-desktop/bin:\"*) ;; *) export PATH=\"$PATH:$HOME/.ofox-desktop/bin\" ;; esac\n"
    }
}

/// 把启动脚本目录追加到 PATH 末尾。每个 rc 文件只加一次：用户删掉后不再加回去。
fn add_to_path(home: &Path, shell: &str) -> Result<(), String> {
    let record = launcher_dir(home).join(PATH_RECORD);
    let mut recorded = std::fs::read_to_string(&record).unwrap_or_default();
    let mut changed = false;
    for rc in rc_files(home, shell) {
        let key = rc.to_string_lossy().into_owned();
        let text = match std::fs::read_to_string(&rc) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(format!("Could not read {}: {error}", rc.display())),
        };
        let seen = recorded.lines().any(|line| line == key);
        if !text.contains(DIR) {
            if seen {
                continue;
            }
            let mut next = text;
            if !next.is_empty() && !next.ends_with('\n') {
                next.push('\n');
            }
            let block = path_block(&rc);
            next.push_str(if next.is_empty() {
                block.trim_start()
            } else {
                block
            });
            if let Some(parent) = rc.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
            }
            crate::config::atomic_write(&rc, next.as_bytes()).map_err(|error| error.to_string())?;
            log::info!("[node-launcher] added ~/{DIR} to the end of PATH in {key}");
        }
        if !seen {
            recorded.push_str(&key);
            recorded.push('\n');
            changed = true;
        }
    }
    if changed {
        crate::config::atomic_write(&record, recorded.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn login_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
}

/// 装好、升级之后：按新开终端看到的安装更新 `tool` 的启动脚本。调用方持有工具锁。
pub(crate) async fn reconcile(tool: &str) {
    let home = crate::config::get_home_dir();
    let result = match super::tool_update::probe(tool).await {
        Ok(install) => ensure(tool, &install, &home, &login_shell()),
        Err(ProbeError::NotFound) => prune(tool, &home),
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = result {
        log::warn!("[node-launcher] {tool}: {error}");
    }
}

/// 有版本管理器的 Node，或者之前写过启动脚本时，才值得逐个去问 shell。
fn worth_checking(home: &Path) -> bool {
    let env_dirs = ["FNM_DIR", "NVM_DIR", "MISE_DATA_DIR", "ASDF_DATA_DIR"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from);
    let home_dirs = [
        ".local/share/fnm",
        "Library/Application Support/fnm",
        ".nvm",
        ".local/share/mise",
        ".asdf",
    ]
    .iter()
    .map(|rel| home.join(rel));
    env_dirs.chain(home_dirs).any(|dir| dir.is_dir())
        || std::fs::read_dir(launcher_dir(home))
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false)
}

/// 启动时补上之前装的工具的启动脚本，清掉失效的。
pub(crate) async fn reconcile_all() {
    if !worth_checking(&crate::config::get_home_dir()) {
        return;
    }
    for tool in NPM_TOOLS {
        // 正在装、升级、卸载的工具由那边收尾。
        let Ok(_guard) = ToolOperationGuard::acquire(tool) else {
            continue;
        };
        reconcile(tool).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::process::Command;

    fn write(path: &Path, text: &str, mode: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// fnm 的一个 Node 版本：`bin/node` 打印自己收到的参数，`bin/<tool>` 指向包里的入口。
    fn fnm_install(
        home: &Path,
        tool: &str,
        package: &str,
        entry: &str,
        body: &str,
    ) -> Installation {
        let prefix = home.join(".local/share/fnm/node-versions/v24.21.0/installation");
        write(
            &prefix.join("bin/node"),
            "#!/bin/sh\necho \"pinned-node $*\"\n",
            0o755,
        );
        let real = prefix.join("lib/node_modules").join(package).join(entry);
        write(&real, body, 0o755);
        let bin = prefix.join("bin").join(tool);
        symlink(&real, &bin).unwrap();
        Installation {
            path: home
                .join(".local/state/fnm_multishells/123_456/bin")
                .join(tool),
            real: std::fs::canonicalize(&real).unwrap(),
            version: "1.0.0".into(),
            error: None,
            search_path: "/usr/bin:/bin".into(),
        }
    }

    /// The npm prefix as the launcher records it: canonical, like `Installation::real`.
    fn prefix(home: &Path) -> PathBuf {
        std::fs::canonicalize(home.join(".local/share/fnm/node-versions/v24.21.0/installation"))
            .unwrap()
    }

    fn codex(home: &Path) -> Installation {
        fnm_install(
            home,
            "codex",
            "@openai/codex",
            "bin/codex.js",
            "#!/usr/bin/env node\nconsole.log('codex')\n",
        )
    }

    fn run(launcher: &Path, args: &[&str]) -> std::process::Output {
        Command::new(launcher)
            .args(args)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    }

    #[test]
    fn node_script_runs_with_the_node_it_was_installed_with() {
        let home = tempfile::tempdir().unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();

        let launcher = launcher_dir(home.path()).join("codex");
        let output = run(&launcher, &["exec", "two words"]);
        let prefix = prefix(home.path());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            format!(
                "pinned-node {} exec two words",
                prefix.join("bin/codex").display()
            )
        );
    }

    #[test]
    fn native_binary_runs_directly() {
        let home = tempfile::tempdir().unwrap();
        // No `#!` line, like the Mach-O claude.exe in the npm package. (A copied
        // system binary cannot stand in: macOS kills relocated platform binaries.)
        let install = fnm_install(
            home.path(),
            "claude",
            "@anthropic-ai/claude-code",
            "bin/claude.exe",
            "echo \"$@\"\n",
        );
        ensure("claude", &install, home.path(), "/bin/zsh").unwrap();

        let output = run(&launcher_dir(home.path()).join("claude"), &["hello"]);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[test]
    fn shell_wrapper_entry_runs_directly() {
        let home = tempfile::tempdir().unwrap();
        let install = fnm_install(
            home.path(),
            "dsh",
            "@deepseek-ai/dsh",
            "lib/bin.sh",
            "#!/bin/sh\necho \"wrapper $*\"\n",
        );
        ensure("dsh", &install, home.path(), "/bin/zsh").unwrap();

        let output = run(&launcher_dir(home.path()).join("dsh"), &["x"]);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "wrapper x");
    }

    #[test]
    fn launcher_survives_quotes_in_the_home_path() {
        let parent = tempfile::tempdir().unwrap();
        let home = parent.path().join("it's me");
        let install = codex(&home);
        ensure("codex", &install, &home, "/bin/zsh").unwrap();

        let output = run(&launcher_dir(&home).join("codex"), &[]);
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("pinned-node "));
    }

    #[test]
    fn missing_target_explains_how_to_recover() {
        let home = tempfile::tempdir().unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();
        std::fs::remove_file(
            home.path()
                .join(".local/share/fnm/node-versions/v24.21.0/installation/bin/codex"),
        )
        .unwrap();

        let output = run(&launcher_dir(home.path()).join("codex"), &[]);
        assert_eq!(output.status.code(), Some(127));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Reinstall codex from Ofox Desktop")
        );
    }

    #[test]
    fn zshrc_gets_one_path_entry_at_the_end_of_path() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join(".zshrc"),
            "eval \"$(fnm env --use-on-cd)\"",
        )
        .unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();

        let zshrc = std::fs::read_to_string(home.path().join(".zshrc")).unwrap();
        assert!(zshrc.starts_with("eval \"$(fnm env --use-on-cd)\"\n\n# Added by Ofox Desktop"));
        assert_eq!(
            zshrc
                .matches("export PATH=\"$PATH:$HOME/.ofox-desktop/bin\"")
                .count(),
            1
        );
    }

    #[test]
    fn path_entry_appends_once_per_shell_session() {
        let home = tempfile::tempdir().unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();
        let block = std::fs::read_to_string(home.path().join(".zshrc")).unwrap();

        let output = Command::new("/bin/sh")
            .args(["-c", &format!("{block}\n{block}\nprintf %s \"$PATH\"")])
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("/usr/bin:/bin:{}/.ofox-desktop/bin", home.path().display())
        );
    }

    #[test]
    fn removed_path_entry_is_not_added_back() {
        let home = tempfile::tempdir().unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();
        std::fs::write(home.path().join(".zshrc"), "# mine\n").unwrap();

        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();
        assert_eq!(
            std::fs::read_to_string(home.path().join(".zshrc")).unwrap(),
            "# mine\n"
        );
    }

    #[test]
    fn bash_keeps_using_an_existing_profile() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".profile"), "# login\n").unwrap();
        std::fs::write(home.path().join(".bashrc"), "# rc\n").unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/bash").unwrap();

        assert!(!home.path().join(".bash_profile").exists());
        for rc in [".profile", ".bashrc"] {
            let text = std::fs::read_to_string(home.path().join(rc)).unwrap();
            assert!(text.contains(".ofox-desktop/bin"), "{rc}: {text}");
        }
    }

    #[test]
    fn a_new_rc_file_starts_with_the_entry() {
        let home = tempfile::tempdir().unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();

        let zshrc = std::fs::read_to_string(home.path().join(".zshrc")).unwrap();
        assert!(zshrc.starts_with("# Added by Ofox Desktop"), "{zshrc}");
    }

    #[test]
    fn fish_gets_its_own_conf_d_file() {
        let home = tempfile::tempdir().unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/opt/homebrew/bin/fish").unwrap();

        let text =
            std::fs::read_to_string(home.path().join(".config/fish/conf.d/ofox-desktop.fish"))
                .unwrap();
        assert!(text.contains("set -gx PATH $PATH $HOME/.ofox-desktop/bin"));
    }

    #[test]
    fn single_node_installs_need_no_launcher() {
        let home = tempfile::tempdir().unwrap();
        let real = home
            .path()
            .join("opt/homebrew/lib/node_modules/@openai/codex/bin/codex.js");
        write(&real, "#!/usr/bin/env node\n", 0o755);
        let install = Installation {
            path: home.path().join("opt/homebrew/bin/codex"),
            real,
            version: "1.0.0".into(),
            error: None,
            search_path: "/usr/bin:/bin".into(),
        };
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();

        assert!(!launcher_dir(home.path()).join("codex").exists());
        assert!(!home.path().join(".zshrc").exists());
    }

    #[test]
    fn someone_elses_file_is_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let mine = launcher_dir(home.path()).join("codex");
        write(&mine, "#!/bin/sh\necho mine\n", 0o755);
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();

        assert_eq!(
            std::fs::read_to_string(&mine).unwrap(),
            "#!/bin/sh\necho mine\n"
        );
    }

    #[test]
    fn prune_removes_only_launchers_whose_install_is_gone() {
        let home = tempfile::tempdir().unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();
        let launcher = launcher_dir(home.path()).join("codex");

        prune("codex", home.path()).unwrap();
        assert!(launcher.exists());

        std::fs::remove_file(
            home.path()
                .join(".local/share/fnm/node-versions/v24.21.0/installation/bin/codex"),
        )
        .unwrap();
        prune("codex", home.path()).unwrap();
        assert!(!launcher.exists());
    }

    #[test]
    fn a_launcher_found_on_path_is_updated_and_removed_through_its_npm_install() {
        let home = tempfile::tempdir().unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();
        let launcher = launcher_dir(home.path()).join("codex");

        let seen = Installation {
            real: resolve_real(&launcher, launcher.clone(), home.path()),
            path: launcher,
            ..install.clone()
        };
        assert_eq!(seen.real, install.real);
        let plan = super::super::tool_update::plan("codex", &seen).unwrap();
        assert_eq!(plan.source, "npm");
        assert_eq!(plan.program, prefix(home.path()).join("bin/npm"));
    }

    /// `CC_SWITCH_TEST_HOME=<scratch> cargo test live_reconcile -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "Manual acceptance: probes this machine's npm CLIs; writes only into CC_SWITCH_TEST_HOME"]
    async fn live_reconcile_writes_launchers_for_this_machine() {
        let home = crate::config::get_home_dir();
        reconcile_all().await;
        for entry in std::fs::read_dir(launcher_dir(&home)).unwrap().flatten() {
            println!("{}", std::fs::read_to_string(entry.path()).unwrap());
        }
    }

    #[test]
    fn launcher_target_reads_only_ofox_launchers() {
        let home = tempfile::tempdir().unwrap();
        let install = codex(home.path());
        ensure("codex", &install, home.path(), "/bin/zsh").unwrap();
        let target = prefix(home.path()).join("bin/codex");

        assert_eq!(
            launcher_target(&launcher_dir(home.path()).join("codex"), home.path()),
            Some(target)
        );
        let elsewhere = home.path().join("bin/codex");
        std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        std::fs::copy(launcher_dir(home.path()).join("codex"), &elsewhere).unwrap();
        assert_eq!(launcher_target(&elsewhere, home.path()), None);
    }
}
