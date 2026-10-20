//! Where a found skill sits in each agent, taking those entries over on import,
//! and taking a found skill back out of its agents (set aside, never deleted).
use super::*;
use crate::services::library::skill_resource;
use std::{fs, path::Path};

/// One place a found skill was seen.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillLocation {
    /// The agent (or "shared") whose skills folder holds it; empty when none does.
    pub target_id: String,
    pub display_path: String,
    pub link: bool,
    /// It can be taken over as it is when the skill is imported.
    pub adoptable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotAdopted {
    pub target_id: String,
    pub reason: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovedLocation {
    pub target_id: String,
    pub display_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
}

/// The path as the user knows it, from ~ (the home may be reached through a link).
fn tilde(path: &Path) -> String {
    let home = crate::config::get_home_dir();
    let real = fs::canonicalize(&home).unwrap_or_else(|_| home.clone());
    [home, real]
        .iter()
        .find_map(|home| path.strip_prefix(home).ok())
        .map(|rest| format!("~/{}", rest.to_string_lossy()))
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}
fn tree_paths(candidate: &Candidate) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = candidate
        .proofs
        .iter()
        .filter_map(|proof| match proof {
            Proof::Tree { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    paths.sort();
    paths.dedup();
    paths
}
/// Each place `candidate` was found, with the agent folder it is a direct child
/// of. `directory` is the folder name the library row has (or would get).
pub(super) fn locations(
    candidate: &Candidate,
    directory: &str,
    snapshot: &super::super::LibrarySnapshot,
) -> Vec<(SkillLocation, PathBuf)> {
    let roots = skill_resource::target_paths().unwrap_or_default();
    tree_paths(candidate)
        .into_iter()
        .map(|path| {
            let leaf = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            let entry = skill_resource::entry_id(&path).ok();
            let direct = roots.iter().find(|(_, root)| {
                entry.is_some() && skill_resource::entry_id(&root.join(&leaf)).ok() == entry
            });
            let inside = roots
                .iter()
                .find(|(_, root)| io::physical(root).is_ok_and(|root| path.starts_with(root)));
            let owned = entry.as_ref().is_some_and(|entry| {
                snapshot
                    .machine
                    .receipts
                    .contains_key(&format!("skills:{entry}"))
            });
            let reason = if direct.is_none() {
                Some("library.skillLocationUnsupported")
            } else if owned {
                Some("library.skillOwned")
            } else if leaf != directory {
                Some("library.skillNameConflict")
            } else {
                None
            };
            let link = fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink());
            (
                SkillLocation {
                    target_id: direct
                        .or(inside)
                        .map(|(tool, _)| tool.clone())
                        .unwrap_or_default(),
                    display_path: tilde(&path),
                    link,
                    adoptable: reason.is_none(),
                    reason: reason.map(str::to_owned),
                },
                path,
            )
        })
        .collect()
}

/// Take over, as they are, the agents' entries an imported skill came from, so
/// those agents show it as theirs from the library without anything rewritten.
pub(super) fn adopt_found(
    db: &Database,
    id: &str,
    candidate: &Candidate,
) -> (Vec<String>, Vec<NotAdopted>) {
    let refused = |target_id: &str, reason: &str| NotAdopted {
        target_id: target_id.into(),
        reason: reason.into(),
    };
    let (Ok(snapshot), Ok(Some(row))) = (db.library_snapshot(), db.get_installed_skill(id)) else {
        return (vec![], vec![]);
    };
    let found = locations(candidate, &row.directory, &snapshot);
    let mut not_adopted: Vec<NotAdopted> = found
        .iter()
        .filter(|(location, _)| !location.adoptable && !location.target_id.is_empty())
        .filter(|(location, _)| location.reason.as_deref() != Some("library.skillOwned"))
        .map(|(location, _)| {
            refused(
                &location.target_id,
                location.reason.as_deref().unwrap_or("library.failed"),
            )
        })
        .collect();
    let adopt: Vec<String> = found
        .iter()
        .filter(|(location, _)| location.adoptable)
        .map(|(location, _)| location.target_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if adopt.is_empty() {
        return (vec![], not_adopted);
    }
    let mut tools: BTreeSet<String> = snapshot
        .machine
        .assignments
        .iter()
        .filter(|(_, resource)| resource.as_str() == id)
        .filter_map(|(key, _)| {
            key.strip_prefix(&format!("skills:{id}:"))
                .map(str::to_owned)
        })
        .collect();
    tools.extend(adopt.iter().cloned());
    let tools: Vec<String> = tools.into_iter().collect();
    let op = match skill_resource::preview_adoption(db, id, &tools, &adopt) {
        Ok(op) => op,
        Err(failure) => {
            not_adopted.extend(adopt.iter().map(|tool| refused(tool, &failure.to_string())));
            return (vec![], not_adopted);
        }
    };
    if let Some(blocked) = op.targets.iter().find(|target| target.status != "pending") {
        let reason = blocked
            .reason
            .clone()
            .unwrap_or_else(|| "library.failed".into());
        if let Err(failure) = io::cancel_operation(db, &op.id) {
            log::warn!("[library] could not cancel a refused take-over: {failure}");
        }
        not_adopted.extend(adopt.iter().map(|tool| refused(tool, &reason)));
        return (vec![], not_adopted);
    }
    match super::super::apply_operation(db, &op.id) {
        Ok(done) => {
            let mut adopted = vec![];
            for target in &done.targets {
                for tool in target.tools.iter().filter(|tool| adopt.contains(tool)) {
                    if target.status == "applied" {
                        adopted.push(tool.clone());
                    } else {
                        not_adopted.push(refused(
                            tool,
                            target.reason.as_deref().unwrap_or("library.failed"),
                        ));
                    }
                }
            }
            (adopted, not_adopted)
        }
        Err(failure) => {
            not_adopted.extend(adopt.iter().map(|tool| refused(tool, &failure.to_string())));
            (vec![], not_adopted)
        }
    }
}

/// Take a found skill (not in the library) out of the agents that have it:
/// links are removed, folders are set aside in the skill backups.
pub fn remove_found(
    db: &Database,
    ticket_id: &str,
    entry_id: &str,
    targets: &[String],
    expected_epoch: u64,
) -> Result<Vec<RemovedLocation>, AppError> {
    let _io = crate::config_io::lock();
    super::super::catalog::verify_epoch(db, expected_epoch)?;
    let candidate = {
        let mut tickets = pending()
            .lock()
            .map_err(|_| error("inventoryUnavailable"))?;
        prune(&mut tickets);
        let ticket = tickets
            .get(ticket_id)
            .ok_or_else(|| error("inventoryExpired"))?;
        if ticket.owner != db as *const Database as usize
            || ticket.root != crate::config::get_app_config_dir()
            || ticket.kind != ResourceKind::Skills
        {
            return Err(error("inventoryExpired"));
        }
        if ticket.epoch != expected_epoch {
            return Err(error("epochChanged"));
        }
        if readers::routing_signature(&ticket.kind)? != ticket.routing {
            return Err(error("inventoryChanged"));
        }
        ticket
            .candidates
            .get(entry_id)
            .cloned()
            .ok_or_else(|| error("inventorySelectionInvalid"))?
    };
    if !matches!(candidate.definition, Definition::Skill { .. }) {
        return Err(error("inventorySelectionInvalid"));
    }
    verify(db, &candidate).map_err(|_| error("inventoryChanged"))?;
    let directory = candidate.view.directory.clone().unwrap_or_default();
    let snapshot = db.library_snapshot()?;
    let mut chosen: Vec<(SkillLocation, PathBuf)> = locations(&candidate, &directory, &snapshot)
        .into_iter()
        .filter(|(location, _)| targets.is_empty() || targets.contains(&location.target_id))
        .collect();
    if chosen.is_empty() {
        return Err(error("inventorySelectionInvalid"));
    }
    if chosen
        .iter()
        .any(|(location, _)| location.reason.as_deref() == Some("library.skillOwned"))
    {
        return Err(error("skillOwned"));
    }
    // Links first, so none is left pointing at a folder that was just moved.
    chosen.sort_by_key(|(location, _)| !location.link);
    let mut removed = vec![];
    for (location, path) in chosen {
        let backup = if location.link {
            fs::remove_file(&path).map_err(|_| error("skillWriteFailed"))?;
            None
        } else {
            let leaf = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| error("invalidSkillPath"))?;
            Some(
                crate::services::skill::SkillService::archive_native_entry(
                    &path,
                    leaf,
                    "nativeRemove",
                )
                .map_err(|_| error("skillWriteFailed"))?
                .to_string_lossy()
                .into_owned(),
            )
        };
        removed.push(RemovedLocation {
            target_id: location.target_id,
            display_path: location.display_path,
            backup,
        });
    }
    if let Ok(mut tickets) = pending().lock() {
        tickets.remove(ticket_id);
    }
    Ok(removed)
}
