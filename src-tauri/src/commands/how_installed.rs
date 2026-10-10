//! Windows 上一个 CLI 是怎么装的、该怎么升级：从它的可执行文件在哪判断。
//!
//! Ported from magpie (https://github.com/yetone/magpie) internal/agent/cliupdate.go
//! (howInstalled, fromNodeModules, shimNames) — MIT License, Copyright (c) 2026 yetone.
//!
//! 厂商安装器装的用 CLI 自带的更新命令；包管理器装的交回给那个包管理器（npm、
//! pnpm、bun）。Volta、Yarn 管理的不接管，判断不了的也不猜。

use std::path::{Path, PathBuf};

/// 升级命令：`program args`，`path_first` 放在 PATH 最前（npm 用自己旁边的 node）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Updater {
    pub source: &'static str,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub path_first: Option<PathBuf>,
}

/// npm 的 `.cmd` shim 很小；读得进来、里面写着 `node_modules/<pkg>/` 才算。
const SHIM_LIMIT: u64 = 64 * 1024;

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `bin` 是 `tool` 的可执行文件，`real` 是跟随链接后的真实路径；`lookup` 按名字找程序。
pub(crate) fn how_installed(
    tool: &str,
    bin: &Path,
    real: &Path,
    lookup: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<Updater> {
    let real_path = slashed(real).to_lowercase();
    let own_updater = match tool {
        // ~/.local/bin/claude → ~/.local/share/claude/versions/<v>; on Windows the binary itself.
        "claude"
            if real_path.contains("/.local/share/claude/versions/")
                || real_path.ends_with("/.local/bin/claude.exe") =>
        {
            Some("update")
        }
        // install.ps1 reaches it through junctions into ~/.codex/packages/standalone.
        "codex" if real_path.contains("/.codex/packages/standalone/") => Some("update"),
        "opencode" if real_path.contains("/.opencode/bin/") => Some("upgrade"),
        "hermes" => Some("update"),
        _ => None,
    };
    if let Some(command) = own_updater {
        return Some(Updater {
            source: "native",
            program: bin.to_path_buf(),
            args: vec![command.into()],
            path_first: None,
        });
    }
    let pkg = super::tool_update::npm_package(tool)?;
    from_node_modules(pkg, bin, real, lookup)
}

/// 装在全局 `node_modules` 里的 `pkg`：可执行文件链到那里（npm、bun 的链接），
/// 或者是写着它的 shim（npm 在 Windows 上的 `.cmd`、pnpm 的脚本）。
fn from_node_modules(
    pkg: &str,
    bin: &Path,
    real: &Path,
    lookup: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<Updater> {
    let real_text = real.to_string_lossy();
    let real_slashed = slashed(real);
    let modules = if let Some(index) = real_slashed.find(&format!("/node_modules/{pkg}/")) {
        // Same byte offsets: only the separators differ.
        Some(PathBuf::from(&real_text[..index]).join("node_modules"))
    } else if shim_names(bin, pkg) {
        let modules = bin.parent()?.join("node_modules");
        // pnpm's shims name a store elsewhere.
        modules.join(pkg).is_dir().then_some(modules)
    } else {
        return None;
    };
    let both = format!("{real_slashed}|{}", slashed(bin)).to_lowercase();
    let latest = format!("{pkg}@latest");
    if both.contains("/.bun/") {
        let beside = bin.parent()?.join("bun.exe");
        let bun = if beside.is_file() {
            beside
        } else {
            lookup("bun")?
        };
        return Some(Updater {
            source: "bun",
            program: bun,
            args: vec!["add".into(), "-g".into(), latest],
            path_first: None,
        });
    }
    if both.contains("/pnpm/") || both.contains("/.pnpm/") {
        return Some(Updater {
            source: "pnpm",
            program: lookup("pnpm")?,
            args: vec!["add".into(), "-g".into(), latest],
            path_first: None,
        });
    }
    // Installed by a manager of its own, which is left to it.
    if ["/.volta/", "/volta/", "/yarn/", "/.yarn/"]
        .iter()
        .any(|manager| both.contains(manager))
    {
        return None;
    }
    // On Windows npm's global prefix is the folder holding node_modules.
    let prefix = modules?.parent()?.to_path_buf();
    let own_npm = prefix.join("npm.cmd");
    let (program, path_first) = if own_npm.is_file() {
        (own_npm, Some(prefix.clone()))
    } else {
        (lookup("npm")?, None)
    };
    Some(Updater {
        source: "npm",
        program,
        args: vec![
            "install".into(),
            "-g".into(),
            "--prefix".into(),
            prefix.to_string_lossy().into_owned(),
            latest,
        ],
        path_first,
    })
}

/// `bin` 是一个写着 `pkg` 所在目录的小脚本（npm 在 Windows 上的 `.cmd`、pnpm 的脚本）。
pub(crate) fn shim_names(bin: &Path, pkg: &str) -> bool {
    let small_file =
        std::fs::metadata(bin).is_ok_and(|meta| meta.is_file() && meta.len() <= SHIM_LIMIT);
    small_file
        && std::fs::read(bin).is_ok_and(|bytes| {
            String::from_utf8_lossy(&bytes)
                .replace('\\', "/")
                .contains(&format!("node_modules/{pkg}/"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn none(_: &str) -> Option<PathBuf> {
        None
    }

    /// `%APPDATA%\npm` as npm lays it out: shims beside `node_modules`.
    fn npm_prefix(root: &Path, pkg: &str, bin_name: &str) -> (PathBuf, PathBuf) {
        let prefix = root.join("Roaming").join("npm");
        let script = format!("node_modules\\{}\\bin\\cli.js", pkg.replace('/', "\\"));
        touch(&prefix.join(script.replace('\\', "/")), "");
        let bin = prefix.join(format!("{bin_name}.cmd"));
        touch(
            &bin,
            &format!("@ECHO off\r\n\"%_prog%\" \"%dp0%\\{script}\" %*\r\n"),
        );
        (prefix, bin)
    }

    #[test]
    fn an_npm_shim_updates_with_npm_into_the_same_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let (prefix, bin) = npm_prefix(temp.path(), "@google/gemini-cli", "gemini");
        let system_npm = temp.path().join("nodejs").join("npm.cmd");
        let lookup = |name: &str| (name == "npm").then(|| system_npm.clone());
        let updater = how_installed("gemini", &bin, &bin, &lookup).unwrap();
        assert_eq!(updater.source, "npm");
        assert_eq!(updater.program, system_npm);
        assert_eq!(
            updater.args,
            [
                "install".to_string(),
                "-g".into(),
                "--prefix".into(),
                prefix.to_string_lossy().into_owned(),
                "@google/gemini-cli@latest".into(),
            ]
        );
        assert_eq!(updater.path_first, None);
    }

    #[test]
    fn npm_in_the_prefix_is_preferred_with_its_folder_first_on_path() {
        let temp = tempfile::tempdir().unwrap();
        let (prefix, bin) = npm_prefix(temp.path(), "opencode-ai", "opencode");
        touch(&prefix.join("npm.cmd"), "");
        let updater = how_installed("opencode", &bin, &bin, &none).unwrap();
        assert_eq!(updater.program, prefix.join("npm.cmd"));
        assert_eq!(updater.path_first, Some(prefix));
    }

    #[test]
    fn vendor_installed_clis_update_themselves() {
        let temp = tempfile::tempdir().unwrap();
        let claude = temp.path().join(".local").join("bin").join("claude.exe");
        let updater = how_installed("claude", &claude, &claude, &none).unwrap();
        assert_eq!(
            (updater.source, updater.args.as_slice()),
            ("native", ["update".to_string()].as_slice())
        );
        assert_eq!(updater.program, claude);

        let codex_bin = temp.path().join("Programs/OpenAI/Codex/bin/codex.exe");
        let codex_real = temp.path().join(
            ".codex/packages/standalone/releases/0.163.0-x86_64-pc-windows-msvc/bin/codex.exe",
        );
        let updater = how_installed("codex", &codex_bin, &codex_real, &none).unwrap();
        assert_eq!(updater.args, ["update".to_string()]);
        assert_eq!(updater.program, codex_bin);

        let opencode = temp
            .path()
            .join(".opencode")
            .join("bin")
            .join("opencode.exe");
        let updater = how_installed("opencode", &opencode, &opencode, &none).unwrap();
        assert_eq!(updater.args, ["upgrade".to_string()]);

        let hermes = temp.path().join("hermes").join("bin").join("hermes.exe");
        let updater = how_installed("hermes", &hermes, &hermes, &none).unwrap();
        assert_eq!(updater.args, ["update".to_string()]);
    }

    #[test]
    fn pnpm_and_bun_installs_go_back_to_their_manager() {
        let temp = tempfile::tempdir().unwrap();
        let pnpm_home = temp.path().join("Local").join("pnpm");
        let shim = pnpm_home.join("gemini.cmd");
        touch(
            &shim,
            "@\"%~dp0\\global\\5\\.pnpm\\@google+gemini-cli@0.63.0\\node_modules\\@google\\gemini-cli\\dist\\index.js\" %*",
        );
        let pnpm = pnpm_home.join("pnpm.exe");
        let lookup = |name: &str| (name == "pnpm").then(|| pnpm.clone());
        let updater = how_installed("gemini", &shim, &shim, &lookup).unwrap();
        assert_eq!(updater.source, "pnpm");
        assert_eq!(updater.program, pnpm);
        assert_eq!(updater.args, ["add", "-g", "@google/gemini-cli@latest"]);

        let bun_bin = temp.path().join(".bun").join("bin");
        let bin = bun_bin.join("opencode.exe");
        touch(&bun_bin.join("bun.exe"), "");
        let real = temp
            .path()
            .join(".bun/install/global/node_modules/opencode-ai/bin/opencode.exe");
        let updater = how_installed("opencode", &bin, &real, &none).unwrap();
        assert_eq!(updater.source, "bun");
        assert_eq!(updater.program, bun_bin.join("bun.exe"));
        assert_eq!(updater.args, ["add", "-g", "opencode-ai@latest"]);
    }

    #[test]
    fn volta_yarn_and_unknown_installs_are_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let volta = temp.path().join(
            "Volta/tools/image/packages/@openai/codex/node_modules/@openai/codex/bin/codex.js",
        );
        let bin = temp.path().join("Volta").join("bin").join("codex.exe");
        assert_eq!(how_installed("codex", &bin, &volta, &none), None);

        let stray = temp.path().join("tools").join("gemini.exe");
        touch(&stray, "MZ");
        assert_eq!(how_installed("gemini", &stray, &stray, &none), None);
        assert_eq!(how_installed("workbuddy", &stray, &stray, &none), None);
    }

    #[test]
    fn shim_names_only_reads_small_scripts_that_name_the_package() {
        let temp = tempfile::tempdir().unwrap();
        let shim = temp.path().join("codex.cmd");
        touch(
            &shim,
            "\"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*",
        );
        assert!(shim_names(&shim, "@openai/codex"));
        assert!(!shim_names(&shim, "@google/gemini-cli"));
        let big = temp.path().join("big.cmd");
        touch(&big, &"x".repeat(70 * 1024));
        assert!(!shim_names(&big, "@openai/codex"));
        assert!(!shim_names(
            &temp.path().join("missing.cmd"),
            "@openai/codex"
        ));
    }
}
