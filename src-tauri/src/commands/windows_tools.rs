//! Windows 上找命令行工具：此刻新开一个终端会运行的那个程序在哪。
//!
//! Ported from magpie (https://github.com/yetone/magpie)
//! internal/proc/{find.go, path_windows.go, realpath_windows.go}
//! — MIT License, Copyright (c) 2026 yetone.
//!
//! 从开始菜单启动的 Ofox 继承的是登录时的 PATH。之后装上或自动升级的 CLI，安装器
//! 把目录写进了注册表里的 PATH，Ofox 自己的 PATH 却没有，于是显示"未安装"。这里按
//! 注册表**当前**的 PATH（机器的、用户的）找，再补上 Ofox 进程 PATH 里独有的目录和
//! 用户工具常装的目录。检测、启动、升级用同一个 PATH，看到的和终端里一致。

use std::path::{Path, PathBuf};

/// 程序文件名的规则：Windows 认扩展名，Unix 只有裸名（让单测能在 macOS 上跑）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flavor {
    Windows,
    Unix,
}

/// `dirs` 里第一个有 `name` 程序的：Windows 上依次 `X.exe`、`X.cmd`、`X`
/// （安装器的二进制、npm 的 shim）。
pub(crate) fn tool_in(name: &str, flavor: Flavor, dirs: &[PathBuf]) -> Option<PathBuf> {
    let names = match flavor {
        Flavor::Windows => vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            name.to_string(),
        ],
        Flavor::Unix => vec![name.to_string()],
    };
    dirs.iter()
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
}

/// 用户命令行工具常装、PATH 未必带上的目录，只保留存在的：npm 全局、Claude 和
/// Codex 安装器的 `~\.local\bin`、bun、Volta、pnpm。
pub(crate) fn user_bin_dirs(
    home: &Path,
    appdata: Option<&Path>,
    local_appdata: Option<&Path>,
) -> Vec<PathBuf> {
    let mut known = Vec::new();
    if let Some(appdata) = appdata {
        known.push(appdata.join("npm"));
    }
    known.push(home.join(".local").join("bin"));
    known.push(home.join(".bun").join("bin"));
    if let Some(local) = local_appdata {
        known.push(local.join("Volta").join("bin"));
        known.push(local.join("pnpm"));
    }
    known
        .into_iter()
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .collect()
}

/// 按 Windows 的规则展开 `%NAME%`：没定义的变量和落单的 `%` 原样保留。
pub(crate) fn expand_env(raw: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        match (!name.is_empty()).then(|| lookup(name)).flatten() {
            Some(value) => out.push_str(&value),
            None => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// 注册表里的 PATH 值拆成目录：先展开变量，去掉空项。
pub(crate) fn split_path_value(raw: &str, lookup: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    expand_env(raw, lookup)
        .split(';')
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// 终端此刻的 PATH：机器的、用户的，再补上 Ofox 进程 PATH 里独有的目录。
/// 大小写不敏感、忽略结尾的 `\` 去重，保留第一次出现的位置。
pub(crate) fn merge_path(
    machine: Vec<PathBuf>,
    user: Vec<PathBuf>,
    process: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let key = |dir: &PathBuf| {
        dir.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    let mut seen = std::collections::HashSet::new();
    machine
        .into_iter()
        .chain(user)
        .chain(process)
        .filter(|dir| seen.insert(key(dir)))
        .collect()
}

/// `canonicalize` 在 Windows 返回 `\\?\` 前缀的路径：`\\?\C:\x` → `C:\x`，
/// `\\?\UNC\server\share` → `\\server\share`。
pub(crate) fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, "").unwrap();
        path
    }

    fn env(name: &str) -> Option<String> {
        match name.to_ascii_uppercase().as_str() {
            "SYSTEMROOT" => Some(r"C:\Windows".into()),
            "USERPROFILE" => Some(r"C:\Users\me".into()),
            _ => None,
        }
    }

    #[test]
    fn tool_in_prefers_an_installer_binary_then_an_npm_shim_then_the_bare_name() {
        let temp = tempfile::tempdir().unwrap();
        let npm = temp.path().join("npm");
        touch(&npm, "codex");
        let shim = touch(&npm, "codex.cmd");
        assert_eq!(
            tool_in("codex", Flavor::Windows, std::slice::from_ref(&npm)),
            Some(shim)
        );
        let exe = touch(&npm, "codex.exe");
        assert_eq!(tool_in("codex", Flavor::Windows, &[npm]), Some(exe));
    }

    #[test]
    fn tool_in_takes_the_first_folder_that_has_it_and_skips_folders_named_like_it() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        fs::create_dir_all(first.join("claude.exe")).unwrap();
        let second = temp.path().join("second");
        let found = touch(&second, "claude.cmd");
        touch(&temp.path().join("third"), "claude.exe");
        let dirs = [first, second, temp.path().join("third")];
        assert_eq!(tool_in("claude", Flavor::Windows, &dirs), Some(found));
        assert_eq!(tool_in("missing", Flavor::Windows, &dirs), None);
    }

    #[test]
    fn tool_in_on_unix_only_takes_the_bare_name() {
        let temp = tempfile::tempdir().unwrap();
        touch(temp.path(), "gemini.cmd");
        assert_eq!(tool_in("gemini", Flavor::Unix, &[temp.path().into()]), None);
        let bare = touch(temp.path(), "gemini");
        assert_eq!(
            tool_in("gemini", Flavor::Unix, &[temp.path().into()]),
            Some(bare)
        );
    }

    #[test]
    fn user_bin_dirs_keeps_only_the_folders_that_exist_in_order() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let appdata = temp.path().join("Roaming");
        let local = temp.path().join("Local");
        fs::create_dir_all(home.join(".local").join("bin")).unwrap();
        fs::create_dir_all(appdata.join("npm")).unwrap();
        fs::create_dir_all(local.join("pnpm")).unwrap();
        assert_eq!(
            user_bin_dirs(&home, Some(&appdata), Some(&local)),
            vec![
                appdata.join("npm"),
                home.join(".local").join("bin"),
                local.join("pnpm"),
            ]
        );
        assert_eq!(
            user_bin_dirs(&home, None, None),
            vec![home.join(".local").join("bin")]
        );
    }

    #[test]
    fn expand_env_follows_windows_rules() {
        assert_eq!(
            expand_env(r"%SystemRoot%\system32", env),
            r"C:\Windows\system32"
        );
        assert_eq!(expand_env(r"%systemroot%\x", env), r"C:\Windows\x");
        assert_eq!(expand_env(r"%NOPE%\bin", env), r"%NOPE%\bin");
        assert_eq!(expand_env("100%", env), "100%");
        assert_eq!(expand_env("%%", env), "%%");
        assert_eq!(
            expand_env(r"%NOPE%%USERPROFILE%\bin", env),
            r"%NOPE%C:\Users\me\bin"
        );
    }

    #[test]
    fn split_path_value_expands_and_drops_empty_entries() {
        assert_eq!(
            split_path_value(r"%SystemRoot%\system32;; C:\Tools ;", env),
            vec![
                PathBuf::from(r"C:\Windows\system32"),
                PathBuf::from(r"C:\Tools")
            ]
        );
    }

    #[test]
    fn merge_path_puts_the_registry_first_and_dedupes_case_insensitively() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            merge_path(
                vec![p(r"C:\Windows\system32"), p(r"C:\Program Files\nodejs\")],
                vec![
                    p(r"C:\Users\me\AppData\Local\Programs\OpenAI\Codex\bin"),
                    p(r"c:\program files\nodejs")
                ],
                vec![
                    p(r"C:\Users\me\AppData\Roaming\npm"),
                    p(r"C:\WINDOWS\System32")
                ],
            ),
            vec![
                p(r"C:\Windows\system32"),
                p(r"C:\Program Files\nodejs\"),
                p(r"C:\Users\me\AppData\Local\Programs\OpenAI\Codex\bin"),
                p(r"C:\Users\me\AppData\Roaming\npm"),
            ]
        );
    }

    #[test]
    fn strip_verbatim_returns_the_path_people_type() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\Users\me\.local\bin\claude.exe")),
            PathBuf::from(r"C:\Users\me\.local\bin\claude.exe")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"C:\x")),
            PathBuf::from(r"C:\x")
        );
    }
}
