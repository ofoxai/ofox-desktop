//! Read-only native resource discovery. Private import definitions never cross IPC.
use super::{coordinator as io, ResourceKind};
use crate::{
    app_config::{InstalledSkill, McpServer},
    error::AppError,
    Database,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[path = "inventory_builtin.rs"]
mod builtin;
#[path = "inventory_details.rs"]
mod details;
pub use builtin::{match_builtin_recipes, BuiltinRecipe};
#[path = "inventory_readers.rs"]
mod readers;
use details::SourceLocation;
pub use details::{
    compare_mcp, match_recipes, mcp_service_host, reveal_source, source_detail,
    InventorySourceDetail, McpComparison, McpComparisonSubject, McpRecipe, McpRecipeMatch,
};
const TTL: Duration = Duration::from_secs(600);
const MAX_TICKETS: usize = 8;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 2000;
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventorySource {
    pub source_id: String,
    pub tool_id: String,
    pub label: String,
    pub state: String,
    pub reason: Option<String>,
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(skip)]
    location: Option<SourceLocation>,
    #[serde(skip)]
    proofs: Vec<Proof>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySource {
    pub source_id: String,
    pub tool_id: String,
    /// The tool keeps this MCP entry switched off (`enabled = false`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryEntry {
    pub entry_id: String,
    pub kind: ResourceKind,
    pub name: String,
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_host: Option<String>,
    pub sources: Vec<EntrySource>,
    pub matched_resource_ids: Vec<String>,
    pub conflict: bool,
    pub variant: Option<String>,
    pub directory: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryInventory {
    pub ticket_id: String,
    pub kind: ResourceKind,
    pub expires_at_ms: i64,
    pub observed_at_ms: i64,
    pub sources: Vec<InventorySource>,
    pub entries: Vec<InventoryEntry>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedItem {
    pub entry_id: String,
    pub resource_id: Option<String>,
    pub name: String,
    pub status: String,
    pub reason: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryImport {
    pub imported: usize,
    pub items: Vec<ImportedItem>,
}
#[derive(Clone)]
enum Definition {
    Instructions {
        content: String,
    },
    Mcp {
        spec: serde_json::Value,
    },
    Skill {
        path: PathBuf,
        hash: String,
        directory: String,
        description: Option<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Proof {
    File {
        path: PathBuf,
        physical: String,
        hash: String,
    },
    Legacy {
        tool: String,
        id: String,
        hash: String,
    },
    Tree {
        path: PathBuf,
        physical: String,
        hash: String,
    },
}
#[derive(Clone)]
struct Candidate {
    view: InventoryEntry,
    definition: Definition,
    proofs: Vec<Proof>,
    hash: String,
}
struct Ticket {
    owner: usize,
    root: PathBuf,
    epoch: u64,
    kind: ResourceKind,
    routing: String,
    created: Instant,
    bytes: usize,
    candidates: BTreeMap<String, Candidate>,
    sources: Vec<InventorySource>,
    observed_at_ms: i64,
    definitions_revision: u64,
    source_proofs: Vec<Proof>,
    source_signature: String,
}
fn pending() -> &'static Mutex<BTreeMap<String, Ticket>> {
    static TICKETS: OnceLock<Mutex<BTreeMap<String, Ticket>>> = OnceLock::new();
    TICKETS.get_or_init(|| Mutex::new(BTreeMap::new()))
}
fn prune(tickets: &mut BTreeMap<String, Ticket>) {
    tickets.retain(|_, ticket| ticket.created.elapsed() < TTL);
}
/// Release private source definitions after a picker is closed. No filesystem or database writes.
pub fn discard(ticket_id: &str) -> bool {
    pending()
        .lock()
        .map(|mut tickets| {
            prune(&mut tickets);
            tickets.remove(ticket_id).is_some()
        })
        .unwrap_or(false)
}
pub fn scan(db: &Database, kind: ResourceKind) -> Result<LibraryInventory, AppError> {
    let _io = crate::config_io::lock();
    let snapshot = db.library_snapshot()?;
    let mut data = readers::collect(db, kind.clone())?;
    let mcp = db.get_all_mcp_servers()?;
    let skills = db.get_all_installed_skills()?;
    for candidate in &mut data.candidates {
        let library_conflict = match &candidate.definition {
            Definition::Instructions { content } => {
                snapshot.definitions.instructions.values().any(|row| {
                    row.name.eq_ignore_ascii_case(&candidate.view.name) && row.content != *content
                })
            }
            Definition::Mcp { spec } => mcp.values().any(|row| {
                (row.name.eq_ignore_ascii_case(&candidate.view.name)
                    || row.id == candidate.view.name)
                    && super::mcp_resource::match_hash(&row.server).ok()
                        != super::mcp_resource::match_hash(spec).ok()
            }),
            Definition::Skill {
                hash, directory, ..
            } => skills.values().any(|row| {
                (row.directory.eq_ignore_ascii_case(directory)
                    || row.name.eq_ignore_ascii_case(&candidate.view.name))
                    && row.content_hash.as_ref() != Some(hash)
            }),
        };
        if library_conflict {
            candidate.view.conflict = true;
            candidate.view.variant = Some(candidate.hash.chars().take(8).collect());
        }
        candidate.view.matched_resource_ids = match &candidate.definition {
            Definition::Instructions { content } => snapshot
                .definitions
                .instructions
                .values()
                .filter(|row| row.content == *content)
                .map(|row| row.id.clone())
                .collect(),
            Definition::Mcp { spec } => mcp
                .values()
                .filter(|row| {
                    mcp_alias_matches(&row.id, &candidate.view.name, &candidate.hash)
                        && super::mcp_resource::match_hash(&row.server).ok()
                            == super::mcp_resource::match_hash(spec).ok()
                })
                .map(|row| row.id.clone())
                .collect(),
            Definition::Skill { hash, .. } => skills
                .values()
                .filter(|row| row.content_hash.as_ref() == Some(hash))
                .map(|row| row.id.clone())
                .collect(),
        };
    }
    for index in 0..data.candidates.len() {
        let conflict = data.candidates.iter().enumerate().any(|(other, row)| {
            other != index
                && row
                    .view
                    .name
                    .eq_ignore_ascii_case(&data.candidates[index].view.name)
                && row.hash != data.candidates[index].hash
        });
        if conflict {
            data.candidates[index].view.conflict = true;
            data.candidates[index].view.variant =
                Some(data.candidates[index].hash.chars().take(8).collect());
        }
    }
    let bytes = data
        .candidates
        .iter()
        .map(|entry| match &entry.definition {
            Definition::Instructions { content } => content.len(),
            Definition::Mcp { spec } => spec.to_string().len(),
            Definition::Skill { path, .. } => path.as_os_str().len() + 256,
        })
        .sum::<usize>();
    if bytes > MAX_BYTES || data.candidates.len() > MAX_ENTRIES {
        return Err(error("inventoryLimit"));
    }
    let source_signature = readers::scan_signature(&data)?;
    let id = uuid::Uuid::new_v4().to_string();
    let observed_at_ms = chrono::Utc::now().timestamp_millis();
    let view = LibraryInventory {
        ticket_id: id.clone(),
        kind: kind.clone(),
        observed_at_ms,
        expires_at_ms: chrono::Utc::now().timestamp_millis() + TTL.as_millis() as i64,
        sources: data.sources.clone(),
        entries: data
            .candidates
            .iter()
            .map(|entry| entry.view.clone())
            .collect(),
    };
    let mut tickets = pending()
        .lock()
        .map_err(|_| error("inventoryUnavailable"))?;
    prune(&mut tickets);
    while tickets.len() >= MAX_TICKETS
        || tickets.values().map(|ticket| ticket.bytes).sum::<usize>() + bytes > MAX_BYTES
    {
        let Some(oldest) = tickets
            .iter()
            .min_by_key(|(_, ticket)| ticket.created)
            .map(|(id, _)| id.clone())
        else {
            break;
        };
        tickets.remove(&oldest);
    }
    tickets.insert(
        id,
        Ticket {
            owner: db as *const Database as usize,
            root: crate::config::get_app_config_dir(),
            epoch: snapshot.machine.epoch,
            kind,
            routing: data.routing,
            created: Instant::now(),
            bytes,
            source_signature,
            source_proofs: data
                .sources
                .iter()
                .flat_map(|source| source.proofs.iter().cloned())
                .collect(),
            sources: data.sources,
            observed_at_ms,
            definitions_revision: snapshot.definitions.revision,
            candidates: data
                .candidates
                .into_iter()
                .map(|entry| (entry.view.entry_id.clone(), entry))
                .collect(),
        },
    );
    Ok(view)
}
fn verify_proof(db: &Database, proof: &Proof) -> Result<(), AppError> {
    match proof {
        Proof::File {
            path,
            physical,
            hash,
        } => {
            if io::physical_id(path)? != *physical
                || io::fingerprint(io::read(path)?.as_deref().map(str::as_bytes)) != *hash
            {
                return Err(error("inventoryChanged"));
            }
        }
        Proof::Legacy { tool, id, hash } => {
            let rows = db.get_prompts(tool)?;
            let row = rows.get(id).ok_or_else(|| error("inventoryChanged"))?;
            if io::fingerprint(Some(
                &serde_json::to_vec(row).map_err(|_| error("inventoryChanged"))?,
            )) != *hash
            {
                return Err(error("inventoryChanged"));
            }
        }
        Proof::Tree {
            path,
            physical,
            hash,
        } => {
            let canonical = path.canonicalize().map_err(|_| error("inventoryChanged"))?;
            if canonical.to_string_lossy() != physical.as_str()
                || super::skill_artifact::tree_hash(&canonical)? != *hash
            {
                return Err(error("inventoryChanged"));
            }
        }
    }
    Ok(())
}
fn verify(db: &Database, candidate: &Candidate) -> Result<(), AppError> {
    for proof in &candidate.proofs {
        verify_proof(db, proof)?;
    }
    Ok(())
}
fn import_one(db: &Database, candidate: &Candidate) -> Result<(String, bool), AppError> {
    let snapshot = db.library_snapshot()?;
    match &candidate.definition {
        Definition::Instructions { content } => {
            if let Some(row) = snapshot
                .definitions
                .instructions
                .values()
                .find(|row| row.content == *content)
            {
                return Ok((row.id.clone(), false));
            }
            let row = super::catalog::import_instruction(db, &candidate.view.name, content)?;
            Ok((row.id, true))
        }
        Definition::Mcp { spec } => {
            let existing = db.get_all_mcp_servers()?;
            if let Some(row) = existing.values().find(|row| {
                mcp_alias_matches(&row.id, &candidate.view.name, &candidate.hash)
                    && super::mcp_resource::match_hash(&row.server).ok()
                        == super::mcp_resource::match_hash(spec).ok()
            }) {
                return Ok((row.id.clone(), false));
            }
            let mut id = candidate.view.name.clone();
            if existing.contains_key(&id) {
                id = variant_name(&id, &candidate.hash, 220);
            }
            if existing.contains_key(&id) {
                return Err(error("mcpNameConflict"));
            }
            super::mcp_resource::save_draft(
                db,
                McpServer {
                    id: id.clone(),
                    name: candidate.view.name.clone(),
                    server: spec.clone(),
                    apps: Default::default(),
                    description: None,
                    homepage: None,
                    docs: None,
                    tags: vec![],
                },
                None,
            )?;
            Ok((id, true))
        }
        Definition::Skill {
            path,
            hash,
            directory,
            description,
        } => {
            let existing = db.get_all_installed_skills()?;
            if let Some(row) = existing
                .values()
                .find(|row| row.content_hash.as_ref() == Some(hash))
            {
                super::skill_resource::artifact_for(db, &row.id)?;
                return Ok((row.id.clone(), false));
            }
            let mut directory = directory.clone();
            if existing
                .values()
                .any(|row| row.directory.eq_ignore_ascii_case(&directory))
            {
                directory = variant_name(&directory, hash, 85);
            }
            if existing
                .values()
                .any(|row| row.directory.eq_ignore_ascii_case(&directory))
            {
                return Err(error("skillNameConflict"));
            }
            let now = chrono::Utc::now().timestamp();
            let id = format!("native:{}", uuid::Uuid::new_v4());
            super::skill_resource::save_draft_checked_with_execution(
                db,
                InstalledSkill {
                    id: id.clone(),
                    name: candidate.view.name.clone(),
                    description: description.clone(),
                    directory,
                    repo_owner: None,
                    repo_name: None,
                    repo_branch: None,
                    readme_url: None,
                    apps: Default::default(),
                    installed_at: now,
                    updated_at: now,
                    content_hash: None,
                },
                &path.canonicalize().map_err(|_| error("inventoryChanged"))?,
                None,
                None,
                Some(hash),
            )?;
            Ok((id, true))
        }
    }
}
fn variant_name(base: &str, hash: &str, max_bytes: usize) -> String {
    let mut base = base.to_owned();
    while base.len() > max_bytes {
        base.pop();
    }
    format!("{base}--{}", &hash[..16])
}
fn mcp_alias_matches(id: &str, native_name: &str, hash: &str) -> bool {
    id == native_name || id == variant_name(native_name, hash, 220)
}
pub fn import(
    db: &Database,
    ticket_id: &str,
    entry_ids: &[String],
    expected_epoch: u64,
) -> Result<InventoryImport, AppError> {
    if entry_ids.is_empty()
        || entry_ids.len() > MAX_ENTRIES
        || entry_ids.iter().collect::<BTreeSet<_>>().len() != entry_ids.len()
    {
        return Err(error("inventorySelectionInvalid"));
    }
    let _io = crate::config_io::lock();
    super::catalog::verify_epoch(db, expected_epoch)?;
    let ticket = {
        let mut tickets = pending()
            .lock()
            .map_err(|_| error("inventoryUnavailable"))?;
        prune(&mut tickets);
        let ticket = tickets
            .get(ticket_id)
            .ok_or_else(|| error("inventoryExpired"))?;
        if ticket.owner != db as *const Database as usize
            || ticket.root != crate::config::get_app_config_dir()
        {
            return Err(error("inventoryExpired"));
        }
        if ticket.epoch != expected_epoch {
            return Err(error("epochChanged"));
        }
        if entry_ids
            .iter()
            .any(|id| !ticket.candidates.contains_key(id))
        {
            return Err(error("inventorySelectionInvalid"));
        }
        tickets
            .remove(ticket_id)
            .ok_or_else(|| error("inventoryExpired"))?
    };
    let changed_routing = readers::routing_signature(&ticket.kind)? != ticket.routing;
    let mut result = InventoryImport {
        imported: 0,
        items: vec![],
    };
    for id in entry_ids {
        let candidate = &ticket.candidates[id];
        let outcome = if changed_routing {
            Err(error("inventoryChanged"))
        } else {
            verify(db, candidate)
        };
        let (resource_id, status, reason) = match outcome {
            Err(_) => (None, "stale", Some("library.inventoryChanged".into())),
            Ok(()) => match import_one(db, candidate) {
                Ok((id, created)) => {
                    if created {
                        result.imported += 1;
                    }
                    (
                        Some(id),
                        if created { "imported" } else { "existing" },
                        None,
                    )
                }
                Err(_) => (None, "failed", Some("library.inventoryImportFailed".into())),
            },
        };
        result.items.push(ImportedItem {
            entry_id: id.clone(),
            resource_id,
            name: candidate.view.name.clone(),
            status: status.into(),
            reason,
        });
    }
    Ok(result)
}

#[cfg(test)]
#[path = "inventory_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "inventory_details_tests.rs"]
mod details_tests;

#[cfg(test)]
#[path = "inventory_builtin_tests.rs"]
mod builtin_tests;

#[cfg(test)]
#[path = "inventory_builtin_extra_tests.rs"]
mod builtin_extra_tests;
