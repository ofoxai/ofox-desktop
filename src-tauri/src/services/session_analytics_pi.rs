//! Pi active conversation and all-branch spending use separate projections.
use super::{
    session_analytics_core::{MessageEvent, Role},
    session_analytics_index::{key, ParsedSource},
    session_analytics_native as native, session_usage_pi as reader, usage_source,
};
use crate::{
    database::Database,
    error::AppError,
    session_manager::{
        providers::{extract_text, pi},
        title::{self, TitleCandidates},
    },
};
use serde_json::Value;
use std::io::Read;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub(crate) fn stamp(roots: &[PathBuf]) -> Result<Value, AppError> {
    let files = pi::usage_files(roots).map_err(|_| invalid())?;
    native::roster_stamp(roots, &files)
}
pub(crate) fn read(db: &Database, roots: &[PathBuf]) -> Result<ParsedSource, AppError> {
    let before = stamp(roots)?;
    let roster = reader::read_analytics_roster(db, roots)?;
    let mut result = native::empty();
    if roots.iter().any(|root| {
        std::fs::symlink_metadata(root)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    }) {
        result.reasons.push("sourceMissing".into());
    }
    let mut sessions = BTreeMap::new();
    let mut facts = BTreeMap::new();
    let mut messages = BTreeMap::new();
    let mut native_streams = BTreeMap::new();
    let mut activity_entries = 0usize;
    let mut projected_entries = 0usize;
    for (path, read) in roster {
        let Some(sid) = read.session_id.as_deref() else {
            result.reasons.push("sessionIdentityUnavailable".into());
            continue;
        };
        // Display and accounting projections share native source validation.
        let text = usage_source::with_stable_regular_file(&path, |file| {
            let mut bytes = Vec::new();
            file.take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| invalid())?;
            if bytes.len() > 64 * 1024 * 1024 {
                return Err(invalid());
            }
            String::from_utf8(bytes).map_err(|_| invalid())
        })?;
        // An unsupported header makes the usage reader stop before counting
        // its suffix. Bound this independent projection as it is decoded too,
        // across the complete roster, with one header excluded per file.
        let mut entries = Vec::new();
        for entry in text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        {
            if !entries.is_empty() {
                projected_entries = projected_entries.checked_add(1).ok_or_else(invalid)?;
                if projected_entries > 100_000 {
                    return Err(invalid());
                }
            }
            entries.push(entry);
        }
        let head = entries
            .iter()
            .find(|entry| entry["type"] == "session")
            .ok_or_else(invalid)?;
        let project = match head.get("cwd") {
            Some(Value::String(path)) => Some((!path.trim().is_empty()).then_some(path.as_str())),
            Some(Value::Null) => Some(None),
            _ => None,
        };
        let branch = pi::active_branch(&entries);
        let first_prompt = branch.iter().find_map(|entry| {
            (entry["message"]["role"] == "user").then(|| extract_text(&entry["message"]["content"]))
        });
        let mut candidates = TitleCandidates::default()
            .with_first_user_text(first_prompt.as_deref())
            .with_directory(project.flatten());
        candidates.custom = branch.iter().rev().find_map(|entry| {
            (entry["type"] == "session_info")
                .then(|| entry["name"].as_str())
                .flatten()
                .map(str::to_owned)
        });
        let name = title::pick_title(&candidates)
            .map(|(name, _)| name)
            .unwrap_or_else(|| sid.into());
        // A fork is a separate conversation, not a subagent root rollup.
        let mut descriptor = native::descriptor("pi", sid, &name, project, None, path.to_str());
        descriptor
            .message_reasons
            .extend(read.native_activity_reasons.iter().cloned());
        result
            .reasons
            .extend(read.native_activity_reasons.iter().cloned());
        let ownership_unknown = read.reasons.iter().any(|reason| {
            matches!(
                reason.as_str(),
                "parentSourceUnavailable"
                    | "forkBoundaryUnavailable"
                    | "parentCycleOrDepthLimit"
                    | "unsupportedSchemaVersion"
                    | "sessionHeaderUnsupported"
            )
        });
        if ownership_unknown {
            descriptor.messages_supported = false;
            descriptor
                .message_reasons
                .push("sessionParentUnavailable".into());
        }
        if project.is_none() {
            descriptor.message_reasons.push("projectNotRecorded".into());
        }
        if read.reasons.contains("incompleteEnumeration") {
            descriptor.message_reasons.push("incompleteSource".into());
        }
        let mut tool_scan = super::session_analytics::interactions::CallScan::default();
        if !ownership_unknown {
            let inherited = read
                .inherited_entries
                .iter()
                .map(|id| native::activity_key("pi", &serde_json::json!(["entry", id]).to_string()))
                .collect::<BTreeSet<_>>();
            let events = read
                .native_activity
                .into_iter()
                .filter(|event| !inherited.contains(&event.event_key))
                .collect();
            activity_entries = activity_entries
                .checked_add(native::merge_activity_stream(
                    &mut native_streams,
                    key("pi", sid),
                    events,
                )?)
                .ok_or_else(invalid)?;
            if activity_entries > 100_000 {
                return Err(invalid());
            }
            for entry in branch {
                if entry["type"] != "message" {
                    continue;
                }
                let Some(id) = entry["id"].as_str() else {
                    descriptor
                        .message_reasons
                        .push("messageNativeIdentityUnavailable".into());
                    continue;
                };
                if read.inherited_entries.contains(id) {
                    continue;
                }
                if entry["message"]["role"] == "assistant" {
                    let at = entry
                        .get("timestamp")
                        .and_then(crate::session_manager::providers::parse_timestamp_to_ms);
                    tool_scan.blocks(entry["message"].get("content"), at);
                }
                let role = match entry["message"]["role"].as_str() {
                    Some("user") => Role::User,
                    Some("assistant") => Role::Assistant,
                    _ => continue,
                };
                if !super::session_analytics_messages::visible(
                    entry["message"].get("content"),
                    if role == Role::User {
                        "user"
                    } else {
                        "assistant"
                    },
                ) {
                    continue;
                }
                let Some(at_ms) = reader::native_time(&entry["message"], entry) else {
                    descriptor
                        .message_reasons
                        .push("messageTimeUnavailable".into());
                    continue;
                };
                let event = MessageEvent {
                    session_key: key("pi", sid),
                    native_id: format!(
                        "{}:{id}",
                        if role == Role::User {
                            "user"
                        } else {
                            "assistant"
                        }
                    ),
                    at_ms,
                    role,
                    main: true,
                };
                let event_key = (event.session_key.clone(), event.native_id.clone());
                if messages
                    .get(&event_key)
                    .is_some_and(|prior| prior != &event)
                {
                    return Err(invalid());
                }
                messages.insert(event_key, event);
            }
        }
        if !ownership_unknown {
            result
                .tool_evidence
                .attach(&descriptor.identity.key, &tool_scan);
        }
        descriptor.message_reasons.sort();
        descriptor.message_reasons.dedup();
        if let Some(prior) = sessions.get(sid) {
            let prior: &super::session_analytics::SessionDescriptor = prior;
            if prior.identity != descriptor.identity
                || prior.title != descriptor.title
                || prior.message_reasons != descriptor.message_reasons
            {
                return Err(invalid());
            }
        } else {
            sessions.insert(sid.to_owned(), descriptor);
        }
        let complete = read.reasons.is_empty();
        result.reasons.extend(read.reasons);
        for fact in read.facts {
            let Some(fact) = native::normalized_fact("pi", "pi_session", fact, complete) else {
                continue;
            };
            if facts
                .get(&fact.event_key)
                .is_some_and(|prior| prior != &fact)
            {
                return Err(invalid());
            }
            facts.insert(fact.event_key.clone(), fact);
        }
    }
    if stamp(roots)? != before {
        return Err(invalid());
    }
    result.sessions = sessions.into_values().collect();
    result.facts = facts.into_values().collect();
    result.messages = messages.into_values().collect();
    result.native_activity = native_streams.into_values().flatten().collect();
    result.reasons.sort();
    result.reasons.dedup();
    Ok(result)
}
fn invalid() -> AppError {
    AppError::InvalidInput("usage.analytics.sourceReadFailed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn rows(sid: &str) -> Vec<Value> {
        vec![
            json!({"type":"session","version":3,"id":sid,"cwd":"/fixture/project","timestamp":"2026-10-15T00:00:00Z"}),
            json!({"type":"message","id":"u","parentId":null,"timestamp":"2026-10-15T00:00:00Z","message":{"role":"user","timestamp":1792022400000_i64,"content":"synthetic question"}}),
            json!({"type":"message","id":"a","parentId":"u","timestamp":"2026-10-15T00:00:01Z","message":{"role":"assistant","timestamp":1792022401000_i64,"content":"private reply","model":"fixture","usage":{"input":10,"output":2,"cacheRead":0,"cacheWrite":0}}}),
        ]
    }
    fn write(root: &std::path::Path, name: &str, rows: &[Value]) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            &path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();
        path
    }
    #[test]
    fn analytics_pi_fork_spending_and_activity_exclude_verified_copied_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sessions");
        let parent = write(&root, "parent.jsonl", &rows("parent"));
        let mut child = rows("child");
        child[0]["parentSession"] = json!(parent);
        let mut new = child[2].clone();
        new["id"] = json!("new");
        new["parentId"] = json!("a");
        new["message"]["usage"]["input"] = json!(5);
        child.push(new);
        write(&root, "child.jsonl", &child);
        let db = Database::memory().unwrap();
        let parsed = read(&db, std::slice::from_ref(&root)).unwrap();
        assert_eq!(parsed.sessions.len(), 2);
        assert_eq!(
            parsed.facts.iter().map(|fact| fact.buckets[0]).sum::<u64>(),
            15
        );
        assert_eq!(
            parsed
                .messages
                .iter()
                .filter(|message| message.session_key == key("pi", "child"))
                .count(),
            1
        );
        assert!(parsed
            .sessions
            .iter()
            .all(|session| session.identity.verified_root_key.is_none()));
        assert_eq!(
            parsed
                .native_activity
                .iter()
                .filter(|event| event.session_key == key("pi", "child"))
                .count(),
            2
        );
        std::fs::remove_file(parent).unwrap();
        let missing = read(&db, &[root]).unwrap();
        assert!(missing.facts.is_empty());
        assert!(missing.native_activity.is_empty());
        assert!(!missing.sessions[0].messages_supported);
    }

    #[test]
    fn native_runtime_pi_uses_all_owned_runtime_but_visible_messages_keep_active_branch() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sessions");
        let mut entries = rows("s");
        let mut abandoned = entries[2].clone();
        abandoned["id"] = json!("abandoned");
        abandoned["message"]["timestamp"] = json!(1792022402000_i64);
        abandoned["timestamp"] = json!("2026-10-15T00:00:02Z");
        entries.push(abandoned);
        let mut active = entries[2].clone();
        active["id"] = json!("active");
        active["message"]["timestamp"] = json!(1792022403000_i64);
        active["timestamp"] = json!("2026-10-15T00:00:03Z");
        entries.push(active);
        write(&root, "s.jsonl", &entries);
        let parsed = read(&Database::memory().unwrap(), &[root]).unwrap();
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.native_activity.len(), 5);
        assert_eq!(
            parsed
                .native_activity
                .iter()
                .map(|event| event.at_ms)
                .collect::<Vec<_>>(),
            [
                1792022400000,
                1792022400000,
                1792022401000,
                1792022402000,
                1792022403000
            ]
        );
        assert!(!serde_json::to_string(&parsed.native_activity)
            .unwrap()
            .contains("private reply"));
    }
    #[test]
    fn analytics_pi_conflicting_replica_body_never_becomes_a_successful_projection() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sessions");
        let original = rows("s");
        write(&root, "one.jsonl", &original);
        let mut conflicting = original;
        conflicting[2]["message"]["content"] = json!("different private reply");
        write(&root, "copy.jsonl", &conflicting);
        assert!(read(&Database::memory().unwrap(), &[root]).is_err());
    }
    #[test]
    fn analytics_pi_unsupported_header_cannot_bypass_projection_entry_budget() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sessions");
        std::fs::create_dir_all(&root).unwrap();
        let file = std::fs::File::create(root.join("unsupported.jsonl")).unwrap();
        let mut file = std::io::BufWriter::new(file);
        writeln!(file, "{}", json!({"type":"session","version":3,"id":"s","cwd":0,"timestamp":"2026-10-15T00:00:00Z"})).unwrap();
        // The native usage reader stops at this unsupported header, before
        // counting entries. The independent active-message projection must
        // still enforce a bound rather than parsing an unlimited suffix.
        for _ in 0..100_001 {
            writeln!(file, "{{\"type\":\"custom\"}}").unwrap();
        }
        file.flush().unwrap();
        assert!(
            read(&Database::memory().unwrap(), &[root]).is_err(),
            "unsupported native metadata must not bypass the second projection's entry budget"
        );
    }
}
