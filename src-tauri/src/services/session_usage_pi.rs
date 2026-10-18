//! Native Pi v3 event accounting. Active context is not total spending.
use super::{
    session_usage::SessionSyncResult,
    usage_source::{self, NormalizedUsageFact},
};
use crate::{
    database::Database, error::AppError, proxy::usage::parser::TokenUsage,
    session_manager::providers::pi,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SCAN_BYTES: u64 = 128 * 1024 * 1024;
const MAX_NATIVE_ENTRIES: usize = 100_000;
fn invalid(code: &str) -> AppError {
    AppError::InvalidInput(format!("usage.pi.{code}"))
}
fn identifier(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= 512)
        .map(str::to_owned)
}
fn event_identity(id: &str) -> String {
    json!(["assistant", id]).to_string()
}
pub(crate) fn native_time(message: &Value, entry: &Value) -> Option<i64> {
    if let Some(value) = message.get("timestamp") {
        let ms = value.as_i64()?;
        return (ms.checked_abs()? <= 9_007_199_254_740_991
            && chrono::DateTime::from_timestamp_millis(ms).is_some())
        .then_some(ms);
    }
    usage_source::timestamp_ms(entry.get("timestamp").and_then(Value::as_str))
}
fn absolute_path(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    Some(normalized)
}
#[derive(Clone)]
struct NativeEntry {
    id: Option<String>,
    parent: Option<String>,
    kind: String,
    digest: String,
}
pub(crate) struct PiRead {
    pub facts: Vec<NormalizedUsageFact>,
    pub reasons: BTreeSet<String>,
    pub line_count: i64,
    pub session_id: Option<String>,
    pub inherited_entries: BTreeSet<String>,
    pub native_activity: Vec<super::session_native_activity::NativeActivityEvent>,
    pub native_activity_reasons: BTreeSet<String>,
    parent_session: Option<PathBuf>,
    entries: Vec<NativeEntry>,
    entry_index: BTreeMap<String, usize>,
    nearest_parents: BTreeMap<String, Option<Option<String>>>,
    // Source indices keep the bounded transient lineage independent of SID size.
    ancestors: Vec<usize>,
    supported: bool,
    bytes: u64,
    #[cfg(test)]
    index_work: std::cell::Cell<usize>,
    #[cfg(test)]
    label_work: std::cell::Cell<usize>,
}
fn read_pi_raw(path: &Path) -> Result<PiRead, AppError> {
    usage_source::with_stable_regular_file(path, |file| {
        let mut bytes = Vec::new();
        file.take(MAX_SOURCE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid("sourceReadFailed"))?;
        if bytes.len() as u64 > MAX_SOURCE_BYTES {
            return Err(invalid("sourceBudgetExceeded"));
        }
        let mut read = PiRead {
            facts: vec![],
            reasons: BTreeSet::new(),
            line_count: 0,
            session_id: None,
            inherited_entries: BTreeSet::new(),
            native_activity: vec![],
            native_activity_reasons: BTreeSet::new(),
            parent_session: None,
            entries: vec![],
            entry_index: BTreeMap::new(),
            nearest_parents: BTreeMap::new(),
            ancestors: vec![],
            supported: false,
            bytes: bytes.len() as u64,
            #[cfg(test)]
            index_work: std::cell::Cell::new(0),
            #[cfg(test)]
            label_work: std::cell::Cell::new(0),
        };
        let mut facts = BTreeMap::new();
        let mut identities = BTreeMap::<String, String>::new();
        let mut header = false;
        let mut runtime_generation = 0u64;
        for raw in bytes.split_inclusive(|b| *b == b'\n') {
            let terminated = raw.last() == Some(&b'\n');
            let value: Value = match std::str::from_utf8(raw)
                .ok()
                .and_then(|s| serde_json::from_str(s).ok())
            {
                Some(value) => value,
                None if raw.iter().all(u8::is_ascii_whitespace) => {
                    if terminated {
                        read.line_count += 1;
                    }
                    continue;
                }
                None => {
                    read.reasons.insert("incompleteEnumeration".into());
                    if terminated {
                        read.line_count += 1;
                        continue;
                    }
                    break;
                }
            };
            read.line_count += 1;
            if !header {
                header = true;
                if value["type"] != "session" {
                    read.reasons.insert("sessionHeaderUnsupported".into());
                    break;
                }
                if value["version"].as_u64() != Some(3) {
                    read.reasons.insert("unsupportedSchemaVersion".into());
                    break;
                }
                read.session_id = identifier(value.get("id"));
                if read.session_id.is_none() {
                    read.reasons.insert("missingNativeSessionIdentity".into());
                    break;
                }
                if !value["cwd"].is_string()
                    || usage_source::timestamp_ms(value["timestamp"].as_str()).is_none()
                {
                    read.reasons.insert("sessionHeaderUnsupported".into());
                    break;
                }
                if let Some(parent) = value.get("parentSession") {
                    read.parent_session =
                        Some(parent.as_str().map(PathBuf::from).unwrap_or_default());
                    if parent.as_str().is_none_or(str::is_empty) {
                        read.reasons.insert("parentSourceUnavailable".into());
                    }
                }
                read.supported = true;
                read.native_activity
                    .push(super::session_analytics_native::runtime_event(
                        "pi",
                        read.session_id.as_deref().expect("validated header"),
                        "[\"header\"]",
                        native_time(&Value::Null, &value).expect("validated header time"),
                    ));
                continue;
            }
            if value["type"] == "session" {
                return Err(invalid("conflictingSessionIdentity"));
            }
            if read.entries.len() >= MAX_NATIVE_ENTRIES {
                return Err(invalid("eventBudgetExceeded"));
            }
            let id = identifier(value.get("id"));
            if let Some(id) = &id {
                let record = format!(
                    "{:x}",
                    Sha256::digest(
                        serde_json::to_vec(&value).map_err(|_| invalid("invalidNativeEntry"))?
                    )
                );
                if let Some(prior) = identities.get(id) {
                    if prior != &record {
                        return Err(invalid("conflictingNativeIdentity"));
                    }
                    continue;
                }
                identities.insert(id.clone(), record);
            }
            let mut signature = value.clone();
            if let Some(object) = signature.as_object_mut() {
                object.remove("parentId");
            }
            let digest = format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&signature).map_err(|_| invalid("invalidNativeEntry"))?
                )
            );
            read.entries.push(NativeEntry {
                id: id.clone(),
                parent: identifier(value.get("parentId")),
                kind: value["type"].as_str().unwrap_or("").to_owned(),
                digest,
            });
            match (id.as_deref(), native_time(&value["message"], &value)) {
                (Some(id), Some(at)) => {
                    let sid = read.session_id.as_deref().expect("validated header");
                    let mut event = super::session_analytics_native::runtime_event(
                        "pi",
                        sid,
                        &json!(["entry", id]).to_string(),
                        at,
                    );
                    if runtime_generation > 0 {
                        event.stream_key = super::session_analytics_index::key(
                            "pi-activity-stream",
                            &json!([sid, runtime_generation]).to_string(),
                        );
                    }
                    read.native_activity.push(event);
                }
                (_, None) => {
                    read.native_activity_reasons
                        .insert("activityTimeUnavailable".into());
                    runtime_generation = runtime_generation
                        .checked_add(1)
                        .ok_or_else(|| invalid("eventBudgetExceeded"))?;
                }
                (None, _) => {
                    read.native_activity_reasons
                        .insert("activityIdentityUnavailable".into());
                    runtime_generation = runtime_generation
                        .checked_add(1)
                        .ok_or_else(|| invalid("eventBudgetExceeded"))?;
                }
            }
            let message = &value["message"];
            let assistant = value["type"] == "message" && message["role"] == "assistant";
            let usage = if assistant {
                message.get("usage")
            } else {
                value.get("usage").or_else(|| message.get("usage"))
            };
            if !assistant {
                if usage.is_some_and(|v| !v.is_null()) {
                    read.reasons.insert("unsupportedUsageEvent".into());
                }
                continue;
            }
            let Some(usage) = usage.filter(|v| {
                v.is_object()
                    && ["input", "output", "cacheRead", "cacheWrite"]
                        .iter()
                        .all(|k| v[*k].is_number())
            }) else {
                read.reasons.insert("usageShapeUnsupported".into());
                continue;
            };
            let model = identifier(message.get("responseModel"))
                .or_else(|| identifier(message.get("model")))
                .unwrap_or_else(|| "unknown".into());
            let at_ms = native_time(message, &value);
            let sid = read.session_id.as_ref().expect("validated header");
            let native_key = id
                .as_ref()
                .map(|id| usage_source::native_key("pi", sid, &event_identity(id)));
            let request_id = id
                .as_ref()
                .map(|id| format!("pi_session:{}", json!([sid, "assistant", id])))
                .unwrap_or_else(|| {
                    format!(
                        "pi_recovered:{:x}",
                        Sha256::digest(
                            json!([path.to_string_lossy(), read.line_count])
                                .to_string()
                                .as_bytes()
                        )
                    )
                });
            let fact = NormalizedUsageFact {
                request_id: request_id.clone(),
                native_key: native_key.clone(),
                session_id: Some(sid.clone()),
                model: model.clone(),
                at_ms,
                usage: TokenUsage {
                    input_tokens: usage_source::checked_token(usage_source::number(
                        usage, "input",
                    )?)?,
                    output_tokens: usage_source::checked_token(usage_source::number(
                        usage, "output",
                    )?)?,
                    cache_read_tokens: usage_source::checked_token(usage_source::number(
                        usage,
                        "cacheRead",
                    )?)?,
                    cache_creation_tokens: usage_source::checked_token(usage_source::number(
                        usage,
                        "cacheWrite",
                    )?)?,
                    model: Some(model),
                    message_id: id,
                },
            };
            facts.insert(native_key.unwrap_or(request_id), fact);
        }
        read.facts = facts.into_values().collect();
        if !header {
            read.reasons.insert("sessionHeaderUnavailable".into());
        }
        if read.facts.iter().any(|f| f.native_key.is_none()) {
            read.reasons.insert("missingNativeEventIdentity".into());
        }
        if read.facts.iter().any(|f| f.at_ms.is_none()) {
            read.reasons.insert("missingTimestamp".into());
        }
        if read.facts.iter().any(|f| f.model == "unknown") {
            read.reasons.insert("missingModel".into());
        }
        prepare_ownership_index(&mut read);
        Ok(read)
    })
}
/// A single file cannot prove fork ownership without its controlled parent
/// roster. The sync path resolves that roster before publishing native facts.
pub(crate) fn read_pi(path: &Path) -> Result<PiRead, AppError> {
    let mut read = read_pi_raw(path)?;
    if read.parent_session.is_some() {
        read.facts.clear();
        read.reasons.insert("forkOwnershipUnresolved".into());
    }
    Ok(read)
}
/// Resolve label chains once per source, iteratively so a native long chain
/// cannot overflow the stack or multiply its work by the number of forks.
fn prepare_ownership_index(read: &mut PiRead) {
    for (i, entry) in read.entries.iter().enumerate() {
        #[cfg(test)]
        read.index_work.set(read.index_work.get() + 1);
        if let Some(id) = &entry.id {
            read.entry_index.insert(id.clone(), i);
        }
    }
    let mut labels = BTreeMap::<String, Option<Option<String>>>::new();
    for entry in &read.entries {
        let Some(entry_id) = &entry.id else {
            continue;
        };
        let mut cursor = entry.parent.clone();
        let mut trail = Vec::new();
        let mut visited = BTreeSet::new();
        let nearest = loop {
            let Some(id) = cursor else {
                break Some(None);
            };
            #[cfg(test)]
            read.label_work.set(read.label_work.get() + 1);
            if let Some(known) = labels.get(&id) {
                break known.clone();
            }
            let Some(index) = read.entry_index.get(&id) else {
                break None;
            };
            let parent = &read.entries[*index];
            if parent.kind != "label" {
                break Some(Some(id));
            }
            if !visited.insert(id.clone()) {
                break None;
            }
            trail.push(id);
            cursor = parent.parent.clone();
        };
        for label in trail {
            labels.insert(label, nearest.clone());
        }
        if entry.kind == "label" {
            labels.insert(entry_id.clone(), nearest.clone());
        }
        read.nearest_parents.insert(entry_id.clone(), nearest);
    }
}
fn inherited_prefix(child: &PiRead, parent: &PiRead) -> Option<BTreeSet<String>> {
    let mut inherited = BTreeSet::new();
    let mut previous = None;
    for entry in &child.entries {
        let id = entry.id.as_deref()?;
        let Some(original_index) = parent.entry_index.get(id) else {
            break;
        };
        let original = &parent.entries[*original_index];
        if original.digest != entry.digest
            || parent.nearest_parents.get(id)?.as_ref()?.as_deref() != previous
            || entry.parent.as_deref() != previous
        {
            return None;
        }
        inherited.insert(id.to_owned());
        previous = Some(id);
    }
    (!inherited.is_empty()).then_some(inherited)
}
fn resolve_fork(
    index: usize,
    reads: &mut [PiRead],
    states: &mut [u8],
    paths: &BTreeMap<PathBuf, usize>,
    history: &usage_source::ProviderIndex,
    depth: usize,
) -> bool {
    if states[index] == 2 {
        return true;
    }
    if states[index] == 3 {
        return false;
    }
    if states[index] == 1 || depth > 64 {
        reads[index]
            .reasons
            .insert("parentCycleOrDepthLimit".into());
        reads[index].facts.clear();
        states[index] = 3;
        return false;
    }
    states[index] = 1;
    let Some(parent_path) = reads[index].parent_session.as_ref() else {
        states[index] = if reads[index].supported { 2 } else { 3 };
        return reads[index].supported;
    };
    let parent = absolute_path(parent_path).and_then(|p| paths.get(&p).copied());
    let reason = match parent {
        None => Some("parentSourceUnavailable"),
        Some(p) if !resolve_fork(p, reads, states, paths, history, depth + 1) => {
            Some("parentSourceUnavailable")
        }
        Some(p)
            if reads[p].reasons.contains("incompleteEnumeration")
                || reads[index].reasons.contains("incompleteEnumeration") =>
        {
            Some("forkBoundaryUnavailable")
        }
        Some(p) if reads[p].ancestors.len() >= 64 => Some("parentCycleOrDepthLimit"),
        Some(p) => match inherited_prefix(&reads[index], &reads[p]) {
            Some(inherited) => {
                let mut ancestors = reads[p].ancestors.clone();
                ancestors.push(p);
                // A parent rewrite can remove a previously counted copied row.
                // Its absence from today's prefix cannot reassign that expense
                // to the child. A copy can have originated at an older ancestor;
                // only trusted acknowledgements in this resolved lineage apply.
                let unresolved_copy = ancestors.iter().any(|ancestor| {
                    let Some(sid) = reads[*ancestor].session_id.as_deref() else {
                        return true;
                    };
                    reads[index].facts.iter().any(|fact| {
                        fact.usage.message_id.as_deref().is_some_and(|id| {
                            !inherited.contains(id)
                                && history.has_acknowledged(&usage_source::native_key(
                                    "pi",
                                    sid,
                                    &event_identity(id),
                                ))
                        })
                    })
                });
                if unresolved_copy {
                    Some("forkBoundaryUnavailable")
                } else {
                    reads[index].ancestors = ancestors;
                    reads[index].inherited_entries = inherited.clone();
                    reads[index].facts.retain(|f| {
                        f.usage
                            .message_id
                            .as_ref()
                            .is_some_and(|id| !inherited.contains(id))
                    });
                    None
                }
            }
            None => Some("forkBoundaryUnavailable"),
        },
    };
    if let Some(reason) = reason {
        reads[index].facts.clear();
        reads[index].reasons.insert(reason.into());
        states[index] = 3;
        false
    } else {
        states[index] = 2;
        true
    }
}

/// Read-only sibling of ingestion's ownership resolution. All roots and files
/// participate, so a copied fork prefix cannot become a new expense.
pub(crate) fn read_analytics_roster(
    db: &Database,
    roots: &[PathBuf],
) -> Result<Vec<(PathBuf, PiRead)>, AppError> {
    let files = pi::usage_files(roots).map_err(|_| invalid("sourceEnumerationFailed"))?;
    let index = usage_source::ProviderIndex::read_roots(db, "pi", roots)?;
    let before = files
        .iter()
        .map(|path| usage_source::Snapshot::file(path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut reads = Vec::with_capacity(files.len());
    let mut bytes = 0u64;
    let mut entries = 0usize;
    for path in &files {
        let read = read_pi_raw(path)?;
        bytes = bytes
            .checked_add(read.bytes)
            .ok_or_else(|| invalid("sourceBudgetExceeded"))?;
        entries = entries
            .checked_add(read.entries.len())
            .ok_or_else(|| invalid("eventBudgetExceeded"))?;
        if bytes > MAX_SCAN_BYTES || entries > MAX_NATIVE_ENTRIES {
            return Err(invalid("sourceBudgetExceeded"));
        }
        reads.push(read);
    }
    let paths = files
        .iter()
        .enumerate()
        .filter_map(|(i, path)| absolute_path(path).map(|path| (path, i)))
        .collect::<BTreeMap<_, _>>();
    let mut states = vec![0; reads.len()];
    for i in 0..reads.len() {
        resolve_fork(i, &mut reads, &mut states, &paths, &index, 0);
    }
    let mut replicas = BTreeMap::<String, Value>::new();
    let mut native_records = BTreeMap::new();
    for read in &reads {
        if let Some(sid) = &read.session_id {
            for entry in &read.entries {
                let Some(id) = &entry.id else {
                    continue;
                };
                let identity = (sid.clone(), id.clone());
                let signature = (
                    entry.parent.clone(),
                    entry.kind.clone(),
                    entry.digest.clone(),
                );
                if native_records
                    .get(&identity)
                    .is_some_and(|prior| prior != &signature)
                {
                    return Err(invalid("conflictingNativeReplicas"));
                }
                native_records.insert(identity, signature);
            }
        }
        for fact in &read.facts {
            if let Some(native) = &fact.native_key {
                let value = serde_json::to_value(fact).map_err(|_| invalid("invalidFact"))?;
                if replicas.get(native).is_some_and(|prior| prior != &value) {
                    return Err(invalid("conflictingNativeReplicas"));
                }
                replicas.insert(native.clone(), value);
            }
        }
    }
    if pi::usage_files(roots).map_err(|_| invalid("sourceEnumerationFailed"))? != files {
        return Err(invalid("sourceRosterChanged"));
    }
    for source in &before {
        source.verify()?;
    }
    {
        let conn = crate::database::lock_conn!(db.conn);
        index.check_on(&conn)?;
    }
    Ok(files.into_iter().zip(reads).collect())
}
pub(crate) fn sync_pi_usage_at(
    db: &Database,
    roots: &[PathBuf],
) -> Result<SessionSyncResult, AppError> {
    use super::usage_source::{Prepared, ProviderIndex, Snapshot};
    use super::usage_sync_db::{commit_sync_batch, read_sync_state, CheckpointChange, SyncState};
    use rusqlite::{params, OptionalExtension};
    let files = pi::usage_files(roots).map_err(|_| invalid("sourceEnumerationFailed"))?;
    let mut result = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: files.len() as u32,
        errors: vec![],
    };
    if files.is_empty() {
        return Ok(result);
    }
    let mut index = ProviderIndex::load_roots(db, "pi", roots)?;
    let roster = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&files).map_err(|_| invalid("invalidRoster"))?)
    );
    let mut sources = Vec::with_capacity(files.len());
    let mut expected = Vec::with_capacity(files.len());
    let mut total = 0u64;
    for path in &files {
        let snapshot = Snapshot::file(path)?;
        total = total
            .checked_add(
                snapshot.stamp["size"]
                    .as_u64()
                    .ok_or_else(|| invalid("invalidSourceStamp"))?,
            )
            .ok_or_else(|| invalid("sourceBudgetExceeded"))?;
        if total > MAX_SCAN_BYTES {
            return Err(invalid("sourceBudgetExceeded"));
        }
        let checkpoint = read_sync_state(db, &path.to_string_lossy())?;
        let mut prepared = Prepared::with_index(db, snapshot, checkpoint, 1, &index)?;
        prepared.observe_dependency(&roster);
        sources.push(prepared);
        expected.push(checkpoint);
    }
    if sources.iter().all(|s| s.unchanged) {
        return Ok(result);
    }
    let mut reads = Vec::with_capacity(files.len());
    let mut checkpoints = Vec::with_capacity(files.len());
    let mut entries = 0usize;
    total = 0;
    for (i, path) in files.iter().enumerate() {
        let read = read_pi_raw(path)?;
        entries = entries
            .checked_add(read.entries.len())
            .ok_or_else(|| invalid("eventBudgetExceeded"))?;
        if entries > MAX_NATIVE_ENTRIES {
            return Err(invalid("eventBudgetExceeded"));
        }
        total = total
            .checked_add(read.bytes)
            .ok_or_else(|| invalid("sourceBudgetExceeded"))?;
        if total > MAX_SCAN_BYTES {
            return Err(invalid("sourceBudgetExceeded"));
        }
        let modified = sources[i].snapshot.stamp["mtimeNs"]
            .as_str()
            .and_then(|s| s.parse::<u128>().ok())
            .and_then(|ns| i64::try_from(ns / 1_000_000_000).ok())
            .ok_or_else(|| invalid("invalidSourceStamp"))?;
        checkpoints.push(CheckpointChange {
            key: path.to_string_lossy().into_owned(),
            expected: expected[i],
            next: Some(SyncState {
                modified,
                offset: read.line_count,
            }),
        });
        reads.push(read);
    }
    let paths = files
        .iter()
        .enumerate()
        .filter_map(|(i, p)| absolute_path(p).map(|p| (p, i)))
        .collect::<BTreeMap<_, _>>();
    let mut states = vec![0; reads.len()];
    for i in 0..reads.len() {
        resolve_fork(i, &mut reads, &mut states, &paths, &index, 0);
    }
    let mut winners = BTreeMap::<String, (usize, Value)>::new();
    for (i, read) in reads.iter().enumerate() {
        for fact in &read.facts {
            let Some(native) = &fact.native_key else {
                continue;
            };
            let value = serde_json::to_value(fact).map_err(|_| invalid("invalidFact"))?;
            if winners
                .get(native)
                .is_some_and(|(_, prior)| prior != &value)
            {
                return Err(invalid("conflictingNativeReplicas"));
            }
            winners.entry(native.clone()).or_insert((i, value));
        }
    }
    commit_sync_batch(db, &checkpoints, |conn| {
        if pi::usage_files(roots).map_err(|_| invalid("sourceEnumerationFailed"))? != files {
            return Err(invalid("sourceRosterChanged"));
        }
        for source in &sources {
            source.check_on(conn)?;
        }
        let mut accepted = BTreeSet::new();
        for (native, (i, value)) in &winners {
            let fact: NormalizedUsageFact =
                serde_json::from_value(value.clone()).map_err(|_| invalid("invalidFact"))?;
            let (Some(sid), Some(at)) = (fact.session_id.as_deref(), fact.at_ms) else {
                result.skipped += 1;
                continue;
            };
            let row:Option<(String,String,Option<String>,Option<String>)>=conn.query_row("SELECT provider_id,app_type,data_source,session_id FROM proxy_request_logs WHERE request_id=?1",[&fact.request_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
            if row.as_ref().is_some_and(|(owner, app, source, session)| {
                owner != "_pi_session"
                    || app != "pi"
                    || source.as_deref() != Some("pi_session")
                    || session.as_deref() != Some(sid)
            }) {
                return Err(invalid("foreignRecordIdentity"));
            }
            if !sources[*i].should_store_on(
                conn,
                &fact.request_id,
                native,
                "pi_session",
                at.div_euclid(1000),
            )? {
                result.skipped += 1;
                if sources[*i].evidence.has(native) {
                    accepted.insert(native.clone());
                }
                continue;
            }
            let u = &fact.usage;
            let changed=conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES(?1,'_pi_session','pi',?2,?3,?4,?5,?6,0,0,?7,?8,'pi_session') ON CONFLICT(request_id) DO UPDATE SET model=excluded.model,input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cache_read_tokens=excluded.cache_read_tokens,cache_creation_tokens=excluded.cache_creation_tokens,created_at=excluded.created_at WHERE model!=excluded.model OR input_tokens!=excluded.input_tokens OR output_tokens!=excluded.output_tokens OR cache_read_tokens!=excluded.cache_read_tokens OR cache_creation_tokens!=excluded.cache_creation_tokens OR created_at!=excluded.created_at",params![fact.request_id,fact.model,u.input_tokens,u.output_tokens,u.cache_read_tokens,u.cache_creation_tokens,sid,at.div_euclid(1000)])?;
            if changed > 0 {
                result.imported += 1;
            } else {
                result.skipped += 1;
            }
            accepted.insert(native.clone());
        }
        for (i, read) in reads.iter().enumerate() {
            let source = &mut sources[i];
            for reason in &read.reasons {
                source.evidence.complete_event_identity_set = false;
                source.evidence.partial(reason);
            }
            for fact in &read.facts {
                if let Some(native) = &fact.native_key {
                    if accepted.contains(native) {
                        source.evidence.acknowledge(native);
                    }
                }
            }
            source.save_on(conn, checkpoints[i].next.expect("native checkpoint"))?;
        }
        Ok(())
    })?;
    index.advance_many(sources);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    const AT: i64 = 1_792_053_987_262;
    const ISO: &str = "2026-10-15T08:46:27.262Z";
    fn header(sid: &str) -> Value {
        json!({"type":"session","version":3,"id":sid,"timestamp":ISO,"cwd":"/synthetic-project"})
    }
    fn user(id: &str) -> Value {
        json!({"type":"message","id":id,"parentId":null,"timestamp":ISO,"message":{"role":"user","content":"synthetic prompt","timestamp":AT}})
    }
    fn call(id: &str, parent: Option<&str>, buckets: [u64; 4]) -> Value {
        json!({"type":"message","id":id,"parentId":parent,"timestamp":ISO,"message":{
            "role":"assistant","api":"openai-responses","provider":"fixture","model":"native-model",
            "content":[{"type":"text","text":"private-pi-body-not-indexed"}],"stopReason":"stop","timestamp":AT,
            "usage":{"input":buckets[0],"output":buckets[1],"cacheRead":buckets[2],"cacheWrite":buckets[3],"totalTokens":170,
                "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}}}})
    }
    fn parent() -> Vec<Value> {
        vec![
            header("base"),
            user("u1"),
            call("a1", Some("u1"), [40, 10, 100, 20]),
            call("a2", Some("u1"), [7, 1, 0, 0]),
        ]
    }
    fn write(root: &Path, name: &str, rows: &[Value]) -> PathBuf {
        let path = root.join("project").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
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
    fn fork(root: &Path, parent_path: &Path) -> PathBuf {
        let mut rows = vec![
            header("child"),
            user("u1"),
            call("a1", Some("u1"), [40, 10, 100, 20]),
            call("a3", Some("a1"), [20, 2, 10, 0]),
        ];
        rows[0]["parentSession"] = json!(parent_path);
        write(root, "child.jsonl", &rows)
    }
    fn buckets(f: &NormalizedUsageFact) -> [u32; 4] {
        [
            f.usage.input_tokens,
            f.usage.output_tokens,
            f.usage.cache_read_tokens,
            f.usage.cache_creation_tokens,
        ]
    }

    #[test]
    fn native_runtime_pi_reader_retains_owned_runtime_and_branch_events_without_body() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.push(json!({"type":"model_change","id":"model","parentId":"a2","timestamp":"2026-10-15T08:46:28.262Z","modelId":"fixture"}));
        rows.push(json!({"type":"message","id":"tool","parentId":"model","timestamp":ISO,"message":{"role":"toolResult","content":"private-runtime-not-indexed","timestamp":AT+2000}}));
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert_eq!(read.facts.len(), 2);
        assert_eq!(read.native_activity.len(), 6);
        assert_eq!(
            read.native_activity
                .iter()
                .map(|event| event.at_ms)
                .collect::<Vec<_>>(),
            [AT, AT, AT, AT, AT + 1000, AT + 2000]
        );
        let encoded = serde_json::to_string(&read.native_activity).unwrap();
        assert!(!encoded.contains("private-pi-body"));
        assert!(!encoded.contains("private-runtime"));
    }

    #[test]
    fn native_runtime_pi_missing_time_cuts_activity_stream() {
        let temp = tempfile::tempdir().unwrap();
        let mut assistant = call("a1", Some("gap"), [40, 10, 100, 20]);
        assistant["message"]["timestamp"] = json!(AT + 60_000);
        let entries = [
            header("base"),
            user("u1"),
            json!({"type":"custom","id":"gap","parentId":"u1","data":"private runtime"}),
            assistant,
        ];
        let read = read_pi(&write(temp.path(), "base.jsonl", &entries)).unwrap();
        let result = super::super::session_native_activity::activity(
            &read.native_activity,
            &super::super::session_analytics_core::Window {
                start_ms: Some(AT),
                end_ms: AT + 100_000,
            },
            &[super::super::session_analytics_core::DayBoundary {
                date: "date".into(),
                start_ms: AT,
                end_ms: AT + 100_000,
            }],
        )
        .unwrap();
        assert_eq!(read.facts.len(), 1);
        assert_eq!(
            result.active_ms, 0,
            "missing native time is a boundary, not permission to bridge it"
        );
        assert!(read
            .native_activity_reasons
            .contains("activityTimeUnavailable"));
    }
    fn totals(db: &Database) -> (u64, u64, u64, u64, u64) {
        db.conn.lock().unwrap().query_row("SELECT COUNT(*),COALESCE(SUM(input_tokens),0),COALESCE(SUM(output_tokens),0),COALESCE(SUM(cache_read_tokens),0),COALESCE(SUM(cache_creation_tokens),0) FROM proxy_request_logs WHERE app_type='pi'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap()
    }
    fn dump(db: &Database) -> String {
        let conn = db.conn.lock().unwrap();
        let mut out = String::new();
        for table in [
            "proxy_request_logs",
            "session_log_sync",
            "session_meta_cache",
            "maintenance_flags",
        ] {
            let mut stmt = conn
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let n = stmt.column_count();
            let mut rows = stmt.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                for i in 0..n {
                    out.push_str(&format!("{:?}", row.get_ref(i).unwrap()));
                }
            }
        }
        out
    }
    #[test]
    fn pi_native_v3_fixture_has_a_real_abandoned_branch_and_equal_fork_times() {
        let rows = parent();
        assert_eq!(
            crate::session_manager::providers::pi::active_branch(&rows)
                .iter()
                .filter(|v| v["message"]["role"] == "assistant")
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|v| v["message"]["role"] == "assistant")
                .count(),
            2
        );
        assert!(rows.iter().all(|v| v["timestamp"] == ISO));
    }
    #[test]
    fn pi_native_four_buckets_are_disjoint_and_reasoning_is_not_added_again() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.truncate(3);
        rows[2]["message"]["usage"]["reasoning"] = json!(8);
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert_eq!(read.facts.len(), 1);
        assert_eq!(buckets(&read.facts[0]), [40, 10, 100, 20]);
        assert!(read.reasons.is_empty());
    }
    #[test]
    fn pi_spent_reads_all_native_branches_instead_of_the_current_projection() {
        let temp = tempfile::tempdir().unwrap();
        let read = read_pi(&write(temp.path(), "base.jsonl", &parent())).unwrap();
        assert_eq!(read.facts.len(), 2);
        assert_eq!(
            read.facts.iter().map(|f| f.usage.input_tokens).sum::<u32>(),
            47
        );
    }
    #[test]
    fn pi_recorded_response_model_wins_over_requested_model_without_route_inference() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2]["message"]["responseModel"] = json!("served-model");
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert!(read.facts.iter().any(|f| f.model == "served-model"));
    }
    #[test]
    fn pi_valid_json_at_eof_without_lf_is_native_readable_and_never_repaired_by_ofox() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(temp.path(), "base.jsonl", &parent());
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .trim_end()
            .to_owned();
        std::fs::write(&path, &text).unwrap();
        assert_eq!(read_pi(&path).unwrap().facts.len(), 2);
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    }
    #[test]
    fn pi_torn_json_eof_keeps_the_complete_prefix_and_checkpoint_before_the_tail() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let path = write(temp.path(), "base.jsonl", &parent());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"type\":\"message\"")
            .unwrap();
        let read = read_pi(&path).unwrap();
        assert_eq!(read.facts.len(), 2);
        assert_eq!(read.line_count, 4);
        assert!(read.reasons.contains("incompleteEnumeration"));
    }
    #[test]
    fn pi_zero_usage_remains_a_real_native_event() {
        let temp = tempfile::tempdir().unwrap();
        let rows = [header("base"), call("zero", None, [0; 4])];
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert_eq!(read.facts.len(), 1);
        assert_eq!(buckets(&read.facts[0]), [0; 4]);
    }
    fn invalid_bucket(value: Value) {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2]["message"]["usage"]["input"] = value;
        assert!(read_pi(&write(temp.path(), "base.jsonl", &rows)).is_err());
    }
    #[test]
    fn pi_negative_token_integer_is_an_error() {
        invalid_bucket(json!(-1));
    }
    #[test]
    fn pi_fractional_token_integer_is_an_error() {
        invalid_bucket(json!(0.5));
    }
    #[test]
    fn pi_overflow_token_integer_is_an_error() {
        invalid_bucket(json!(u64::MAX));
    }
    #[test]
    fn pi_missing_cache_bucket_is_unknown_usage_instead_of_zero() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.truncate(3);
        rows[2]["message"]["usage"]
            .as_object_mut()
            .unwrap()
            .remove("cacheRead");
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert!(read.facts.is_empty());
        assert!(read.reasons.contains("usageShapeUnsupported"));
    }
    #[test]
    fn pi_missing_native_entry_id_stays_partial_without_ordinal_ack_identity() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.truncate(3);
        rows[2].as_object_mut().unwrap().remove("id");
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert_eq!(read.facts.len(), 1);
        assert!(read.facts[0].native_key.is_none());
        assert!(read.reasons.contains("missingNativeEventIdentity"));
    }
    #[test]
    fn pi_missing_native_time_never_uses_header_mtime_or_now() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.truncate(3);
        rows[2].as_object_mut().unwrap().remove("timestamp");
        rows[2]["message"]
            .as_object_mut()
            .unwrap()
            .remove("timestamp");
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert!(read.facts[0].at_ms.is_none());
        assert!(read.reasons.contains("missingTimestamp"));
    }
    #[test]
    fn pi_native_message_time_is_exact_milliseconds_without_a_size_threshold() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.truncate(3);
        rows[2]["message"]["timestamp"] = json!(1000);
        assert_eq!(
            read_pi(&write(temp.path(), "base.jsonl", &rows))
                .unwrap()
                .facts[0]
                .at_ms,
            Some(1000)
        );
    }
    #[test]
    fn pi_native_entry_iso_time_is_available_when_message_time_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.truncate(3);
        rows[2]["message"]
            .as_object_mut()
            .unwrap()
            .remove("timestamp");
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert_eq!(
            read.facts[0].at_ms,
            chrono::DateTime::parse_from_rfc3339(ISO)
                .ok()
                .map(|t| t.timestamp_millis())
        );
    }
    #[test]
    fn pi_future_schema_is_partial_without_reinterpreting_tree_identity() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[0]["version"] = json!(4);
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert!(read.facts.is_empty());
        assert!(read.reasons.contains("unsupportedSchemaVersion"));
    }
    #[test]
    fn pi_native_compaction_and_tool_usage_do_not_borrow_the_current_model() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.truncate(3);
        rows.push(json!({"type":"compaction","id":"c1","parentId":"a1","timestamp":ISO,"summary":"synthetic summary","usage":rows[2]["message"]["usage"]}));
        rows.push(json!({"type":"message","id":"t1","parentId":"c1","timestamp":ISO,"message":{"role":"toolResult","content":[],"timestamp":AT,"usage":rows[2]["message"]["usage"]}}));
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert_eq!(read.facts.len(), 1);
        assert!(read.reasons.contains("unsupportedUsageEvent"));
    }
    #[test]
    fn pi_same_file_native_revision_replaces_model_day_and_buckets() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.truncate(3);
        let before = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        rows[2]["message"]["model"] = json!("revised");
        rows[2]["message"]["usage"]["input"] = json!(80);
        rows[2]["message"]["timestamp"] = json!(AT + 86_400_000);
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert_eq!(read.facts.len(), 1);
        assert_eq!(read.facts[0].native_key, before.facts[0].native_key);
        assert_eq!(read.facts[0].model, "revised");
        assert_eq!(read.facts[0].usage.input_tokens, 80);
        assert_eq!(read.facts[0].at_ms, Some(AT + 86_400_000));
    }
    #[test]
    fn pi_native_identity_survives_file_moves_and_is_scoped_to_sid() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(temp.path(), "base.jsonl", &parent());
        let before = read_pi(&path).unwrap();
        let moved = path.with_file_name("moved.jsonl");
        std::fs::rename(path, &moved).unwrap();
        let after = read_pi(&moved).unwrap();
        assert_eq!(before.facts[0].request_id, after.facts[0].request_id);
        assert_eq!(before.facts[0].native_key, after.facts[0].native_key);
        let mut other = parent();
        other[0]["id"] = json!("unrelated");
        let other = read_pi(&write(temp.path(), "other.jsonl", &other)).unwrap();
        assert_ne!(before.facts[0].native_key, other.facts[0].native_key);
    }
    #[test]
    fn pi_fork_copy_at_the_same_header_millisecond_is_counted_only_at_its_origin() {
        let temp = tempfile::tempdir().unwrap();
        let p = write(temp.path(), "base.jsonl", &parent());
        fork(temp.path(), &p);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (3, 67, 13, 110, 20));
        let conn = db.conn.lock().unwrap();
        let child:u64=conn.query_row("SELECT COUNT(*) FROM proxy_request_logs WHERE app_type='pi' AND session_id='child'",[],|r|r.get(0)).unwrap();
        assert_eq!(child, 1);
    }
    fn parent_loses_a_previously_acknowledged_fork_event(move_parent: bool) {
        let temp = tempfile::tempdir().unwrap();
        let p = write(temp.path(), "base.jsonl", &parent());
        let child = fork(temp.path(), &p);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (3, 67, 13, 110, 20));

        let mut rows = parent();
        rows[2] = json!({"type":"model_change","id":"m1","parentId":"u1","timestamp":ISO,"provider":"fixture","modelId":"native-model"});
        if move_parent {
            std::fs::remove_file(&p).unwrap();
            let moved = write(temp.path(), "moved.jsonl", &rows);
            let mut child_rows: Vec<Value> = std::fs::read_to_string(&child)
                .unwrap()
                .lines()
                .map(|s| serde_json::from_str(s).unwrap())
                .collect();
            child_rows[0]["parentSession"] = json!(moved);
            write(temp.path(), "child.jsonl", &child_rows);
        } else {
            write(temp.path(), "base.jsonl", &rows);
        }
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(
            totals(&db),
            (3, 67, 13, 110, 20),
            "a historical parent expense cannot become a new child expense when its current parent row disappears"
        );
        let conn = db.conn.lock().unwrap();
        let meta: String = conn
            .query_row(
                "SELECT meta_json FROM session_meta_cache WHERE provider_id='pi' AND source_path=?1",
                [child.to_string_lossy().as_ref()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(meta.contains("forkBoundaryUnavailable"));
    }
    #[test]
    fn pi_fork_history_loss_does_not_reclassify_old_parent_spend_as_child() {
        parent_loses_a_previously_acknowledged_fork_event(false);
    }
    #[test]
    fn pi_fork_history_loss_after_parent_move_does_not_reclassify_old_parent_spend_as_child() {
        parent_loses_a_previously_acknowledged_fork_event(true);
    }
    #[test]
    fn pi_fork_history_in_another_native_session_cannot_block_a_child_event() {
        let temp = tempfile::tempdir().unwrap();
        let mut unrelated = parent();
        unrelated[0]["id"] = json!("unrelated");
        write(temp.path(), "unrelated.jsonl", &unrelated);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        let p = write(temp.path(), "base.jsonl", &[header("base"), user("u1")]);
        fork(temp.path(), &p);
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (4, 107, 23, 210, 40));
    }
    #[test]
    fn pi_fork_history_loss_in_a_middle_session_does_not_reclassify_ancestor_spend() {
        let temp = tempfile::tempdir().unwrap();
        let p = write(temp.path(), "base.jsonl", &parent());
        let child = fork(temp.path(), &p);
        let mut grandchild = vec![
            header("grandchild"),
            user("u1"),
            call("a1", Some("u1"), [40, 10, 100, 20]),
            call("g3", Some("a1"), [3, 1, 0, 0]),
        ];
        grandchild[0]["parentSession"] = json!(child);
        write(temp.path(), "grandchild.jsonl", &grandchild);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (4, 70, 14, 110, 20));
        let mut child_rows: Vec<Value> = std::fs::read_to_string(&child)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        child_rows[2] = json!({"type":"model_change","id":"m1","parentId":"u1","timestamp":ISO,"provider":"fixture","modelId":"native-model"});
        write(temp.path(), "child.jsonl", &child_rows);
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(
            totals(&db),
            (4, 70, 14, 110, 20),
            "a missing intermediate copy cannot move trusted ancestor spending into the grandchild"
        );
    }
    #[test]
    fn pi_missing_parent_cannot_turn_fork_copies_into_new_spend() {
        let temp = tempfile::tempdir().unwrap();
        fork(temp.path(), &temp.path().join("missing.jsonl"));
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (0, 0, 0, 0, 0));
        let conn = db.conn.lock().unwrap();
        let meta: String = conn
            .query_row(
                "SELECT meta_json FROM session_meta_cache WHERE provider_id='pi'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(meta.contains("parentSourceUnavailable"));
    }
    #[test]
    fn pi_parent_outside_the_controlled_roster_is_not_opened() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let p = write(outside.path(), "parent.jsonl", &parent());
        fork(temp.path(), &p);
        let db = Database::memory().unwrap();
        let before = super::super::usage_source::body_reads_under(&p);
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db).0, 0);
        assert_eq!(super::super::usage_source::body_reads_under(&p), before);
    }
    #[test]
    fn pi_identical_physical_twins_do_not_duplicate_native_events() {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), "a.jsonl", &parent());
        write(temp.path(), "b.jsonl", &parent());
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (2, 47, 11, 100, 20));
    }
    #[test]
    fn pi_conflicting_physical_twins_fail_without_an_mtime_winner() {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), "a.jsonl", &parent());
        let mut other = parent();
        other[2]["message"]["usage"]["input"] = json!(80);
        write(temp.path(), "b.jsonl", &other);
        let db = Database::memory().unwrap();
        let before = dump(&db);
        assert!(sync_pi_usage_at(&db, &[temp.path().to_owned()]).is_err());
        assert_eq!(dump(&db), before);
    }
    #[test]
    fn pi_live_revision_preserves_saved_cost_and_replaces_the_native_fact() {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), "base.jsonl", &parent());
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE proxy_request_logs SET total_cost_usd='1e-7' WHERE app_type='pi'",
                [],
            )
            .unwrap();
        let mut changed = parent();
        changed[2]["message"]["usage"]["input"] = json!(80);
        changed[2]["message"]["model"] = json!("revised");
        write(temp.path(), "base.jsonl", &changed);
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (2, 87, 11, 100, 20));
        let cost: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT total_cost_usd FROM proxy_request_logs WHERE app_type='pi' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cost, "1e-7");
    }
    #[test]
    fn pi_late_checkpoint_failure_rolls_back_facts_receipts_and_all_checkpoints() {
        let temp = tempfile::tempdir().unwrap();
        let p = write(temp.path(), "base.jsonl", &parent());
        let child = fork(temp.path(), &p);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        let before = dump(&db);
        let mut rows = parent();
        rows.push(call("new", Some("a2"), [1, 1, 0, 0]));
        write(temp.path(), "base.jsonl", &rows);
        {
            let conn = db.conn.lock().unwrap();
            let literal: String = conn
                .query_row("SELECT quote(?1)", [child.to_str().unwrap()], |r| r.get(0))
                .unwrap();
            conn.execute_batch(&format!("CREATE TEMP TRIGGER fail_pi_checkpoint BEFORE INSERT ON session_log_sync WHEN NEW.file_path={literal} BEGIN SELECT RAISE(ABORT,'fixture checkpoint failure'); END;")).unwrap();
        }
        assert!(sync_pi_usage_at(&db, &[temp.path().to_owned()]).is_err());
        assert_eq!(dump(&db), before);
    }
    #[test]
    fn pi_missing_time_or_id_is_not_persisted_or_acknowledged() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2].as_object_mut().unwrap().remove("id");
        rows[3].as_object_mut().unwrap().remove("timestamp");
        rows[3]["message"]
            .as_object_mut()
            .unwrap()
            .remove("timestamp");
        write(temp.path(), "base.jsonl", &rows);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db).0, 0);
        assert!(!dump(&db).contains("private-pi-body"));
    }
    #[test]
    fn pi_unchanged_roster_skips_source_body_reads() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(temp.path(), "base.jsonl", &parent());
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        let before = super::super::usage_source::body_reads_under(&path);
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(super::super::usage_source::body_reads_under(&path), before);
    }
    #[test]
    fn pi_usage_capability_requires_native_source_registration() {
        let cap = super::super::usage_query::source_capabilities()
            .into_iter()
            .find(|c| c.tool_id == "pi")
            .unwrap();
        assert!(cap.usage_supported);
        assert_eq!(cap.source_ids, ["pi_session"]);
    }
    #[test]
    fn pi_session_capabilities_expose_the_verified_native_session_bridge() {
        let cap = super::super::usage_query::source_capabilities()
            .into_iter()
            .find(|c| c.tool_id == "pi")
            .unwrap();
        assert!(cap.session_analytics_supported);
        assert!(cap.project_filter_supported);
    }
    #[test]
    fn pi_usage_resolver_missing_settings_keeps_the_default_root() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            crate::session_manager::providers::pi::usage_roots_in(temp.path(), None).unwrap(),
            [temp.path().join("sessions")]
        );
    }
    #[test]
    fn pi_usage_resolver_environment_wins_without_parsing_shadowed_bad_settings() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("settings.json"), "{").unwrap();
        let custom = temp.path().join("custom ");
        assert_eq!(
            crate::session_manager::providers::pi::usage_roots_in(temp.path(), custom.to_str())
                .unwrap(),
            [temp.path().join("sessions"), custom]
        );
    }
    #[test]
    fn pi_usage_resolver_malformed_settings_is_unknown_instead_of_default_only() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("settings.json"), "{").unwrap();
        assert!(crate::session_manager::providers::pi::usage_roots_in(temp.path(), None).is_err());
    }
    #[test]
    fn pi_usage_resolver_non_file_settings_is_unknown_instead_of_default_only() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("settings.json")).unwrap();
        assert!(crate::session_manager::providers::pi::usage_roots_in(temp.path(), None).is_err());
    }
    #[test]
    fn pi_usage_resolver_non_string_session_directory_is_unknown() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("settings.json"), "{sessionDir:12}").unwrap();
        assert!(crate::session_manager::providers::pi::usage_roots_in(temp.path(), None).is_err());
    }
    #[test]
    fn pi_usage_files_enumerates_default_project_and_custom_flat_sources_once() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().join("default");
        let custom = temp.path().join("custom");
        let a = write(&default, "base.jsonl", &parent());
        std::fs::create_dir(&custom).unwrap();
        let b = custom.join("child.jsonl");
        std::fs::write(&b, "{}\n").unwrap();
        let files = crate::session_manager::providers::pi::usage_files(&[default, custom]).unwrap();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(files, expected);
    }
    #[test]
    fn pi_usage_files_missing_roots_are_empty() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            crate::session_manager::providers::pi::usage_files(&[temp.path().join("missing")])
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn pi_usage_files_an_existing_plain_file_root_is_unknown_instead_of_empty() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plain-file");
        std::fs::write(&root, "synthetic").unwrap();
        assert!(crate::session_manager::providers::pi::usage_files(&[root]).is_err());
    }
    #[test]
    fn pi_multiroot_custom_checkpoint_blocks_fresh_proof_when_default_has_no_history() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().join("default");
        let custom = temp.path().join("custom");
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO session_log_sync(file_path,last_modified,last_line_offset,last_synced_at) VALUES(?1,1,3,1)",[custom.join("old.jsonl").to_str().unwrap()]).unwrap();
        let index =
            super::super::usage_source::ProviderIndex::load_roots(&db, "pi", &[default, custom])
                .unwrap();
        assert!(
            !index.origin_empty(),
            "an old custom-root checkpoint is positive history even if the default root is unused"
        );
    }
    #[test]
    fn pi_multiroot_custom_checkpoint_insert_after_prepare_is_rechecked_in_the_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().join("default");
        let custom = temp.path().join("custom");
        let path = write(&default, "base.jsonl", &parent());
        let db = Database::memory().unwrap();
        let index = super::super::usage_source::ProviderIndex::load_roots(
            &db,
            "pi",
            &[default, custom.clone()],
        )
        .unwrap();
        let prepared = super::super::usage_source::Prepared::with_index(
            &db,
            super::super::usage_source::Snapshot::file(&path).unwrap(),
            None,
            1,
            &index,
        )
        .unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO session_log_sync(file_path,last_modified,last_line_offset,last_synced_at) VALUES(?1,1,3,1)",
            [custom.join("old.jsonl").to_str().unwrap()],
        )
        .unwrap();
        assert!(prepared.check_on(&conn).is_err());
    }
    #[test]
    fn pi_multiroot_checkpoint_path_prefix_sibling_does_not_poison_this_namespace() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().join("default");
        let custom = temp.path().join("custom");
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO session_log_sync(file_path,last_modified,last_line_offset,last_synced_at) VALUES(?1,1,3,1)",[temp.path().join("custom-other/old.jsonl").to_str().unwrap()]).unwrap();
        let index =
            super::super::usage_source::ProviderIndex::load_roots(&db, "pi", &[default, custom])
                .unwrap();
        assert!(index.origin_empty());
    }
    #[test]
    #[serial_test::serial]
    fn pi_sandbox_clears_the_native_custom_session_directory_environment() {
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("PI_CODING_AGENT_SESSION_DIR", v),
                    None => std::env::remove_var("PI_CODING_AGENT_SESSION_DIR"),
                }
            }
        }
        let _restore = Restore(std::env::var_os("PI_CODING_AGENT_SESSION_DIR"));
        let outside = tempfile::tempdir().unwrap();
        std::env::set_var("PI_CODING_AGENT_SESSION_DIR", outside.path());
        let _home = crate::test_sandbox::sandbox();
        assert!(
            std::env::var_os("PI_CODING_AGENT_SESSION_DIR").is_none(),
            "a scan must never inherit a developer's real custom session directory"
        );
    }
    #[test]
    fn pi_nested_forks_keep_original_spending_and_only_each_native_child_own_event() {
        let temp = tempfile::tempdir().unwrap();
        let p = write(temp.path(), "base.jsonl", &parent());
        let child = fork(temp.path(), &p);
        let mut rows: Vec<Value> = std::fs::read_to_string(&child)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        rows[0]["id"] = json!("grandchild");
        rows[0]["parentSession"] = json!(child);
        rows.push(call("a4", Some("a3"), [3, 1, 0, 0]));
        write(temp.path(), "grandchild.jsonl", &rows);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (4, 70, 14, 110, 20));
    }
    #[test]
    fn pi_fork_native_label_removal_rechains_the_copied_prefix_without_double_spending() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2]["parentId"] = json!("label1");
        rows.insert(2,json!({"type":"label","id":"label1","parentId":"u1","timestamp":ISO,"targetId":"u1","label":"synthetic label"}));
        let p = write(temp.path(), "base.jsonl", &rows);
        fork(temp.path(), &p);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (3, 67, 13, 110, 20));
    }
    #[test]
    fn pi_parent_move_preserves_old_child_facts_but_does_not_claim_new_fork_ownership() {
        let temp = tempfile::tempdir().unwrap();
        let p = write(temp.path(), "base.jsonl", &parent());
        let child = fork(temp.path(), &p);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        let moved = p.with_file_name("moved.jsonl");
        std::fs::rename(&p, &moved).unwrap();
        let mut child_rows: Vec<Value> = std::fs::read_to_string(&child)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        child_rows.push(call("a4", Some("a3"), [3, 1, 0, 0]));
        write(temp.path(), "child.jsonl", &child_rows);
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (3, 67, 13, 110, 20));
        child_rows[0]["parentSession"] = json!(moved);
        write(temp.path(), "child.jsonl", &child_rows);
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (4, 70, 14, 110, 20));
    }
    #[test]
    fn pi_partial_utf8_tail_preserves_the_native_complete_prefix() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let path = write(temp.path(), "base.jsonl", &parent());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[b'{', b'"', 0xe4, 0xb8])
            .unwrap();
        let read = read_pi(&path).unwrap();
        assert_eq!(read.facts.len(), 2);
        assert_eq!(read.line_count, 4);
        assert!(read.reasons.contains("incompleteEnumeration"));
    }
    #[test]
    fn pi_native_zero_old_gap_fill_prune_and_move_keep_the_same_positive_identities() {
        let temp = tempfile::tempdir().unwrap();
        let old: i64 = 1_577_836_800_123;
        let mut rows = vec![
            header("base"),
            call("zero", None, [0; 4]),
            call("a", Some("zero"), [40, 10, 100, 20]),
        ];
        for r in &mut rows[1..] {
            r["message"]["timestamp"] = json!(old);
        }
        let path = write(temp.path(), "base.jsonl", &rows);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(db.rollup_and_prune(30).unwrap(), 2);
        let mut gap = call("old-gap", Some("a"), [20, 2, 10, 0]);
        gap["message"]["timestamp"] = json!(old + 1);
        rows.push(gap);
        write(temp.path(), "base.jsonl", &rows);
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(db.rollup_and_prune(30).unwrap(), 1);
        std::fs::rename(&path, path.with_file_name("moved.jsonl")).unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db).0, 0);
        let rollup:(u64,u64,u64)=db.conn.lock().unwrap().query_row("SELECT SUM(request_count),SUM(input_tokens),SUM(output_tokens) FROM usage_daily_rollups WHERE app_type='pi'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(rollup, (3, 60, 12));
    }
    #[test]
    fn pi_conflicting_repeated_id_inside_one_source_is_not_a_complete_last_winner() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        let mut conflicting = rows[2].clone();
        conflicting["message"]["usage"]["input"] = json!(80);
        rows.push(conflicting);
        let path = write(temp.path(), "base.jsonl", &rows);
        assert!(
            read_pi(&path).is_err(),
            "two conflicting native records cannot silently select the later value"
        );
    }
    #[test]
    fn pi_identical_repeated_id_inside_one_source_has_one_native_fact() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.push(rows[2].clone());
        let read = read_pi(&write(temp.path(), "base.jsonl", &rows)).unwrap();
        assert_eq!(read.facts.len(), 2);
        assert_eq!(
            read.facts.iter().map(|f| f.usage.input_tokens).sum::<u32>(),
            47
        );
    }
    #[cfg(unix)]
    fn assert_fifo_rejected(name: &str, action: impl FnOnce(PathBuf) -> bool + Send + 'static) {
        use std::{
            ffi::CString,
            os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
            sync::mpsc,
            time::Duration,
        };
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(name);
        let native = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(native.as_ptr(), 0o600) }, 0);
        let (tx, rx) = mpsc::channel();
        let thread_path = path.clone();
        let worker = std::thread::spawn(move || {
            tx.send(action(thread_path)).unwrap();
        });
        let (bounded, result) = match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(result) => (true, result),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // RW nonblocking opens without another endpoint. Unlink before
                // closing so a not-yet-started reader cannot become orphaned.
                let release = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&path)
                    .unwrap();
                std::fs::remove_file(&path).unwrap();
                drop(release);
                (
                    false,
                    rx.recv_timeout(Duration::from_secs(5))
                        .expect("the temporary FIFO reader must be released"),
                )
            }
            Err(error) => panic!("fixture reader disconnected: {error}"),
        };
        worker.join().unwrap();
        assert!(
            bounded,
            "a non-regular source must be rejected without waiting for a writer"
        );
        assert!(
            result,
            "a FIFO is unknown input, never empty settings or a session"
        );
    }
    #[test]
    #[cfg(unix)]
    fn pi_usage_resolver_fifo_is_rejected_without_waiting_for_a_writer() {
        assert_fifo_rejected("settings.json", |path| {
            crate::session_manager::providers::pi::usage_roots_in(path.parent().unwrap(), None)
                .is_err()
        });
    }
    #[test]
    #[cfg(unix)]
    fn pi_native_source_fifo_is_rejected_without_waiting_for_a_writer() {
        assert_fifo_rejected("source.jsonl", |path| read_pi(&path).is_err());
    }
    #[test]
    fn pi_usage_resolver_oversized_regular_settings_is_an_error() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::File::create(temp.path().join("settings.json"))
            .unwrap()
            .set_len(2 * 1024 * 1024)
            .unwrap();
        assert!(crate::session_manager::providers::pi::usage_roots_in(temp.path(), None).is_err());
    }
    #[test]
    fn pi_many_short_forks_do_not_rebuild_the_large_parent_index_each_time() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = parent();
        for i in 0..2000 {
            rows.push(call(&format!("extra-{i}"), Some("u1"), [0; 4]));
        }
        let path = write(temp.path(), "parent.jsonl", &rows);
        let parent = read_pi_raw(&path).unwrap();
        let mut copied = 0;
        for i in 0..200 {
            let mut child = vec![
                header(&format!("child-{i}")),
                user("u1"),
                call("a1", Some("u1"), [40, 10, 100, 20]),
                call("own", Some("a1"), [1, 1, 0, 0]),
            ];
            child[0]["parentSession"] = json!(path);
            let child =
                read_pi_raw(&write(temp.path(), &format!("child-{i}.jsonl"), &child)).unwrap();
            copied += child.entries.len();
            assert!(inherited_prefix(&child, &parent).is_some());
        }
        assert!(
            parent.index_work.get() <= parent.entries.len() + copied,
            "parent index work {} must be bounded by total entries",
            parent.index_work.get()
        );
    }
    #[test]
    fn pi_many_forks_do_not_rewalk_the_same_long_native_label_ancestor_chain() {
        let temp = tempfile::tempdir().unwrap();
        let mut rows = vec![header("base"), user("u1")];
        let mut previous = "u1".to_owned();
        for i in 0..4000 {
            let id = format!("label-{i}");
            rows.push(json!({"type":"label","id":id,"parentId":previous,"timestamp":ISO,"targetId":"u1","label":"synthetic label"}));
            previous = id;
        }
        rows.push(call("a1", Some(&previous), [40, 10, 100, 20]));
        let path = write(temp.path(), "parent.jsonl", &rows);
        let parent = read_pi_raw(&path).unwrap();
        let mut copied = 0;
        for i in 0..200 {
            let mut child = vec![
                header(&format!("child-{i}")),
                user("u1"),
                call("a1", Some("u1"), [40, 10, 100, 20]),
                call("own", Some("a1"), [1, 1, 0, 0]),
            ];
            child[0]["parentSession"] = json!(path);
            let child =
                read_pi_raw(&write(temp.path(), &format!("child-{i}.jsonl"), &child)).unwrap();
            copied += child.entries.len();
            assert!(inherited_prefix(&child, &parent).is_some());
        }
        assert!(
            parent.label_work.get() <= 2 * (parent.entries.len() + copied),
            "label ancestor work {} must be bounded by total entries",
            parent.label_work.get()
        );
    }
    #[test]
    fn pi_fixed_official_0844_native_api_roundtrip_imports_three_assistant_events_once() {
        let evidence: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/pi-native-0.84.4.json"))
                .unwrap();
        assert_eq!(evidence["version"], "0.84.4");
        assert_eq!(evidence["schemaVersion"], 3);
        assert_eq!(evidence["installedCliVerified"], false);
        assert_eq!(evidence["validJsonAtEof"]["accepted"], true);
        let temp = tempfile::tempdir().unwrap();
        let parent = evidence["parent"].as_array().unwrap().clone();
        let p = write(temp.path(), "parent.jsonl", &parent);
        let mut child = evidence["child"].as_array().unwrap().clone();
        child[0]["parentSession"] = json!(p);
        assert!(parent
            .iter()
            .all(|r| r["timestamp"] == parent[0]["timestamp"]));
        assert!(child
            .iter()
            .all(|r| r["timestamp"] == parent[0]["timestamp"]));
        write(temp.path(), "child.jsonl", &child);
        let db = Database::memory().unwrap();
        sync_pi_usage_at(&db, &[temp.path().to_owned()]).unwrap();
        assert_eq!(totals(&db), (3, 67, 13, 110, 20));
        assert!(!dump(&db).contains("synthetic response"));
    }
}
