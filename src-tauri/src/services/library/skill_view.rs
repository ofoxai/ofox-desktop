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
        let intact = receipt.is_some_and(skills::receipt_intact);
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
                if visible && skills::receipt_intact(candidate) {
                    readable_groups.push(json!({"path":candidate.path,"targetGroupId":candidate.physical_target,"owners":candidate.owners,"revision":candidate.resource_revision}));
                }
            }
        }
        let readable = !readable_groups.is_empty();
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
            "scopeKnown":scope_known,"disabledReason":if scope_known{None}else{Some("library.nativeScopeUnsupported")}}));
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
    let source_url = row
        .readme_url
        .as_ref()
        .and_then(|raw| url::Url::parse(raw).ok())
        .filter(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("github.com")
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
        .map(|url| url.to_string());
    Ok(
        json!({"id":row.id,"name":row.name,"description":row.description,"directory":row.directory,"revision":meta.map(|meta|meta.revision),"artifactHash":row.content_hash,
        "sourceState":source_state,"sourceUrl":source_url,"installedAtMs":row.installed_at.checked_mul(1000).ok_or_else(||error("stateUnreadable"))?,"updatedAtMs":row.updated_at.checked_mul(1000).ok_or_else(||error("stateUnreadable"))?,"targets":targets}),
    )
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
        coordinator::apply_operation(&db, &op.id).unwrap();
        let view = definition_view(&db, &saved).unwrap();
        assert_eq!(target(&view, "shared")["applied"], true);
        assert_eq!(target(&view, "gemini")["status"], "shared");
        assert_eq!(target(&view, "gemini")["desired"], false);
        assert_eq!(target(&view, "gemini")["applied"], false);
        assert_eq!(target(&view, "gemini")["readable"], true);
    }
}
