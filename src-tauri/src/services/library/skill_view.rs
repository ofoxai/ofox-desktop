//! Library projections expose desired, verified and shared visibility separately.

use super::{skill_artifact as artifacts, skill_resource as skills, ResourceKind};
use crate::{app_config::InstalledSkill, error::AppError, Database};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path};
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}

pub fn definition_view(db: &Database, row: &InstalledSkill) -> Result<Value, AppError> {
    let _io = crate::config_io::lock();
    skills::validate_skill(row)?;
    let snapshot = db.library_snapshot()?;
    let meta = snapshot.definitions.skills.get(&row.id);
    let matches = meta.is_some_and(|meta| skills::spec_hash(row).ok().as_ref() == Some(&meta.hash));
    let roots = skills::target_paths()?;
    let readers = skills::reader_paths();
    let scope_known = readers.is_ok();
    // A full tree hash per receipt, once, rather than once per agent.
    let intact_by_target: std::collections::BTreeMap<&str, bool> = snapshot
        .machine
        .receipts
        .values()
        .filter(|receipt| receipt.kind == ResourceKind::Skills && receipt.resource_id == row.id)
        .map(|receipt| {
            (
                receipt.physical_target.as_str(),
                skills::receipt_intact(receipt),
            )
        })
        .collect();
    let intact_of = |receipt: &super::Receipt| {
        intact_by_target
            .get(receipt.physical_target.as_str())
            .copied()
            .unwrap_or(false)
    };
    let mut targets = Vec::new();
    for (tool, root) in roots {
        let path = root.join(&row.directory);
        let physical = skills::entry_id(&path)?;
        let receipt = snapshot
            .machine
            .receipts
            .get(&format!("skills:{physical}"))
            .filter(|receipt| {
                receipt.resource_id == row.id && receipt.kind == ResourceKind::Skills
            });
        let own = receipt.filter(|receipt| receipt.owners.contains(&tool));
        let intact = receipt.is_some_and(intact_of);
        let desired = snapshot
            .machine
            .assignments
            .get(&format!("skills:{}:{tool}", row.id))
            == Some(&row.id);
        let current = own.is_some_and(|receipt| {
            matches
                && meta.is_some_and(|meta| {
                    meta.hash == receipt.definition_hash
                        && meta.revision == receipt.resource_revision
                })
        });
        let mut readable_groups = Vec::new();
        let mut potential = BTreeSet::new();
        if let Ok(readers) = &readers {
            for (reader, roots) in readers {
                for root in roots {
                    if skills::reads_entry(reader, root, Path::new(&physical), &row.directory)? {
                        potential.insert(reader.clone());
                    }
                }
            }
            for candidate in snapshot.machine.receipts.values().filter(|receipt| {
                receipt.kind == ResourceKind::Skills && receipt.resource_id == row.id
            }) {
                let visible = tool == "shared" && candidate.physical_target == physical
                    || readers.get(&tool).is_some_and(|roots| {
                        roots.iter().any(|root| {
                            skills::reads_entry(
                                &tool,
                                root,
                                Path::new(&candidate.physical_target),
                                &row.directory,
                            )
                            .unwrap_or(false)
                        })
                    });
                if visible && intact_of(candidate) {
                    readable_groups.push(json!({"path":candidate.path,"targetGroupId":candidate.physical_target,"owners":candidate.owners,"revision":candidate.resource_revision}));
                }
            }
        }
        let readable = !readable_groups.is_empty();
        let entry = std::fs::symlink_metadata(&path).ok();
        let entry_kind = entry.as_ref().map(|meta| {
            if meta.file_type().is_symlink() {
                "link"
            } else {
                "directory"
            }
        });
        let adopted = own.is_some_and(|receipt| {
            serde_json::from_str::<skills::DirectoryOwned>(&receipt.owned_segment)
                .is_ok_and(|owned| owned.adopted.is_some())
        });
        // The same skill is already there and nobody owns it: switching it on takes it over.
        let adoptable = tool != "claude-desktop"
            && entry.is_some()
            && !snapshot
                .machine
                .receipts
                .contains_key(&format!("skills:{physical}"))
            && row.content_hash.is_some()
            && skills::content_hash(&path).ok() == row.content_hash;
        let status = if !scope_known && (desired || own.is_some()) {
            "needsReview"
        } else if own.is_some() && !intact {
            "conflict"
        } else if desired && own.is_some() && intact && current {
            "applied"
        } else if desired {
            "pending"
        } else if own.is_some() {
            "needsReview"
        } else if readable {
            "shared"
        } else {
            "unassigned"
        };
        targets.push(json!({"toolId":tool,"path":path.to_string_lossy(),"targetGroupId":physical,"desired":desired,"applied":scope_known&&desired&&own.is_some()&&intact&&current,
            "readable":if scope_known{Some(readable)}else{None},"readers":if scope_known{Some(potential)}else{None},"readableGroups":readable_groups,"status":status,
            "lastAppliedRevision":own.map(|receipt|receipt.resource_revision),"pathState":if path.exists(){"exists"}else if root.is_dir(){"missing"}else{"missingParent"},
            "scopeKnown":scope_known,"disabledReason":if scope_known{None}else{Some("library.nativeScopeUnsupported")},
            "adopted":adopted,"adoptable":adoptable,"entryKind":entry_kind}));
    }
    let source_state = if meta.is_none() {
        "needsMigration"
    } else if !matches {
        "changed"
    } else if skills::artifact_for(db, &row.id).is_ok() {
        "ready"
    } else {
        "unverified"
    };
    let mut view = json!({"id":row.id,"name":row.name,"description":row.description,"directory":row.directory,"revision":meta.map(|meta|meta.revision),"artifactHash":row.content_hash,
        "sourceState":source_state,"installedAtMs":row.installed_at.checked_mul(1000).ok_or_else(||error("stateUnreadable"))?,"updatedAtMs":row.updated_at.checked_mul(1000).ok_or_else(||error("stateUnreadable"))?,"targets":targets});
    view.as_object_mut().unwrap().extend(
        super::skill_actions::provenance_view(row)
            .as_object()
            .unwrap()
            .clone(),
    );
    Ok(view)
}

pub fn detail(db: &Database, id: &str, relative: Option<&str>) -> Result<Value, AppError> {
    let _io = crate::config_io::lock();
    let row = db
        .get_installed_skill(id)?
        .ok_or_else(|| error("resourceMissing"))?;
    skills::artifact_for(db, id)?;
    let hash = row
        .content_hash
        .as_deref()
        .ok_or_else(|| error("skillArtifactMissing"))?;
    let (_, entries) = artifacts::archive_entries(hash)?;
    let requested = relative.unwrap_or("SKILL.md");
    artifacts::safe_relative(requested)?;
    let entry = entries
        .iter()
        .find(|entry| entry.path == requested)
        .ok_or_else(|| error("skillFileMissing"))?;
    let bytes = entry
        .bytes
        .as_deref()
        .ok_or_else(|| error("skillFileNotText"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(error("skillFileTooLarge"));
    }
    let content = std::str::from_utf8(bytes).map_err(|_| error("skillFileNotText"))?;
    Ok(
        json!({"resourceId":id,"artifactHash":hash,"path":requested,"content":content,"files":entries.iter().map(|entry|json!({"path":entry.path,"isDirectory":entry.bytes.is_none(),"sizeBytes":entry.bytes.as_ref().map(|bytes|bytes.len()),"executable":entry.executable})).collect::<Vec<_>>()}),
    )
}

pub fn detail_revisioned(
    db: &Database,
    id: &str,
    relative: Option<&str>,
    revision: Option<u64>,
    epoch: Option<u64>,
    hash: Option<&str>,
) -> Result<Value, AppError> {
    let _io = crate::config_io::lock();
    let snapshot = db.library_snapshot()?;
    if epoch.is_some_and(|epoch| epoch != snapshot.machine.epoch) {
        return Err(error("epochChanged"));
    }
    if revision.is_some_and(|revision| {
        snapshot
            .definitions
            .skills
            .get(id)
            .is_none_or(|meta| meta.revision != revision)
    }) {
        return Err(error("revisionChanged"));
    }
    let row = db
        .get_installed_skill(id)?
        .ok_or_else(|| error("resourceMissing"))?;
    if hash.is_some_and(|hash| row.content_hash.as_deref() != Some(hash)) {
        return Err(error("revisionChanged"));
    }
    detail(db, id, relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app_config::InstalledSkill,
        services::library::{coordinator, skill_resource},
        Database,
    };
    use std::fs;
    fn row() -> InstalledSkill {
        InstalledSkill {
            id: "local:fixture".into(),
            name: "Fixture".into(),
            description: None,
            directory: "fixture".into(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: Default::default(),
            installed_at: 0,
            content_hash: None,
            updated_at: 0,
        }
    }
    fn source(body: &str) -> tempfile::TempDir {
        let source = tempfile::tempdir().unwrap();
        fs::write(
            source.path().join("SKILL.md"),
            format!("---\nname: fixture\ndescription: Fixture\n---\n{body}"),
        )
        .unwrap();
        fs::write(source.path().join(".hidden"), "private file").unwrap();
        source
    }
    fn target<'a>(view: &'a serde_json::Value, tool: &str) -> &'a serde_json::Value {
        view["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["toolId"] == tool)
            .unwrap()
    }
    #[test]
    #[serial_test::serial]
    fn details_reject_a_previous_revision_or_restored_epoch_and_show_validated_source() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let original = source("Original");
        let mut row = row();
        row.repo_owner = Some("fixture-owner".into());
        row.repo_name = Some("fixture-repo".into());
        row.repo_branch = Some("feature/skills".into());
        let sha = "b".repeat(40);
        row.readme_url = Some(format!(
            "https://github.com/fixture-owner/fixture-repo/blob/{sha}/nested/SKILL.md"
        ));
        let saved = skill_resource::save_draft(&db, row, original.path(), None).unwrap();
        let view = definition_view(&db, &saved).unwrap();
        assert_eq!(view["sourceKind"], "repository");
        assert_eq!(view["trackingRef"], "feature/skills");
        assert_eq!(view["commitSha"], sha);
        let epoch = db.library_snapshot().unwrap().machine.epoch;
        assert!(detail_revisioned(&db, &saved.id, None, None, None, None).is_ok());
        assert!(detail_revisioned(
            &db,
            &saved.id,
            None,
            Some(1),
            Some(epoch),
            Some("incorrect-hash")
        )
        .is_err());
        assert!(detail_revisioned(
            &db,
            &saved.id,
            None,
            Some(1),
            Some(epoch),
            saved.content_hash.as_deref()
        )
        .is_ok());
        assert!(detail_revisioned(
            &db,
            &saved.id,
            None,
            Some(1),
            Some(epoch + 1),
            saved.content_hash.as_deref()
        )
        .is_err());
        let updated = source("New body");
        skill_resource::save_draft(&db, saved.clone(), updated.path(), Some(1)).unwrap();
        assert!(detail_revisioned(
            &db,
            &saved.id,
            None,
            Some(1),
            Some(epoch),
            saved.content_hash.as_deref()
        )
        .is_err());
        let mut unsafe_row = saved;
        unsafe_row.readme_url = Some("https://github.com:8443/fixture-owner/fixture-repo/blob/main/SKILL.md?token=private-sentinel".into());
        let unsafe_view = definition_view(&db, &unsafe_row).unwrap();
        assert_eq!(unsafe_view["sourceKind"], "unknown");
        assert!(unsafe_view["sourceUrl"].is_null());
        assert!(!unsafe_view.to_string().contains("private-sentinel"));
        unsafe_row.readme_url = Some(
            "https://github.com/fixture-owner/fixture-repo/blob/feature/skills/nested/SKILL.md"
                .into(),
        );
        let tracked = super::super::skill_actions::provenance_view(&unsafe_row);
        assert_eq!(tracked["sourceKind"], "repository");
        assert_eq!(tracked["trackingRef"], "feature/skills");
        assert!(tracked["commitSha"].is_null());
        unsafe_row.repo_owner = None;
        let incomplete = super::super::skill_actions::provenance_view(&unsafe_row);
        assert_eq!(incomplete["sourceKind"], "unknown");
        assert!(incomplete["sourceUrl"].is_null());
    }
    #[test]
    #[serial_test::serial]
    fn queries_distinguish_draft_desired_applied_and_updated_content() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let original = source("Original");
        let saved = skill_resource::save_draft(&db, row(), original.path(), None).unwrap();
        let draft = definition_view(&db, &saved).unwrap();
        assert_eq!(target(&draft, "codex")["status"], "unassigned");
        let op = skill_resource::preview(&db, &saved.id, &["codex".into()]).unwrap();
        coordinator::apply_operation(&db, &op.id).unwrap();
        assert_eq!(
            target(&definition_view(&db, &saved).unwrap(), "codex")["applied"],
            true
        );
        let update = source("Updated");
        let saved = skill_resource::save_draft(&db, saved, update.path(), Some(1)).unwrap();
        let pending = definition_view(&db, &saved).unwrap();
        assert_eq!(target(&pending, "codex")["status"], "pending");
        assert_eq!(target(&pending, "codex")["desired"], true);
        assert_eq!(target(&pending, "codex")["applied"], false);
        assert_eq!(target(&pending, "codex")["readable"], true);
    }
    #[test]
    #[serial_test::serial]
    fn detail_reads_only_verified_artifact_relative_files() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let source = source("Document");
        let saved = skill_resource::save_draft(&db, row(), source.path(), None).unwrap();
        let view = detail(&db, &saved.id, Some("SKILL.md")).unwrap();
        assert!(view["content"].as_str().unwrap().contains("Document"));
        assert!(view["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == ".hidden"));
        assert!(detail(&db, &saved.id, Some("../outside")).is_err());
        assert!(detail(&db, &saved.id, Some("/etc/passwd")).is_err());
        let artifact = skill_resource::artifact_for(&db, &saved.id).unwrap();
        fs::write(artifact.join("SKILL.md"), "external edit").unwrap();
        assert!(detail(&db, &saved.id, Some("SKILL.md")).is_err());
    }
    #[test]
    #[serial_test::serial]
    fn a_shared_group_is_visible_without_claiming_independent_tool_assignments() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let source = source("Shared");
        let saved = skill_resource::save_draft(&db, row(), source.path(), None).unwrap();
        let op = skill_resource::preview(&db, &saved.id, &["shared".into()]).unwrap();
        assert_eq!(op.targets[0].tools, ["shared"]);
        assert_eq!(
            op.targets[0].readers,
            ["codex", "copilot", "dsh", "gemini", "opencode"]
        );
        coordinator::apply_operation(&db, &op.id).unwrap();
        let view = definition_view(&db, &saved).unwrap();
        assert_eq!(target(&view, "shared")["applied"], true);
        assert_eq!(
            target(&view, "shared")["readers"],
            serde_json::json!(op.targets[0].readers)
        );
        assert_eq!(target(&view, "gemini")["status"], "shared");
        assert_eq!(target(&view, "gemini")["desired"], false);
        assert_eq!(target(&view, "gemini")["applied"], false);
        assert_eq!(target(&view, "gemini")["readable"], true);
    }
}
