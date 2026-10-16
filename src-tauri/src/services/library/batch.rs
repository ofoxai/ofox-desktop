//! Explicit cross-resource plans. A physical file is written once, regardless
//! of how many resources contribute to it. Journals never leave this host.
use super::{coordinator as io, LibrarySnapshot, Operation, ResourceKind};
use crate::{error::AppError, Database};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

#[path = "batch_store.rs"]
mod persistence;
#[path = "batch_plan.rs"]
mod plan;
pub(crate) use persistence::validate as validate_store;
pub(crate) use persistence::{load_on, restore_local, BatchStore};
pub(crate) const SETTING_KEY: &str = "library_batch_operations_v1";
pub(crate) const CHILD_PREFIX: &str = "batch-child:";
const MAX_RESOURCES: usize = 50;
const MAX_BATCH_BYTES: usize = 16 * 1024 * 1024;
const MAX_GROUPS: usize = 300;
pub(crate) const MAX_STORE_BYTES: usize = 32 * 1024 * 1024;
// With at most 300 groups per batch, status/reason and staging-method updates
// are bounded. Reserve their space before the first native write is allowed.
const WRITE_RESERVE: usize = 512 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchResource {
    pub resource_id: String,
    pub expected_revision: u64,
    #[serde(default)]
    pub add_tools: Vec<String>,
    #[serde(default)]
    pub withdraw_tools: Vec<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BatchAction {
    Apply,
    Withdraw,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Member {
    child: usize,
    target: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Group {
    group_id: String,
    path: String,
    physical_target: String,
    before_hash: String,
    after_hash: String,
    after_text: String,
    status: String,
    reason: Option<String>,
    prepared: bool,
    members: Vec<Member>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    operation_id: String,
    kind: ResourceKind,
    epoch: u64,
    created_at: i64,
    status: String,
    #[serde(default)]
    metadata_applied: bool,
    #[serde(default)]
    names: BTreeMap<String, String>,
    plans: Vec<Operation>,
    groups: Vec<Group>,
    children: BTreeMap<String, String>,
}
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}
fn reason(failure: &AppError) -> String {
    let raw = failure.to_string();
    raw.find("library.")
        .map(|start| {
            raw[start..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
                .collect()
        })
        .unwrap_or_else(|| "library.batchWriteFailed".into())
}
fn digest(op: &Operation) -> Result<String, AppError> {
    serde_json::to_vec(op)
        .map(|bytes| io::fingerprint(Some(&bytes)))
        .map_err(|_| error("stateUnreadable"))
}
fn child_id(journal: &Journal, index: usize) -> String {
    format!("{CHILD_PREFIX}{}:{index}", journal.operation_id)
}
fn has_recovery_material(journal: &Journal) -> bool {
    journal.groups.iter().any(|group| {
        group.prepared
            && (group.status != "applied"
                || group.members.iter().any(|member| {
                    journal.plans[member.child].targets[member.target]
                        .directory
                        .is_some()
                }))
    })
}
fn prunable(journal: &Journal) -> bool {
    ["applied", "cancelled"].contains(&journal.status.as_str()) && !has_recovery_material(journal)
}

/// Incomplete children contain no executable write text/hash or recovery paths.
fn project_children(journal: &mut Journal, snapshot: &mut LibrarySnapshot) -> Result<(), AppError> {
    // Private native plans need matching evidence in the independent child
    // journal. Editing only a parent must not authorize a different write.
    let commitment = io::fingerprint(Some(
        &serde_json::to_vec(&(
            &journal.operation_id,
            &journal.kind,
            journal.epoch,
            journal.metadata_applied,
            &journal.names,
            &journal.plans,
            &journal.groups,
        ))
        .map_err(|_| error("stateUnreadable"))?,
    ));
    journal.children.clear();
    for (index, plan) in journal.plans.iter().enumerate() {
        let mut child = plan.clone();
        child.id = child_id(journal, index);
        if !child.targets.is_empty() {
            child.desired_updates.clear();
            child.previous_assignments.clear();
        }
        let mut all_applied = !child.targets.is_empty() || journal.metadata_applied;
        for (target_index, target) in child.targets.iter_mut().enumerate() {
            let applied = journal
                .groups
                .iter()
                .find(|g| {
                    g.members
                        .iter()
                        .any(|m| m.child == index && m.target == target_index)
                })
                .is_some_and(|g| g.status == "applied");
            target.directory = None;
            target.previous_receipt = None;
            target.receipt = None;
            target.after_text.clear();
            target.assignment_updates.clear();
            target.source_fingerprints.clear();
            if applied {
                target.status = "applied".into();
                target.reason = None;
            } else {
                all_applied = false;
                target.status = "needsReview".into();
                target.reason = Some(format!("library.batchParentRequired:{commitment}"));
                target.after_hash.clear();
                target.transfer = None;
            }
        }
        child.status =
            if all_applied && journal.status != "previewed" && journal.status != "blocked" {
                "applied"
            } else if journal.status == "cancelled" {
                "cancelled"
            } else {
                "needsReview"
            }
            .into();
        journal.children.insert(child.id.clone(), digest(&child)?);
        snapshot.machine.operations.insert(child.id.clone(), child);
    }
    Ok(())
}
fn view(journal: &Journal) -> Value {
    let operation_views: Vec<_> = journal
        .plans
        .iter()
        .map(crate::services::library::operation_view)
        .collect();
    let name = |id: &str| {
        journal
            .names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_owned())
    };
    let groups: Vec<Value> = journal.groups.iter().map(|group| {
        let mut tools = BTreeSet::new();
        let mut changed_tools = BTreeSet::new();
        let mut readers = BTreeSet::new();
        let mut resources = BTreeSet::new();
        let mut before = None;
        let mut after = None;
        let mut changes = Vec::new();
        for member in &group.members {
            let plan = &journal.plans[member.child];
            let target = &plan.targets[member.target];
            tools.extend(target.tools.iter().cloned());
            readers.extend(target.readers.iter().cloned());
            resources.insert(plan.resource_id.clone());
            if journal.kind == ResourceKind::Instructions {
                if let Some(receipt) = &target.previous_receipt {
                    before = Some(receipt.owned_segment.clone());
                }
                if let Some(receipt) = &target.receipt {
                    after = Some(receipt.owned_segment.clone());
                }
            }
            let mut actions: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
            for (key, value) in &target.assignment_updates {
                if let Some(tool) = key.rsplit(':').next() {
                    changed_tools.insert(tool.to_owned());
                    let intent = plan.desired_updates.get(key).unwrap_or(value);
                    let action = if intent.as_ref() == Some(&plan.resource_id) {
                        "apply"
                    } else {
                        "withdraw"
                    };
                    actions.entry(action).or_default().insert(tool.to_owned());
                }
            }
            if actions.is_empty() {
                actions.insert(if target.receipt.is_some() { "apply" } else { "withdraw" }, BTreeSet::new());
            }
            let fields = operation_views[member.child]["targets"][member.target]["changedFields"].clone();
            let method = target.transfer.as_ref().map(|transfer| transfer.method.as_str())
                .or_else(|| target.directory.as_ref().map(|directory| directory.method.as_str()));
            for (action, tools) in actions {
                changes.push(json!({"resourceId":plan.resource_id,"name":name(&plan.resource_id),"action":action,"tools":tools,"changedFields":fields,"method":method}));
            }
        }
        json!({"groupId":group.group_id,"tools":tools,"changedTools":changed_tools,"readers":readers,"path":group.path,"status":group.status,"reason":group.reason,"beforeManaged":before,"afterManaged":after,"resourceIds":resources,"changes":changes})
    }).collect();
    let completed = journal
        .groups
        .iter()
        .filter(|g| g.status == "applied")
        .count();
    let mut metadata_changes = Vec::new();
    for plan in journal.plans.iter().filter(|plan| plan.targets.is_empty()) {
        let mut actions: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (key, value) in &plan.desired_updates {
            if let Some(tool) = key.rsplit(':').next() {
                actions
                    .entry(if value.as_ref() == Some(&plan.resource_id) {
                        "apply"
                    } else {
                        "withdraw"
                    })
                    .or_default()
                    .insert(tool);
            }
        }
        let status = if journal.metadata_applied {
            "applied"
        } else if journal.status == "cancelled" {
            "cancelled"
        } else {
            "pending"
        };
        for (action, tools) in actions {
            metadata_changes.push(json!({"resourceId":plan.resource_id,"name":name(&plan.resource_id),"action":action,"tools":tools,"changedFields":[],"method":null,"status":status}));
        }
    }
    json!({"operationId":journal.operation_id,"kind":journal.kind,"status":journal.status,"createdAt":journal.created_at,"resources":journal.plans.iter().map(|p|json!({"resourceId":p.resource_id,"name":name(&p.resource_id),"expectedRevision":p.resource_revision})).collect::<Vec<_>>(),"groups":groups,"metadataChanges":metadata_changes,"completed":completed,"remaining":journal.groups.len()-completed})
}
fn paired(db: &Database) -> Result<(LibrarySnapshot, BatchStore), AppError> {
    let conn = crate::database::lock_conn!(db.conn);
    let library = super::store::load(&conn)?;
    let batches = load_on(&conn, &library)?;
    Ok((library, batches))
}
fn commit(
    db: &Database,
    old: &LibrarySnapshot,
    batches: &BatchStore,
    mut next: LibrarySnapshot,
    mut journal: Journal,
) -> Result<(LibrarySnapshot, BatchStore), AppError> {
    project_children(&mut journal, &mut next)?;
    let mut next_batches = batches.clone();
    next_batches
        .operations
        .insert(journal.operation_id.clone(), journal);
    db.library_commit_batch(old, &next, batches, &next_batches)
}
pub fn preview_batch(
    db: &Database,
    kind: ResourceKind,
    resources: Vec<BatchResource>,
    tools: Vec<String>,
    action: BatchAction,
    expected_epoch: u64,
) -> Result<Value, AppError> {
    let _io = crate::config_io::lock();
    let (current, batches) = paired(db)?;
    if current.machine.epoch != expected_epoch {
        return Err(error("epochChanged"));
    }
    if resources.is_empty()
        || resources.len() > MAX_RESOURCES
        || resources
            .iter()
            .map(|r| &r.resource_id)
            .collect::<BTreeSet<_>>()
            .len()
            != resources.len()
    {
        return Err(error("batchInvalidSelection"));
    }
    if batches.operations.values().filter(|b| !prunable(b)).count() >= 8 {
        return Err(error("batchTooMany"));
    }
    let journal = plan::build(db, &current, kind, resources, tools, action)?;
    if serde_json::to_vec(&journal)
        .map_err(|_| error("stateUnreadable"))?
        .len()
        > MAX_BATCH_BYTES
        || journal.groups.len() > MAX_GROUPS
    {
        return Err(error("batchTooLarge"));
    }
    let result = view(&journal);
    let mut next = current.clone();
    let mut next_batches = batches.clone();
    let mut finished: Vec<_> = next_batches
        .operations
        .values()
        .filter(|b| prunable(b))
        .map(|b| (b.created_at, b.operation_id.clone()))
        .collect();
    finished.sort();
    for (_, id) in finished
        .into_iter()
        .take(next_batches.operations.len().saturating_sub(19))
    {
        if let Some(old) = next_batches.operations.remove(&id) {
            for child in old.children.keys() {
                next.machine.operations.remove(child);
            }
        }
    }
    // CAS compares the original complete store, including entries being pruned.
    let mut projected = journal.clone();
    project_children(&mut projected, &mut next)?;
    next_batches
        .operations
        .insert(projected.operation_id.clone(), projected);
    while serde_json::to_vec(&next_batches)
        .map_err(|_| error("stateUnreadable"))?
        .len()
        > MAX_STORE_BYTES - WRITE_RESERVE
    {
        let oldest = next_batches
            .operations
            .values()
            .filter(|batch| batch.operation_id != journal.operation_id && prunable(batch))
            .min_by_key(|batch| batch.created_at)
            .map(|batch| batch.operation_id.clone())
            .ok_or_else(|| error("batchTooLarge"))?;
        if let Some(expired) = next_batches.operations.remove(&oldest) {
            for child in expired.children.keys() {
                next.machine.operations.remove(child);
            }
        }
    }
    db.library_commit_batch(&current, &next, &batches, &next_batches)?;
    Ok(result)
}
type Signals = BTreeMap<String, Arc<AtomicBool>>;
fn signals() -> &'static Mutex<Signals> {
    static SIGNALS: OnceLock<Mutex<Signals>> = OnceLock::new();
    SIGNALS.get_or_init(|| Mutex::new(BTreeMap::new()))
}
struct Running(String);
impl Drop for Running {
    fn drop(&mut self) {
        signals()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}
pub fn run_batch(db: &Database, id: &str) -> Result<Value, AppError> {
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut running = signals().lock().unwrap_or_else(|e| e.into_inner());
        if running.contains_key(id) {
            return Err(error("batchRunning"));
        }
        running.insert(id.into(), cancelled.clone());
    }
    let _running = Running(id.into());
    run_inner(db, id, &cancelled, false)
}
fn run_inner(
    db: &Database,
    id: &str,
    cancelled: &AtomicBool,
    interrupt_after_write: bool,
) -> Result<Value, AppError> {
    let _io = crate::config_io::lock();
    let (mut current, mut batches) = paired(db)?;
    let mut journal = batches
        .operations
        .get(id)
        .cloned()
        .ok_or_else(|| error("operationMissing"))?;
    if journal.status == "cancelled" {
        return Err(error("operationCancelled"));
    }
    if journal.status == "applied" || journal.status == "blocked" {
        return Ok(view(&journal));
    }
    if current.machine.epoch != journal.epoch {
        return Err(error("epochChanged"));
    }
    if !journal.metadata_applied && !cancelled.load(Ordering::SeqCst) {
        let mut next = current.clone();
        for operation in journal.plans.iter().filter(|op| op.targets.is_empty()) {
            io::confirm_version(db, &current, operation)?;
            io::verify_routing(operation)?;
            for (key, value) in &operation.desired_updates {
                if current.machine.assignments.get(key)
                    != operation
                        .previous_assignments
                        .get(key)
                        .and_then(Option::as_ref)
                {
                    return Err(error("desiredChanged"));
                }
                if let Some(value) = value {
                    next.machine.assignments.insert(key.clone(), value.clone());
                } else {
                    next.machine.assignments.remove(key);
                }
            }
        }
        journal.metadata_applied = true;
        journal.status = "prepared".into();
        let saved = commit(db, &current, &batches, next, journal.clone())?;
        current = saved.0;
        batches = saved.1;
    }
    for index in 0..journal.groups.len() {
        if cancelled.load(Ordering::SeqCst) {
            break;
        }
        if ["applied", "conflict", "needsReview"].contains(&journal.groups[index].status.as_str()) {
            continue;
        }
        if journal.groups[index].prepared {
            journal.groups[index].status = "needsReview".into();
            journal.groups[index].reason = Some("library.operationNeedsReview".into());
            continue;
        }
        if let Err(failure) = plan::verify(db, &current, &journal, index, false) {
            journal.groups[index].status = "conflict".into();
            journal.groups[index].reason = Some(reason(&failure));
            let saved = commit(db, &current, &batches, current.clone(), journal.clone())?;
            current = saved.0;
            batches = saved.1;
            continue;
        }
        let mut next = current.clone();
        for member in &journal.groups[index].members {
            for (key, value) in
                &journal.plans[member.child].targets[member.target].assignment_updates
            {
                if let Some(value) = value {
                    next.machine.assignments.insert(key.clone(), value.clone());
                } else {
                    next.machine.assignments.remove(key);
                }
            }
        }
        journal.groups[index].prepared = true;
        journal.groups[index].status = "prepared".into();
        journal.status = "prepared".into();
        let saved = commit(db, &current, &batches, next, journal.clone())?;
        current = saved.0;
        batches = saved.1;
        if journal.kind == ResourceKind::Skills {
            let member = journal.groups[index].members[0].clone();
            let target = &mut journal.plans[member.child].targets[member.target];
            if let Err(failure) = super::skill_resource::stage(target) {
                journal.groups[index].status = "failed".into();
                journal.groups[index].reason = Some(reason(&failure));
                let saved = commit(db, &current, &batches, current.clone(), journal.clone())?;
                current = saved.0;
                batches = saved.1;
                continue;
            }
            journal.groups[index].after_hash = target.after_hash.clone();
            let saved = commit(db, &current, &batches, current.clone(), journal.clone())?;
            current = saved.0;
            batches = saved.1;
        }
        let written = plan::write_group(&journal, index);
        if written.is_ok() && interrupt_after_write {
            return Err(error("batchInterrupted"));
        }
        match written {
            Ok(()) => {
                journal.groups[index].status = "applied".into();
                journal.groups[index].reason = None;
                let mut next = current.clone();
                plan::record_group(&mut journal, index, &mut next);
                let saved = commit(db, &current, &batches, next, journal.clone())?;
                current = saved.0;
                batches = saved.1;
                if journal.kind == ResourceKind::Skills {
                    let member = journal.groups[index].members[0].clone();
                    let target = &mut journal.plans[member.child].targets[member.target];
                    if super::skill_resource::cleanup(target).is_ok() {
                        target.directory = None;
                    } else {
                        journal.groups[index].reason = Some("library.cleanupNeedsReview".into());
                    }
                }
            }
            Err(failure) => {
                journal.groups[index].status = "failed".into();
                journal.groups[index].reason = Some(reason(&failure));
            }
        }
        let saved = commit(db, &current, &batches, current.clone(), journal.clone())?;
        current = saved.0;
        batches = saved.1;
    }
    journal.status = if cancelled.load(Ordering::SeqCst) {
        if has_recovery_material(&journal) {
            "needsReview"
        } else {
            "cancelled"
        }
    } else if journal
        .groups
        .iter()
        .all(|g| g.status == "applied" && g.reason.is_none())
    {
        "applied"
    } else {
        "partial"
    }
    .into();
    commit(db, &current, &batches, current.clone(), journal.clone())?;
    Ok(view(&journal))
}
pub fn cancel_batch(db: &Database, id: &str) -> Result<Value, AppError> {
    if let Some(signal) = signals()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
    {
        signal.store(true, Ordering::SeqCst);
        let (_, batches) = paired(db)?;
        let mut result = view(
            batches
                .operations
                .get(id)
                .ok_or_else(|| error("operationMissing"))?,
        );
        result["status"] = json!("cancelling");
        return Ok(result);
    }
    let _io = crate::config_io::lock();
    let (mut current, mut batches) = paired(db)?;
    let mut journal = batches
        .operations
        .get(id)
        .cloned()
        .ok_or_else(|| error("operationMissing"))?;
    if journal.groups.iter().any(|group| {
        group.status == "applied"
            && group.members.iter().any(|member| {
                journal.plans[member.child].targets[member.target]
                    .directory
                    .is_some()
            })
    }) {
        return Err(error("cleanupNeedsReview"));
    }
    for index in 0..journal.groups.len() {
        if journal.groups[index].status == "applied" {
            continue;
        }
        if journal.groups[index].prepared {
            // A stopped IPC waiter may have left a physical write ahead of its
            // receipt. Cancellation may verify/account it, but never replay it.
            if plan::verify(db, &current, &journal, index, true).is_ok() {
                journal.groups[index].status = "applied".into();
                journal.groups[index].reason = None;
                let mut next = current.clone();
                plan::record_group(&mut journal, index, &mut next);
                if journal.kind == ResourceKind::Skills {
                    journal.groups[index].reason = Some("library.cleanupNeedsReview".into());
                }
                let saved = commit(db, &current, &batches, next, journal.clone())?;
                current = saved.0;
                batches = saved.1;
                continue;
            }
            if plan::verify(db, &current, &journal, index, false).is_err() {
                journal.groups[index].status = "needsReview".into();
                journal.groups[index].reason = Some("library.operationNeedsReview".into());
                continue;
            }
            // The unchanged before-state proves that no native write needs
            // accounting. Only then may its unused staging evidence be removed.
            journal.groups[index].prepared = false;
        }
        for member in &journal.groups[index].members {
            let target = &mut journal.plans[member.child].targets[member.target];
            if target.directory.is_some() {
                super::skill_resource::discard_staged(target)?;
                target.status = "needsReview".into();
            }
        }
        journal.groups[index].status = "needsReview".into();
        journal.groups[index].reason = Some("library.operationCancelled".into());
    }
    journal.status = if has_recovery_material(&journal) {
        "needsReview"
    } else {
        "cancelled"
    }
    .into();
    commit(db, &current, &batches, current.clone(), journal.clone())?;
    Ok(view(&journal))
}
/// Observe completed writes; never replay a missing write.
pub fn reconcile_batch(db: &Database, id: &str) -> Result<Value, AppError> {
    if signals()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(id)
    {
        return Err(error("batchRunning"));
    }
    let _io = crate::config_io::lock();
    let (mut current, mut batches) = paired(db)?;
    let mut journal = batches
        .operations
        .get(id)
        .cloned()
        .ok_or_else(|| error("operationMissing"))?;
    if ["previewed", "blocked", "cancelled"].contains(&journal.status.as_str()) {
        return Ok(view(&journal));
    }
    for index in 0..journal.groups.len() {
        if journal.groups[index].status == "applied" && journal.kind != ResourceKind::Skills {
            continue;
        }
        if !journal.groups[index].prepared {
            continue;
        }
        match plan::verify(db, &current, &journal, index, true) {
            Ok(()) => {
                journal.groups[index].status = "applied".into();
                journal.groups[index].reason = None;
                let mut next = current.clone();
                plan::record_group(&mut journal, index, &mut next);
                let saved = commit(db, &current, &batches, next, journal.clone())?;
                current = saved.0;
                batches = saved.1;
                if journal.kind == ResourceKind::Skills {
                    let member = journal.groups[index].members[0].clone();
                    let target = &mut journal.plans[member.child].targets[member.target];
                    if super::skill_resource::cleanup(target).is_ok() {
                        target.directory = None;
                    } else {
                        journal.groups[index].reason = Some("library.cleanupNeedsReview".into());
                    }
                }
            }
            Err(failure) => {
                journal.groups[index].status = "needsReview".into();
                journal.groups[index].reason = Some(reason(&failure));
            }
        }
    }
    journal.status = if journal
        .groups
        .iter()
        .all(|g| g.status == "applied" && g.reason.is_none())
    {
        "applied"
    } else {
        "needsReview"
    }
    .into();
    commit(db, &current, &batches, current.clone(), journal.clone())?;
    Ok(view(&journal))
}
pub fn list_batches(db: &Database, kind: Option<ResourceKind>) -> Result<Value, AppError> {
    let (_, batches) = paired(db)?;
    let mut rows: Vec<_> = batches
        .operations
        .values()
        .filter(|b| kind.as_ref().is_none_or(|kind| kind == &b.kind))
        .collect();
    rows.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    Ok(json!(rows.into_iter().map(view).collect::<Vec<_>>()))
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;
