use std::path::{Path, PathBuf};

fn base(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .trim_end_matches(".exe")
        .to_owned()
}
fn blocked(path: &Path) -> bool {
    let name = base(path);
    [
        "npx",
        "npm",
        "pnpm",
        "yarn",
        "bun",
        "bunx",
        "uv",
        "uvx",
        "pip",
        "pip3",
        "pipx",
        "docker",
        "podman",
        "curl",
        "wget",
        "env",
        "sudo",
        "doas",
        "sh",
        "bash",
        "zsh",
        "fish",
        "dash",
        "cmd",
        "powershell",
        "pwsh",
        "osascript",
        "wscript",
        "cscript",
        "npx-cli.js",
        "npm-cli.js",
        "pnpm.cjs",
        "yarn.js",
        "corepack",
        "deno",
    ]
    .contains(&name.as_str())
        || path.extension().is_some_and(|ext| {
            ["cmd", "bat", "ps1", "sh"]
                .contains(&ext.to_string_lossy().to_ascii_lowercase().as_str())
        })
}
fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(windows)]
    {
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    }
}
pub(super) fn installed_program(
    command: &str,
    args: &[String],
    path: &str,
    cwd: &Path,
) -> Result<PathBuf, &'static str> {
    let requested = Path::new(command);
    if blocked(requested) {
        return Err("runnerBlocked");
    }
    let candidates = if requested.is_absolute() {
        vec![requested.to_owned()]
    } else if requested.components().count() > 1 {
        vec![cwd.join(requested)]
    } else {
        std::env::split_paths(path)
            .filter(|dir| dir.is_absolute())
            .flat_map(|dir| {
                #[cfg(windows)]
                {
                    vec![dir.join(format!("{command}.exe")), dir.join(command)]
                }
                #[cfg(unix)]
                {
                    vec![dir.join(command)]
                }
            })
            .collect()
    };
    let selected = candidates
        .into_iter()
        .find(|path| executable(path))
        .ok_or("programMissing")?;
    let resolved = selected.canonicalize().map_err(|_| "programMissing")?;
    if blocked(&resolved) {
        return Err("runnerBlocked");
    }
    let name = base(&resolved);
    let is_interpreter = |name: &str| {
        ["node", "nodejs", "python", "ruby", "perl"]
            .iter()
            .any(|prefix| {
                name.strip_prefix(prefix).is_some_and(|suffix| {
                    suffix.is_empty()
                        || suffix
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || b".-".contains(&byte))
                })
            })
    };
    if is_interpreter(&name) || is_interpreter(&base(requested)) {
        let script = args
            .first()
            .filter(|arg| !arg.starts_with('-'))
            .ok_or("localScriptRequired")?;
        let script = cwd
            .join(script)
            .canonicalize()
            .map_err(|_| "localScriptRequired")?;
        if !script.is_file() || blocked(&script) {
            return Err("localScriptRequired");
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_runners_shells_and_missing_programs_never_get_a_spawn_permit() {
        let dir = tempfile::tempdir().unwrap();
        for command in [
            "npx",
            "NPX.EXE",
            "npm",
            "pnpm",
            "bunx",
            "uvx",
            "uv",
            "pipx",
            "docker",
            "podman",
            "env",
            "bash",
            "cmd.exe",
            "powershell.exe",
            "pwsh",
            "curl",
            "wget",
        ] {
            assert!(
                installed_program(command, &["anything".into()], "", dir.path()).is_err(),
                "{command}"
            );
        }
        assert!(installed_program("missing-fixture-program", &[], "", dir.path()).is_err());
    }

    #[test]
    fn interpreters_require_an_existing_local_script_and_never_eval_or_package_modules() {
        let dir = tempfile::tempdir().unwrap();
        let node = dir
            .path()
            .join(if cfg!(windows) { "node.exe" } else { "node" });
        std::fs::write(&node, "fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        for args in [
            vec!["-e", "process.exit()"],
            vec!["--eval=process.exit()"],
            vec!["-m", "pip"],
            vec!["-c", "print('x')"],
            vec!["missing.js"],
        ] {
            assert!(installed_program(
                node.to_str().unwrap(),
                &args.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                "",
                dir.path()
            )
            .is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn deno_remote_run_and_versioned_node_eval_never_get_spawn_permission() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("run"), "local decoy").unwrap();
        for (name, args) in [
            ("deno", vec!["run", "https://example.invalid/server.ts"]),
            ("node22", vec!["-e", "eval"]),
        ] {
            let program = dir.path().join(name);
            std::fs::write(&program, "fixture").unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert!(installed_program(
                program.to_str().unwrap(),
                &args.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                "",
                dir.path()
            )
            .is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn executable_search_rejects_directories_and_non_executables_and_checks_aliases() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let server = dir.path().join("fixture-server");
        std::fs::write(&server, "fixture").unwrap();
        assert!(installed_program(server.to_str().unwrap(), &[], "", dir.path()).is_err());
        assert!(installed_program(dir.path().to_str().unwrap(), &[], "", dir.path()).is_err());
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            installed_program(
                "fixture-server",
                &[],
                dir.path().to_str().unwrap(),
                dir.path()
            )
            .unwrap(),
            server.canonicalize().unwrap()
        );
        let runner = dir.path().join("npx");
        std::fs::write(&runner, "fixture").unwrap();
        std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o700)).unwrap();
        let alias = dir.path().join("innocent-alias");
        symlink(&runner, &alias).unwrap();
        assert!(installed_program(alias.to_str().unwrap(), &[], "", dir.path()).is_err());
    }
}
