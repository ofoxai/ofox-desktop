//! Machine-local, independently revisioned cache parts. Callers own the DB
//! transaction: these helpers neither lock Database nor commit a connection.
use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, time::UNIX_EPOCH};

const FORMAT: u64 = 1;
const PROVIDER_PREFIX: &str = "usage_provider_evidence_v1:";
pub(crate) const MAX_RECEIPTS: usize = 20_000;
pub(crate) const MAX_PROVIDER_ACKNOWLEDGED_EVENTS: usize = 1_000_000;
const MAX_PROVIDER_BYTES: usize = 4 * 1024 * 1024;
const MAX_ENVELOPE_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_ACKNOWLEDGED_EVENTS: usize = 100_000;
pub(crate) const MAX_USAGE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const LEDGER_EPOCH_KEY: &str = "usage_ledger_epoch_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceStamp {
    pub size: u64,
    pub mtime_ns: String,
    pub physical_identity: String,
    pub change_ns: Option<String>,
}
impl SourceStamp {
    #[cfg(unix)]
    fn from_metadata(meta: &fs::Metadata) -> Result<Self, AppError> {
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            size: meta.len(),
            mtime_ns: meta
                .modified()
                .map_err(|_| invalid("source time"))?
                .duration_since(UNIX_EPOCH)
                .map_err(|_| invalid("source time"))?
                .as_nanos()
                .to_string(),
            physical_identity: format!("{}:{}", meta.dev(), meta.ino()),
            change_ns: Some(format!("{}:{}", meta.ctime(), meta.ctime_nsec())),
        })
    }
    pub(crate) fn at(path: &Path) -> Result<Self, AppError> {
        #[cfg(unix)]
        {
            Self::from_metadata(&fs::metadata(path).map_err(|_| invalid("source metadata"))?)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
                .open(path)
                .map_err(|_| invalid("source open"))?;
            Self::of(&file)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Self::of(&fs::File::open(path).map_err(|_| invalid("source open"))?)
        }
    }
    pub(crate) fn of(file: &fs::File) -> Result<Self, AppError> {
        #[cfg(unix)]
        {
            Self::from_metadata(&file.metadata().map_err(|_| invalid("source metadata"))?)
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows::Win32::{
                Foundation::HANDLE,
                Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
            };
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
                .map_err(|_| invalid("source identity"))?;
            let meta = file.metadata().map_err(|_| invalid("source metadata"))?;
            Ok(Self {
                size: meta.len(),
                mtime_ns: meta
                    .modified()
                    .map_err(|_| invalid("source time"))?
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| invalid("source time"))?
                    .as_nanos()
                    .to_string(),
                physical_identity: format!(
                    "{}:{}:{}",
                    info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
                ),
                change_ns: None,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = file;
            Err(invalid("unsupported source identity"))
        }
    }
    pub(crate) fn verify(&self, path: &Path) -> Result<(), AppError> {
        if &Self::at(path)? != self {
            return Err(invalid("source changed; retry"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Part {
    Metadata,
    UsageSync,
    Analytics,
    Lifetime,
}
impl Part {
    fn key(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::UsageSync => "usageSync",
            Self::Analytics => "analytics",
            Self::Lifetime => "lifetimeUsage",
        }
    }
}
pub(crate) struct CacheParts {
    root: Value,
    ledger_epoch: Option<String>,
}
impl CacheParts {
    pub(crate) fn token(&self, part: Part) -> Option<String> {
        let revision = self.root["parts"][part.key()]["revision"]
            .as_str()
            .map(str::to_owned);
        if matches!(part, Part::UsageSync) {
            if let Some(epoch) = &self.ledger_epoch {
                return Some(format!("usageEpoch:{}", json!([epoch, revision])));
            }
        }
        revision
    }
    pub(crate) fn receipt(&self) -> Option<Value> {
        let wrapper = self.root["parts"].get("usageSync")?;
        Some(
            json!({"revision":wrapper["revision"],"digest":value_digest(wrapper),
            "acknowledgedEvents":wrapper["value"]["acknowledgedEventIds"].as_array().map_or(0,Vec::len)}),
        )
    }
    pub(crate) fn value(&self, part: Part) -> Option<&Value> {
        self.root["parts"][part.key()]
            .get("value")
            .filter(|v| !v.is_null())
    }
}
fn invalid(reason: &str) -> AppError {
    AppError::Database(format!("session source cache: {reason}"))
}
fn sql_error(e: rusqlite::Error) -> AppError {
    invalid(&e.to_string())
}

pub(crate) fn read_on(
    conn: &Connection,
    path: &str,
    provider: &str,
) -> Result<CacheParts, AppError> {
    let ledger_epoch = conn
        .query_row(
            "SELECT value FROM maintenance_flags WHERE key=?1",
            [LEDGER_EPOCH_KEY],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(sql_error)?;
    let row:Option<CacheRow>=conn.query_row(
        "SELECT provider_id,stamp,parser_version,meta_json FROM session_meta_cache WHERE source_path=?1",
        [path],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql_error)?;
    let Some(row) = row else {
        return Ok(CacheParts {
            root: json!({"formatVersion":FORMAT,"parts":{}}),
            ledger_epoch,
        });
    };
    let mut parts = decode_row(row, provider)?;
    parts.ledger_epoch = ledger_epoch;
    Ok(parts)
}

pub(crate) type CacheRow = (Option<String>, Option<String>, Option<i64>, Option<String>);
/// Shared by point reads and the metadata listing's one batch SELECT.
pub(crate) fn decode_row(row: CacheRow, provider: &str) -> Result<CacheParts, AppError> {
    let (owner, stamp, version, raw) = row;
    if owner.as_deref().is_some_and(|p| p != provider) {
        return Err(invalid("provider conflict"));
    }
    let raw = raw.ok_or_else(|| invalid("missing JSON"))?;
    if raw.len() > MAX_ENVELOPE_BYTES {
        return Err(invalid("envelope exceeds byte budget"));
    }
    let mut root: Value = serde_json::from_str(&raw).map_err(|_| invalid("invalid JSON"))?;
    let object = root
        .as_object_mut()
        .ok_or_else(|| invalid("expected object"))?;
    if let Some(format) = object.get("formatVersion") {
        if format.as_u64() != Some(FORMAT) {
            return Err(invalid("unsupported envelope version"));
        }
        let parts = object
            .get("parts")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("invalid parts"))?;
        for part in [
            Part::Metadata,
            Part::UsageSync,
            Part::Analytics,
            Part::Lifetime,
        ] {
            if let Some(v) = parts.get(part.key()) {
                if v.get("revision")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                    || v.get("value").is_none()
                {
                    return Err(invalid("invalid part"));
                }
            }
        }
    } else {
        if object.contains_key("parts") {
            return Err(invalid("missing envelope version"));
        }
        let metadata = object.contains_key("candidates") || object.contains_key("messageCount");
        let details = Value::Object(object.clone());
        object.insert("formatVersion".into(), json!(FORMAT));
        let mut parts = Map::new();
        if metadata {
            let identity = format!("{raw}:{stamp:?}:{version:?}");
            parts.insert(
                "metadata".into(),
                json!({"revision":format!("legacy:{:x}",Sha256::digest(identity.as_bytes())),
                "value":{"parserVersion":version.unwrap_or(0),"stamp":stamp,"details":details}}),
            );
        }
        object.insert("parts".into(), Value::Object(parts));
    }
    Ok(CacheParts {
        root,
        ledger_epoch: None,
    })
}

/// The roster contains receipt hashes, never a second copy of native event IDs.
/// A generation is a CAS fence; only an intact roster rooted in an empty
/// ledger may supply negative evidence. Loss of proof is deliberately sticky.
pub(crate) fn provider_on(conn: &Connection, provider: &str) -> Result<Option<Value>, AppError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM maintenance_flags WHERE key=?1",
            [format!("{PROVIDER_PREFIX}{provider}")],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    raw.map(|raw| {
        if raw.len() > MAX_PROVIDER_BYTES {
            return Err(invalid("provider proof exceeds budget"));
        }
        let value: Value =
            serde_json::from_str(&raw).map_err(|_| invalid("invalid provider proof"))?;
        if value["version"].as_u64() != Some(1)
            || value["generation"].as_str().is_none_or(str::is_empty)
            || value["receipts"]
                .as_object()
                .is_none_or(|r| r.len() > MAX_RECEIPTS)
            || !value["originEmpty"].is_boolean()
            || !value["complete"].is_boolean()
        {
            return Err(invalid("unsupported provider proof"));
        }
        Ok(value)
    })
    .transpose()
}
pub(crate) fn provider_generation(value: &Option<Value>) -> Option<String> {
    value
        .as_ref()
        .and_then(|v| v["generation"].as_str())
        .map(str::to_owned)
}
pub(crate) fn ledger_epoch_on(conn: &Connection) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT value FROM maintenance_flags WHERE key=?1",
        [LEDGER_EPOCH_KEY],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql_error)
}
pub(crate) fn receipt_key(path: &str) -> String {
    format!("{:x}", Sha256::digest(path.as_bytes()))
}
fn value_digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
fn write_provider_on(conn: &Connection, provider: &str, value: &Value) -> Result<(), AppError> {
    let raw = serde_json::to_string(value).map_err(|_| invalid("serialize provider proof"))?;
    if raw.len() > MAX_PROVIDER_BYTES {
        return Err(invalid("provider proof exceeds budget"));
    }
    conn.execute(
        "INSERT OR REPLACE INTO maintenance_flags(key,value) VALUES(?1,?2)",
        params![format!("{PROVIDER_PREFIX}{provider}"), raw],
    )
    .map_err(sql_error)?;
    Ok(())
}
fn new_provider(epoch: Option<String>, origin_empty: bool) -> Value {
    json!({"version":1,"generation":uuid::Uuid::new_v4().to_string(),"ledgerEpoch":epoch,
        "originEmpty":origin_empty,"complete":origin_empty,"receipts":{}})
}
pub(crate) fn bootstrap_provider_on(
    conn: &Connection,
    provider: &str,
    origin_empty: bool,
) -> Result<(), AppError> {
    if provider_on(conn, provider)?.is_none() {
        write_provider_on(
            conn,
            provider,
            &new_provider(ledger_epoch_on(conn)?, origin_empty),
        )?;
    }
    Ok(())
}
pub(crate) fn invalidate_provider_on(
    conn: &Connection,
    provider: &str,
    reason: &str,
) -> Result<(), AppError> {
    let epoch = ledger_epoch_on(conn)?;
    let mut manifest = provider_on(conn, provider)?.unwrap_or_else(|| new_provider(epoch, false));
    manifest["complete"] = json!(false);
    manifest["generation"] = json!(uuid::Uuid::new_v4().to_string());
    let reasons = manifest
        .as_object_mut()
        .ok_or_else(|| invalid("provider proof"))?
        .entry("reasons")
        .or_insert_with(|| json!({}));
    reasons
        .as_object_mut()
        .ok_or_else(|| invalid("provider reasons"))?
        .insert(reason.into(), json!(true));
    write_provider_on(conn, provider, &manifest)
}

fn record_receipt_on(
    conn: &Connection,
    path: &str,
    provider: &str,
    current: &CacheParts,
) -> Result<(), AppError> {
    let epoch = ledger_epoch_on(conn)?;
    let mut manifest =
        provider_on(conn, provider)?.unwrap_or_else(|| new_provider(epoch.clone(), false));
    if manifest["ledgerEpoch"] != json!(epoch) {
        manifest["complete"] = json!(false);
    }
    manifest["ledgerEpoch"] = json!(epoch);
    manifest["generation"] = json!(uuid::Uuid::new_v4().to_string());
    let value = current.value(Part::UsageSync);
    if value.is_none_or(|v| v["trusted"] != true || v["completeEventIdentitySet"] != true) {
        manifest["complete"] = json!(false);
    }
    let receipts = manifest["receipts"]
        .as_object_mut()
        .ok_or_else(|| invalid("provider receipts"))?;
    let key = receipt_key(path);
    if receipts.contains_key(&key) || receipts.len() < MAX_RECEIPTS {
        receipts.insert(
            key,
            current
                .receipt()
                .ok_or_else(|| invalid("missing usage receipt"))?,
        );
    } else {
        manifest["complete"] = json!(false);
        manifest["receiptBudget"] = json!(true);
    }
    let total = manifest["receipts"]
        .as_object()
        .ok_or_else(|| invalid("provider receipts"))?
        .values()
        .try_fold(0u64, |n, r| {
            n.checked_add(r["acknowledgedEvents"].as_u64().unwrap_or(u64::MAX))
        })
        .unwrap_or(u64::MAX);
    if total > MAX_PROVIDER_ACKNOWLEDGED_EVENTS as u64 {
        manifest["complete"] = json!(false);
        manifest["acknowledgementBudget"] = json!(true);
    }
    // The hard byte ceiling rejects the entire caller transaction; no receipt
    // or fact can become visible without the matching generation write.
    write_provider_on(conn, provider, &manifest)
}

fn validate_owned(current: &CacheParts, part: Part) -> Result<(), AppError> {
    if let Some(value) = current.value(part) {
        if !value.is_object() {
            return Err(invalid("invalid owned part"));
        }
        match part {
            Part::UsageSync if value["version"].as_u64() != Some(1) => {
                return Err(invalid("unsupported usage version"))
            }
            Part::Lifetime if value["version"].as_u64() != Some(1) => {
                return Err(invalid("unsupported lifetime version"))
            }
            Part::Metadata
                if value["parserVersion"]
                    .as_i64()
                    .is_none_or(|v| !(0..=super::meta_cache::PARSER_VERSION).contains(&v)) =>
            {
                return Err(invalid("unsupported metadata parser"))
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn check_on(
    conn: &Connection,
    path: &str,
    provider: &str,
    part: Part,
    expected: &Option<String>,
) -> Result<(), AppError> {
    let current = read_on(conn, path, provider)?;
    validate_owned(&current, part)?;
    if &current.token(part) != expected {
        return Err(invalid("part changed; retry"));
    }
    Ok(())
}
fn merge_values(before: &mut Value, next: Value) {
    match (before, next) {
        (Value::Object(old), Value::Object(new)) => {
            for (key, value) in new {
                merge_values(old.entry(key).or_insert(Value::Null), value);
            }
        }
        (old, new) => *old = new,
    }
}
pub(crate) fn merge_on(
    conn: &Connection,
    path: &str,
    provider: &str,
    session: Option<&str>,
    part: Part,
    expected: &Option<String>,
    next: Option<Value>,
) -> Result<String, AppError> {
    let mut current = read_on(conn, path, provider)?;
    validate_owned(&current, part)?;
    if &current.token(part) != expected {
        return Err(invalid("part changed; retry"));
    }
    let revision = uuid::Uuid::new_v4().to_string();
    let parts = current.root["parts"]
        .as_object_mut()
        .ok_or_else(|| invalid("invalid parts"))?;
    let wrapper = parts.entry(part.key()).or_insert_with(|| json!({}));
    let wrapper = wrapper
        .as_object_mut()
        .ok_or_else(|| invalid("invalid part"))?;
    match next {
        Some(next) => merge_values(wrapper.entry("value").or_insert(Value::Null), next),
        None => {
            wrapper.insert("value".into(), Value::Null);
        }
    }
    wrapper.insert("revision".into(), json!(revision));
    if matches!(part, Part::UsageSync) {
        let value = &wrapper["value"];
        if serde_json::to_vec(value)
            .map_err(|_| invalid("serialize usage part"))?
            .len()
            > MAX_USAGE_BYTES
            || value["acknowledgedEventIds"]
                .as_array()
                .is_some_and(|ids| ids.len() > MAX_ACKNOWLEDGED_EVENTS)
        {
            return Err(invalid("usage proof exceeds budget"));
        }
    }
    validate_owned(&current, part)?;
    let raw = serde_json::to_string(&current.root).map_err(|_| invalid("serialize envelope"))?;
    if raw.len() > MAX_ENVELOPE_BYTES {
        return Err(invalid("envelope exceeds byte budget"));
    }
    let meta = current.value(Part::Metadata);
    let stamp = meta.and_then(|v| v.get("stamp")).and_then(Value::as_str);
    let parser = meta.and_then(|v| v["parserVersion"].as_i64()).unwrap_or(0);
    conn.execute("INSERT INTO session_meta_cache(source_path,provider_id,session_id,stamp,parser_version,meta_json,updated_at)
        VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(source_path) DO UPDATE SET
        provider_id=excluded.provider_id,session_id=COALESCE(excluded.session_id,session_id),stamp=excluded.stamp,
        parser_version=excluded.parser_version,meta_json=excluded.meta_json,updated_at=excluded.updated_at",
        params![path,provider,session,stamp,parser,raw,chrono::Utc::now().timestamp_millis()]).map_err(sql_error)?;
    if matches!(part, Part::UsageSync) {
        record_receipt_on(conn, path, provider, &current)?;
    }
    current
        .token(part)
        .ok_or_else(|| invalid("missing saved revision"))
}

/// Replacing the ledger invalidates local acknowledgement evidence even if its
/// checkpoint numbers happen to match the restored database. Metadata survives.
pub(crate) fn revoke_usage_on(conn: &Connection) -> Result<(), AppError> {
    let rows = conn
        .prepare("SELECT source_path,provider_id FROM session_meta_cache")
        .map_err(sql_error)?
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    for (path, provider) in rows {
        let provider = provider.unwrap_or_default();
        let current = read_on(conn, &path, &provider)?;
        if current.value(Part::UsageSync).is_some() {
            merge_on(
                conn,
                &path,
                &provider,
                None,
                Part::UsageSync,
                &current.token(Part::UsageSync),
                Some(json!({
                "trusted":false,"coverage":"legacyUnknownBoundary","revocationReason":"ledgerRestored"})),
            )?;
        }
    }
    // Fence every pre-restore prepare, including paths/parts that did not
    // exist and therefore cannot be found by the row traversal above.
    conn.execute(
        "INSERT OR REPLACE INTO maintenance_flags(key,value) VALUES(?1,?2)",
        params![LEDGER_EPOCH_KEY, uuid::Uuid::new_v4().to_string()],
    )
    .map_err(sql_error)?;
    let providers = conn
        .prepare("SELECT key FROM maintenance_flags WHERE substr(key,1,length('usage_provider_evidence_v1:'))='usage_provider_evidence_v1:'")
        .map_err(sql_error)?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    for key in providers {
        let provider = key
            .strip_prefix(PROVIDER_PREFIX)
            .ok_or_else(|| invalid("provider key"))?;
        let mut manifest =
            provider_on(conn, provider)?.ok_or_else(|| invalid("provider proof disappeared"))?;
        manifest["ledgerEpoch"] = json!(ledger_epoch_on(conn)?);
        manifest["generation"] = json!(uuid::Uuid::new_v4().to_string());
        manifest["complete"] = json!(false);
        manifest["revocationReason"] = json!("ledgerRestored");
        write_provider_on(conn, provider, &manifest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        database::Database,
        services::usage_sync_db::{commit_sync_batch, CheckpointChange, SyncState},
    };

    #[test]
    fn foundation_part_cas_preserves_interleaved_writes_and_unknown_fields() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO session_meta_cache VALUES('source','claude','s','old',1,?1,0)",
            [json!({"candidates":{},"messageCount":1,"futureLegacy":7}).to_string()],
        )
        .unwrap();
        let before = read_on(&conn, "source", "claude").unwrap();
        let u = merge_on(
            &conn,
            "source",
            "claude",
            None,
            Part::UsageSync,
            &None,
            Some(json!({"version":1,"futureUsage":42,"trusted":true})),
        )
        .unwrap();
        merge_on(&conn,"source","claude",Some("s"),Part::Metadata,&before.token(Part::Metadata),
            Some(json!({"parserVersion":2,"stamp":"new","details":{"candidates":{},"messageCount":2}}))).unwrap();
        let after = read_on(&conn, "source", "claude").unwrap();
        assert_eq!(after.token(Part::UsageSync), Some(u));
        assert_eq!(after.root["futureLegacy"], 7);
        assert_eq!(after.value(Part::UsageSync).unwrap()["futureUsage"], 42);
        assert!(check_on(
            &conn,
            "source",
            "claude",
            Part::Metadata,
            &before.token(Part::Metadata)
        )
        .is_err());
        assert!(read_on(&conn, "source", "gemini").is_err());
    }
    #[test]
    fn foundation_clear_tombstone_prevents_absent_part_aba() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let prepared = read_on(&conn, "source", "claude").unwrap();
        assert!(prepared.token(Part::Metadata).is_none());
        let revision =
            merge_on(&conn, "source", "claude", None, Part::Metadata, &None, None).unwrap();
        assert!(merge_on(
            &conn,
            "source",
            "claude",
            None,
            Part::Metadata,
            &prepared.token(Part::Metadata),
            Some(json!({}))
        )
        .is_err());
        assert_eq!(
            read_on(&conn, "source", "claude")
                .unwrap()
                .token(Part::Metadata),
            Some(revision)
        );
    }
    #[test]
    fn foundation_cache_facts_checkpoint_failures_roll_back_together() {
        for table in [
            "session_meta_cache",
            "proxy_request_logs",
            "session_log_sync",
            "maintenance_flags",
        ] {
            let db = Database::memory().unwrap();
            db.conn.lock().unwrap().execute_batch(&format!("CREATE TEMP TRIGGER fail BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'fixture'); END;")).unwrap();
            let result = commit_sync_batch(
                &db,
                &[CheckpointChange {
                    key: "source".into(),
                    expected: None,
                    next: Some(SyncState {
                        modified: 1,
                        offset: 2,
                    }),
                }],
                |conn| {
                    check_on(conn, "source", "claude", Part::UsageSync, &None)?;
                    conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,latency_ms,status_code,created_at)
                    VALUES('fact','fixture','claude','fixture',0,200,1)",[])?;
                    merge_on(
                        conn,
                        "source",
                        "claude",
                        None,
                        Part::UsageSync,
                        &None,
                        Some(json!({"version":1,"acknowledgedEventIds":["native"]})),
                    )?;
                    Ok(())
                },
            );
            assert!(result.is_err());
            let conn = db.conn.lock().unwrap();
            for target in [
                "session_meta_cache",
                "proxy_request_logs",
                "session_log_sync",
                "maintenance_flags",
            ] {
                let n: i64 = conn
                    .query_row(&format!("SELECT COUNT(*) FROM {target}"), [], |r| r.get(0))
                    .unwrap();
                assert_eq!(n, 0, "{table} left {target}");
            }
        }
    }
    #[test]
    fn foundation_usage_proof_rejects_an_over_budget_write_without_replacing_evidence() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let revision = merge_on(
            &conn,
            "source",
            "claude",
            None,
            Part::UsageSync,
            &None,
            Some(json!({"version":1,"acknowledgedEventIds":["native"]})),
        )
        .unwrap();
        assert!(merge_on(
            &conn,
            "source",
            "claude",
            None,
            Part::UsageSync,
            &Some(revision.clone()),
            Some(json!({"extra":"x".repeat(MAX_USAGE_BYTES)}))
        )
        .is_err());
        let after = read_on(&conn, "source", "claude").unwrap();
        assert_eq!(after.token(Part::UsageSync), Some(revision));
        assert_eq!(
            after.value(Part::UsageSync).unwrap()["acknowledgedEventIds"],
            json!(["native"])
        );
    }
}
