//! Body-free provider-batch helpers. These do not grant ledger ownership.
use super::{
    session_analytics::SessionDescriptor,
    session_analytics_core::{Project, SessionIdentity, UsageFact},
    session_analytics_index::{key, project_key, ParsedSource},
    session_native_activity::NativeActivityEvent,
    usage_source::NormalizedUsageFact,
};
use crate::{error::AppError, session_manager::source_cache::SourceStamp};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(crate) fn empty() -> ParsedSource {
    ParsedSource {
        sessions: vec![],
        facts: vec![],
        messages: vec![],
        native_activity: vec![],
        reasons: vec![],
        tool_evidence: Default::default(),
    }
}

pub(crate) fn activity_key(tool: &str, native_identity: &str) -> String {
    key(&format!("{tool}-activity"), native_identity)
}

pub(crate) fn runtime_event(
    tool: &str,
    sid: &str,
    native_identity: &str,
    at_ms: i64,
) -> NativeActivityEvent {
    NativeActivityEvent {
        session_key: key(tool, sid),
        event_key: activity_key(tool, native_identity),
        at_ms,
        main: true,
        stream_key: key(&format!("{tool}-activity-stream"), sid),
    }
}

/// A native SID names one append-only runtime stream. Full replicas and
/// verified prefixes keep one stream, while divergent copies cannot pick a
/// winner by path order. The returned delta maintains the provider budget.
pub(crate) fn merge_activity_stream(
    streams: &mut BTreeMap<String, Vec<NativeActivityEvent>>,
    session_key: String,
    events: Vec<NativeActivityEvent>,
) -> Result<usize, AppError> {
    let invalid = || AppError::InvalidInput("usage.analytics.sourceReadFailed".into());
    if events.iter().any(|event| event.session_key != session_key) {
        return Err(invalid());
    }
    match streams.entry(session_key) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            let added = events.len();
            entry.insert(events);
            Ok(added)
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            if entry.get().starts_with(&events) {
                Ok(0)
            } else if events.starts_with(entry.get()) {
                let added = events.len() - entry.get().len();
                entry.insert(events);
                Ok(added)
            } else {
                Err(invalid())
            }
        }
    }
}

pub(crate) fn descriptor(
    tool: &str,
    sid: &str,
    title: &str,
    project: Option<Option<&str>>,
    native_root_id: Option<&str>,
    source_path: Option<&str>,
) -> SessionDescriptor {
    let project_path = project.flatten().map(str::to_owned);
    let project_id = match project {
        Some(Some(path)) => Project::Known(project_key(path)),
        Some(None) => Project::Unknown,
        None => Project::Unparsed,
    };
    let meta = crate::session_manager::SessionMeta {
        provider_id: tool.into(),
        session_id: sid.into(),
        source_path: source_path.map(str::to_owned),
        ..Default::default()
    };
    SessionDescriptor {
        identity: SessionIdentity {
            key: key(tool, sid),
            tool_id: tool.into(),
            project: project_id,
            verified_root_key: native_root_id
                .filter(|root| *root != sid)
                .map(|root| key(tool, root)),
        },
        native_session_id: sid.into(),
        title: crate::session_manager::title::one_line(&crate::redact::scrub(title)),
        project_label: project_path.as_ref().map(|path| {
            Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone())
        }),
        project_path,
        manager_key: source_path.map(|_| crate::session_manager::managed::key(&meta)),
        source_available: true,
        messages_supported: true,
        message_reasons: vec![],
    }
}

pub(crate) fn normalized_fact(
    tool: &str,
    source: &str,
    fact: NormalizedUsageFact,
    complete: bool,
) -> Option<UsageFact> {
    let at_ms = fact.at_ms?;
    Some(UsageFact {
        event_key: key(tool, fact.native_key.as_deref().unwrap_or(&fact.request_id)),
        session_key: fact.session_id.as_deref().map(|sid| key(tool, sid)),
        tool_id: tool.into(),
        source_id: source.into(),
        model: fact.model,
        at_ms,
        buckets: [
            fact.usage.input_tokens,
            fact.usage.output_tokens,
            fact.usage.cache_read_tokens,
            fact.usage.cache_creation_tokens,
        ]
        .map(u64::from),
        complete,
        cache_write_1h: 0,
    })
}

/// Include missing roots as evidence and reject unreadable/symlink aliases.
pub(crate) fn roster_stamp(roots: &[PathBuf], files: &[PathBuf]) -> Result<Value, AppError> {
    let invalid = || AppError::InvalidInput("usage.analytics.sourceReadFailed".into());
    let mut observed_roots = Vec::new();
    for root in roots {
        if !root.is_absolute()
            || root
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(invalid());
        }
        let stamp = match std::fs::symlink_metadata(root) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
                Some(SourceStamp::at(root)?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            _ => return Err(invalid()),
        };
        observed_roots.push((root, stamp));
    }
    let observed_files = files
        .iter()
        .map(|path| {
            let metadata = std::fs::symlink_metadata(path).map_err(|_| invalid())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(invalid());
            }
            Ok((path, SourceStamp::at(path)?))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(json!({"version":1,"roots":observed_roots,"files":observed_files}))
}
