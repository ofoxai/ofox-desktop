//! dsh metadata/generation/grouping adapted from Magpie internal/sessions/dsh.go.
//! Copyright (c) 2026 yetone, MIT; see ../MAGPIE_LICENSE.
//! dsh is intentionally metadata-only: no transcript or generic resume adapter.
use super::utils::{extract_text, truncate_summary};
use crate::session_manager::SessionMeta;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

pub fn home_dir() -> Result<PathBuf, String> {
    crate::native_tools::config_dir(crate::app_config::BindableTool::Dsh)
}

pub(crate) fn open(path: &Path) -> Result<Box<dyn Read>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if path.extension().is_some_and(|e| e == "zstd" || e == "zst") {
        // Decoder defaults to concatenated frames, as dsh appends a frame per batch.
        Ok(Box::new(
            zstd::stream::read::Decoder::new(file).map_err(|e| e.to_string())?,
        ))
    } else {
        Ok(Box::new(file))
    }
}

pub(crate) struct FileMeta {
    pub meta: SessionMeta,
    pub parent: Option<String>,
}

fn generation(name: &str) -> Option<u32> {
    let name = name.strip_suffix(".zstd").unwrap_or(name);
    if name == "session.jsonl" {
        return Some(0);
    }
    name.strip_prefix("session.v")?
        .strip_suffix(".jsonl")?
        .parse()
        .ok()
}

pub(crate) fn scan_files(home: &Path) -> Vec<FileMeta> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(home.join("sessions")) else {
        return out;
    };
    for project in projects
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
    {
        let Ok(dirs) = std::fs::read_dir(project.path()) else {
            continue;
        };
        for dir in dirs
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        {
            let Ok(files) = std::fs::read_dir(dir.path()) else {
                continue;
            };
            let latest = files
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter_map(|e| {
                    Some((
                        generation(&e.file_name().to_string_lossy())?,
                        e.metadata().ok()?.modified().ok()?,
                        e.path(),
                    ))
                })
                .max_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
            if let Some((_, _, path)) = latest {
                if let Some(meta) = parse_file(&path) {
                    out.push(meta);
                }
            }
        }
    }
    out
}

fn parse_file(path: &Path) -> Option<FileMeta> {
    let reader = BufReader::new(open(path).ok()?);
    let mut values = reader
        .lines()
        .map_while(Result::ok)
        .filter_map(|s| serde_json::from_str::<Value>(&s).ok());
    let head = values.next()?;
    if head["type"] != "session" {
        return None;
    }
    let id = head["id"].as_str()?.to_string();
    if id.is_empty() {
        return None;
    }
    let mut title = None;
    let mut last_active = head["createdAt"].as_i64();
    for v in values {
        if let Some(time) = v["time"].as_i64() {
            last_active = Some(last_active.unwrap_or(0).max(time));
        }
        if v["type"] == "session/title" {
            title = v["data"]["title"].as_str().map(str::to_string).or(title);
        }
        if title.is_none() && v["type"] == "user/message" && v["data"]["source"]["kind"] == "user" {
            let text = extract_text(&v["data"]["content"]);
            if !text.trim().is_empty() {
                title = Some(text);
            }
        }
    }
    Some(FileMeta {
        parent: (head["delegationDepth"].as_i64().unwrap_or(0) > 0)
            .then(|| head["parentSession"].as_str().map(str::to_string))
            .flatten(),
        meta: SessionMeta {
            provider_id: "dsh".into(),
            session_id: id,
            title: title.map(|t| truncate_summary(&t, 80)),
            summary: None,
            project_dir: head["cwd"].as_str().map(str::to_string),
            created_at: head["createdAt"].as_i64(),
            last_active_at: last_active,
            source_path: Some(path.to_string_lossy().into()),
            resume_command: None,
        },
    })
}

pub(crate) fn root_id<'a>(id: &'a str, parents: &'a HashMap<String, String>) -> Option<&'a str> {
    let mut root = id;
    let mut seen = HashSet::new();
    for _ in 0..64 {
        if !seen.insert(root) {
            return None;
        }
        let Some(parent) = parents.get(root) else {
            return Some(root);
        };
        root = parent;
    }
    None
}

pub(crate) fn grouped_files(home: &Path, session_id: &str) -> Vec<SessionMeta> {
    let files = scan_files(home);
    let parents = files
        .iter()
        .filter_map(|f| Some((f.meta.session_id.clone(), f.parent.clone()?)))
        .collect();
    files
        .into_iter()
        .filter(|f| root_id(&f.meta.session_id, &parents) == Some(session_id))
        .map(|f| f.meta)
        .collect()
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    home_dir().map(|home| scan_in(&home)).unwrap_or_default()
}

pub(crate) fn scan_in(home: &Path) -> Vec<SessionMeta> {
    let files = scan_files(home);
    let ids: HashSet<_> = files.iter().map(|f| f.meta.session_id.as_str()).collect();
    let parents = files
        .iter()
        .filter_map(|f| Some((f.meta.session_id.clone(), f.parent.clone()?)))
        .collect();
    files
        .iter()
        .filter(|f| {
            f.parent.is_none()
                || root_id(&f.meta.session_id, &parents).is_none_or(|root| !ids.contains(root))
        })
        .map(|f| {
            let mut meta = f.meta.clone();
            meta.last_active_at = files
                .iter()
                .filter(|c| {
                    root_id(&c.meta.session_id, &parents) == Some(f.meta.session_id.as_str())
                })
                .filter_map(|c| c.meta.last_active_at)
                .max()
                .or(meta.last_active_at);
            meta
        })
        .collect()
}

pub(crate) fn segment(id: &str) -> String {
    id.encode_utf16()
        .map(|unit| {
            if id != "."
                && id != ".."
                && unit < 128
                && ((unit as u8).is_ascii_alphanumeric() || matches!(unit, 46 | 95 | 45))
            {
                char::from_u32(unit as u32).unwrap().to_string()
            } else {
                format!("~{unit:04X}")
            }
        })
        .collect()
}

pub(crate) fn dedicated_dir(home: &Path, meta: &SessionMeta) -> Result<PathBuf, String> {
    let path = Path::new(meta.source_path.as_deref().ok_or("Missing dsh source")?);
    let dir = path.parent().ok_or("Missing dsh session directory")?;
    if dir.file_name().and_then(|n| n.to_str()) != Some(segment(&meta.session_id).as_str())
        || dir.parent().and_then(Path::parent) != Some(home.join("sessions").as_path())
    {
        return Err("dsh source is not in its own dedicated session directory".into());
    }
    Ok(dir.into())
}

pub(crate) fn models(path: &Path) -> Vec<String> {
    let Ok(reader) = open(path) else {
        return Vec::new();
    };
    let mut models = Vec::new();
    for line in BufReader::new(reader).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value["type"] != "assistant/message" {
            continue;
        }
        if let Some(model) = value["data"]["message"]["source"]["model"]
            .as_str()
            .filter(|m| !m.is_empty())
        {
            if !models.iter().any(|m| m == model) {
                models.push(model.to_string());
            }
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write_frames(path: &Path, lines: &[&str]) {
        let mut bytes = Vec::new();
        for line in lines {
            bytes.extend(zstd::stream::encode_all(format!("{line}\n").as_bytes(), 1).unwrap());
        }
        std::fs::write(path, bytes).unwrap();
    }
    #[test]
    fn highest_generation_multiframe_and_children_grouped() {
        let home = tempfile::tempdir().unwrap();
        for (id, parent, title) in [("root", "", "new title"), ("child", "root", "child title")] {
            let dir = home.path().join("sessions/project").join(id);
            std::fs::create_dir_all(&dir).unwrap();
            let head = format!(
                r#"{{"type":"session","version":4,"id":"{id}","cwd":"/project","createdAt":1000,"parentSession":"{parent}","delegationDepth":{}}}"#,
                if parent.is_empty() { 0 } else { 1 }
            );
            let event =
                format!(r#"{{"type":"session/title","time":3000,"data":{{"title":"{title}"}}}}"#);
            write_frames(&dir.join("session.v4.jsonl.zstd"), &[&head, &event]);
            std::fs::write(
                dir.join("session.v3.jsonl"),
                format!("{head}\n{{\"type\":\"session/title\",\"data\":{{\"title\":\"old\"}}}}\n"),
            )
            .unwrap();
        }
        let list = scan_in(home.path());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title.as_deref(), Some("new title"));
        assert_eq!(grouped_files(home.path(), "root").len(), 2);
    }
    #[test]
    fn utf16_segment_and_wrong_directory_rejection() {
        assert_eq!(segment("😀"), "~D83D~DE00");
        assert_eq!(segment("会话"), "~4F1A~8BDD");
        assert_eq!(segment(".."), "~002E~002E");
        assert_eq!(segment("a/b"), "a~002Fb");
    }
}
