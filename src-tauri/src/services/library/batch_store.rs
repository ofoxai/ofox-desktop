//! Separate machine-only contract. It does not extend the v1 MachineState JSON.
use super::*;
use rusqlite::{Connection, OptionalExtension};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BatchStore {
    pub schema_version: u32,
    pub revision: u64,
    pub(super) operations: BTreeMap<String, Journal>,
}
impl Default for BatchStore {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            operations: BTreeMap::new(),
        }
    }
}
pub(crate) fn load_on(
    conn: &Connection,
    library: &LibrarySnapshot,
) -> Result<BatchStore, AppError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            [SETTING_KEY],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| error("stateUnreadable"))?;
    let state: BatchStore = match raw {
        Some(raw) => {
            if raw.len() > MAX_STORE_BYTES {
                return Err(error("batchTooLarge"));
            }
            serde_json::from_str(&raw).map_err(|_| error("stateUnreadable"))?
        }
        None => BatchStore::default(),
    };
    validate(&state, library)?;
    Ok(state)
}
pub(crate) fn validate(state: &BatchStore, library: &LibrarySnapshot) -> Result<(), AppError> {
    if state.schema_version != 1 {
        return Err(error("stateVersionUnsupported"));
    }
    let mut claimed = BTreeSet::new();
    for (id, journal) in &state.operations {
        if id != &journal.operation_id
            || uuid::Uuid::parse_str(id).is_err()
            || journal.plans.is_empty()
            || journal.plans.len() > MAX_RESOURCES
            || journal.children.len() != journal.plans.len()
            || journal.groups.len() > MAX_GROUPS
            || ![
                "previewed",
                "blocked",
                "prepared",
                "partial",
                "applied",
                "needsReview",
                "cancelled",
            ]
            .contains(&journal.status.as_str())
        {
            return Err(error("stateUnreadable"));
        }
        let mut membership = BTreeSet::new();
        for (index, operation) in journal.plans.iter().enumerate() {
            if operation.kind != journal.kind
                || operation.epoch != journal.epoch
                || operation.resource_revision == 0
            {
                return Err(error("stateUnreadable"));
            }
            let child = child_id(journal, index);
            let stored = library
                .machine
                .operations
                .get(&child)
                .ok_or_else(|| error("batchParentRequired"))?;
            if journal.children.get(&child) != Some(&digest(stored)?)
                || stored.resource_id != operation.resource_id
                || stored.resource_hash != operation.resource_hash
                || stored.resource_revision != operation.resource_revision
                || !claimed.insert(child)
            {
                return Err(error("batchParentRequired"));
            }
        }
        let mut physical = BTreeSet::new();
        for group in &journal.groups {
            if group.members.is_empty()
                || group.group_id != io::fingerprint(Some(group.physical_target.as_bytes()))
                || !std::path::Path::new(&group.path).is_absolute()
                || !std::path::Path::new(&group.physical_target).is_absolute()
                || !physical.insert(&group.physical_target)
                || ![
                    "pending",
                    "prepared",
                    "applied",
                    "conflict",
                    "failed",
                    "needsReview",
                ]
                .contains(&group.status.as_str())
            {
                return Err(error("stateUnreadable"));
            }
            if !group.after_hash.is_empty()
                && journal.kind != ResourceKind::Skills
                && io::fingerprint(Some(group.after_text.as_bytes())) != group.after_hash
            {
                return Err(error("stateUnreadable"));
            }
            for member in &group.members {
                let target = journal
                    .plans
                    .get(member.child)
                    .and_then(|p| p.targets.get(member.target))
                    .ok_or_else(|| error("stateUnreadable"))?;
                if target.physical_target != group.physical_target
                    || !membership.insert((member.child, member.target))
                {
                    return Err(error("stateUnreadable"));
                }
                if group.status != "conflict" && target.after_hash != group.after_hash {
                    return Err(error("stateUnreadable"));
                }
                if journal.kind == ResourceKind::Skills {
                    super::super::skill_resource::validate_target(
                        target,
                        &journal.plans[member.child],
                    )?;
                } else if let Some(receipt) = &target.receipt {
                    if journal.kind == ResourceKind::Mcp {
                        let owned = serde_json::from_str(&receipt.owned_segment)
                            .map_err(|_| error("stateUnreadable"))?;
                        if group.status != "conflict" {
                            super::super::mcp_codec::verify_owned_text(&group.after_text, &owned)?;
                        }
                    } else if group.status != "conflict"
                        && group.after_text.matches(&receipt.owned_segment).count() != 1
                    {
                        return Err(error("stateUnreadable"));
                    }
                }
            }
        }
        if membership.len() != journal.plans.iter().map(|p| p.targets.len()).sum::<usize>() {
            return Err(error("stateUnreadable"));
        }
        let mut projected = journal.clone();
        let mut expected = library.clone();
        project_children(&mut projected, &mut expected)?;
        if projected.children != journal.children
            || projected.children.keys().any(|child| {
                expected.machine.operations.get(child) != library.machine.operations.get(child)
            })
        {
            return Err(error("batchParentRequired"));
        }
    }
    if library
        .machine
        .operations
        .keys()
        .any(|id| id.starts_with(CHILD_PREFIX) && !claimed.contains(id))
    {
        return Err(error("batchParentRequired"));
    }
    Ok(())
}
pub(crate) fn restore_local(
    source: &Connection,
    target: &Connection,
    new_epoch: u64,
) -> Result<(), AppError> {
    let local = super::super::store::load(source)?;
    let mut batches = load_on(source, &local)?;
    batches.revision = batches
        .revision
        .checked_add(1)
        .ok_or_else(|| error("revisionOverflow"))?;
    for journal in batches.operations.values_mut() {
        if ["applied", "cancelled"].contains(&journal.status.as_str()) {
            continue;
        }
        journal.status = "needsReview".into();
        for group in &mut journal.groups {
            if group.status != "applied" {
                group.status = "needsReview".into();
                group.reason = Some("library.epochChanged".into());
            }
        }
    }
    // The caller has preserved local child journals, incremented epoch, and owns
    // this transaction. Re-project child digests after its recovery invalidation.
    let mut library = super::super::store::load(target)?;
    if library.machine.epoch != new_epoch {
        return Err(error("epochChanged"));
    }
    for journal in batches.operations.values_mut() {
        project_children(journal, &mut library)?;
    }
    validate(&batches, &library)?;
    let raw = serde_json::to_string(&batches).map_err(|_| error("stateUnreadable"))?;
    target
        .execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
            rusqlite::params![SETTING_KEY, raw],
        )
        .map_err(|_| error("stateUnreadable"))?;
    target
        .execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
            rusqlite::params![
                crate::services::library::MACHINE_KEY,
                serde_json::to_string(&library.machine).map_err(|_| error("stateUnreadable"))?
            ],
        )
        .map_err(|_| error("stateUnreadable"))?;
    Ok(())
}
