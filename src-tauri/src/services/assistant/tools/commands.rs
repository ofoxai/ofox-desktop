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

#[cfg(test)]
mod tests {
    use super::*;
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
