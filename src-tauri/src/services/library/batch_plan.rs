//! Reuse the existing adapters' guard plans in an isolated database, then
//! combine their disjoint leaves. The isolated planner never applies a plan.
use super::*;
use crate::services::library::{mcp_codec, mcp_resource, skill_resource, TargetWrite};
use std::path::Path;

fn assignment(kind: &ResourceKind, id: &str, tool: &str) -> String {
    match kind {
        ResourceKind::Instructions => format!("instructions:{tool}"),
        ResourceKind::Mcp => format!("mcp:{id}:{tool}"),
        ResourceKind::Skills => format!("skills:{id}:{tool}"),
    }
}
fn tool_supported(kind: &ResourceKind, tool: &str) -> bool {
    if *kind == ResourceKind::Skills {
        return tool == "shared" || crate::services::library::skill_targets::find(tool).is_some();
    }
    if *kind == ResourceKind::Mcp {
        return crate::services::library::mcp_targets::TOOLS.contains(&tool);
    }
    ["claude", "codex", "gemini", "opencode", "hermes"].contains(&tool)
}
fn planner_db(
    db: &Database,
    snapshot: &LibrarySnapshot,
    kind: &ResourceKind,
    id: &str,
) -> Result<Database, AppError> {
    let memory = Database::memory()?;
    if *kind == ResourceKind::Mcp {
        let server = db
            .get_all_mcp_servers()?
            .shift_remove(id)
            .ok_or_else(|| error("resourceMissing"))?;
        memory.save_mcp_server(&server)?;
    }
    if *kind == ResourceKind::Skills {
        let skill = db
            .get_installed_skill(id)?
            .ok_or_else(|| error("resourceMissing"))?;
        memory.save_skill(&skill)?;
    }
    memory.set_setting(
        crate::services::library::DEFINITIONS_KEY,
        &serde_json::to_string(&snapshot.definitions).map_err(|_| error("stateUnreadable"))?,
    )?;
    memory.set_setting(
        crate::services::library::MACHINE_KEY,
        &serde_json::to_string(&snapshot.machine).map_err(|_| error("stateUnreadable"))?,
    )?;
    Ok(memory)
}
pub(super) fn build(
    db: &Database,
    snapshot: &LibrarySnapshot,
    kind: ResourceKind,
    resources: Vec<BatchResource>,
    tools: Vec<String>,
    action: BatchAction,
) -> Result<Journal, AppError> {
    let mut plans = Vec::new();
    let mut names = BTreeMap::new();
    for request in resources {
        if request.expected_revision == 0 {
            return Err(error("revisionChanged"));
        }
        let mut add: BTreeSet<_> = request.add_tools.into_iter().collect();
        let mut withdraw: BTreeSet<_> = request.withdraw_tools.into_iter().collect();
        match action {
            BatchAction::Apply => add.extend(tools.iter().cloned()),
            BatchAction::Withdraw => withdraw.extend(tools.iter().cloned()),
        }
        if !add.is_disjoint(&withdraw)
            || add
                .iter()
                .chain(withdraw.iter())
                .any(|tool| !tool_supported(&kind, tool))
        {
            return Err(error("batchInvalidSelection"));
        }
        let changed: BTreeSet<_> = add.union(&withdraw).cloned().collect();
        if changed.is_empty() {
            return Err(error("batchInvalidSelection"));
        }
        let id = &request.resource_id;
        let revision = match kind {
            ResourceKind::Instructions => snapshot
                .definitions
                .instructions
                .get(id)
                .map(|s| s.revision),
            ResourceKind::Mcp => snapshot.definitions.mcp.get(id).map(|m| m.revision),
            ResourceKind::Skills => snapshot.definitions.skills.get(id).map(|m| m.revision),
        };
        if revision != Some(request.expected_revision) {
            return Err(error("revisionChanged"));
        }
        let name = match kind {
            ResourceKind::Instructions => snapshot
                .definitions
                .instructions
                .get(id)
                .map(|set| set.name.clone()),
            ResourceKind::Mcp => db
                .get_all_mcp_servers()?
                .get(id)
                .map(|server| server.name.clone()),
            ResourceKind::Skills => db.get_installed_skill(id)?.map(|skill| skill.name),
        }
        .ok_or_else(|| error("resourceMissing"))?;
        names.insert(id.clone(), name);
        let known: Vec<&str> = if kind == ResourceKind::Skills {
            crate::services::library::skill_targets::all()
                .map(|target| target.id)
                .chain(["shared"])
                .collect()
        } else if kind == ResourceKind::Mcp {
            crate::services::library::mcp_targets::TOOLS.to_vec()
        } else {
            vec!["claude", "codex", "gemini", "opencode", "hermes", "shared"]
        };
        let mut selected: BTreeSet<String> = known
            .into_iter()
            .filter(|tool| {
                snapshot
                    .machine
                    .assignments
                    .get(&assignment(&kind, id, tool))
                    == Some(id)
            })
            .map(str::to_owned)
            .collect();
        selected.extend(add);
        selected.retain(|tool| !withdraw.contains(tool));
        let full: Vec<_> = selected.into_iter().collect();
        let memory = planner_db(db, snapshot, &kind, id)?;
        let mut operation = match kind {
            ResourceKind::Instructions => io::preview_instructions(&memory, id, &full)?,
            ResourceKind::Mcp => mcp_resource::preview(&memory, id, &full)?,
            ResourceKind::Skills => skill_resource::preview(&memory, id, &full)?,
        };
        let paths = match kind {
            ResourceKind::Instructions => io::native_instruction_paths()?,
            ResourceKind::Mcp => mcp_resource::native_paths()?,
            ResourceKind::Skills => skill_resource::target_paths()?,
        };
        let mut affected = BTreeSet::new();
        for tool in &changed {
            let root = paths.get(tool).ok_or_else(|| error("targetUnsupported"))?;
            let physical = if kind == ResourceKind::Skills {
                let row = db
                    .get_installed_skill(id)?
                    .ok_or_else(|| error("resourceMissing"))?;
                skill_resource::entry_id(&root.join(row.directory))?
            } else {
                io::physical_id(root)?
            };
            affected.insert(physical);
        }
        for receipt in snapshot.machine.receipts.values().filter(|r| {
            r.kind == kind
                && r.resource_id == *id
                && r.owners.iter().any(|tool| changed.contains(tool))
        }) {
            affected.insert(receipt.physical_target.clone());
        }
        operation
            .targets
            .retain(|target| affected.contains(&target.physical_target));
        let keys: BTreeSet<_> = changed
            .iter()
            .map(|tool| assignment(&kind, id, tool))
            .collect();
        operation
            .desired_updates
            .retain(|key, _| keys.contains(key));
        operation
            .previous_assignments
            .retain(|key, _| keys.contains(key));
        for target in &mut operation.targets {
            target
                .assignment_updates
                .retain(|key, _| keys.contains(key));
        }
        plans.push(operation);
    }
    let operation_id = uuid::Uuid::new_v4().to_string();
    let metadata_applied = !plans
        .iter()
        .any(|op| op.targets.is_empty() && !op.desired_updates.is_empty());
    let mut journal = Journal {
        operation_id,
        kind,
        epoch: snapshot.machine.epoch,
        created_at: chrono::Utc::now().timestamp_millis(),
        status: "previewed".into(),
        metadata_applied,
        names,
        plans,
        groups: vec![],
        children: BTreeMap::new(),
    };
    let mut by_physical: BTreeMap<String, Vec<Member>> = BTreeMap::new();
    for (child, operation) in journal.plans.iter().enumerate() {
        for (target, value) in operation.targets.iter().enumerate() {
            by_physical
                .entry(value.physical_target.clone())
                .or_default()
                .push(Member { child, target });
        }
    }
    for (physical, members) in by_physical {
        let first = &journal.plans[members[0].child].targets[members[0].target];
        let mut group = Group {
            group_id: io::fingerprint(Some(physical.as_bytes())),
            path: first.path.clone(),
            physical_target: physical,
            before_hash: first.before_hash.clone(),
            after_hash: first.after_hash.clone(),
            after_text: first.after_text.clone(),
            status: "pending".into(),
            reason: None,
            prepared: false,
            members,
        };
        let result = compose(db, &mut journal.plans, &mut group, &journal.kind);
        if let Err(failure) = result {
            group.status = "conflict".into();
            group.reason = Some(reason(&failure));
        }
        journal.groups.push(group);
    }
    if !journal.groups.is_empty() && journal.groups.iter().all(|g| g.status == "conflict") {
        journal.status = "blocked".into();
    }
    Ok(journal)
}
fn compose(
    db: &Database,
    plans: &mut [Operation],
    group: &mut Group,
    kind: &ResourceKind,
) -> Result<(), AppError> {
    for member in &group.members {
        let target = &plans[member.child].targets[member.target];
        if target.status != "pending" || target.after_hash.is_empty() {
            return Err(AppError::Config(
                target
                    .reason
                    .clone()
                    .unwrap_or_else(|| "library.operationNeedsReview".into()),
            ));
        }
        if target.before_hash != group.before_hash {
            return Err(error("targetChanged"));
        }
    }
    if *kind == ResourceKind::Instructions {
        return compose_instructions(db, plans, group);
    }
    if *kind != ResourceKind::Mcp {
        if group.members.len() > 1 {
            return Err(error(if *kind == ResourceKind::Instructions {
                "sharedTargetConflict"
            } else {
                "skillNameConflict"
            }));
        }
        return Ok(());
    }
    let mut text = io::read(Path::new(&group.path))?;
    if io::fingerprint(text.as_deref().map(str::as_bytes)) != group.before_hash {
        return Err(error("targetChanged"));
    }
    let servers = db.get_all_mcp_servers()?;
    let resources: Vec<String> = group
        .members
        .iter()
        .map(|member| plans[member.child].resource_id.clone())
        .collect();
    let physical = plans[group.members[0].child].targets[group.members[0].target]
        .physical_target
        .clone();
    // Proofs on this file from servers outside the batch, then each one composed below.
    let mut siblings: Vec<mcp_codec::OwnedFields> = db
        .library_snapshot()?
        .machine
        .receipts
        .values()
        .filter(|receipt| {
            receipt.kind == ResourceKind::Mcp
                && receipt.physical_target == physical
                && !resources.contains(&receipt.resource_id)
        })
        .filter_map(|receipt| serde_json::from_str(&receipt.owned_segment).ok())
        .collect();
    // Stable order is important for ownership of containers introduced by a peer.
    group
        .members
        .sort_by(|a, b| plans[a.child].resource_id.cmp(&plans[b.child].resource_id));
    for member in &group.members {
        let operation = &mut plans[member.child];
        let target = &mut operation.targets[member.target];
        let old = target
            .previous_receipt
            .as_ref()
            .map(|r| {
                serde_json::from_str::<mcp_codec::OwnedFields>(&r.owned_segment)
                    .map_err(|_| error("stateUnreadable"))
            })
            .transpose()?;
        let formatter = target
            .receipt
            .as_ref()
            .or(target.previous_receipt.as_ref())
            .map(|r| {
                serde_json::from_str::<mcp_codec::OwnedFields>(&r.owned_segment)
                    .map(|o| o.tool)
                    .map_err(|_| error("stateUnreadable"))
            })
            .transpose()?
            .ok_or_else(|| error("stateUnreadable"))?;
        let native = if target.receipt.is_some() {
            let server = servers
                .get(&operation.resource_id)
                .ok_or_else(|| error("resourceMissing"))?;
            let mut native =
                mcp_codec::native_server(&formatter, &operation.resource_id, &server.server)?;
            if formatter == "claude" {
                crate::claude_mcp::prepare_mcp_spec_for_path(&mut native, Path::new(&target.path));
            }
            Some(native)
        } else {
            None
        };
        let (after, owned) = mcp_codec::patch_server(
            text.as_deref(),
            &formatter,
            &operation.resource_id,
            old.as_ref(),
            native.as_ref(),
        )?;
        if let (Some(receipt), Some(mut owned)) = (&mut target.receipt, owned) {
            mcp_codec::inherit_created_section(&mut owned, &siblings);
            receipt.owned_segment =
                serde_json::to_string(&owned).map_err(|_| error("stateUnreadable"))?;
            siblings.push(owned);
        }
        text = Some(after);
    }
    group.after_text = text.unwrap_or_default();
    if group.after_text.len() > 2 * 1024 * 1024 {
        return Err(error("targetTooLarge"));
    }
    group.after_hash = io::fingerprint(Some(group.after_text.as_bytes()));
    for member in &group.members {
        let target = &mut plans[member.child].targets[member.target];
        for reader in &target.readers {
            mcp_codec::parse_native(&group.after_text, reader)?;
        }
        if let Some(receipt) = &target.receipt {
            let owned = serde_json::from_str(&receipt.owned_segment)
                .map_err(|_| error("stateUnreadable"))?;
            mcp_codec::verify_owned_text(&group.after_text, &owned)?;
        }
        target.after_hash = group.after_hash.clone();
        target.after_text.clear();
    }
    Ok(())
}

fn compose_instructions(
    db: &Database,
    plans: &mut [Operation],
    group: &mut Group,
) -> Result<(), AppError> {
    let mut old = None;
    let mut updates: BTreeMap<String, Option<String>> = BTreeMap::new();
    for member in &group.members {
        let target = &plans[member.child].targets[member.target];
        if let Some(receipt) = &target.previous_receipt {
            if old.as_ref().is_some_and(|other| other != receipt) {
                return Err(error("ownershipChanged"));
            }
            old = Some(receipt.clone());
        }
        for (key, value) in &target.assignment_updates {
            match (updates.get(key).and_then(Option::as_ref), value.as_ref()) {
                (Some(before), Some(after)) if before != after => {
                    return Err(error("sharedTargetConflict"))
                }
                (_, Some(_)) => {
                    updates.insert(key.clone(), value.clone());
                }
                _ => {
                    updates.entry(key.clone()).or_insert(None);
                }
            }
        }
    }
    let mut owners: BTreeMap<String, String> = old
        .as_ref()
        .map(|receipt| {
            receipt
                .owners
                .iter()
                .map(|tool| (tool.clone(), receipt.resource_id.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (key, value) in &updates {
        let tool = key
            .strip_prefix("instructions:")
            .ok_or_else(|| error("stateUnreadable"))?;
        if let Some(value) = value {
            owners.insert(tool.into(), value.clone());
        } else {
            owners.remove(tool);
        }
    }
    let sets: BTreeSet<_> = owners.values().cloned().collect();
    if sets.len() > 1 {
        return Err(error("sharedTargetConflict"));
    }
    let set = sets
        .iter()
        .next()
        .map(|id| {
            db.library_snapshot()?
                .definitions
                .instructions
                .get(id)
                .cloned()
                .ok_or_else(|| error("resourceMissing"))
        })
        .transpose()?;
    let text = io::read(Path::new(&group.path))?;
    if io::fingerprint(text.as_deref().map(str::as_bytes)) != group.before_hash {
        return Err(error("targetChanged"));
    }
    let patch = crate::services::library::patch_instructions(
        text.as_deref().unwrap_or(""),
        old.as_ref().map(|r| r.owned_segment.as_str()),
        set.as_ref().map(|s| s.content.as_str()),
    )?;
    group.after_text = patch.text;
    group.after_hash = io::fingerprint(Some(group.after_text.as_bytes()));
    for member in &group.members {
        let operation = &mut plans[member.child];
        let target = &mut operation.targets[member.target];
        for (key, value) in &mut target.assignment_updates {
            *value = updates
                .get(key)
                .cloned()
                .ok_or_else(|| error("stateUnreadable"))?;
        }
        target.after_hash = group.after_hash.clone();
        target.after_text.clear();
        target.receipt = match (&set, &patch.owned_segment) {
            (Some(set), Some(segment)) if operation.resource_id == set.id => {
                Some(crate::services::library::Receipt {
                    kind: ResourceKind::Instructions,
                    resource_id: set.id.clone(),
                    resource_revision: set.revision,
                    definition_hash: io::fingerprint(Some(set.content.as_bytes())),
                    physical_target: group.physical_target.clone(),
                    path: group.path.clone(),
                    owners: owners.keys().cloned().collect(),
                    owned_segment: segment.clone(),
                })
            }
            _ => None,
        };
        target.tools = target
            .receipt
            .as_ref()
            .map(|r| r.owners.clone())
            .unwrap_or_default();
    }
    Ok(())
}
pub(super) fn verify(
    db: &Database,
    snapshot: &LibrarySnapshot,
    journal: &Journal,
    index: usize,
    after: bool,
) -> Result<(), AppError> {
    if snapshot.machine.epoch != journal.epoch {
        return Err(error("epochChanged"));
    }
    let group = &journal.groups[index];
    let mut all: Vec<TargetWrite> = journal
        .plans
        .iter()
        .flat_map(|plan| plan.targets.iter().cloned())
        .collect();
    for target in &mut all {
        if journal
            .groups
            .iter()
            .any(|g| g.physical_target == target.physical_target && g.status == "applied")
        {
            target.status = "applied".into();
        }
    }
    for member in &group.members {
        let operation = &journal.plans[member.child];
        let target = &operation.targets[member.target];
        io::confirm_version(db, snapshot, operation)?;
        io::verify_routing(operation)?;
        io::verify_sources(target, &all)?;
        let expected_receipt = if group.status == "applied" {
            target.receipt.as_ref()
        } else {
            target.previous_receipt.as_ref()
        };
        if snapshot.machine.receipts.get(&target.receipt_key) != expected_receipt {
            return Err(error("ownershipChanged"));
        }
        for (key, value) in &target.assignment_updates {
            let expected = if group.prepared {
                value.as_ref()
            } else {
                operation
                    .previous_assignments
                    .get(key)
                    .and_then(Option::as_ref)
            };
            if snapshot.machine.assignments.get(key) != expected {
                return Err(error("desiredChanged"));
            }
        }
    }
    let physical = if journal.kind == ResourceKind::Skills {
        skill_resource::entry_id(Path::new(&group.path))?
    } else {
        io::physical_id(Path::new(&group.path))?
    };
    if physical != group.physical_target {
        return Err(error("targetChanged"));
    }
    let hash = if journal.kind == ResourceKind::Skills {
        let member = &group.members[0];
        let target = &journal.plans[member.child].targets[member.target];
        let previous = target
            .previous_receipt
            .as_ref()
            .map(|receipt| {
                serde_json::from_str::<skill_resource::DirectoryOwned>(&receipt.owned_segment)
                    .map_err(|_| error("stateUnreadable"))
            })
            .transpose()?;
        if after {
            skill_resource::entry_hash_with(
                Path::new(&group.path),
                target.transfer.as_ref().map(|s| s.artifact_hash.as_str()),
            )?
        } else {
            skill_resource::owned_hash(Path::new(&group.path), previous.as_ref())?
        }
    } else {
        io::fingerprint(
            io::read(Path::new(&group.path))?
                .as_deref()
                .map(str::as_bytes),
        )
    };
    if &hash
        != if after {
            &group.after_hash
        } else {
            &group.before_hash
        }
    {
        return Err(error("targetChanged"));
    }
    Ok(())
}
pub(super) fn write_group(journal: &Journal, index: usize) -> Result<(), AppError> {
    let group = &journal.groups[index];
    if journal.kind == ResourceKind::Skills {
        let member = &group.members[0];
        return skill_resource::write(&journal.plans[member.child].targets[member.target]);
    }
    if io::physical_id(Path::new(&group.path))? != group.physical_target
        || io::fingerprint(
            io::read(Path::new(&group.path))?
                .as_deref()
                .map(str::as_bytes),
        ) != group.before_hash
    {
        return Err(error("targetChanged"));
    }
    if group.before_hash != group.after_hash {
        // Literal MCP secrets keep the file owner-only, as a single apply does.
        let private = group
            .members
            .iter()
            .any(|m| journal.plans[m.child].targets[m.target].private_values);
        let path = Path::new(&group.path);
        let mode =
            private.then(|| crate::config::file_mode(path).map_or(0o600, |mode| mode & 0o700));
        crate::config::atomic_write_with(
            path,
            group.after_text.as_bytes(),
            crate::config::AtomicOpts { mode },
        )
        .map_err(|_| error("writeFailed"))?;
    }
    if io::fingerprint(
        io::read(Path::new(&group.path))?
            .as_deref()
            .map(str::as_bytes),
    ) != group.after_hash
    {
        return Err(error("verifyFailed"));
    }
    Ok(())
}
pub(super) fn record_group(journal: &mut Journal, index: usize, next: &mut LibrarySnapshot) {
    // Withdrawals and additions may target the same instruction receipt key.
    // Remove old claims first, then publish the one final surviving claim.
    for member in &journal.groups[index].members {
        let target = &journal.plans[member.child].targets[member.target];
        next.machine.receipts.remove(&target.receipt_key);
    }
    for member in &journal.groups[index].members {
        let target = &mut journal.plans[member.child].targets[member.target];
        target.status = "applied".into();
        if let Some(receipt) = &target.receipt {
            next.machine
                .receipts
                .insert(target.receipt_key.clone(), receipt.clone());
        }
    }
}
