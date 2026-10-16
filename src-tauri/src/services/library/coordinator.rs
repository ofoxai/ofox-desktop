use super::{patch_instructions, InstructionSet, LibrarySnapshot, Operation, Receipt, TargetWrite};
use crate::{error::AppError, Database};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}

pub(crate) fn fingerprint(bytes: Option<&[u8]>) -> String {
    bytes
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .unwrap_or_else(|| "missing".into())
}

const MAX_TARGET_BYTES: usize = 2 * 1024 * 1024;

pub(crate) fn read(path: &Path) -> Result<Option<String>, AppError> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(error("targetUnreadable")),
    };
    let mut bytes = Vec::new();
    file.take((MAX_TARGET_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| error("targetUnreadable"))?;
    if bytes.len() > MAX_TARGET_BYTES {
        return Err(error("targetTooLarge"));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| error("targetUnreadable"))
}

pub(crate) fn physical(path: &Path) -> Result<PathBuf, AppError> {
    let resolved = crate::config::write_target(path)?;
    if resolved.exists() {
        return std::fs::canonicalize(&resolved).map_err(|_| error("targetUnreadable"));
    }
    let parent = resolved.parent().ok_or_else(|| error("targetUnreadable"))?;
    let leaf = resolved
        .file_name()
        .ok_or_else(|| error("targetUnreadable"))?;
    if parent == resolved {
        return Err(error("targetUnreadable"));
    }
    Ok(physical(parent)?.join(leaf))
}

pub fn save_instruction(
    db: &Database,
    mut set: InstructionSet,
    expected_revision: Option<u64>,
) -> Result<InstructionSet, AppError> {
    let _io = crate::config_io::lock();
    if set.id.is_empty()
        || set.id.len() > 256
        || set.name.trim().is_empty()
        || set.content.len() > 1024 * 1024
        || set.content.contains("<!-- OFOX:")
    {
        return Err(error("invalidInstructions"));
    }
    let before = db.library_snapshot()?;
    let existing = before.definitions.instructions.get(&set.id);
    if existing.map(|set| set.revision) != expected_revision {
        return Err(error("revisionChanged"));
    }
    set.revision = expected_revision
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| error("revisionOverflow"))?;
    let mut next = before.clone();
    next.definitions
        .instructions
        .insert(set.id.clone(), set.clone());
    db.library_commit(&before, &next)?;
    Ok(set)
}

/// Stop an obsolete journal without reverting or modifying native content.
/// Recovery material must be inspected before an operation can be discarded.
pub fn cancel_operation(db: &Database, id: &str) -> Result<Operation, AppError> {
    if id.starts_with(super::batch::CHILD_PREFIX) {
        return Err(error("batchParentRequired"));
    }
    let _io = crate::config_io::lock();
    let before = db.library_snapshot()?;
    let mut op = before
        .machine
        .operations
        .get(id)
        .cloned()
        .ok_or_else(|| error("operationMissing"))?;
    for target in &mut op.targets {
        if target.directory.is_some() {
            super::skill_resource::discard_staged(target)?;
        }
        if target.status != "applied" {
            target.status = "needsReview".into();
            target.reason = Some("library.operationCancelled".into());
        }
    }
    op.status = "cancelled".into();
    let mut next = before.clone();
    next.machine.operations.insert(op.id.clone(), op.clone());
    db.library_commit(&before, &next)?;
    Ok(op)
}

pub(crate) fn native_instruction_paths() -> Result<BTreeMap<String, PathBuf>, AppError> {
    use std::str::FromStr;
    ["claude", "codex", "gemini", "opencode", "hermes"]
        .iter()
        .map(|tool| {
            Ok((
                tool.to_string(),
                crate::prompt_files::prompt_file_path(&crate::app_config::AppType::from_str(
                    tool,
                )?)?,
            ))
        })
        .collect()
}

pub(crate) fn physical_id(path: &Path) -> Result<String, AppError> {
    physical(path)?
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| error("targetUnreadable"))
}

pub fn preview_instructions(
    db: &Database,
    id: &str,
    tools: &[String],
) -> Result<Operation, AppError> {
    use std::str::FromStr;
    let _io = crate::config_io::lock();
    let targets: Result<Vec<_>, AppError> = tools
        .iter()
        .map(|tool| {
            if !["claude", "codex", "gemini", "opencode", "hermes"].contains(&tool.as_str()) {
                return Err(error("targetUnsupported"));
            }
            let app = crate::app_config::AppType::from_str(tool)?;
            Ok((tool.clone(), crate::prompt_files::prompt_file_path(&app)?))
        })
        .collect();
    preview_internal(db, id, &targets?, true)
}

#[derive(Default)]
struct Group {
    path: PathBuf,
    selected: BTreeSet<String>,
    receipt: Option<Receipt>,
}

pub(crate) fn preview_at(
    db: &Database,
    id: &str,
    targets: &[(String, PathBuf)],
) -> Result<Operation, AppError> {
    preview_internal(db, id, targets, false)
}

fn preview_internal(
    db: &Database,
    id: &str,
    targets: &[(String, PathBuf)],
    verify_resolver: bool,
) -> Result<Operation, AppError> {
    let _io = crate::config_io::lock();
    let before = db.library_snapshot()?;
    let set = before
        .definitions
        .instructions
        .get(id)
        .ok_or_else(|| error("resourceMissing"))?;
    let selected: BTreeSet<_> = targets.iter().map(|(tool, _)| tool.clone()).collect();
    if selected.len() != targets.len() {
        return Err(error("targetDuplicate"));
    }
    let mut desired_updates: BTreeMap<String, Option<String>> = selected
        .iter()
        .map(|tool| (format!("instructions:{tool}"), Some(id.to_owned())))
        .collect();
    for (key, resource) in &before.machine.assignments {
        if resource == id
            && key
                .strip_prefix("instructions:")
                .is_some_and(|tool| !selected.contains(tool))
        {
            desired_updates.insert(key.clone(), None);
        }
    }
    let previous_assignments = desired_updates
        .keys()
        .map(|key| (key.clone(), before.machine.assignments.get(key).cloned()))
        .collect();
    let paths = if verify_resolver {
        native_instruction_paths()?
    } else {
        BTreeMap::new()
    };
    let resolved_paths = paths
        .iter()
        .map(|(tool, path)| (tool.clone(), path.to_string_lossy().into_owned()))
        .collect();
    let resolved_physical: BTreeMap<String, String> = paths
        .iter()
        .map(|(tool, path)| Ok((tool.clone(), physical_id(path)?)))
        .collect::<Result<_, AppError>>()?;
    let native_readers: Vec<_> = resolved_physical
        .iter()
        .map(|(tool, target)| (tool.clone(), target.clone()))
        .collect();
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for (tool, path) in targets {
        let target = physical_id(path)?;
        let group = groups.entry(target).or_default();
        group.path = path.clone();
        group.selected.insert(tool.clone());
    }
    for receipt in before
        .machine
        .receipts
        .values()
        .filter(|receipt| receipt.kind == super::ResourceKind::Instructions)
    {
        // This first codec owns only the instruction block, never whole files.
        if receipt.resource_id != id && !receipt.owners.iter().any(|tool| selected.contains(tool)) {
            continue;
        }
        let group = groups.entry(receipt.physical_target.clone()).or_default();
        if group.path.as_os_str().is_empty() {
            group.path = PathBuf::from(&receipt.path);
        }
        group.receipt = Some(receipt.clone());
    }
    let mut writes = Vec::new();
    for (target, group) in groups {
        let key = format!("instructions:{target}");
        let old = group
            .receipt
            .or_else(|| before.machine.receipts.get(&key).cloned());
        let remaining: BTreeSet<String> = old
            .as_ref()
            .map(|receipt| {
                receipt
                    .owners
                    .iter()
                    .filter(|tool| !selected.contains(*tool) && receipt.resource_id != id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let owners: Vec<_> = remaining.union(&group.selected).cloned().collect();
        let mut updates = BTreeMap::new();
        for tool in &group.selected {
            updates.insert(format!("instructions:{tool}"), Some(id.to_owned()));
        }
        if let Some(receipt) = &old {
            for tool in &receipt.owners {
                if receipt.resource_id == id
                    && !selected.contains(tool)
                    && desired_updates.contains_key(&format!("instructions:{tool}"))
                {
                    updates.insert(format!("instructions:{tool}"), None);
                }
            }
        }
        let current = read(&group.path);
        let mut write = TargetWrite {
            directory: None,
            transfer: None,
            tools: owners.clone(),
            readers: owners
                .iter()
                .cloned()
                .chain(
                    native_readers
                        .iter()
                        .filter(|(_, path)| path == &target)
                        .map(|(tool, _)| tool.clone()),
                )
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            path: group.path.to_string_lossy().into_owned(),
            physical_target: target.clone(),
            receipt_key: key,
            previous_receipt: old.clone(),
            assignment_updates: updates,
            source_fingerprints: BTreeMap::new(),
            before_hash: String::new(),
            after_hash: String::new(),
            after_text: String::new(),
            receipt: None,
            status: "pending".into(),
            reason: None,
        };
        match current {
            Err(_) => {
                write.status = "failed".into();
                write.reason = Some("library.targetUnreadable".into());
            }
            Ok(current) => {
                write.before_hash = fingerprint(current.as_deref().map(str::as_bytes));
                let text = current.as_deref().unwrap_or("");
                let patch = if !remaining.is_empty()
                    && old
                        .as_ref()
                        .is_some_and(|receipt| receipt.resource_id != id)
                    && !group.selected.is_empty()
                {
                    Err(error("sharedTargetConflict"))
                } else if group.selected.is_empty() && !remaining.is_empty() {
                    patch_instructions(text, old.as_ref().map(|r| r.owned_segment.as_str()), None)
                        .map(|_| super::InstructionPatch {
                            text: text.into(),
                            owned_segment: old.as_ref().map(|r| r.owned_segment.clone()),
                        })
                } else if old.is_none()
                    && !group.selected.is_empty()
                    && !text.trim().is_empty()
                    && text.trim() == set.content.trim()
                {
                    Err(error("legacyDuplicate"))
                } else {
                    patch_instructions(
                        text,
                        old.as_ref().map(|r| r.owned_segment.as_str()),
                        (!owners.is_empty()).then_some(set.content.as_str()),
                    )
                };
                match patch {
                    Err(e) => {
                        write.status = "conflict".into();
                        write.reason = Some(e.to_string());
                    }
                    Ok(patch) if patch.text.len() > MAX_TARGET_BYTES => {
                        write.status = "failed".into();
                        write.reason = Some("library.targetTooLarge".into());
                    }
                    Ok(patch) => {
                        write.after_text = patch.text;
                        write.after_hash = fingerprint(Some(write.after_text.as_bytes()));
                        write.receipt = patch.owned_segment.map(|owned_segment| {
                            let (resource_id, resource_revision, definition_hash) =
                                if group.selected.is_empty() && !remaining.is_empty() {
                                    let old =
                                        old.as_ref().expect("remaining owners have a receipt");
                                    (
                                        old.resource_id.clone(),
                                        old.resource_revision,
                                        old.definition_hash.clone(),
                                    )
                                } else {
                                    (
                                        id.to_owned(),
                                        set.revision,
                                        fingerprint(Some(set.content.as_bytes())),
                                    )
                                };
                            Receipt {
                                kind: super::ResourceKind::Instructions,
                                resource_id,
                                resource_revision,
                                definition_hash,
                                physical_target: target,
                                path: write.path.clone(),
                                owners,
                                owned_segment,
                            }
                        });
                    }
                }
            }
        }
        writes.push(write);
    }
    let operation = Operation {
        kind: super::ResourceKind::Instructions,
        id: uuid::Uuid::new_v4().to_string(),
        epoch: before.machine.epoch,
        resource_id: id.into(),
        resource_revision: set.revision,
        resource_hash: fingerprint(Some(set.content.as_bytes())),
        created_at: chrono::Utc::now().timestamp_millis(),
        status: "previewed".into(),
        targets: writes,
        desired_updates,
        previous_assignments,
        resolved_paths,
        resolved_physical,
        routing: None,
    };
    let mut next = before.clone();
    next.machine
        .operations
        .insert(operation.id.clone(), operation.clone());
    prune_history(&mut next, &operation.id);
    db.library_commit(&before, &next)?;
    Ok(operation)
}

pub(crate) fn confirm_version(
    db: &Database,
    snapshot: &LibrarySnapshot,
    op: &Operation,
) -> Result<(), AppError> {
    if snapshot.machine.epoch != op.epoch {
        return Err(error("epochChanged"));
    }
    let (revision, hash) = if op.kind == super::ResourceKind::Mcp {
        let metadata = snapshot
            .definitions
            .mcp
            .get(&op.resource_id)
            .ok_or_else(|| error("resourceMissing"))?;
        let server = db
            .get_all_mcp_servers()?
            .shift_remove(&op.resource_id)
            .ok_or_else(|| error("resourceMissing"))?;
        (
            metadata.revision,
            super::mcp_resource::spec_hash(&server.server)?,
        )
    } else if op.kind == super::ResourceKind::Skills {
        let meta = snapshot
            .definitions
            .skills
            .get(&op.resource_id)
            .ok_or_else(|| error("resourceMissing"))?;
        let skill = db
            .get_installed_skill(&op.resource_id)?
            .ok_or_else(|| error("resourceMissing"))?;
        (meta.revision, super::skill_resource::spec_hash(&skill)?)
    } else {
        let definition = snapshot
            .definitions
            .instructions
            .get(&op.resource_id)
            .ok_or_else(|| error("resourceMissing"))?;
        (
            definition.revision,
            fingerprint(Some(definition.content.as_bytes())),
        )
    };
    if revision != op.resource_revision || hash != op.resource_hash {
        return Err(error("revisionChanged"));
    }
    Ok(())
}

fn record_target(snapshot: &mut LibrarySnapshot, target: &TargetWrite) {
    if let Some(receipt) = &target.receipt {
        snapshot
            .machine
            .receipts
            .insert(target.receipt_key.clone(), receipt.clone());
    } else {
        snapshot.machine.receipts.remove(&target.receipt_key);
    }
}

fn compact_finished(op: &mut Operation) {
    for target in &mut op.targets {
        if target.status == "applied" && target.directory.is_none() {
            target.after_text.clear();
            target.previous_receipt = None;
            target.receipt = None;
        }
    }
}

pub(crate) fn prune_history(snapshot: &mut LibrarySnapshot, keep_id: &str) {
    for (status, limit) in [
        ("previewed", 8usize),
        ("applied", 20usize),
        ("cancelled", 20usize),
    ] {
        let mut candidates: Vec<_> = snapshot
            .machine
            .operations
            .iter()
            .filter(|(id, op)| {
                op.status == status
                    && id.as_str() != keep_id
                    && !id.starts_with(super::batch::CHILD_PREFIX)
            })
            .map(|(id, op)| (op.created_at, id.clone()))
            .collect();
        candidates.sort();
        let kept_new = usize::from(
            snapshot
                .machine
                .operations
                .get(keep_id)
                .is_some_and(|op| op.status == status),
        );
        let remove = candidates.len().saturating_sub(limit - kept_new);
        for (_, id) in candidates.into_iter().take(remove) {
            snapshot.machine.operations.remove(&id);
        }
    }
}

fn persist(
    db: &Database,
    current: &LibrarySnapshot,
    op: &Operation,
    target: Option<&TargetWrite>,
) -> Result<LibrarySnapshot, AppError> {
    let mut next = current.clone();
    if let Some(target) = target {
        record_target(&mut next, target);
    }
    next.machine.operations.insert(op.id.clone(), op.clone());
    db.library_commit(current, &next)
}

pub(crate) fn verify_sources(
    target: &TargetWrite,
    targets: &[TargetWrite],
) -> Result<(), AppError> {
    for (path, source) in &target.source_fingerprints {
        let path = PathBuf::from(path);
        if physical_id(&path)? != source.physical_target {
            return Err(error("sourceChanged"));
        }
        // The write target has its own before/after verification. Empty hashes
        // capture resolver identity only (including potential shared readers).
        if source.hash.is_empty() || source.physical_target == target.physical_target {
            continue;
        }
        let expected = targets
            .iter()
            .find(|peer| peer.status == "applied" && peer.physical_target == source.physical_target)
            .map(|peer| &peer.after_hash)
            .unwrap_or(&source.hash);
        let current = read(&path)?;
        if fingerprint(current.as_deref().map(str::as_bytes)) != *expected {
            return Err(error("sourceChanged"));
        }
    }
    Ok(())
}

pub(crate) fn verify_routing(op: &Operation) -> Result<(), AppError> {
    if op.kind == super::ResourceKind::Mcp
        && op.resolved_paths.contains_key("opencode")
        && op.routing.as_ref() != Some(&super::mcp_resource::routing_dependencies()?)
    {
        return Err(error("resolverChanged"));
    }
    if op.kind == super::ResourceKind::Skills && !op.resolved_paths.is_empty() {
        let paths = super::skill_resource::resolver_paths()?;
        let current: BTreeMap<_, _> = paths
            .iter()
            .map(|(key, path)| (key.clone(), path.to_string_lossy().into_owned()))
            .collect();
        let physical = paths
            .iter()
            .map(|(key, path)| Ok((key.clone(), physical_id(path)?)))
            .collect::<Result<BTreeMap<_, _>, AppError>>()?;
        if current != op.resolved_paths || physical != op.resolved_physical {
            return Err(error("resolverChanged"));
        }
    }
    use std::str::FromStr;
    for (tool, path) in &op.resolved_paths {
        let current = if op.kind == super::ResourceKind::Skills {
            super::skill_resource::resolver_paths()?
                .remove(tool)
                .ok_or_else(|| error("resolverChanged"))?
        } else if op.kind == super::ResourceKind::Mcp {
            super::mcp_resource::native_paths()?
                .remove(tool)
                .ok_or_else(|| error("targetUnsupported"))?
        } else {
            crate::prompt_files::prompt_file_path(&crate::app_config::AppType::from_str(tool)?)?
        };
        if current.to_string_lossy() != *path
            || op.resolved_physical.get(tool) != Some(&physical_id(&current)?)
        {
            return Err(error("resolverChanged"));
        }
    }
    Ok(())
}

pub fn apply_operation(db: &Database, id: &str) -> Result<Operation, AppError> {
    if id.starts_with(super::batch::CHILD_PREFIX) {
        return Err(error("batchParentRequired"));
    }
    let _io = crate::config_io::lock();
    let mut snapshot = db.library_snapshot()?;
    let mut op = snapshot
        .machine
        .operations
        .get(id)
        .cloned()
        .ok_or_else(|| error("operationMissing"))?;
    confirm_version(db, &snapshot, &op)?;
    if op.status == "cancelled" {
        return Err(error("operationCancelled"));
    }
    if op.status == "applied" {
        return Ok(op);
    }
    verify_routing(&op)?;
    let first_confirmation = op.status == "previewed";
    let mut prepared = snapshot.clone();
    let mut blocked_updates = BTreeSet::new();
    if first_confirmation
        && op
            .previous_assignments
            .iter()
            .any(|(key, expected)| snapshot.machine.assignments.get(key) != expected.as_ref())
    {
        return Err(error("desiredChanged"));
    }
    for target in &mut op.targets {
        if target.status == "applied" {
            continue;
        }
        let ownership_changed =
            snapshot.machine.receipts.get(&target.receipt_key) != target.previous_receipt.as_ref();
        let desired_changed = !first_confirmation
            && target
                .assignment_updates
                .iter()
                .any(|(key, expected)| snapshot.machine.assignments.get(key) != expected.as_ref());
        if ownership_changed || desired_changed {
            target.status = "conflict".into();
            target.reason = Some(
                if ownership_changed {
                    "library.ownershipChanged"
                } else {
                    "library.desiredChanged"
                }
                .into(),
            );
            blocked_updates.extend(target.assignment_updates.keys().cloned());
        }
    }
    if first_confirmation {
        for (key, resource) in &op.desired_updates {
            if blocked_updates.contains(key) {
                continue;
            }
            if let Some(resource) = resource {
                prepared
                    .machine
                    .assignments
                    .insert(key.clone(), resource.clone());
            } else {
                prepared.machine.assignments.remove(key);
            }
        }
    }
    op.status = "prepared".into();
    prepared
        .machine
        .operations
        .insert(op.id.clone(), op.clone());
    snapshot = db.library_commit(&snapshot, &prepared)?;
    for index in 0..op.targets.len() {
        if op.kind == super::ResourceKind::Skills
            && op.targets[index].status != "applied"
            && op.targets[index].status != "conflict"
            && op.targets[index].directory.is_some()
        {
            let staged = super::skill_resource::stage(&mut op.targets[index]);
            if let Err(failure) = staged {
                op.targets[index].status = "failed".into();
                op.targets[index].reason = Some(failure.to_string());
                snapshot = persist(db, &snapshot, &op, None)?;
                continue;
            }
            // The actual link/copy method is durable before either rename.
            snapshot = persist(db, &snapshot, &op, None)?;
        }
        let source_valid = verify_sources(&op.targets[index], &op.targets);
        let target = &mut op.targets[index];
        if target.status == "applied" || target.status == "conflict" || target.after_hash.is_empty()
        {
            continue;
        }
        let path = PathBuf::from(&target.path);
        let result = (|| {
            source_valid?;
            if op.kind == super::ResourceKind::Skills {
                return super::skill_resource::write(target);
            }
            if physical(&path)?.to_string_lossy() != target.physical_target {
                return Err(error("targetChanged"));
            }
            let current = read(&path)?;
            let hash = fingerprint(current.as_deref().map(str::as_bytes));
            if hash != target.before_hash {
                return Err(error("targetChanged"));
            }
            crate::config::atomic_write(&path, target.after_text.as_bytes())
                .map_err(|_| error("writeFailed"))?;
            let written = read(&path)?;
            if fingerprint(written.as_deref().map(str::as_bytes)) != target.after_hash {
                return Err(error("verifyFailed"));
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                target.status = "applied".into();
                target.reason = None;
            }
            Err(e) => {
                target.status = if e.to_string().contains("targetChanged")
                    || e.to_string().contains("sourceChanged")
                {
                    "conflict"
                } else {
                    "failed"
                }
                .into();
                target.reason = Some(e.to_string());
            }
        }
        let receipt = (target.status == "applied").then(|| target.clone());
        snapshot = persist(db, &snapshot, &op, receipt.as_ref())?;
        if op.kind == super::ResourceKind::Skills && op.targets[index].status == "applied" {
            if super::skill_resource::cleanup(&op.targets[index]).is_ok() {
                op.targets[index].directory = None;
            } else {
                op.targets[index].reason = Some("library.cleanupNeedsReview".into());
            }
        }
    }
    op.status = if op
        .targets
        .iter()
        .any(|target| target.reason.as_deref() == Some("library.cleanupNeedsReview"))
    {
        "needsReview"
    } else if op.targets.iter().all(|target| target.status == "applied") {
        "applied"
    } else {
        "partial"
    }
    .into();
    compact_finished(&mut op);
    persist(db, &snapshot, &op, None)?;
    Ok(op)
}

/// Crash recovery only verifies. It never resumes writes or revives stale epochs.
pub fn recover_operation(db: &Database, id: &str) -> Result<Operation, AppError> {
    if id.starts_with(super::batch::CHILD_PREFIX) {
        return Err(error("batchParentRequired"));
    }
    let _io = crate::config_io::lock();
    let mut snapshot = db.library_snapshot()?;
    let mut op = snapshot
        .machine
        .operations
        .get(id)
        .cloned()
        .ok_or_else(|| error("operationMissing"))?;
    if op.status == "previewed" || op.status == "applied" || op.status == "cancelled" {
        return Ok(op);
    }
    let current_version =
        confirm_version(db, &snapshot, &op).is_ok() && verify_routing(&op).is_ok();
    for index in 0..op.targets.len() {
        let sources_intact = verify_sources(&op.targets[index], &op.targets).is_ok();
        let target = &mut op.targets[index];
        if target.status == "applied"
            && (op.kind != super::ResourceKind::Skills || target.directory.is_none())
        {
            continue;
        }
        let path = PathBuf::from(&target.path);
        if op.kind == super::ResourceKind::Skills {
            let ownership = if target.status == "applied" {
                target.receipt.as_ref()
            } else {
                target.previous_receipt.as_ref()
            };
            let intact = current_version
                && snapshot.machine.receipts.get(&target.receipt_key) == ownership
                && super::skill_resource::entry_id(&path).ok().as_ref()
                    == Some(&target.physical_target)
                && super::skill_resource::entry_hash_with(
                    &path,
                    target
                        .directory
                        .as_ref()
                        .map(|plan| plan.artifact_hash.as_str()),
                )
                .ok()
                .as_ref()
                    == Some(&target.after_hash);
            if intact {
                target.status = "applied".into();
                target.reason = None;
                let receipt = target.clone();
                snapshot = persist(db, &snapshot, &op, Some(&receipt))?;
                if super::skill_resource::cleanup(&op.targets[index]).is_ok() {
                    op.targets[index].directory = None;
                } else {
                    op.targets[index].reason = Some("library.cleanupNeedsReview".into());
                }
            } else {
                target.status = "needsReview".into();
                target.reason = Some("library.operationNeedsReview".into());
            }
            continue;
        }
        let written = read(&path).ok().flatten();
        if current_version
            && sources_intact
            && snapshot.machine.receipts.get(&target.receipt_key)
                == target.previous_receipt.as_ref()
            && physical(&path)
                .ok()
                .is_some_and(|p| p.to_string_lossy() == target.physical_target)
            && fingerprint(written.as_deref().map(str::as_bytes)) == target.after_hash
        {
            target.status = "applied".into();
            target.reason = None;
            let receipt = target.clone();
            snapshot = persist(db, &snapshot, &op, Some(&receipt))?;
        } else {
            target.status = "needsReview".into();
            target.reason = Some(
                if current_version {
                    "library.operationNeedsReview"
                } else {
                    "library.epochChanged"
                }
                .into(),
            );
        }
    }
    op.status = if op
        .targets
        .iter()
        .all(|target| target.status == "applied" && target.directory.is_none())
    {
        "applied"
    } else {
        "needsReview"
    }
    .into();
    compact_finished(&mut op);
    persist(db, &snapshot, &op, None)?;
    Ok(op)
}

pub(crate) fn target_available(status: &str, _path: &Path) -> bool {
    status == "installed"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Database, tempfile::TempDir, PathBuf) {
        let db = Database::memory().unwrap();
        save_instruction(
            &db,
            InstructionSet {
                id: "fixture".into(),
                name: "Fixture".into(),
                content: "shared".into(),
                revision: 0,
            },
            None,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "user-owned\n").unwrap();
        (db, dir, path)
    }

    #[test]
    fn completed_operations_drop_full_payload_but_keep_machine_proof() {
        let (db, _dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path)]).unwrap();
        apply_operation(&db, &op.id).unwrap();
        let snapshot = db.library_snapshot().unwrap();
        let finished = &snapshot.machine.operations[&op.id].targets[0];
        assert!(finished.after_text.is_empty());
        assert!(finished.previous_receipt.is_none());
        assert!(finished.receipt.is_none());
        assert_eq!(snapshot.machine.receipts.len(), 1);
    }

    #[test]
    fn old_previews_expire_but_unresolved_operations_retain_evidence() {
        let (db, _dir, path) = setup();
        let prepared = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        next.machine
            .operations
            .get_mut(&prepared.id)
            .unwrap()
            .status = "prepared".into();
        db.library_commit(&before, &next).unwrap();
        let old = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        let mut newest = old.clone();
        for _ in 0..20 {
            newest = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        }
        let snapshot = db.library_snapshot().unwrap();
        assert!(
            snapshot
                .machine
                .operations
                .values()
                .filter(|op| op.status == "previewed")
                .count()
                <= 8
        );
        assert!(snapshot.machine.operations.contains_key(&newest.id));
        assert_eq!(
            snapshot.machine.operations[&prepared.id].targets[0].after_text,
            prepared.targets[0].after_text
        );
        assert!(!snapshot.machine.operations.contains_key(&old.id));
        assert!(apply_operation(&db, &old.id).is_err());
    }

    #[test]
    fn corrupt_journal_is_rejected_before_writing_any_file() {
        let (db, _dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        let mut snapshot = db.library_snapshot().unwrap();
        snapshot.machine.operations.get_mut(&op.id).unwrap().targets[0].after_text =
            "corrupt but valid JSON".into();
        db.set_setting(
            super::super::MACHINE_KEY,
            &serde_json::to_string(&snapshot.machine).unwrap(),
        )
        .unwrap();
        assert!(apply_operation(&db, &op.id).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user-owned\n");
    }

    #[test]
    fn prepared_operation_survives_closing_and_reopening_a_disk_database() {
        let (db, dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        next.machine.operations.get_mut(&op.id).unwrap().status = "prepared".into();
        next.machine
            .assignments
            .insert("instructions:codex".into(), "fixture".into());
        db.library_commit(&before, &next).unwrap();
        let db_path = dir.path().join("library.db");
        {
            let source = db.conn.lock().unwrap();
            let mut disk = rusqlite::Connection::open(&db_path).unwrap();
            rusqlite::backup::Backup::new(&source, &mut disk)
                .unwrap()
                .step(-1)
                .unwrap();
        }
        drop(db);
        std::fs::write(&path, &op.targets[0].after_text).unwrap();
        let reopened = Database {
            conn: std::sync::Mutex::new(rusqlite::Connection::open(&db_path).unwrap()),
        };
        assert_eq!(
            recover_operation(&reopened, &op.id).unwrap().status,
            "applied"
        );
        assert_eq!(
            reopened.library_snapshot().unwrap().machine.receipts.len(),
            1
        );
    }

    #[test]
    #[serial_test::serial]
    fn shared_readers_are_disclosed_without_granting_them_ownership() {
        let sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let mut settings = crate::settings::get_settings();
        let directory = sandbox.home().join("shared").to_string_lossy().into_owned();
        settings.codex_config_dir = Some(directory.clone());
        settings.opencode_config_dir = Some(directory);
        crate::settings::update_settings(settings).unwrap();
        save_instruction(
            &db,
            InstructionSet {
                id: "fixture".into(),
                name: "Fixture".into(),
                content: "shared".into(),
                revision: 0,
            },
            None,
        )
        .unwrap();
        let op = preview_instructions(&db, "fixture", &["codex".into()]).unwrap();
        assert!(op.targets[0].readers.contains(&"opencode".into()));
        assert_eq!(
            op.targets[0].receipt.as_ref().unwrap().owners,
            vec!["codex"]
        );
        apply_operation(&db, &op.id).unwrap();
        assert!(!db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .contains_key("instructions:opencode"));
    }

    #[test]
    #[serial_test::serial]
    fn a_sibling_joining_the_shared_path_requires_a_new_preview() {
        let sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        save_instruction(
            &db,
            InstructionSet {
                id: "fixture".into(),
                name: "Fixture".into(),
                content: "shared".into(),
                revision: 0,
            },
            None,
        )
        .unwrap();
        let old =
            crate::prompt_files::prompt_file_path(&crate::app_config::AppType::Codex).unwrap();
        crate::config::write_text_file(&old, "personal").unwrap();
        let op = preview_instructions(&db, "fixture", &["codex".into()]).unwrap();
        let mut settings = crate::settings::get_settings();
        settings.opencode_config_dir = Some(old.parent().unwrap().to_string_lossy().into_owned());
        crate::settings::update_settings(settings).unwrap();
        assert!(apply_operation(&db, &op.id).is_err());
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "personal");
        assert!(!sandbox.home().join("unrelated").exists());
    }

    #[test]
    fn partial_retry_does_not_replay_a_successful_targets_old_assignment() {
        let (db, dir, path) = setup();
        let second = dir.path().join("GEMINI.md");
        std::fs::write(&second, "personal").unwrap();
        let a = preview_at(
            &db,
            "fixture",
            &[
                ("codex".into(), path.clone()),
                ("gemini".into(), second.clone()),
            ],
        )
        .unwrap();
        std::fs::write(&second, "manual change").unwrap();
        assert_eq!(apply_operation(&db, &a.id).unwrap().status, "partial");
        save_instruction(
            &db,
            InstructionSet {
                id: "b".into(),
                name: "B".into(),
                content: "new set B".into(),
                revision: 0,
            },
            None,
        )
        .unwrap();
        let b = preview_at(&db, "b", &[("codex".into(), path.clone())]).unwrap();
        apply_operation(&db, &b.id).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        apply_operation(&db, &a.id).unwrap();
        let current = db.library_snapshot().unwrap();
        assert_eq!(current.machine.assignments["instructions:codex"], "b");
        assert_eq!(
            current
                .machine
                .receipts
                .values()
                .next()
                .unwrap()
                .resource_id,
            "b"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn withdrawing_unapplied_desired_never_touches_user_files() {
        let (db, _dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        std::fs::write(&path, "manual change").unwrap();
        apply_operation(&db, &op.id).unwrap();
        let op = preview_at(&db, "fixture", &[]).unwrap();
        apply_operation(&db, &op.id).unwrap();
        assert!(db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "manual change");
    }

    #[test]
    fn withdrawing_old_applied_does_not_clear_a_new_desired() {
        let (db, _dir, path) = setup();
        let a = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        apply_operation(&db, &a.id).unwrap();
        save_instruction(
            &db,
            InstructionSet {
                id: "b".into(),
                name: "B".into(),
                content: "new set B".into(),
                revision: 0,
            },
            None,
        )
        .unwrap();
        let b = preview_at(&db, "b", &[("codex".into(), path.clone())]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}new personal\n")).unwrap();
        apply_operation(&db, &b.id).unwrap();
        let remove = preview_at(&db, "fixture", &[]).unwrap();
        apply_operation(&db, &remove.id).unwrap();
        assert_eq!(
            db.library_snapshot().unwrap().machine.assignments["instructions:codex"],
            "b"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "user-owned\nnew personal\n"
        );
    }

    #[test]
    fn oversized_patched_file_is_rejected_before_any_write() {
        let (db, _dir, path) = setup();
        save_instruction(
            &db,
            InstructionSet {
                id: "fixture".into(),
                name: "Fixture".into(),
                content: "x".repeat(1024 * 1024),
                revision: 0,
            },
            Some(1),
        )
        .unwrap();
        let original = "u".repeat(1536 * 1024);
        std::fs::write(&path, &original).unwrap();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        assert_ne!(op.targets[0].status, "pending");
        apply_operation(&db, &op.id).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    #[serial_test::serial]
    fn changing_the_native_resolver_invalidates_confirmation() {
        let sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        save_instruction(
            &db,
            InstructionSet {
                id: "fixture".into(),
                name: "Fixture".into(),
                content: "shared".into(),
                revision: 0,
            },
            None,
        )
        .unwrap();
        let old =
            crate::prompt_files::prompt_file_path(&crate::app_config::AppType::Codex).unwrap();
        crate::config::write_text_file(&old, "old personal").unwrap();
        let op = preview_instructions(&db, "fixture", &["codex".into()]).unwrap();
        let mut settings = crate::settings::get_settings();
        settings.codex_config_dir = Some(
            sandbox
                .home()
                .join("new-codex")
                .to_string_lossy()
                .into_owned(),
        );
        crate::settings::update_settings(settings).unwrap();
        assert!(apply_operation(&db, &op.id).is_err());
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "old personal");
    }

    #[test]
    fn changed_ownership_invalidates_preview_even_if_file_bytes_are_identical() {
        let (db, _dir, path) = setup();
        let first = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        apply_operation(&db, &first.id).unwrap();
        let old = preview_at(&db, "fixture", &[]).unwrap();
        let expanded = preview_at(
            &db,
            "fixture",
            &[
                ("codex".into(), path.clone()),
                ("claude".into(), path.clone()),
            ],
        )
        .unwrap();
        apply_operation(&db, &expanded.id).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let result = apply_operation(&db, &old.id).unwrap();
        assert_eq!(result.targets[0].status, "conflict");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert_eq!(
            db.library_snapshot()
                .unwrap()
                .machine
                .receipts
                .values()
                .next()
                .unwrap()
                .owners
                .len(),
            2
        );
    }

    #[test]
    fn desired_survives_a_conflict_without_claiming_applied() {
        let (db, _dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        std::fs::write(&path, "manual change").unwrap();
        apply_operation(&db, &op.id).unwrap();
        let state = db.library_snapshot().unwrap();
        assert_eq!(
            state
                .machine
                .assignments
                .get("instructions:codex")
                .map(String::as_str),
            Some("fixture")
        );
        assert!(state.machine.receipts.is_empty());
    }

    #[test]
    fn preview_is_read_only_and_apply_is_idempotent() {
        let (db, _dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user-owned\n");
        assert!(db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
        assert_eq!(op.status, "previewed");
        let applied = apply_operation(&db, &op.id).unwrap();
        assert_eq!(applied.status, "applied");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("user-owned\n"));
        assert_eq!(apply_operation(&db, &op.id).unwrap(), applied);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn preview_staleness_does_not_overwrite_user_changes() {
        let (db, _dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        std::fs::write(&path, "manual change").unwrap();
        let result = apply_operation(&db, &op.id).unwrap();
        assert_eq!(result.targets[0].status, "conflict");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "manual change");
        assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
    }

    #[test]
    fn withdrawing_last_owner_removes_only_the_managed_segment() {
        let (db, _dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        apply_operation(&db, &op.id).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}new user line\n")).unwrap();
        let op = preview_at(&db, "fixture", &[]).unwrap();
        assert_eq!(apply_operation(&db, &op.id).unwrap().status, "applied");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "user-owned\nnew user line\n"
        );
        assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
    }

    #[test]
    fn a_shared_physical_target_retains_the_other_owners_content() {
        let (db, _dir, path) = setup();
        let op = preview_at(
            &db,
            "fixture",
            &[
                ("codex".into(), path.clone()),
                ("claude".into(), path.clone()),
            ],
        )
        .unwrap();
        assert_eq!(op.targets.len(), 1);
        apply_operation(&db, &op.id).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let op = preview_at(&db, "fixture", &[("claude".into(), path.clone())]).unwrap();
        apply_operation(&db, &op.id).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        let receipt = db
            .library_snapshot()
            .unwrap()
            .machine
            .receipts
            .values()
            .next()
            .unwrap()
            .clone();
        assert_eq!(receipt.owners, vec!["claude"]);
    }

    #[test]
    fn recover_after_write_proves_receipt_without_repeating_the_write() {
        let (db, _dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        let before = db.library_snapshot().unwrap();
        let mut prepared = before.clone();
        prepared.machine.operations.get_mut(&op.id).unwrap().status = "prepared".into();
        db.library_commit(&before, &prepared).unwrap();
        std::fs::write(&path, &op.targets[0].after_text).unwrap();
        let recovered = recover_operation(&db, &op.id).unwrap();
        assert_eq!(recovered.targets[0].status, "applied");
        assert_eq!(db.library_snapshot().unwrap().machine.receipts.len(), 1);
    }

    #[test]
    fn recovery_never_resumes_unwritten_targets_or_old_epochs() {
        let (db, _dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        let before = db.library_snapshot().unwrap();
        let mut prepared = before.clone();
        prepared.machine.operations.get_mut(&op.id).unwrap().status = "prepared".into();
        db.library_commit(&before, &prepared).unwrap();
        let result = recover_operation(&db, &op.id).unwrap();
        assert_eq!(result.targets[0].status, "needsReview");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user-owned\n");
        let before = db.library_snapshot().unwrap();
        let mut restored = before.clone();
        restored.machine.epoch += 1;
        db.library_commit(&before, &restored).unwrap();
        std::fs::write(&path, &op.targets[0].after_text).unwrap();
        assert!(apply_operation(&db, &op.id).is_err());
        recover_operation(&db, &op.id).unwrap();
        assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
    }

    #[test]
    fn updating_definition_invalidates_older_confirmation() {
        let (db, _dir, path) = setup();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        save_instruction(
            &db,
            InstructionSet {
                id: "fixture".into(),
                name: "Fixture".into(),
                content: "updated".into(),
                revision: 0,
            },
            Some(1),
        )
        .unwrap();
        assert!(apply_operation(&db, &op.id).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user-owned\n");
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn home_alone_cannot_advertise_a_resource_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        for status in ["notInstalled", "unknown", ""] {
            assert!(!target_available(status, &path));
        }
        assert!(target_available("installed", &path));
    }
}
