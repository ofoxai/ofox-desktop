//! Server-owned session references and capabilities. Paths and shell commands from
//! the renderer are never authoritative for reads, resume, or file management.
use super::usage::{self, SessionUsage};
use super::{providers, SessionMessage, SessionMeta};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// What a file of a session holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    /// The session's own file.
    Main,
    /// A subagent's transcript.
    Subagent,
    /// Another file of the same session (a continued rollout, a compressed twin).
    Segment,
    /// A database the session shares with others.
    Database,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFile {
    pub path: String,
    pub size_bytes: u64,
    pub kind: FileKind,
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
    /// Bytes of every file of the session (0 for one kept in a shared database).
    pub size_bytes: u64,
    /// Its conversation files.
    pub files: Vec<SessionFile>,
    /// What it spent, where its agent's usage is read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionUsage>,
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

/// How long `resolve` reuses the last listing; the matched record is still
/// planned and fingerprinted afresh.
const LISTING_TTL: Duration = Duration::from_secs(5);

static LISTING: Mutex<Option<(Instant, Arc<Vec<SessionMeta>>)>> = Mutex::new(None);

fn remember(listing: Vec<SessionMeta>) -> Arc<Vec<SessionMeta>> {
    let listing = Arc::new(listing);
    *LISTING.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((Instant::now(), Arc::clone(&listing)));
    listing
}

/// The last listing when it is less than 5 s old, else a new scan.
fn recent_listing() -> Arc<Vec<SessionMeta>> {
    if let Some((at, listing)) = LISTING.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if at.elapsed() < LISTING_TTL {
            return Arc::clone(listing);
        }
    }
    remember(super::scan_sessions())
}

/// Drop the cached listing (after sessions were moved to or from the trash).
pub(crate) fn forget_listing() {
    *LISTING.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The listing's sessions that are shown: inside their provider's roots, and
/// one per (provider, session id).
fn shown(listing: &[SessionMeta]) -> impl Iterator<Item = &SessionMeta> {
    let mut seen = HashSet::new();
    listing.iter().filter(move |meta| {
        let Ok(roots) = roots(&meta.provider_id) else {
            return false;
        };
        if source_file(meta)
            .and_then(|source| validate_path(&source, &roots))
            .is_err()
        {
            return false;
        }
        // Codex may have several rollout segments for the same session. One UI row,
        // and a bundle planner that retains every segment, including compressed twins.
        seen.insert((meta.provider_id.clone(), meta.session_id.clone()))
    })
}

/// The listing without a database: every file read whole, no usage.
#[cfg(test)]
pub fn list() -> Vec<ManagedSession> {
    list_with(None)
}

/// The listing, titled and counted from whole files (cached in `db`), with
/// each session's usage from `db`.
pub fn list_with(db: Option<&crate::database::Database>) -> Vec<ManagedSession> {
    let mut listing = super::scan_sessions();
    super::meta_cache::enrich(&mut listing, db);
    let listing = remember(listing);
    let mut usage = db
        .map(|db| {
            usage::load(db).unwrap_or_else(|e| {
                log::warn!("[SESSIONS] {e}");
                Default::default()
            })
        })
        .unwrap_or_default();
    shown(&listing)
        .map(|meta| {
            let spent = usage::app_type(&meta.provider_id)
                .and_then(|app| usage.remove(&(app.to_string(), meta.session_id.clone())));
            ManagedSession {
                usage: spent,
                ..record(meta.clone(), &listing)
            }
        })
        .collect()
}

fn record(meta: SessionMeta, listing: &[SessionMeta]) -> ManagedSession {
    let readable = meta.provider_id != "dsh";
    let resume = can_resume(&meta);
    let plan = super::trash::plan_from_listing(&meta, listing);
    let walked = plan
        .as_ref()
        .ok()
        .and_then(|p| super::trash::fingerprint(&p.paths).ok());
    let revision = walked.as_ref().map(|w| w.hash.clone());
    let (size_bytes, files) = match &walked {
        Some(walked) => (walked.size, session_files(&meta, &walked.files)),
        None => loose_files(&meta),
    };
    let subagent_count = if matches!(meta.provider_id.as_str(), "dsh" | "gemini") {
        plan.as_ref()
            .map(|p| p.member_ids.len().saturating_sub(1))
            .unwrap_or(0)
    } else {
        0
    };
    ManagedSession {
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
        size_bytes,
        files,
        usage: None,
        meta,
    }
}

/// What a file of a session's bundle is, when it holds conversation: the
/// session's own file, a subagent's, or another segment of the session.
fn file_kind(meta: &SessionMeta, path: &Path) -> Option<FileKind> {
    let source = Path::new(meta.source_path.as_deref()?);
    if path == source {
        return Some(FileKind::Main);
    }
    let name = path.file_name()?.to_str()?;
    let transcript = [".jsonl", ".json", ".jsonl.zst", ".jsonl.zstd"]
        .iter()
        .any(|ext| name.ends_with(ext));
    if !transcript {
        return None;
    }
    let beside_source = path.parent() == source.parent();
    match meta.provider_id.as_str() {
        // a Codex bundle holds only the thread's rollouts
        "codex" => Some(FileKind::Segment),
        "claude" => path
            .components()
            .any(|c| c.as_os_str() == "subagents")
            .then_some(FileKind::Subagent),
        // chats/<id>.json(l) beside it, chats/<id>/<child>.json(l) below it
        "gemini" if beside_source => Some(FileKind::Segment),
        "gemini" => {
            (path.parent().and_then(Path::parent) == source.parent()).then_some(FileKind::Subagent)
        }
        "dsh" if !name.starts_with("session") => None,
        "dsh" if beside_source => Some(FileKind::Segment),
        "dsh" => Some(FileKind::Subagent),
        _ => None,
    }
}

fn session_files(meta: &SessionMeta, walked: &[(PathBuf, u64)]) -> Vec<SessionFile> {
    let mut files = walked
        .iter()
        .filter_map(|(path, size)| {
            Some(SessionFile {
                kind: file_kind(meta, path)?,
                path: path.to_string_lossy().into_owned(),
                size_bytes: *size,
            })
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|f| f.kind);
    files
}

/// A session that is not a movable bundle: its own file (or folder), or the
/// database it shares with others, which is not its size.
fn loose_files(meta: &SessionMeta) -> (u64, Vec<SessionFile>) {
    let Ok(path) = source_file(meta) else {
        return (0, Vec::new());
    };
    let size = tree_size(&path);
    let shared = meta
        .source_path
        .as_deref()
        .is_some_and(|s| s.starts_with("sqlite:"));
    let file = SessionFile {
        path: path.to_string_lossy().into_owned(),
        size_bytes: size,
        kind: if shared {
            FileKind::Database
        } else {
            FileKind::Main
        },
    };
    (if shared { 0 } else { size }, vec![file])
}

/// The bytes of a file, or of the files below a folder (links not followed).
fn tree_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return if meta.is_file() { meta.len() } else { 0 };
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| tree_size(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

pub(crate) fn resolve(session_key: &str) -> Result<ManagedSession, String> {
    if session_key.len() != 64 || !session_key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid session reference".into());
    }
    let listing = recent_listing();
    let meta = shown(&listing)
        .find(|meta| key(meta) == session_key)
        .cloned()
        .ok_or("Session is no longer available; refresh the listing")?;
    Ok(record(meta, &listing))
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

fn can_resume(meta: &SessionMeta) -> bool {
    resume_args(meta).is_ok() && resume_project(meta).is_ok()
}

pub fn resume(session_key: &str, launch: bool) -> Result<ResumeResult, String> {
    let record = resolve(session_key)?;
    resume_record(&record.meta, launch)
}

fn resume_record(meta: &SessionMeta, launch: bool) -> Result<ResumeResult, String> {
    let args = resume_args(meta)?;
    let cwd = Some(resume_project(meta)?);
    let pi_dir = if meta.provider_id == "pi" {
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
        let heading = match &message.name {
            Some(name) => format!("{} ({name})", message.role),
            None => message.role.clone(),
        };
        content.push_str(&format!(
            "\n## {}\n\n{fence}\n{}\n{fence}\n",
            heading.replace(['\r', '\n'], " "),
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
    use crate::test_sandbox::sandbox;

    const CLAUDE_ID: &str = "11111111-2222-3333-4444-555555555555";

    /// A Claude Code session in the sandbox home: its file, a subagent's
    /// transcript and a tool result kept beside it.
    fn claude_session(home: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let project = home.join(".claude/projects/-work-app");
        let subagents = project.join(CLAUDE_ID).join("subagents");
        let results = project.join(CLAUDE_ID).join("tool-results");
        std::fs::create_dir_all(&subagents).unwrap();
        std::fs::create_dir_all(&results).unwrap();
        let main = project.join(format!("{CLAUDE_ID}.jsonl"));
        std::fs::write(
            &main,
            format!(
                "{}\n",
                serde_json::json!({"type": "user", "sessionId": CLAUDE_ID, "cwd": "/work/app",
                    "timestamp": "2026-10-14T10:00:00Z",
                    "message": {"role": "user", "content": "fix the login"}})
            ),
        )
        .unwrap();
        let subagent = subagents.join("agent-a1.jsonl");
        std::fs::write(&subagent, "{\"sessionId\":\"x\"}\n").unwrap();
        let result = results.join("toolu_1.txt");
        std::fs::write(&result, "a tool result").unwrap();
        (main, subagent, result)
    }

    fn size(path: &Path) -> u64 {
        std::fs::metadata(path).unwrap().len()
    }

    #[test]
    fn a_listed_session_reports_its_size_and_its_transcript_files() {
        let sandbox = sandbox();
        let (main, subagent, result) = claude_session(sandbox.home());

        let listed = list();
        let record = listed
            .iter()
            .find(|r| r.meta.session_id == CLAUDE_ID)
            .expect("listed");
        let json = serde_json::to_value(record).unwrap();

        assert_eq!(
            json["sizeBytes"],
            serde_json::json!(size(&main) + size(&subagent) + size(&result))
        );
        assert_eq!(
            json["files"],
            serde_json::json!([
                {"path": main.to_string_lossy(), "sizeBytes": size(&main), "kind": "main"},
                {"path": subagent.to_string_lossy(), "sizeBytes": size(&subagent), "kind": "subagent"},
            ])
        );
    }

    /// A long Claude Code session: its title made by Claude Code sits past the
    /// head the quick scan reads, and its lines count 3 messages.
    fn long_claude_session(home: &Path) -> PathBuf {
        let project = home.join(".claude/projects/-work-app");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join(format!("{CLAUDE_ID}.jsonl"));
        let user = |content: serde_json::Value| {
            serde_json::json!({"type": "user", "sessionId": CLAUDE_ID, "cwd": "/work/app",
                "timestamp": "2026-10-14T10:00:00Z", "message": {"role": "user", "content": content}})
        };
        let reply = |id: &str, model: &str| {
            serde_json::json!({"type": "assistant", "sessionId": CLAUDE_ID, "timestamp": "2026-10-14T10:00:01Z",
                "message": {"id": id, "role": "assistant", "model": model,
                    "content": [{"type": "text", "text": "ok"}],
                    "usage": {"input_tokens": 1, "output_tokens": 1}}})
        };
        let mut lines = vec![user(serde_json::json!("fix the login"))];
        let mut meta = user(serde_json::json!("[Image: source: /tmp/1.png]"));
        meta["isMeta"] = serde_json::json!(true);
        lines.push(meta);
        // one reply streamed over three lines
        lines.extend((0..3).map(|_| reply("msg_1", "claude-opus-5-5")));
        lines.push(reply("msg_synthetic", "<synthetic>"));
        lines.push(user(serde_json::json!([
            {"type": "tool_result", "tool_use_id": "t", "content": "done"}
        ])));
        lines.extend(
            (0..8).map(|_| serde_json::json!({"type": "progress", "sessionId": CLAUDE_ID})),
        );
        lines.push(serde_json::json!({"type": "ai-title", "aiTitle": "Fix login bug", "sessionId": CLAUDE_ID}));
        lines.extend(
            (0..40).map(|_| serde_json::json!({"type": "progress", "sessionId": CLAUDE_ID})),
        );
        lines.push(user(serde_json::json!(
            "<command-name>/compact</command-name>\n<command-args></command-args>"
        )));
        let body = lines.iter().map(|l| format!("{l}\n")).collect::<String>();
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn a_listed_session_is_titled_and_counted_from_its_whole_file() {
        let sandbox = sandbox();
        long_claude_session(sandbox.home());

        let listed = list();
        let record = listed
            .iter()
            .find(|r| r.meta.session_id == CLAUDE_ID)
            .expect("listed");
        let json = serde_json::to_value(record).unwrap();

        assert_eq!(json["title"], "Fix login bug");
        assert_eq!(json["titleSource"], "agent");
        // two prompts (one a slash command) and one reply
        assert_eq!(json["messageCount"], 3);
    }

    #[test]
    fn a_listed_session_carries_what_it_spent() {
        let sandbox = sandbox();
        claude_session(sandbox.home());
        let db = crate::database::Database::memory().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, latency_ms, status_code, session_id, created_at)
                 VALUES ('r1', '_session', 'claude', 'claude-opus-5-5', 3, 5, 700, 11, '0.42', 0, 200, ?1, 1)",
                [CLAUDE_ID],
            )
            .unwrap();

        let listed = list_with(Some(&db));
        let record = listed
            .iter()
            .find(|r| r.meta.session_id == CLAUDE_ID)
            .expect("listed");
        let usage = &serde_json::to_value(record).unwrap()["usage"];
        assert_eq!(usage["models"][0]["model"], "claude-opus-5-5");
        assert_eq!(usage["models"][0]["requests"], 1);
        assert_eq!(usage["inputTokens"], 3);
        assert_eq!(usage["outputTokens"], 5);
        assert_eq!(usage["cacheReadTokens"], 700);
        assert_eq!(usage["cacheWriteTokens"], 11);
        assert_eq!(usage["totalTokens"], 719);
        assert_eq!(usage["costUsd"], 0.42);
        assert_eq!(usage["unpricedModels"], 0);
    }

    #[test]
    fn bundle_files_are_told_apart_by_what_they_hold() {
        let meta = |provider: &str, source: &str| SessionMeta {
            provider_id: provider.into(),
            source_path: Some(source.into()),
            ..Default::default()
        };
        let cases = [
            (
                "claude",
                "/c/p/s.jsonl",
                "/c/p/s.jsonl",
                Some(FileKind::Main),
            ),
            (
                "claude",
                "/c/p/s.jsonl",
                "/c/p/s/subagents/agent-1.jsonl",
                Some(FileKind::Subagent),
            ),
            ("claude", "/c/p/s.jsonl", "/c/p/s/tool-results/t.json", None),
            ("claude", "/c/p/s.jsonl", "/c/todos/s-agent.json", None),
            (
                "codex",
                "/x/rollout-b.jsonl",
                "/x/rollout-a.jsonl.zst",
                Some(FileKind::Segment),
            ),
            (
                "gemini",
                "/g/p/chats/s.jsonl",
                "/g/p/chats/s.json",
                Some(FileKind::Segment),
            ),
            (
                "gemini",
                "/g/p/chats/s.jsonl",
                "/g/p/chats/s/child.json",
                Some(FileKind::Subagent),
            ),
            (
                "gemini",
                "/g/p/chats/s.jsonl",
                "/g/p/logs/session-s.jsonl",
                None,
            ),
            (
                "dsh",
                "/d/a/session.v2.jsonl.zstd",
                "/d/a/session.jsonl",
                Some(FileKind::Segment),
            ),
            (
                "dsh",
                "/d/a/session.v2.jsonl.zstd",
                "/d/b/session.jsonl",
                Some(FileKind::Subagent),
            ),
            ("dsh", "/d/a/session.v2.jsonl.zstd", "/d/cache/a.json", None),
        ];
        for (provider, source, path, want) in cases {
            assert_eq!(
                file_kind(&meta(provider, source), Path::new(path)),
                want,
                "{path}"
            );
        }
    }

    #[test]
    fn resolve_fingerprints_the_matched_session_again_from_the_cached_listing() {
        let sandbox = sandbox();
        let (main, _, _) = claude_session(sandbox.home());
        let listed = list();
        let record = listed
            .iter()
            .find(|r| r.meta.session_id == CLAUDE_ID)
            .expect("listed");

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&main)
            .unwrap();
        std::io::Write::write_all(&mut file, b"{\"type\":\"summary\"}\n").unwrap();
        let resolved = resolve(&record.session_key).expect("resolve");
        assert_ne!(resolved.revision, record.revision);

        std::fs::remove_file(&main).unwrap();
        assert!(resolve(&record.session_key).is_err());
    }
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
        let project = temp.path().join("project ' with spaces");
        std::fs::create_dir(&project).unwrap();
        let missing = temp.path().join("missing").to_string_lossy().into_owned();
        let existing = project.to_string_lossy().into_owned();
        let id = "12345678-1234-1234-1234-123456789abc";
        for (name, cwd, allowed) in [
            ("unknown", None, false),
            ("relative", Some("."), false),
            ("missing", Some(missing.as_str()), false),
            ("existing", Some(existing.as_str()), true),
        ] {
            let path = temp.path().join(format!("rollout-{name}.jsonl"));
            let mut payload = serde_json::json!({"id": id});
            if let Some(cwd) = cwd {
                payload["cwd"] = serde_json::json!(cwd);
            }
            let record = serde_json::json!({
                "type": "session_meta",
                "timestamp": "2026-10-14T00:00:00Z",
                "payload": payload,
            });
            std::fs::write(&path, format!("{record}\n")).unwrap();
            let meta = providers::codex::parse_session(&path).unwrap();
            assert_eq!(meta.project_dir.as_deref(), cwd, "{name}");
            assert_eq!(can_resume(&meta), allowed, "{name}");
            let result = resume_record(&meta, false);
            if allowed {
                let preview = result.unwrap();
                assert!(!preview.launched);
                assert_eq!(
                    preview.command,
                    render_resume_command(
                        &["codex".into(), "resume".into(), id.into()],
                        cwd,
                        None,
                        cfg!(target_os = "windows"),
                    )
                );
            } else {
                let error = result.err().expect("Invalid project must reject resume");
                assert!(
                    error.contains("Session project directory"),
                    "{name}: {error}"
                );
            }
        }
    }
}
