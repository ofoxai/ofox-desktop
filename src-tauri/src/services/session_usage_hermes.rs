//! Read-only Hermes cumulative counters. Never produces ledger event facts.
//! Source contract: SessionDB 0.21.7+18.g590af66, schema 31. Only statistics
//! columns are selected. Current routing and transcript bodies are never read.
use super::usage_lifetime::{
    LifetimeAmountKind, LifetimeBuckets, LifetimeCategory, LifetimeCoverage, LifetimeModelFact,
    LifetimeNativeSnapshot,
};
use crate::{error::AppError, session_manager::source_cache::SourceStamp};
use rusqlite::{types::ValueRef, Connection, OpenFlags, Row};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ROWS: usize = 200_000;
const MAX_LABEL_BYTES: usize = 1024;

fn invalid(code: &str) -> AppError {
    AppError::Database(format!("usage.hermes.{code}"))
}

struct Component {
    path: PathBuf,
    file: File,
    stamp: SourceStamp,
}
impl Component {
    fn open(path: PathBuf) -> Result<Self, AppError> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC);
        }
        let file = options
            .open(&path)
            .map_err(|_| invalid("sourceUnreadable"))?;
        if !file
            .metadata()
            .map_err(|_| invalid("sourceUnreadable"))?
            .is_file()
        {
            return Err(invalid("sourceUnreadable"));
        }
        let stamp = SourceStamp::of(&file).map_err(|_| invalid("sourceUnreadable"))?;
        if stamp.size > MAX_SOURCE_BYTES {
            return Err(invalid("sourceTooLarge"));
        }
        Ok(Self { path, file, stamp })
    }
    fn verify(&self) -> Result<(), AppError> {
        self.stamp
            .verify(&self.path)
            .map_err(|_| invalid("sourceChanged"))?;
        if SourceStamp::of(&self.file).map_err(|_| invalid("sourceChanged"))? != self.stamp {
            return Err(invalid("sourceChanged"));
        }
        Ok(())
    }
    fn copy_to(&mut self, destination: &Path) -> Result<(), AppError> {
        let mut target = File::create(destination).map_err(|_| invalid("snapshotFailed"))?;
        let copied = std::io::copy(&mut (&mut self.file).take(self.stamp.size + 1), &mut target)
            .map_err(|_| invalid("snapshotFailed"))?;
        target.flush().map_err(|_| invalid("snapshotFailed"))?;
        if copied != self.stamp.size {
            return Err(invalid("sourceChanged"));
        }
        self.verify()
    }
}

fn exists(path: &Path) -> Result<bool, AppError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(invalid("sourceUnreadable")),
    }
}

struct SourceFiles {
    main: Component,
    wal: Option<Component>,
    root: PathBuf,
}
impl SourceFiles {
    fn open(root: &Path) -> Result<Self, AppError> {
        let main = root.join("state.db");
        if !exists(&main)? {
            return Err(invalid("sourceMissing"));
        }
        // A rollback writer may hold stable but uncommitted main pages while
        // paused. Without its journal that is not a valid native snapshot.
        if exists(&root.join("state.db-journal"))? {
            return Err(invalid("sourceBusy"));
        }
        let main = Component::open(main)?;
        let wal_path = root.join("state.db-wal");
        let wal = exists(&wal_path)?
            .then(|| Component::open(wal_path))
            .transpose()?;
        if main
            .stamp
            .size
            .checked_add(wal.as_ref().map_or(0, |f| f.stamp.size))
            .is_none_or(|size| size > MAX_SOURCE_BYTES)
        {
            return Err(invalid("sourceTooLarge"));
        }
        let files = Self {
            main,
            wal,
            root: root.to_owned(),
        };
        files.verify()?;
        Ok(files)
    }
    fn stamp(&self) -> Value {
        json!({"database":self.main.stamp,"wal":self.wal.as_ref().map(|f| &f.stamp)})
    }
    fn verify(&self) -> Result<(), AppError> {
        self.main.verify()?;
        if exists(&self.root.join("state.db-journal"))?
            || exists(&self.root.join("state.db-wal"))? != self.wal.is_some()
        {
            return Err(invalid("sourceChanged"));
        }
        if let Some(wal) = &self.wal {
            wal.verify()?;
        }
        Ok(())
    }
}

pub(crate) fn current_stamp(root: &Path) -> Result<Value, AppError> {
    Ok(SourceFiles::open(root)?.stamp())
}

#[derive(Deserialize, PartialEq, Eq)]
struct NativeSourceStamp {
    database: SourceStamp,
    wal: Option<SourceStamp>,
}

/// This cache owns the understood file identity fields. Opaque peer
/// extensions are retained by source_cache and do not invalidate a read.
/// Ledger proof/hash comparisons deliberately do not use this helper.
pub(crate) fn same_stamp(left: &Value, right: &Value) -> bool {
    match (
        serde_json::from_value::<NativeSourceStamp>(left.clone()),
        serde_json::from_value::<NativeSourceStamp>(right.clone()),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

pub(crate) fn verify_stamp(root: &Path, stamp: &Value) -> Result<(), AppError> {
    if same_stamp(&current_stamp(root)?, stamp) {
        Ok(())
    } else {
        Err(invalid("sourceChanged"))
    }
}

pub fn read_lifetime(root: &Path) -> Result<LifetimeNativeSnapshot, AppError> {
    read_snapshot(root, || {})
}

#[cfg(test)]
pub(crate) fn read_lifetime_after_copy(
    root: &Path,
    after_copy: impl FnOnce(),
) -> Result<LifetimeNativeSnapshot, AppError> {
    read_snapshot(root, after_copy)
}

fn read_snapshot(
    root: &Path,
    after_copy: impl FnOnce(),
) -> Result<LifetimeNativeSnapshot, AppError> {
    let mut files = SourceFiles::open(root)?;
    let temporary = tempfile::tempdir().map_err(|_| invalid("snapshotFailed"))?;
    let copy = temporary.path().join("state.db");
    files.main.copy_to(&copy)?;
    if let Some(wal) = &mut files.wal {
        wal.copy_to(&temporary.path().join("state.db-wal"))?;
    }
    files.verify()?;
    after_copy();
    let conn = Connection::open_with_flags(
        &copy,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| invalid("invalidDatabase"))?;
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(|_| invalid("invalidDatabase"))?;
    conn.pragma_update(None, "query_only", true)
        .map_err(|_| invalid("invalidDatabase"))?;
    let data_version: i64 = conn
        .pragma_query_value(None, "data_version", |r| r.get(0))
        .map_err(|_| invalid("invalidDatabase"))?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|_| invalid("invalidDatabase"))?;
    let version: i64 = tx
        .query_row(
            "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| invalid("unsupportedSchema"))?;
    if version != 31 {
        return Err(invalid("unsupportedSchema"));
    }
    let mut result = read_counters(&tx)?;
    tx.commit().map_err(|_| invalid("invalidDatabase"))?;
    let after_version: i64 = conn
        .pragma_query_value(None, "data_version", |r| r.get(0))
        .map_err(|_| invalid("invalidDatabase"))?;
    if data_version != after_version {
        return Err(invalid("sourceChanged"));
    }
    files.verify()?;
    result.source_stamp = files.stamp();
    Ok(result)
}

#[derive(Clone)]
struct Counts([Option<u64>; 5]);
impl Default for Counts {
    fn default() -> Self {
        Self([Some(0); 5])
    }
}
impl Counts {
    fn row(row: &Row<'_>, start: usize, reasons: &mut BTreeSet<String>) -> Result<Self, AppError> {
        let mut out = Self::default();
        for (index, value) in out.0.iter_mut().enumerate() {
            *value = match row
                .get_ref(start + index)
                .map_err(|_| invalid("invalidDatabase"))?
            {
                ValueRef::Integer(value) if value >= 0 => Some(value as u64),
                ValueRef::Null => {
                    reasons.insert("missingCounters".into());
                    None
                }
                _ => {
                    reasons.insert("invalidCounters".into());
                    None
                }
            };
        }
        Ok(out)
    }
    fn add(&mut self, other: &Self, reasons: &mut BTreeSet<String>) {
        for (left, right) in self.0.iter_mut().zip(other.0) {
            let next = left.zip(right).and_then(|(a, b)| a.checked_add(b));
            if left.is_some() && right.is_some() && next.is_none() {
                reasons.insert("counterOverflow".into());
            }
            *left = next;
        }
    }
    fn subtract(&self, other: &Self) -> Option<Self> {
        let mut out = Self::default();
        for (index, (left, right)) in self.0.iter().zip(other.0).enumerate() {
            out.0[index] = Some((*left)?.checked_sub(right?)?);
        }
        Some(out)
    }
    fn nonzero(&self) -> bool {
        self.0.iter().any(|v| v.is_none_or(|n| n > 0))
    }
    fn buckets(&self, lower_bound: bool) -> LifetimeBuckets {
        let total = self.0[..2]
            .iter()
            .try_fold(0u64, |sum, n| sum.checked_add((*n)?));
        LifetimeBuckets {
            input_tokens: self.0[0],
            output_tokens: self.0[1],
            cache_read_tokens: self.0[2],
            cache_write_tokens: self.0[3],
            total_tokens: total,
            api_call_count: self.0[4],
            amount_kind: if lower_bound || self.0.iter().any(Option::is_none) || total.is_none() {
                LifetimeAmountKind::LowerBound
            } else {
                LifetimeAmountKind::Known
            },
        }
    }
}

struct ModelRow {
    model: Option<String>,
    category: LifetimeCategory,
    counts: Counts,
}
struct SessionPool {
    counts: Counts,
    included: Vec<ModelRow>,
}

fn native_label(row: &Row<'_>, index: usize) -> Result<String, AppError> {
    let label: String = row.get(index).map_err(|_| invalid("invalidDatabase"))?;
    if label.len() > MAX_LABEL_BYTES {
        return Err(invalid("sourceTooLarge"));
    }
    Ok(label)
}

fn read_counters(conn: &Connection) -> Result<LifetimeNativeSnapshot, AppError> {
    let mut reasons = BTreeSet::new();
    let mut sessions = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT id,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,api_call_count FROM sessions LIMIT ?1").map_err(|_| invalid("unsupportedSchema"))?;
    let mut rows = stmt
        .query([(MAX_ROWS + 1) as i64])
        .map_err(|_| invalid("invalidDatabase"))?;
    while let Some(row) = rows.next().map_err(|_| invalid("invalidDatabase"))? {
        if sessions.len() >= MAX_ROWS {
            return Err(invalid("sourceTooLarge"));
        }
        let id = native_label(row, 0)?;
        if id.is_empty() || sessions.contains_key(&id) {
            return Err(invalid("invalidDatabase"));
        }
        sessions.insert(
            id,
            SessionPool {
                counts: Counts::row(row, 1, &mut reasons)?,
                included: vec![],
            },
        );
    }
    let mut auxiliary = Vec::new();
    let mut lower_bound = false;
    let mut stmt = conn.prepare("SELECT session_id,model,task,billing_mode,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,api_call_count FROM session_model_usage LIMIT ?1").map_err(|_| invalid("unsupportedSchema"))?;
    let mut rows = stmt
        .query([(MAX_ROWS + 1) as i64])
        .map_err(|_| invalid("invalidDatabase"))?;
    let mut row_count = 0;
    while let Some(row) = rows.next().map_err(|_| invalid("invalidDatabase"))? {
        row_count += 1;
        if row_count > MAX_ROWS {
            return Err(invalid("sourceTooLarge"));
        }
        let id = native_label(row, 0)?;
        let Some(pool) = sessions.get_mut(&id) else {
            reasons.insert("orphanModelRows".into());
            lower_bound = true;
            continue;
        };
        let model = native_label(row, 1)?;
        let model = if model.is_empty() || model == "unknown" {
            reasons.insert("modelAttributionUnavailable".into());
            None
        } else {
            Some(model)
        };
        let task = native_label(row, 2)?;
        let mode = native_label(row, 3)?;
        if !matches!(
            mode.as_str(),
            "" | "subscription_included"
                | "official_models_api"
                | "official_docs_snapshot"
                | "chat_completions"
                | "responses"
                | "anthropic_messages"
        ) {
            reasons.insert("routeUnverified".into());
        }
        let category = match task.as_str() {
            "" => LifetimeCategory::Main,
            "voice_chat" => LifetimeCategory::Voice,
            "vision" | "compression" | "title_generation" | "background_review" | "web_extract"
            | "session_search" | "approval" | "mcp" => LifetimeCategory::Auxiliary,
            _ => {
                reasons.insert("unknownTask".into());
                lower_bound = true;
                continue;
            }
        };
        let counts = Counts::row(row, 4, &mut reasons)?;
        let model_row = ModelRow {
            model,
            category,
            counts,
        };
        if model_row.category == LifetimeCategory::Auxiliary {
            auxiliary.push(model_row);
        } else {
            pool.included.push(model_row);
        }
    }
    let mut total = Counts::default();
    let mut facts = Vec::new();
    for pool in sessions.values() {
        total.add(&pool.counts, &mut reasons);
        let mut included = Counts::default();
        for row in &pool.included {
            included.add(&row.counts, &mut reasons);
        }
        match pool.counts.subtract(&included) {
            Some(residual) => {
                for row in &pool.included {
                    facts.push((row.model.clone(), row.category.clone(), row.counts.clone()));
                }
                if residual.nonzero() {
                    reasons.insert("modelAttributionUnavailable".into());
                    facts.push((None, LifetimeCategory::Unattributed, residual));
                }
            }
            None => {
                reasons.insert("inconsistentModelCounters".into());
                facts.push((None, LifetimeCategory::Unattributed, pool.counts.clone()));
            }
        }
    }
    for row in auxiliary {
        total.add(&row.counts, &mut reasons);
        facts.push((row.model, row.category, row.counts));
    }
    // v20 migrations seeded model rows using then-current session routing;
    // schema 31 has no marker to separate those historical rows from native
    // per-call attribution. Never claim historical model provenance is exact.
    if !sessions.is_empty() {
        reasons.insert("historicalModelAttributionUnverified".into());
    }
    let mut grouped: BTreeMap<(Option<String>, u8), Counts> = BTreeMap::new();
    for (model, category, counts) in facts {
        let category = match category {
            LifetimeCategory::Main => 0,
            LifetimeCategory::Voice => 1,
            LifetimeCategory::Auxiliary => 2,
            LifetimeCategory::Unattributed => 3,
        };
        grouped
            .entry((model, category))
            .or_default()
            .add(&counts, &mut reasons);
    }
    let models = grouped
        .into_iter()
        .map(|((model_id, category), counts)| LifetimeModelFact {
            model_id,
            category: match category {
                0 => LifetimeCategory::Main,
                1 => LifetimeCategory::Voice,
                2 => LifetimeCategory::Auxiliary,
                _ => LifetimeCategory::Unattributed,
            },
            buckets: counts.buckets(false),
        })
        .collect();
    let totals = total.buckets(lower_bound);
    let counter_coverage = if totals.amount_kind == LifetimeAmountKind::Known {
        LifetimeCoverage::Complete
    } else {
        LifetimeCoverage::Partial
    };
    let model_coverage = if reasons.is_empty() {
        LifetimeCoverage::Complete
    } else {
        LifetimeCoverage::Partial
    };
    Ok(LifetimeNativeSnapshot {
        version: 1,
        format: "hermes-sqlite-schema31".into(),
        source_stamp: Value::Null,
        session_records: sessions.len() as u64,
        totals,
        models,
        counter_coverage,
        model_coverage,
        reasons: reasons.into_iter().collect(),
    })
}
