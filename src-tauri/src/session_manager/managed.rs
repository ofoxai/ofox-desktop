//! Server-owned session references and capabilities. Paths and shell commands from
//! the renderer are never authoritative for reads, resume, or file management.
use super::{providers, SessionMessage, SessionMeta};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub can_read_messages: bool,
    pub can_export: bool,
    pub can_resume: bool,
    pub can_trash: bool,
    pub resume_uses_session_model: bool,
    pub read_only_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedSession {
    #[serde(flatten)]
    pub meta: SessionMeta,
    pub session_key: String,
    pub revision: Option<String>,
    pub capabilities: Capabilities,
    pub models: Vec<String>,
    pub subagent_count: usize,
}

pub fn roots(provider: &str) -> Result<Vec<PathBuf>, String> {
    Ok(match provider {
        "claude" => vec![crate::config::get_claude_config_dir()],
        "codex" => vec![crate::codex_config::get_codex_config_dir().join("sessions")],
        "gemini" => vec![crate::gemini_config::get_gemini_dir().join("tmp")],
        "opencode" => vec![providers::opencode::get_opencode_data_dir()],
        "hermes" => vec![crate::hermes_config::get_hermes_dir()],
        "openclaw" => vec![crate::openclaw_config::get_openclaw_dir().join("agents")],
        "pi" => providers::pi::session_roots()?,
        "dsh" => vec![providers::dsh::home_dir()?],
        _ => return Err("Unknown session provider".into()),
    })
}

fn source_file(meta: &SessionMeta) -> Result<PathBuf, String> {
    let source = meta.source_path.as_deref().ok_or("Session has no source")?;
    if let Some(sqlite) = source.strip_prefix("sqlite:") {
        let path = match meta.provider_id.as_str() {
            "opencode" => sqlite.rsplit_once(":ses_").map(|(path, _)| path),
            "hermes" => sqlite.rsplit_once('#').map(|(path, _)| path),
            _ => None,
        }
        .ok_or("Invalid SQLite session reference")?;
        return Ok(PathBuf::from(path));
    }
    Ok(PathBuf::from(source))
}

pub(crate) fn validate_path(path: &Path, roots: &[PathBuf]) -> Result<(), String> {
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Session source cannot be resolved: {e}"))?;
    if !roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| canonical.starts_with(root))
    {
        return Err("Session source is outside its configured provider roots".into());
    }
    // Refuse symlink ancestors: a later rename must not follow a replaced link.
    let mut cursor = Some(path);
    while let Some(p) = cursor {
        if std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Session source contains a symbolic link".into());
        }
        if roots.iter().any(|r| p == r) {
            break;
        }
        cursor = p.parent();
    }
    Ok(())
}

pub(crate) fn key(meta: &SessionMeta) -> String {
    let mut hash = Sha256::new();
    for text in [
        meta.provider_id.as_str(),
        meta.session_id.as_str(),
        meta.source_path.as_deref().unwrap_or(""),
    ] {
        hash.update(text.as_bytes());
        hash.update([0]);
    }
    format!("{:x}", hash.finalize())
}

pub fn list() -> Vec<ManagedSession> {
    let mut seen = HashSet::new();
    let listing = super::scan_sessions();
    listing
        .iter()
        .cloned()
        .filter_map(|meta| {
            let roots = roots(&meta.provider_id).ok()?;
            validate_path(&source_file(&meta).ok()?, &roots).ok()?;
            // Codex may have several rollout segments for the same session. One UI row,
            // and a bundle planner that retains every segment, including compressed twins.
            if !seen.insert((meta.provider_id.clone(), meta.session_id.clone())) {
                return None;
            }
            let readable = meta.provider_id != "dsh";
            let resume = resume_args(&meta).is_ok() && resume_project(&meta).is_ok();
            let plan = super::trash::plan_from_listing(&meta, &listing);
            let revision = plan
                .as_ref()
                .ok()
                .and_then(|p| super::trash::fingerprint(&p.paths).ok());
            let subagent_count = if matches!(meta.provider_id.as_str(), "dsh" | "gemini") {
                plan.as_ref()
                    .map(|p| p.member_ids.len().saturating_sub(1))
                    .unwrap_or(0)
            } else {
                0
            };
            Some(ManagedSession {
                session_key: key(&meta),
                revision: revision.clone(),
                capabilities: Capabilities {
                    can_read_messages: readable,
                    can_export: readable,
                    can_resume: resume,
                    can_trash: plan.is_ok() && revision.is_some(),
                    resume_uses_session_model: meta.provider_id == "pi",
                    read_only_reason: plan.err().or_else(|| {
                        revision
                            .is_none()
                            .then(|| "Session bundle cannot be safely fingerprinted".into())
                    }),
                },
                models: meta
                    .source_path
                    .as_deref()
                    .map(Path::new)
                    .map(|path| match meta.provider_id.as_str() {
                        "pi" => providers::pi::models(path),
                        "dsh" => providers::dsh::models(path),
                        _ => Vec::new(),
                    })
                    .unwrap_or_default(),
                subagent_count,
                meta,
            })
        })
        .collect()
}

pub(crate) fn resolve(session_key: &str) -> Result<ManagedSession, String> {
    if session_key.len() != 64 || !session_key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid session reference".into());
    }
    list()
        .into_iter()
        .find(|record| record.session_key == session_key)
        .ok_or("Session is no longer available; refresh the listing".into())
}

pub fn messages(session_key: &str) -> Result<Vec<SessionMessage>, String> {
    let record = resolve(session_key)?;
    if !record.capabilities.can_read_messages {
        return Err("This provider supports metadata only".into());
    }
    if record.meta.provider_id == "codex" {
        let mut messages = Vec::new();
        let plan = super::trash::plan(&record.meta)?;
        let mut seen = HashSet::new();
        // Prefer an uncompressed rollout when its compressed mirror is present.
        let mut paths = plan.paths.clone();
        paths.sort_by_key(|p| p.extension().is_some_and(|e| e == "zst" || e == "zstd"));
        for path in paths.iter() {
            let base = if path.extension().is_some_and(|e| e == "zst" || e == "zstd") {
                path.with_extension("")
            } else {
                path.clone()
            };
            if !seen.insert(base) {
                continue;
            }
            messages.extend(providers::codex::load_messages(path)?);
        }
        return Ok(messages);
    }
    super::load_messages_unchecked(
        &record.meta.provider_id,
        record
            .meta
            .source_path
            .as_deref()
            .ok_or("Missing session source")?,
    )
}

pub(crate) fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn resume_args(meta: &SessionMeta) -> Result<Vec<String>, String> {
    let id = &meta.session_id;
    if id.is_empty()
        || id == "."
        || id == ".."
        || id.starts_with('-')
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err("Session ID cannot be safely resumed".into());
    }
    let args = match meta.provider_id.as_str() {
        "claude" => vec!["claude", "--resume", id],
        "codex" => vec!["codex", "resume", id],
        "gemini" => vec!["gemini", "--resume", id],
        "opencode" => vec!["opencode", "--session", id],
        // Pi resolves the precise discovered file, including a custom session directory.
        "pi" => vec![
            "pi",
            "--session",
            meta.source_path.as_deref().ok_or("Missing Pi source")?,
        ],
        _ => return Err("This provider has no generic resume adapter".into()),
    };
    Ok(args.into_iter().map(str::to_string).collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeResult {
    pub command: String,
    pub launched: bool,
}

fn resume_project(meta: &SessionMeta) -> Result<&str, String> {
    let cwd = meta
        .project_dir
        .as_deref()
        .filter(|d| !d.trim().is_empty())
        .ok_or("Session project directory is unknown")?;
    if !Path::new(cwd).is_absolute() || !Path::new(cwd).is_dir() {
        return Err("Session project directory no longer exists or is not absolute".into());
    }
    Ok(cwd)
}

pub fn resume(session_key: &str, launch: bool) -> Result<ResumeResult, String> {
    let record = resolve(session_key)?;
    let args = resume_args(&record.meta)?;
    let cwd = Some(resume_project(&record.meta)?);
    let pi_dir = if record.meta.provider_id == "pi" {
        Some(providers::pi::agent_dir()?.to_string_lossy().into_owned())
    } else {
        None
    };
    let command = render_resume_command(&args, cwd, pi_dir.as_deref(), cfg!(target_os = "windows"));
    if launch {
        let preferred = crate::settings::get_preferred_terminal();
        let target = match preferred.as_deref() {
            Some("iterm2") => "iterm",
            Some(target) => target,
            None => "terminal",
        };
        super::terminal::launch_terminal(target, &command, None, None)?;
    }
    Ok(ResumeResult {
        command,
        launched: launch,
    })
}

fn render_resume_command(
    args: &[String],
    cwd: Option<&str>,
    pi_dir: Option<&str>,
    windows: bool,
) -> String {
    if windows {
        // PowerShell literal quoting avoids interpolation and supports Windows paths.
        let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let cmd = format!(
            "& {}",
            args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
        );
        let cmd = pi_dir
            .map(|dir| format!("$env:PI_CODING_AGENT_DIR = {}; {cmd}", quote(dir)))
            .unwrap_or(cmd);
        cwd.map(|d| format!("Set-Location -LiteralPath {}; {cmd}", quote(d)))
            .unwrap_or(cmd)
    } else {
        let cmd = args
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        let cmd = pi_dir
            .map(|dir| {
                format!(
                    "env {} {cmd}",
                    shell_quote(&format!("PI_CODING_AGENT_DIR={dir}"))
                )
            })
            .unwrap_or(cmd);
        cwd.map(|d| format!("cd -- {} && {cmd}", shell_quote(d)))
            .unwrap_or(cmd)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownExport {
    pub filename: String,
    pub content: String,
}

// Dynamic fences adapted from Magpie internal/sessions/markdown.go (MIT).
fn fence(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(3.max(longest + 1))
}

pub fn export_markdown(
    session_key: &str,
    destination: Option<&Path>,
) -> Result<MarkdownExport, String> {
    let record = resolve(session_key)?;
    if !record.capabilities.can_export {
        return Err("This provider has no transcript export adapter".into());
    }
    let messages = messages(session_key)?;
    let title = record
        .meta
        .title
        .as_deref()
        .unwrap_or(&record.meta.session_id)
        .replace(['\r', '\n'], " ");
    let mut content = format!(
        "# {title}\n\n- Tool: {}\n- Session: {}\n",
        record.meta.provider_id, record.meta.session_id
    );
    if let Some(cwd) = &record.meta.project_dir {
        content.push_str(&format!("- Project: {}\n", cwd.replace(['\r', '\n'], " ")));
    }
    for message in messages {
        let fence = fence(&message.content);
        content.push_str(&format!(
            "\n## {}\n\n{fence}\n{}\n{fence}\n",
            message.role.replace(['\r', '\n'], " "),
            message.content
        ));
    }
    let filename = format!(
        "ofox-session-{}-{}.md",
        record.meta.provider_id,
        &session_key[..12]
    );
    if let Some(destination) = destination {
        use std::io::Write;
        if !destination.is_absolute() {
            return Err("Export destination must be absolute".into());
        }
        let parent = destination.parent().ok_or("Invalid export destination")?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        file.write_all(content.as_bytes())
            .and_then(|()| file.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        file.persist_noclobber(destination).map_err(|e| {
            format!("Could not save export without overwriting an existing file: {e}")
        })?;
    }
    Ok(MarkdownExport { filename, content })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fences_and_shell_literals_preserve_untrusted_text() {
        assert_eq!(fence("nested ```` code"), "`````");
        assert_eq!(
            shell_quote("a'$(touch /tmp/no)"),
            "'a'\\''$(touch /tmp/no)'"
        );
    }
    #[test]
    fn source_symlink_escape_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("session.jsonl");
        std::fs::write(&file, "{}").unwrap();
        assert!(validate_path(&file, &[root.path().into()]).is_err());
    }

    #[test]
    fn resume_commands_quote_cwd_and_pin_pi_agent_directory_on_both_platforms() {
        let args = vec![
            "pi".into(),
            "--session".into(),
            "C:\\sessions\\a'$(bad).jsonl".into(),
        ];
        let unix =
            render_resume_command(&args, Some("/project ' $(bad)"), Some("/config pi"), false);
        assert!(unix.contains("env 'PI_CODING_AGENT_DIR=/config pi' 'pi' '--session'"));
        assert!(unix.contains("cd -- '/project '\\'' $(bad)'"));
        let windows = render_resume_command(&args, Some("C:\\project's"), Some("C:\\pi"), true);
        assert!(windows.starts_with(
            "Set-Location -LiteralPath 'C:\\project''s'; $env:PI_CODING_AGENT_DIR = 'C:\\pi';"
        ));
        assert!(windows.contains("'C:\\sessions\\a''$(bad).jsonl'"));
    }

    #[test]
    fn resume_requires_the_known_existing_absolute_project() {
        let temp = tempfile::tempdir().unwrap();
        let mut meta = SessionMeta {
            provider_id: "pi".into(),
            session_id: "s".into(),
            title: None,
            summary: None,
            project_dir: None,
            created_at: None,
            last_active_at: None,
            source_path: None,
            resume_command: None,
        };
        assert!(resume_project(&meta).is_err());
        meta.project_dir = Some(".".into());
        assert!(resume_project(&meta).is_err());
        meta.project_dir = Some(temp.path().join("missing").to_string_lossy().into());
        assert!(resume_project(&meta).is_err());
        meta.project_dir = Some(temp.path().to_string_lossy().into());
        assert!(resume_project(&meta).is_ok());
    }
}
