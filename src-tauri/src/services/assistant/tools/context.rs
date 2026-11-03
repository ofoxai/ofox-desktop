use super::scope::Scope;
use std::path::PathBuf;

#[derive(Clone)]
pub struct LocalPaths {
    pub home: PathBuf,
    pub config_roots: Vec<PathBuf>,
    pub shell_files: Vec<PathBuf>,
    pub skill: PathBuf,
}
impl LocalPaths {
    pub fn scope(&self, folders: &[PathBuf]) -> Result<Scope, String> {
        let mut roots = self.config_roots.clone();
        if let Some(parent) = self.skill.parent() {
            roots.push(parent.into());
        }
        roots.extend_from_slice(folders);
        Scope::new(self.home.clone(), roots, self.shell_files.clone())?
            .with_history_roots(&self.config_roots)
    }
}

pub async fn application_paths(runtime: PathBuf) -> Result<LocalPaths, String> {
    let runtime = std::fs::canonicalize(runtime).map_err(|_| "assistant_local_unavailable")?;
    crate::agent_env::ready().await;
    let home = crate::config::get_home_dir();
    let mut roots = Vec::new();
    for tool in [
        "claude",
        "codex",
        "gemini",
        "opencode",
        "openclaw",
        "hermes",
        "workbuddy",
        "dsh",
        "pi",
    ] {
        let file = crate::commands::get_tool_config_file_path(tool.into())
            .map_err(|_| "assistant_local_unavailable")?;
        if let Some(parent) = PathBuf::from(file).parent() {
            roots.push(parent.into());
        }
    }
    let mut shell_files: Vec<_> = [
        ".zshrc",
        ".zprofile",
        ".bashrc",
        ".bash_profile",
        ".bash_login",
        ".config/fish/config.fish",
        ".config/fish/conf.d/ofox-desktop.fish",
        ".profile",
    ]
    .iter()
    .map(|name| home.join(name))
    .collect();
    shell_files.extend(
        [
            "Documents/PowerShell/Microsoft.PowerShell_profile.ps1",
            "Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1",
        ]
        .iter()
        .map(|name| home.join(name)),
    );
    Ok(LocalPaths {
        home,
        config_roots: roots,
        shell_files,
        skill: runtime.join("runtime/skills/local-config/SKILL.md"),
    })
}
