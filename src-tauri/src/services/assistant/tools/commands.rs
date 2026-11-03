//! Conservative shell recognition, not shell evaluation. Files must be read
//! through scoped tools; cat/Get-Content/env are intentionally not automatic.
pub fn read_only(command: &str, powershell: bool) -> bool {
    let command = command.trim();
    if powershell && command == "$PSVersionTable.PSVersion" {
        return true;
    }
    if command.is_empty()
        || command.contains([
            '\n', '\r', ';', '&', '|', '>', '<', '$', '`', '(', ')', '{', '}', '[', ']', '\\',
            '\'', '"',
        ])
    {
        return false;
    }
    let words: Vec<_> = command.split_whitespace().collect();
    let known = |name: &str| {
        matches!(
            name,
            "claude"
                | "codex"
                | "gemini"
                | "opencode"
                | "openclaw"
                | "hermes"
                | "workbuddy"
                | "dsh"
                | "pi"
                | "node"
                | "npm"
                | "pnpm"
                | "git"
                | "fnm"
                | "mise"
        )
    };
    match words.as_slice() {
        ["pwd" | "whoami"] => true,
        [name, "--version"] => known(name),
        ["command", "-v", name] if !powershell => known(name),
        ["which", name] if !powershell => known(name),
        [name] if powershell && name.eq_ignore_ascii_case("Get-Location") => true,
        [name, tool] if powershell && name.eq_ignore_ascii_case("Get-Command") => known(tool),
        _ => false,
    }
}

/// The program a simple command runs, for "always allow in this conversation".
/// A command with shell composition, a path-qualified program or an
/// interpreter (which runs whatever it is handed) has none: only that exact
/// command can be allowed again.
pub fn program(command: &str) -> Option<String> {
    let command = command.trim();
    if command.is_empty()
        || command.contains([
            '\n', '\r', ';', '&', '|', '>', '<', '$', '`', '(', ')', '{', '}', '[', ']', '\'', '"',
        ])
    {
        return None;
    }
    let name = command.split_whitespace().next()?.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    let runs_anything = matches!(
        name,
        "sh" | "bash"
            | "zsh"
            | "fish"
            | "dash"
            | "ksh"
            | "csh"
            | "tcsh"
            | "pwsh"
            | "powershell"
            | "cmd"
            | "python"
            | "python3"
            | "node"
            | "bun"
            | "deno"
            | "ruby"
            | "perl"
            | "php"
            | "osascript"
            | "env"
            | "sudo"
            | "doas"
            | "xargs"
            | "nohup"
            | "exec"
            | "eval"
            | "source"
            | "."
            | "npx"
            | "pnpx"
            | "bunx"
            | "invoke-expression"
            | "iex"
            | "start-process"
    );
    (!name.is_empty() && !name.contains(['=', '/', '\\', ':']) && !runs_anything)
        .then(|| name.to_string())
}

/// Block direct attempts to read protected stores even in FullAuto. Arbitrary
/// programs still run with the user's OS rights; this is intentionally not a sandbox.
pub fn forbidden(command: &str) -> bool {
    command
        .split(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | ';' | '|' | '&'))
        .any(|token| {
            matches!(token, "security" | "secret-tool" | "cmdkey" | "cmdkey.exe")
                || (!token.starts_with("https://")
                    && !token.starts_with("http://")
                    && super::scope::sensitive_path(token))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_policy_recognizes_protected_access_syntax() {
        for command in [
            "cat ~/.ssh/id_rsa",
            "security find-generic-password -w",
            "Get-Content C:/Users/me/.ssh/id_ed25519",
            "sqlite3 ~/Library/Application\\ Support/Google/Chrome/Default/Cookies",
        ] {
            assert!(forbidden(command));
        }
        assert!(!forbidden("node --version"));
    }
    #[test]
    fn always_allow_names_the_program_of_a_simple_command_only() {
        for (command, expected) in [
            ("npm install -g opencode-ai", Some("npm")),
            ("Brew upgrade codex", Some("brew")),
            // A path names a file, not the program the user allowed.
            ("./npm install", None),
            ("/tmp/x/npm install", None),
            (r"C:\Tools\npm.cmd install", None),
            // An interpreter runs whatever it is given.
            ("sh install.sh", None),
            ("python3 -m pip list", None),
            ("node script.js", None),
            ("env npm test", None),
            ("osascript -e x", None),
            ("npm i x && rm -rf ~", None),
            ("curl x | sh", None),
            ("echo $(id)", None),
            ("FOO=1 npm test", None),
            ("", None),
        ] {
            assert_eq!(program(command).as_deref(), expected, "{command}");
        }
    }
    #[test]
    fn local_commands_allow_only_bounded_inspection_without_shell_composition() {
        for command in [
            "pwd",
            "whoami",
            "node --version",
            "opencode --version",
            "command -v codex",
            "which claude",
        ] {
            assert!(read_only(command, false), "{command}");
        }
        for command in [
            "cat ~/.ssh/id_rsa",
            "env",
            "printenv",
            "npm install x",
            "node -e 'process.exit()'",
            "pwd > file",
            "pwd; rm x",
            "pwd && rm x",
            "pwd | tee x",
            "echo $(cat file)",
            "echo `cat file`",
            "node --version\nrm x",
            "command -v $(x)",
            "/tmp/node --version",
            "unknown --version",
            "git -c core.pager=evil status",
        ] {
            assert!(!read_only(command, false), "{command}");
        }
    }
    #[test]
    fn local_commands_powershell_does_not_treat_aliases_or_expressions_as_read_only() {
        for command in [
            "Get-Location",
            "Get-Command codex",
            "node --version",
            "$PSVersionTable.PSVersion",
        ] {
            assert!(read_only(command, true), "{command}");
        }
        for command in [
            "Get-Content ~/.ssh/id_rsa",
            "Get-Location | Set-Content x",
            "Get-Location; Remove-Item x",
            "& node --version",
            "node --version > x",
            "iex 'x'",
            "Get-Command $(x)",
            "Get-Command -Name x -ErrorAction $(x)",
            "$env:SECRET",
            "Get-Location`nRemove-Item x",
        ] {
            assert!(!read_only(command, true), "{command}");
        }
    }
}
