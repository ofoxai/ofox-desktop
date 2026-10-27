use super::types::*;
use crate::database::Database;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub fn ensure_new_operation(detail: &Detail, operation_id: &str) -> Result<(), String> {
    if let Some(operation) = detail
        .operations
        .iter()
        .find(|operation| operation["id"] == operation_id)
    {
        return Err(if operation["status"] == "cancelled" {
            "assistant_cancelled"
        } else {
            "assistant_operation_already_submitted"
        }
        .into());
    }
    Ok(())
}

pub fn consume_draft(detail: &mut Detail, request: &SendRequest) {
    if request.edit_message_id.is_none() && request.regenerate_message_id.is_none() {
        detail.session.draft.clear();
        detail.session.draft_attachment_ids.clear();
    }
}

pub fn readable_export(detail: Detail) -> Result<Value, String> {
    let mut value = serde_json::to_value(detail).map_err(db_error)?;
    value["session"]["permissionMode"] = json!("ask");
    value["session"]["activeRun"] = Value::Null;
    value["pendingApproval"] = Value::Null;
    value
        .as_object_mut()
        .ok_or("assistant_export_failed")?
        .remove("memories");
    scrub_value(&mut value, &[]);
    Ok(value)
}

pub fn namespace(user: &str, org: &str) -> Result<String, String> {
    if user.is_empty() || org.is_empty() {
        return Err("assistant_identity_unavailable".into());
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(format!("{user}\0{org}").as_bytes())
    ))
}
fn prefix(owner: &str) -> String {
    format!("assistant_v1:{owner}:")
}
fn key(owner: &str, sid: &str) -> Result<String, String> {
    validate_id(sid)?;
    Ok(format!("{}{sid}", prefix(owner)))
}
fn db_error(error: impl std::fmt::Display) -> String {
    format!("assistant_storage: {error}")
}

pub fn create(
    db: &Database,
    owner: &str,
    kind: AssistantKind,
    model: &str,
    web: bool,
) -> Result<Detail, String> {
    let at = now();
    let detail = Detail {
        pending_approval: None,
        session: Session {
            id: id(),
            kind,
            title: String::new(),
            model_id: model.into(),
            permission_mode: PermissionMode::Ask,
            web_search: web,
            created_at: at,
            updated_at: at,
            deleted_at: None,
            active_run: None,
            draft: String::new(),
            draft_attachment_ids: vec![],
        },
        messages: vec![],
        memories: memories(db, owner, kind)?,
        attachments: vec![],
        usage: json!({}),
        operations: vec![],
    };
    let record_key = key(owner, &detail.session.id)?;
    let raw = serde_json::to_string(&detail).map_err(db_error)?;
    super::local_data::set(db, &record_key, &raw)?;
    Ok(detail)
}

pub fn remember_model(
    db: &Database,
    owner: &str,
    kind: AssistantKind,
    model: &str,
) -> Result<(), String> {
    super::local_data::set(db, &format!("assistant_model_v1:{owner}:{kind:?}"), model)
}
pub fn remembered_model(
    db: &Database,
    owner: &str,
    kind: AssistantKind,
) -> Result<Option<String>, String> {
    db.get_setting(&format!("assistant_model_v1:{owner}:{kind:?}"))
        .map_err(db_error)
}

fn decode(raw: &str, sid: &str) -> Result<Detail, String> {
    let detail: Detail = serde_json::from_str(raw).map_err(db_error)?;
    if detail.session.id != sid || !detail.usage.is_object() {
        return Err("assistant_invalid_session".into());
    }
    Ok(detail)
}

pub fn get(db: &Database, owner: &str, sid: &str) -> Result<Detail, String> {
    let raw = db
        .get_setting(&key(owner, sid)?)
        .map_err(db_error)?
        .ok_or("assistant_session_not_found")?;
    let mut detail = decode(&raw, sid)?;
    if detail.session.id != sid {
        return Err("assistant_invalid_session".into());
    }
    detail.memories = memories(db, owner, detail.session.kind)?;
    Ok(detail)
}

pub fn list(
    db: &Database,
    owner: &str,
    kind: AssistantKind,
    deleted: bool,
    search: &str,
) -> Result<Vec<Session>, String> {
    let conn = db.conn.lock().map_err(db_error)?;
    super::local_data::ensure_passive(&conn)?;
    let mut stmt = conn
        .prepare("SELECT value FROM settings WHERE key LIKE ?1 ORDER BY key")
        .map_err(db_error)?;
    let rows = stmt
        .query_map(params![format!("{}%", prefix(owner))], |row| {
            row.get::<_, String>(0)
        })
        .map_err(db_error)?;
    let mut result = Vec::new();
    let search = search.to_lowercase();
    for raw in rows {
        let detail: Detail = serde_json::from_str(&raw.map_err(db_error)?).map_err(db_error)?;
        if detail.session.kind != kind || detail.session.deleted_at.is_some() != deleted {
            continue;
        }
        if !search.is_empty()
            && !detail.session.title.to_lowercase().contains(&search)
            && !detail
                .messages
                .iter()
                .any(|m| m.content.to_lowercase().contains(&search))
        {
            continue;
        }
        result.push(detail.session);
    }
    result.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    Ok(result)
}

/// Serialize read/modify/write under the same connection lock. No async work or
/// user configuration writes may run inside this critical section.
pub fn mutate<T>(
    db: &Database,
    owner: &str,
    sid: &str,
    change: impl FnOnce(&mut Detail) -> Result<T, String>,
) -> Result<T, String> {
    let key = key(owner, sid)?;
    let conn = db.conn.lock().map_err(db_error)?;
    super::local_data::ensure_passive(&conn)?;
    let raw: String = conn
        .query_row("SELECT value FROM settings WHERE key=?1", [&key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(db_error)?
        .ok_or("assistant_session_not_found")?;
    let mut detail = decode(&raw, sid)?;
    let result = change(&mut detail)?;
    let raw = serde_json::to_string(&detail).map_err(db_error)?;
    super::local_data::write(|| {
        conn.execute(
            "UPDATE settings SET value=?1 WHERE key=?2",
            params![raw, key],
        )
    })
    .map_err(db_error)?;
    Ok(result)
}

pub fn usage(db: &Database, owner: &str) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(db_error)?;
    super::local_data::ensure_passive(&conn)?;
    let mut query = conn
        .prepare("SELECT value FROM settings WHERE substr(key,1,length(?1))=?1")
        .map_err(db_error)?;
    let rows = query
        .query_map([prefix(owner)], |row| row.get::<_, String>(0))
        .map_err(db_error)?;
    let mut sessions = 0u64;
    let mut runs = 0u64;
    let mut input = 0u64;
    let mut output = 0u64;
    let mut cache_read = 0u64;
    let mut cache_write = 0u64;
    let mut searches = 0u64;
    let mut estimate = rust_decimal::Decimal::ZERO;
    let mut search_estimate = rust_decimal::Decimal::ZERO;
    let mut priced = true;
    for raw in rows {
        let detail: Detail = serde_json::from_str(&raw.map_err(db_error)?).map_err(db_error)?;
        sessions += 1;
        let interrupted = detail.session.active_run.is_some()
            || detail.messages.iter().any(|m| m.status == "streaming")
            || detail.operations.iter().any(|op| op["status"] == "running");
        let u = detail.usage;
        if !u.is_object() {
            return Err("assistant_invalid_session".into());
        }
        if interrupted || u["usageCoverage"] == "incomplete" {
            priced = false;
        }
        let requests = u["modelRequests"].as_u64().unwrap_or(0);
        runs += requests;
        input += u["inputTokens"].as_u64().unwrap_or(0);
        output += u["outputTokens"].as_u64().unwrap_or(0);
        cache_read += u["cacheReadTokens"].as_u64().unwrap_or(0);
        cache_write += u["cacheWriteTokens"].as_u64().unwrap_or(0);
        searches += u["searchCount"].as_u64().unwrap_or(0);
        let value = if let Some(v) = u["estimatedCostUsd"].as_str() {
            v.to_owned()
        } else {
            u["estimatedCostUsd"].to_string()
        };
        if let Ok(value) = value.parse::<rust_decimal::Decimal>() {
            estimate += value;
        } else if requests > 0 {
            priced = false;
        }
        if let Some(value) = u["searchEstimatedCostUsd"]
            .as_str()
            .and_then(|v| v.parse::<rust_decimal::Decimal>().ok())
        {
            search_estimate += value;
        }
    }
    Ok(
        json!({"sessionCount":sessions,"runCount":runs,"inputTokens":input,"outputTokens":output,"cacheReadTokens":cache_read,"cacheWriteTokens":cache_write,"estimatedCostUsd":if priced {Some(estimate.normalize().to_string())} else {None},"billedCostUsd":null,"searchCount":searches,"usageCoverage":if priced {"complete"} else {"incomplete"},"searchEstimatedCostUsd":if priced {Some(search_estimate.normalize().to_string())} else {None}}),
    )
}

pub fn purge(db: &Database, owner: &str, sid: &str) -> Result<(), String> {
    let detail = get(db, owner, sid)?;
    if detail.session.deleted_at.is_none() {
        return Err("assistant_trash_required".into());
    }
    let conn = db.conn.lock().map_err(db_error)?;
    super::local_data::ensure_passive(&conn)?;
    let key = key(owner, sid)?;
    super::local_data::write(|| conn.execute("DELETE FROM settings WHERE key=?1", [key]))
        .map_err(db_error)?;
    Ok(())
}

fn memory_key(owner: &str, kind: AssistantKind) -> String {
    format!("assistant_memory_v1:{owner}:{kind:?}")
}
pub fn memories(db: &Database, owner: &str, kind: AssistantKind) -> Result<Vec<Memory>, String> {
    db.get_setting(&memory_key(owner, kind))
        .map_err(db_error)?
        .map(|s| serde_json::from_str(&s).map_err(db_error))
        .transpose()
        .map(Option::unwrap_or_default)
}
pub fn save_memory(
    db: &Database,
    owner: &str,
    kind: AssistantKind,
    mid: Option<&str>,
    content: &str,
) -> Result<Memory, String> {
    if content.trim().is_empty() || content.len() > 16_000 {
        return Err("assistant_invalid_memory".into());
    }
    let key = memory_key(owner, kind);
    let conn = db.conn.lock().map_err(db_error)?;
    super::local_data::ensure_passive(&conn)?;
    let raw: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key=?1", [&key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(db_error)?;
    let mut items: Vec<Memory> = raw
        .map(|v| serde_json::from_str(&v).map_err(db_error))
        .transpose()?
        .unwrap_or_default();
    let memory = Memory {
        id: mid.map(str::to_owned).unwrap_or_else(id),
        content: crate::redact::scrub(content).into_owned(),
        updated_at: now(),
    };
    validate_id(&memory.id)?;
    if let Some(item) = items.iter_mut().find(|m| m.id == memory.id) {
        *item = memory.clone();
    } else {
        items.push(memory.clone());
    }
    let raw = serde_json::to_string(&items).map_err(db_error)?;
    super::local_data::write(|| {
        conn.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
            params![key, raw],
        )
    })
    .map_err(db_error)?;
    Ok(memory)
}
pub fn delete_memory(
    db: &Database,
    owner: &str,
    kind: AssistantKind,
    mid: &str,
) -> Result<(), String> {
    validate_id(mid)?;
    let key = memory_key(owner, kind);
    let conn = db.conn.lock().map_err(db_error)?;
    super::local_data::ensure_passive(&conn)?;
    let raw: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key=?1", [&key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(db_error)?;
    let mut items: Vec<Memory> = raw
        .map(|v| serde_json::from_str(&v).map_err(db_error))
        .transpose()?
        .unwrap_or_default();
    items.retain(|m| m.id != mid);
    let raw = serde_json::to_string(&items).map_err(db_error)?;
    super::local_data::write(|| {
        conn.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
            params![key, raw],
        )
    })
    .map_err(db_error)?;
    Ok(())
}

pub fn scrub_value(value: &mut Value, secrets: &[&str]) {
    match value {
        Value::String(text) => {
            for secret in secrets.iter().filter(|s| s.len() >= 8) {
                *text = text.replace(secret, crate::redact::REDACTED);
            }
            *text = crate::redact::scrub(text).into_owned();
        }
        Value::Array(items) => {
            for item in items {
                scrub_value(item, secrets);
            }
        }
        Value::Object(map) => {
            for (name, item) in map {
                let name = name.to_ascii_lowercase().replace(['_', '-'], "");
                if [
                    "apikey",
                    "accesstoken",
                    "refreshtoken",
                    "authorization",
                    "password",
                    "clientsecret",
                ]
                .contains(&name.as_str())
                {
                    *item = Value::String(crate::redact::REDACTED.into());
                } else {
                    scrub_value(item, secrets);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_a_new_user_turn_consumes_the_composer_draft() {
        for (edit, regenerate) in [
            (None, None),
            (Some("edit"), None),
            (None, Some("regenerate")),
        ] {
            let db = Database::memory().unwrap();
            let owner = namespace("a", "b").unwrap();
            let mut detail =
                create(&db, &owner, AssistantKind::General, "test/model", false).unwrap();
            detail.session.draft = "Unsent text".into();
            detail.session.draft_attachment_ids = vec!["unsent-attachment".into()];
            let request = SendRequest {
                session_id: detail.session.id.clone(),
                run_id: id(),
                operation_id: id(),
                text: "New text".into(),
                attachment_ids: vec![],
                edit_message_id: edit.map(str::to_owned),
                regenerate_message_id: regenerate.map(str::to_owned),
            };
            consume_draft(&mut detail, &request);
            let revision = edit.is_some() || regenerate.is_some();
            assert_eq!(
                detail.session.draft,
                if revision { "Unsent text" } else { "" }
            );
            assert_eq!(
                detail.session.draft_attachment_ids.len(),
                usize::from(revision)
            );
        }
    }
    #[test]
    fn readable_export_excludes_unrelated_assistant_memory() {
        let db = Database::memory().unwrap();
        let owner = namespace("a", "b").unwrap();
        save_memory(
            &db,
            &owner,
            AssistantKind::General,
            None,
            "unrelated-private-memory",
        )
        .unwrap();
        let detail = create(&db, &owner, AssistantKind::General, "test/model", false).unwrap();
        assert_eq!(detail.memories.len(), 1);
        let exported = readable_export(detail).unwrap();
        assert!(exported.get("memories").is_none());
        assert!(!exported.to_string().contains("unrelated-private-memory"));
        assert_eq!(
            memories(&db, &owner, AssistantKind::General).unwrap().len(),
            1
        );
    }

    #[test]
    fn persisted_conversation_keeps_attachment_draft_references() {
        let db = Database::memory().unwrap();
        let owner = namespace("a", "b").unwrap();
        let detail = create(&db, &owner, AssistantKind::General, "test/model", false).unwrap();
        let mut saved = serde_json::to_value(&detail).unwrap();
        saved["session"]
            .as_object_mut()
            .unwrap()
            .remove("draftAttachmentIds");
        assert!(serde_json::from_value::<Detail>(saved.clone())
            .unwrap()
            .session
            .draft_attachment_ids
            .is_empty());
        saved["session"]["draftAttachmentIds"] = json!(["attachment-id"]);
        let reloaded: Detail = serde_json::from_value(saved).unwrap();
        assert_eq!(
            serde_json::to_value(reloaded).unwrap()["session"]["draftAttachmentIds"],
            json!(["attachment-id"])
        );
    }
    #[test]
    fn unknown_usage_cannot_become_a_zero_price_in_the_account_summary() {
        let db = Database::memory().unwrap();
        let owner = namespace("a", "b").unwrap();
        let d = create(&db, &owner, AssistantKind::General, "test/model", false).unwrap();
        mutate(&db, &owner, &d.session.id, |d| {
            d.usage = json!({"usageCoverage":"incomplete","modelRequests":0});
            Ok(())
        })
        .unwrap();
        assert!(usage(&db, &owner).unwrap()["estimatedCostUsd"].is_null());
        assert_eq!(
            usage(&db, &namespace("a", "other-org").unwrap()).unwrap()["sessionCount"],
            0
        );
    }

    #[test]
    fn account_and_organization_cannot_read_each_others_sessions_or_memories() {
        let db = Database::memory().unwrap();
        let a = namespace("alice", "one").unwrap();
        let b = namespace("alice", "two").unwrap();
        let detail = create(&db, &a, AssistantKind::General, "test/model", false).unwrap();
        assert!(get(&db, &b, &detail.session.id).is_err());
        save_memory(&db, &a, AssistantKind::General, None, "use concise replies").unwrap();
        assert!(memories(&db, &b, AssistantKind::General)
            .unwrap()
            .is_empty());
        assert!(memories(&db, &a, AssistantKind::Configuration)
            .unwrap()
            .is_empty());
    }
    #[test]
    fn malformed_saved_history_is_not_an_empty_history() {
        let db = Database::memory().unwrap();
        let owner = namespace("a", "b").unwrap();
        let sid = id();
        db.set_setting(&key(&owner, &sid).unwrap(), "damaged")
            .unwrap();
        assert!(get(&db, &owner, &sid).is_err());
        assert!(mutate(&db, &owner, &sid, |d| {
            d.messages.clear();
            Ok(())
        })
        .is_err());
        assert_eq!(
            db.get_setting(&key(&owner, &sid).unwrap())
                .unwrap()
                .as_deref(),
            Some("damaged")
        );
    }
    #[test]
    fn secret_fields_and_exact_credentials_are_removed_recursively() {
        let mut v = json!({"apiKey":"not-a-recognized-pattern", "nested":[{"text":"opaque-test-credential"}],"count":2});
        scrub_value(&mut v, &["opaque-test-credential"]);
        assert!(!v.to_string().contains("opaque-test-credential"));
        assert!(!v.to_string().contains("not-a-recognized-pattern"));
        assert_eq!(v["count"], 2);
    }
}
