//! Native-tool transactions. Only fields recorded as Ofox-owned are restored.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::config::{file_mode, FileTxn};
use crate::database::Database;
use crate::json5_sections::KeyStyle;
use crate::services::ofox_bind::{json_file, report::UnbindReport, status::ToolBindingStatus};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub(super) enum Format {
    Json,
    YamlMap,
    Patch,
    Env,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct Owned {
    pub path: Vec<String>,
    pub original: Option<Value>,
    pub written: Option<Value>,
    pub connection: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FileRecord {
    pub path: PathBuf,
    pub format: Format,
    pub original: Option<String>,
    pub last_text: String,
    #[serde(default)]
    pub input_hash: Option<String>,
    /// Planning intent is local to this transaction, never persisted.
    #[serde(skip)]
    pub pending: bool,
    pub mode: Option<u32>,
    pub secret: bool,
    pub owned: Vec<Owned>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Record {
    pub version: u32,
    pub directory: PathBuf,
    pub files: Vec<FileRecord>,
}

pub(super) fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(format!("无法读取配置文件 {}", path.display())),
    }
}

pub(super) fn load(db: &Database, tool: &str) -> Result<Option<Record>, String> {
    db.get_bind_record(tool)
        .map_err(|_| "读取原生工具绑定记录失败".to_string())?
        .map(|row| {
            serde_json::from_str::<Record>(&row.record)
                .map_err(|_| "原生工具绑定记录损坏".to_string())
                .and_then(|record| {
                    if record.version == 1 {
                        Ok(record)
                    } else {
                        Err("不支持的原生工具绑定记录版本".into())
                    }
                })
        })
        .transpose()
}

pub(super) fn get<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut cursor = value;
    for part in path {
        cursor = cursor.get(part)?;
    }
    Some(cursor)
}

pub(super) fn put(value: &mut Value, path: &[String], next: Option<Value>) {
    if path.is_empty() {
        return;
    }
    if path.len() == 1 {
        if !value.is_object() {
            *value = json!({});
        }
        if let Some(next) = next {
            value.as_object_mut().unwrap().insert(path[0].clone(), next);
        } else {
            value.as_object_mut().unwrap().remove(&path[0]);
        }
        return;
    }
    if !value.is_object() {
        *value = json!({});
    }
    if next.is_none() && value.get(&path[0]).is_none() {
        return;
    }
    let child = value
        .as_object_mut()
        .unwrap()
        .entry(path[0].clone())
        .or_insert_with(|| json!({}));
    put(child, &path[1..], next);
    if child.as_object().is_some_and(|map| map.is_empty()) {
        value.as_object_mut().unwrap().remove(&path[0]);
    }
}

pub(super) fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).into()).collect()
}

pub(super) fn parse(text: Option<&str>, format: Format) -> Result<Value, String> {
    let text = text.unwrap_or("");
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    match format {
        Format::Json => json_file::parse_object(Some(text), "原生工具")
            .map_err(|_| "原生工具配置不是有效的 JSON 对象".to_string()),
        Format::YamlMap => serde_yaml::from_str::<Value>(text)
            .map_err(|_| "凭据不是有效的 YAML".to_string())
            .and_then(|value| {
                if value.is_object() {
                    Ok(value)
                } else {
                    Err("凭据必须是 YAML 对象".into())
                }
            }),
        Format::Patch => patch_parse(text).map(|(_, entries)| {
            let mut value = json!({});
            for (_, row) in entries {
                if let Some(id) = row.get("id").and_then(Value::as_str) {
                    let mut merged = value.get(id).cloned().unwrap_or_else(|| json!({}));
                    for (key, field) in row.as_object().unwrap() {
                        merged
                            .as_object_mut()
                            .unwrap()
                            .insert(key.clone(), field.clone());
                    }
                    value.as_object_mut().unwrap().insert(id.into(), merged);
                }
            }
            value
        }),
        Format::Env => {
            let mut value = json!({});
            for line in text.lines() {
                let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
                if line.starts_with('#') {
                    continue;
                }
                if let Some((name, raw)) = line.split_once('=') {
                    let raw = raw.trim();
                    let decoded = if raw.starts_with('"') {
                        serde_json::from_str::<String>(raw)
                            .unwrap_or_else(|_| raw.trim_matches('"').into())
                    } else {
                        raw.trim_matches('\'').into()
                    };
                    value[name.trim()] = Value::String(decoded);
                }
            }
            Ok(value)
        }
    }
}

// Ported from Magpie internal/agent/dsh.go: preserve unmodified patch rows,
// comments and metadata-only home overrides; normalize flow lists only when edited.
type PatchRows = (String, Vec<(String, Value)>);
fn patch_parse(text: &str) -> Result<PatchRows, String> {
    let parsed: Value =
        serde_yaml::from_str(text).map_err(|_| "dsh patch 不是有效的 YAML 列表".to_string())?;
    let rows: Vec<Value> = if parsed.is_null() {
        vec![]
    } else {
        parsed
            .as_array()
            .cloned()
            .ok_or_else(|| "dsh patch 必须是 YAML 列表".to_string())?
    };
    if rows
        .iter()
        .any(|row| !row.is_object() || row.get("id").and_then(Value::as_str).is_none())
    {
        return Err("dsh patch 条目必须有 id".into());
    }
    let mut header = String::new();
    let mut blocks: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with("- ") || line == "-" {
            blocks.push(format!("{line}\n"));
        } else if let Some(block) = blocks.last_mut() {
            block.push_str(line);
            block.push('\n');
        } else if line.trim().is_empty() || line.trim().starts_with('#') {
            header.push_str(line);
            header.push('\n');
        }
    }
    if blocks.len() != rows.len() {
        blocks = rows.iter().map(yaml_row).collect::<Result<_, _>>()?;
    }
    Ok((header, blocks.into_iter().zip(rows).collect()))
}

fn yaml_row(value: &Value) -> Result<String, String> {
    serde_yaml::to_string(&vec![value]).map_err(|_| "序列化 dsh patch 失败".into())
}

fn yaml_row_changed(text: &str, before: &Value, after: &Value) -> Result<String, String> {
    let Some((first, rest)) = text.split_once('\n') else {
        return yaml_row(after);
    };
    let Some(first) = first.strip_prefix("- ") else {
        return yaml_row(after);
    };
    let source = format!(
        "{first}\n{}",
        rest.lines()
            .map(|l| format!("{}\n", l.strip_prefix("  ").unwrap_or(l)))
            .collect::<String>()
    );
    let rendered = yaml_map_render(&source, before, after, 0)?;
    let mut lines = rendered.lines();
    let Some(first) = lines.next() else {
        return yaml_row(after);
    };
    Ok(format!(
        "- {first}\n{}",
        lines.map(|l| format!("  {l}\n")).collect::<String>()
    ))
}

/// Credential refs are flat named strings. Keep unmodified YAML blocks,
/// including records and comments, rather than rewriting the whole store.
fn yaml_map_render(
    text: &str,
    before: &Value,
    after: &Value,
    indent: usize,
) -> Result<String, String> {
    let mut header = String::new();
    let mut blocks: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let clean = line.trim_start();
        let depth = line.len() - clean.len();
        let name = if depth == indent && !clean.starts_with('#') {
            clean
                .split_once(':')
                .and_then(|(key, _)| serde_yaml::from_str::<String>(key).ok())
        } else {
            None
        };
        if let Some(name) = name {
            blocks.push((name, format!("{line}\n")));
        } else if let Some((_, block)) = blocks.last_mut() {
            block.push_str(line);
            block.push('\n');
        } else {
            header.push_str(line);
            header.push('\n');
        }
    }
    if !before.as_object().unwrap().is_empty() && blocks.len() != before.as_object().unwrap().len()
    {
        return Err("凭据 YAML 使用了无法保留的布局，未写入".into());
    }
    let mut output = header;
    let mut seen = std::collections::HashSet::new();
    let write = |name: &str, value: &Value| -> Result<String, String> {
        let mut map = serde_json::Map::new();
        map.insert(name.into(), value.clone());
        let encoded =
            serde_yaml::to_string(&Value::Object(map)).map_err(|_| "序列化凭据失败".to_string())?;
        Ok(encoded
            .lines()
            .map(|l| format!("{}{l}\n", " ".repeat(indent)))
            .collect())
    };
    for (name, block) in blocks {
        seen.insert(name.clone());
        let old = before.get(&name);
        let next = after.get(&name);
        if old == next {
            output.push_str(&block);
            continue;
        }
        let Some(next) = next else { continue };
        let (head, body) = block.split_once('\n').unwrap_or((&block, ""));
        // A flow mapping spread over lines (`refs:` then `{ … }`) has no
        // per-key lines to keep, so that node is rewritten in block style.
        // Unbind still restores the original bytes when nothing else changed.
        let flow = body
            .lines()
            .map(str::trim_start)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .is_some_and(|l| l.starts_with('{'));
        if old.is_some_and(Value::is_object)
            && next.is_object()
            && !flow
            && head
                .split('#')
                .next()
                .unwrap_or(head)
                .trim_end()
                .ends_with(':')
        {
            output.push_str(head);
            output.push('\n');
            output.push_str(&yaml_map_render(body, old.unwrap(), next, indent + 2)?);
        } else {
            output.push_str(&write(&name, next)?);
        }
    }
    for (name, value) in after.as_object().unwrap() {
        if !seen.contains(name) {
            output.push_str(&write(name, value)?);
        }
    }
    Ok(output)
}

pub(super) fn render(
    text: &str,
    format: Format,
    before: &Value,
    after: &Value,
) -> Result<String, String> {
    if before == after {
        return Ok(text.into());
    }
    match format {
        Format::Json => json_file::render_changed(text, KeyStyle::Json, before, after),
        Format::YamlMap => yaml_map_render(text, before, after, 0),
        Format::Env => {
            let mut output = String::new();
            let mut seen = std::collections::HashSet::new();
            for line in text.lines() {
                let clean = line.trim().strip_prefix("export ").unwrap_or(line.trim());
                let name = clean.split_once('=').map(|(name, _)| name.trim());
                if let Some(name) = name.filter(|name| before.get(*name) != after.get(*name)) {
                    if seen.insert(name.to_string()) {
                        if let Some(value) = after.get(name).and_then(Value::as_str) {
                            output.push_str(&format!(
                                "{name}={}\n",
                                serde_json::to_string(value).unwrap()
                            ));
                        }
                    }
                } else {
                    output.push_str(line);
                    output.push('\n');
                }
            }
            for (name, value) in after.as_object().unwrap() {
                if before.get(name).is_none() && !seen.contains(name) {
                    output.push_str(&format!(
                        "{name}={}\n",
                        serde_json::to_string(value.as_str().unwrap_or("")).unwrap()
                    ));
                }
            }
            Ok(output)
        }
        Format::Patch => {
            let (header, mut rows) =
                patch_parse(if text.trim().is_empty() { "[]\n" } else { text })?;
            let keys: std::collections::BTreeSet<_> = before
                .as_object()
                .unwrap()
                .keys()
                .chain(after.as_object().unwrap().keys())
                .collect();
            for id in keys {
                if before.get(id) == after.get(id) {
                    continue;
                }
                let old = before.get(id);
                let new = after.get(id);
                let fields: std::collections::BTreeSet<_> = old
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|v| v.keys())
                    .chain(
                        new.and_then(Value::as_object)
                            .into_iter()
                            .flat_map(|v| v.keys()),
                    )
                    .collect();
                for field in fields {
                    if field == "id"
                        || old.and_then(|v| v.get(field)) == new.and_then(|v| v.get(field))
                    {
                        continue;
                    }
                    let at = rows.iter().rposition(|(_, row)| {
                        row.get("id").and_then(Value::as_str) == Some(id)
                            && row.get(field).is_some()
                    });
                    if let Some(at) = at {
                        let before_row = rows[at].1.clone();
                        if let Some(next) = new.and_then(|v| v.get(field)) {
                            rows[at].1[field] = next.clone();
                        } else {
                            rows[at].1.as_object_mut().unwrap().remove(field);
                        }
                        rows[at].0 = yaml_row_changed(&rows[at].0, &before_row, &rows[at].1)?;
                    } else if let Some(next) = new.and_then(|v| v.get(field)) {
                        let at = rows.iter().rposition(|(_, row)| {
                            row.get("id").and_then(Value::as_str) == Some(id)
                                && row.get("config").is_some()
                        });
                        if let Some(at) = at {
                            let before_row = rows[at].1.clone();
                            rows[at].1[field] = next.clone();
                            rows[at].0 = yaml_row_changed(&rows[at].0, &before_row, &rows[at].1)?;
                        } else {
                            let row = json!({"id":id,field:next});
                            rows.push((yaml_row(&row)?, row));
                        }
                    }
                }
            }
            // Drop only entries removed from the effective map. Metadata-only
            // user rows must survive edits to another plugin.
            rows.retain(|(_, row)| after.get(row["id"].as_str().unwrap_or("")).is_some());
            let body = rows
                .iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>();
            Ok(format!(
                "{header}{}",
                if body.is_empty() { "[]\n" } else { &body }
            ))
        }
    }
}

pub(super) fn plan(
    path: PathBuf,
    format: Format,
    secret: bool,
    changes: Vec<(Vec<String>, Option<Value>, bool)>,
    existing: Option<&FileRecord>,
) -> Result<FileRecord, String> {
    let text = read(&path)?;
    let before = parse(text.as_deref(), format)?;
    let mut after = before.clone();
    let mut record = existing.cloned().unwrap_or_else(|| FileRecord {
        path: path.clone(),
        format,
        original: text.clone(),
        last_text: String::new(),
        input_hash: None,
        pending: true,
        mode: file_mode(&path),
        secret,
        owned: vec![],
    });
    record.input_hash = Some(input_hash(&path, text.as_deref()));
    record.pending = true;
    for (path, value, connection) in changes {
        if let Some(owned) = record.owned.iter_mut().find(|owned| owned.path == path) {
            owned.written = value.clone();
        } else {
            record.owned.push(Owned {
                original: get(&before, &path).cloned(),
                written: value.clone(),
                path: path.clone(),
                connection,
            });
        }
        put(&mut after, &path, value);
    }
    // A patch entry created by Ofox includes its structural id. Record it so
    // complete removal restores an empty patch instead of leaving an id-only row.
    if format == Format::Patch {
        for id in after
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
        {
            if before.get(&id).is_none() && after.get(&id).and_then(|v| v.get("id")).is_none() {
                let path = vec![id.clone(), "id".into()];
                let value = Some(Value::String(id));
                record.owned.push(Owned {
                    path: path.clone(),
                    original: None,
                    written: value.clone(),
                    connection: false,
                });
                put(&mut after, &path, value);
            }
        }
    }
    record.last_text = render(text.as_deref().unwrap_or(""), format, &before, &after)?;
    Ok(record)
}

pub(super) fn commit(db: &Database, tool: &str, record: &Record) -> Result<(), String> {
    let mut next = record.clone();
    let mut txn = FileTxn::new();
    let result = (|| {
        for file in &mut next.files {
            let current = read(&file.path)?;
            match (file.pending, current) {
                (false, Some(current)) => {
                    // A model or endpoint edit must never rewrite unrelated files
                    // from an old snapshot (CLI edits/extensions/credential refs).
                    file.last_text = current;
                }
                (_, current) => {
                    if file.input_hash.as_ref().is_some_and(|expected| {
                        *expected != input_hash(&file.path, current.as_deref())
                    }) {
                        return Err("配置在规划后已改变，请刷新后重试".to_string());
                    }
                    txn.write(&file.path, file.last_text.as_bytes())
                        .map_err(|_| format!("写入配置失败 {}", file.path.display()))?;
                    if file.secret {
                        txn.set_mode(&file.path, 0o600)
                            .map_err(|_| "设置凭据文件权限失败".to_string())?;
                    }
                }
            }
            file.input_hash = Some(input_hash(&file.path, Some(&file.last_text)));
        }
        let serialized =
            serde_json::to_string(&next).map_err(|_| "保存绑定记录失败".to_string())?;
        db.upsert_bind_record(tool, &serialized)
            .map_err(|_| "保存绑定记录失败".to_string())
    })();
    if let Err(error) = result {
        return Err(match txn.rollback() {
            Ok(()) => error,
            Err(_) => format!("{error}；配置回滚失败"),
        });
    }
    crate::model_selection::record_write_receipt(db, tool);
    Ok(())
}

fn input_hash(path: &Path, text: Option<&str>) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&json!({"text":text,"mode":file_mode(path)})).unwrap())
    )
}

pub(super) fn replan_pending(
    file: &FileRecord,
    changes: Vec<(Vec<String>, Option<Value>, bool)>,
) -> Result<FileRecord, String> {
    let before = parse(Some(&file.last_text), file.format)?;
    let mut after = before.clone();
    let mut next = file.clone();
    next.pending = true;
    for (path, value, connection) in changes {
        if let Some(owned) = next.owned.iter_mut().find(|o| o.path == path) {
            owned.written = value.clone();
        } else {
            next.owned.push(Owned {
                path: path.clone(),
                original: get(&before, &path).cloned(),
                written: value.clone(),
                connection,
            });
        }
        put(&mut after, &path, value);
    }
    next.last_text = render(&file.last_text, file.format, &before, &after)?;
    Ok(next)
}

pub(super) fn status(record: &Record, directory: &Path) -> Result<ToolBindingStatus, String> {
    if record.directory != directory {
        return Ok(ToolBindingStatus::modified(vec![
            "configuration-directory".into()
        ]));
    }
    let mut missing = Vec::new();
    let mut changed = Vec::new();
    for file in &record.files {
        let text = read(&file.path)?;
        if text.is_none() {
            missing.push(file.path.display().to_string());
            continue;
        }
        if file.secret && file_mode(&file.path).is_some_and(|mode| mode & 0o777 != 0o600) {
            changed.push(format!("{} · permissions", file.path.display()));
        }
        let value = parse(text.as_deref(), file.format)?;
        for owned in &file.owned {
            let now = get(&value, &owned.path);
            if now != owned.written.as_ref() {
                if now.is_none() && owned.written.is_some() {
                    missing.push(file.path.display().to_string());
                } else if owned.connection {
                    changed.push(format!(
                        "{} · {}",
                        file.path.display(),
                        owned.path.join(".")
                    ));
                }
            }
        }
    }
    if !changed.is_empty() {
        Ok(ToolBindingStatus::modified(changed))
    } else if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        Ok(ToolBindingStatus::missing(missing))
    } else {
        Ok(ToolBindingStatus::configured())
    }
}

pub(super) fn revision(record: &Record) -> Result<Value, String> {
    let files=record.files.iter().map(|file|Ok(json!({"path":file.path,"mode":file_mode(&file.path),"digest":format!("{:x}",Sha256::digest(read(&file.path)?.unwrap_or_default().as_bytes()))}))).collect::<Result<Vec<_>,String>>()?;
    Ok(json!({"directory":record.directory,"files":files}))
}

pub(super) fn unbind(
    db: &Database,
    tool: &str,
    record: &Record,
    dry_run: bool,
) -> Result<UnbindReport, String> {
    let mut report = UnbindReport::new(tool, dry_run);
    let mut txn = FileTxn::new();
    let result = (|| {
        for file in &record.files {
            let Some(text) = read(&file.path)? else {
                report.warn(
                    "configAlreadyMissing",
                    Some(file.path.display().to_string()),
                );
                continue;
            };
            let before = parse(Some(&text), file.format)?;
            let mut after = before.clone();
            for owned in &file.owned {
                put(&mut after, &owned.path, owned.original.clone());
                if owned.original.is_some() {
                    report.restored_keys.push(format!(
                        "{} · {}",
                        file.path.display(),
                        owned.path.join(".")
                    ));
                } else {
                    report.removed_keys.push(format!(
                        "{} · {}",
                        file.path.display(),
                        owned.path.join(".")
                    ));
                }
            }
            let exact = parse(file.original.as_deref(), file.format)? == after;
            let content = if exact {
                file.original.clone()
            } else if after.as_object().is_some_and(|v| v.is_empty()) && file.original.is_none() {
                None
            } else {
                Some(render(&text, file.format, &before, &after)?)
            };
            if exact {
                report.exact_files.push(file.path.display().to_string());
            }
            match content {
                Some(content) => {
                    if !dry_run {
                        txn.write(&file.path, content.as_bytes())
                            .map_err(|_| "还原配置失败".to_string())?;
                        if let Some(mode) = file.mode {
                            txn.set_mode(&file.path, mode)
                                .map_err(|_| "还原权限失败".to_string())?;
                        }
                    }
                }
                None => {
                    report.files_removed.push(file.path.display().to_string());
                    if !dry_run {
                        txn.remove(&file.path)
                            .map_err(|_| "移除受管文件失败".to_string())?;
                    }
                }
            }
        }
        if !dry_run {
            db.delete_bind_record(tool)
                .map_err(|_| "移除绑定记录失败".to_string())?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        return Err(match txn.rollback() {
            Ok(()) => error,
            Err(_) => format!("{error}；配置回滚失败"),
        });
    }
    Ok(report)
}
