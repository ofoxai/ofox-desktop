//! Append-only portable artifacts. Restore never replaces native/legacy roots.
use super::MAX_SYNC_ARTIFACT_BYTES;
use crate::{
    error::AppError,
    services::library::{self, skill_artifact as artifacts, skill_resource},
    Database,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactIndex {
    schema_version: u32,
    hashes: Vec<String>,
}
fn skill_rows(
    conn: &rusqlite::Connection,
) -> Result<Vec<crate::app_config::InstalledSkill>, AppError> {
    let mut stmt = conn
        .prepare("SELECT id FROM skills ORDER BY id")
        .map_err(|e| AppError::Database(e.to_string()))?;
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| AppError::Database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(e.to_string()))?;
    ids.iter()
        .map(|id| {
            Database::skill_from_connection(conn, id)?.ok_or_else(|| error("resourceMissing"))
        })
        .collect()
}
pub(crate) fn zip_skill_artifacts(
    dest: &Path,
    conn: &rusqlite::Connection,
) -> Result<(), AppError> {
    zip_skill_artifacts_with_limits(
        dest,
        conn,
        artifacts::PACKAGE_MAX_OBJECTS,
        artifacts::PACKAGE_MAX_ENTRIES,
    )
}
pub(crate) fn zip_selected_skill_artifacts(
    dest: &Path,
    conn: &rusqlite::Connection,
    selected: &BTreeSet<String>,
) -> Result<(), AppError> {
    zip_skill_artifacts_filtered(
        dest,
        conn,
        artifacts::PACKAGE_MAX_OBJECTS,
        artifacts::PACKAGE_MAX_ENTRIES,
        Some(selected),
    )
}
fn zip_skill_artifacts_with_limits(
    dest: &Path,
    conn: &rusqlite::Connection,
    max_objects: usize,
    max_entries: usize,
) -> Result<(), AppError> {
    zip_skill_artifacts_filtered(dest, conn, max_objects, max_entries, None)
}
fn zip_skill_artifacts_filtered(
    dest: &Path,
    conn: &rusqlite::Connection,
    max_objects: usize,
    max_entries: usize,
    selected: Option<&BTreeSet<String>>,
) -> Result<(), AppError> {
    let snapshot = library::snapshot_from_connection(conn)?;
    let mut hashes = BTreeSet::new();
    for row in skill_rows(conn)?
        .into_iter()
        .filter(|row| selected.is_none_or(|ids| ids.contains(&row.id)))
    {
        skill_resource::validate_skill(&row)?;
        let meta = snapshot
            .definitions
            .skills
            .get(&row.id)
            .ok_or_else(|| error("skillNeedsMigration"))?;
        if meta.hash != skill_resource::spec_hash(&row)? {
            return Err(error("revisionChanged"));
        }
        hashes.insert(
            row.content_hash
                .ok_or_else(|| error("skillArtifactMissing"))?,
        );
    }
    if hashes.len() > max_objects {
        return Err(error("skillTooLarge"));
    }
    let mut entry_count = 1usize;
    let mut writer =
        zip::ZipWriter::new(fs::File::create(dest).map_err(|e| AppError::io(dest, e))?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    let index = serde_json::to_vec(&ArtifactIndex {
        schema_version: 1,
        hashes: hashes.iter().cloned().collect(),
    })
    .map_err(|_| error("skillArchiveInvalid"))?;
    let mut total = index.len() as u64;
    writer
        .start_file("artifact-index.json", options)
        .map_err(|_| error("skillArchiveInvalid"))?;
    writer
        .write_all(&index)
        .map_err(|_| error("skillWriteFailed"))?;
    for hash in hashes {
        let (manifest, entries) = artifacts::archive_entries(&hash)?;
        entry_count = entry_count
            .checked_add(2 + entries.len())
            .ok_or_else(|| error("skillTooLarge"))?;
        if entry_count > max_entries {
            return Err(error("skillTooLarge"));
        }
        writer
            .start_file(format!("artifacts/{hash}/manifest.json"), options)
            .map_err(|_| error("skillArchiveInvalid"))?;
        writer
            .write_all(&manifest)
            .map_err(|_| error("skillWriteFailed"))?;
        total += manifest.len() as u64;
        writer
            .add_directory(
                format!("artifacts/{hash}/tree/"),
                options.unix_permissions(0o755),
            )
            .map_err(|_| error("skillArchiveInvalid"))?;
        for entry in entries {
            let name = format!("artifacts/{hash}/tree/{}", entry.path);
            if let Some(bytes) = entry.bytes {
                total = total
                    .checked_add(bytes.len() as u64)
                    .ok_or_else(|| error("skillTooLarge"))?;
                if total > MAX_SYNC_ARTIFACT_BYTES {
                    return Err(error("skillTooLarge"));
                }
                writer
                    .start_file(
                        name,
                        options.unix_permissions(if entry.executable { 0o755 } else { 0o644 }),
                    )
                    .map_err(|_| error("skillArchiveInvalid"))?;
                writer
                    .write_all(&bytes)
                    .map_err(|_| error("skillWriteFailed"))?;
            } else {
                writer
                    .add_directory(format!("{name}/"), options.unix_permissions(0o755))
                    .map_err(|_| error("skillArchiveInvalid"))?;
            }
        }
    }
    writer.finish().map_err(|_| error("skillArchiveInvalid"))?;
    if fs::metadata(dest).map_err(|e| AppError::io(dest, e))?.len() > MAX_SYNC_ARTIFACT_BYTES {
        return Err(error("skillTooLarge"));
    }
    Ok(())
}
pub(crate) struct PreparedBundle {
    extracted: artifacts::ExtractedDir,
    objects: Option<BTreeMap<String, (PathBuf, artifacts::ExecutionMap)>>,
}
pub(crate) fn prepare_zip(raw: &[u8], immutable: bool) -> Result<PreparedBundle, AppError> {
    let extracted = artifacts::extract_artifact_package(raw)?;
    let objects = if immutable {
        let bytes = fs::read(extracted.path().join("artifact-index.json"))
            .map_err(|_| error("skillArchiveInvalid"))?;
        if bytes.len() > 1024 * 1024 {
            return Err(error("skillTooLarge"));
        }
        let index: ArtifactIndex =
            serde_json::from_slice(&bytes).map_err(|_| error("skillArchiveInvalid"))?;
        if index.schema_version != 1
            || index.hashes.len() > artifacts::PACKAGE_MAX_OBJECTS
            || index.hashes.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(error("skillArchiveInvalid"));
        }
        let children = names(extracted.path())?;
        if children
            .iter()
            .any(|name| name != "artifact-index.json" && name != "artifacts")
        {
            return Err(error("skillArchiveInvalid"));
        }
        let root = extracted.path().join("artifacts");
        let actual = if root.exists() {
            names(&root)?
        } else {
            BTreeSet::new()
        };
        if actual != index.hashes.iter().cloned().collect() {
            return Err(error("skillArchiveInvalid"));
        }
        let mut objects = BTreeMap::new();
        for hash in index.hashes {
            artifacts::path(&hash)?;
            let container = root.join(&hash);
            if names(&container)? != BTreeSet::from(["manifest.json".into(), "tree".into()]) {
                return Err(error("skillArchiveInvalid"));
            }
            let execution = artifacts::parse_manifest_bytes(
                &fs::read(container.join("manifest.json"))
                    .map_err(|_| error("skillManifestInvalid"))?,
            )?;
            let tree = container.join("tree");
            artifacts::verify_remote_artifact(&hash, &tree, &execution)?;
            objects.insert(hash, (tree, execution));
        }
        Some(objects)
    } else {
        if extracted.path().join("artifact-index.json").is_file() {
            return Err(error("skillArchiveInvalid"));
        }
        None
    };
    Ok(PreparedBundle { extracted, objects })
}
fn names(path: &Path) -> Result<BTreeSet<String>, AppError> {
    fs::read_dir(path)
        .map_err(|_| error("skillArchiveInvalid"))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|_| error("skillArchiveInvalid"))
}
impl PreparedBundle {
    /// Verified immutable objects only; the extraction stays alive with this bundle.
    pub(crate) fn objects(
        &self,
    ) -> Result<&BTreeMap<String, (PathBuf, artifacts::ExecutionMap)>, AppError> {
        self.objects
            .as_ref()
            .ok_or_else(|| error("skillArchiveInvalid"))
    }
    /// Runs on a staged DB under the IO barrier; validate every definition first.
    pub(super) fn prepare_database(&self, conn: &rusqlite::Connection) -> Result<(), AppError> {
        let mut snapshot = library::snapshot_from_connection(conn)?;
        let mut pending = Vec::new();
        let mut referenced = BTreeSet::new();
        let mut slugs = BTreeSet::new();
        for mut row in skill_rows(conn)? {
            skill_resource::validate_skill(&row)?;
            if !slugs.insert(row.directory.to_lowercase()) {
                return Err(error("skillNameConflict"));
            }
            row.apps = Default::default();
            let (source, execution) = if let Some(objects) = &self.objects {
                let meta = snapshot
                    .definitions
                    .skills
                    .get(&row.id)
                    .ok_or_else(|| error("skillNeedsMigration"))?;
                if meta.hash != skill_resource::spec_hash(&row)? {
                    return Err(error("revisionChanged"));
                }
                let hash = row
                    .content_hash
                    .as_ref()
                    .ok_or_else(|| error("skillArtifactMissing"))?;
                referenced.insert(hash.clone());
                objects
                    .get(hash)
                    .cloned()
                    .ok_or_else(|| error("skillArtifactMissing"))?
            } else {
                let source = self.extracted.path().join(&row.directory);
                let execution = self.extracted.execution_for(&source)?;
                (source, execution)
            };
            let hash = artifacts::source_hash(&source, &execution)?;
            if let Some(meta) = snapshot.definitions.skills.get(&row.id) {
                if row.content_hash.as_deref() != Some(&hash)
                    || meta.hash != skill_resource::spec_hash(&row)?
                {
                    return Err(error("skillArtifactChanged"));
                }
            }
            row.content_hash = Some(hash.clone());
            snapshot.definitions.skills.entry(row.id.clone()).or_insert(
                library::ResourceRevision {
                    revision: 1,
                    hash: skill_resource::spec_hash(&row)?,
                },
            );
            pending.push((row, source, execution, hash));
        }
        if let Some(objects) = &self.objects {
            if objects.keys().cloned().collect::<BTreeSet<_>>() != referenced {
                return Err(error("skillArchiveInvalid"));
            }
        }
        for (row, _, _, _) in &pending {
            conn.execute("UPDATE skills SET content_hash=?1,enabled_claude=0,enabled_codex=0,enabled_gemini=0,enabled_opencode=0,enabled_hermes=0 WHERE id=?2",rusqlite::params![row.content_hash,row.id]).map_err(|e|AppError::Database(e.to_string()))?;
        }
        conn.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES (?1,?2)",
            rusqlite::params![
                library::DEFINITIONS_KEY,
                serde_json::to_string(&snapshot.definitions)
                    .map_err(|_| error("stateUnreadable"))?
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        library::snapshot_from_connection(conn)?;
        for (_, source, execution, expected) in pending {
            if artifacts::publish_with_execution(&source, Some(&execution))? != expected {
                return Err(error("skillArtifactChanged"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[serial_test::serial]
    fn zip_entry_budget_includes_index_manifest_tree_and_empty_directories() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("SKILL.md"), "Fixture").unwrap();
        fs::create_dir(source.path().join("empty")).unwrap();
        let db = Database::memory().unwrap();
        let row = crate::app_config::InstalledSkill {
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
        };
        skill_resource::save_draft(&db, row, source.path(), None).unwrap();
        let captured = db.snapshot_to_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("out.zip");
        assert!(zip_skill_artifacts_with_limits(&path, &captured, 1, 4).is_err());
        zip_skill_artifacts_with_limits(&path, &captured, 1, 5).unwrap();
        assert!(prepare_zip(&fs::read(path).unwrap(), true).is_ok());
    }
    #[test]
    fn a_legacy_skill_can_be_named_like_the_new_index_file() {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file(
                "artifact-index.json/SKILL.md",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(b"---\nname: fixture\ndescription: Legacy\n---\n")
            .unwrap();
        let raw = writer.finish().unwrap().into_inner();
        assert!(prepare_zip(&raw, false).is_ok());
    }
    #[test]
    #[serial_test::serial]
    fn the_producer_refuses_more_objects_than_the_reader_can_restore() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        for index in 0..1001 {
            let row = crate::app_config::InstalledSkill {
                id: format!("local:skill-{index}"),
                name: format!("Skill {index}"),
                description: None,
                directory: format!("skill-{index}"),
                repo_owner: None,
                repo_name: None,
                repo_branch: None,
                readme_url: None,
                apps: Default::default(),
                installed_at: 0,
                content_hash: Some(format!("{index:064x}")),
                updated_at: 0,
            };
            db.save_skill(&row).unwrap();
            next.definitions.skills.insert(
                row.id.clone(),
                library::ResourceRevision {
                    revision: 1,
                    hash: skill_resource::spec_hash(&row).unwrap(),
                },
            );
        }
        db.library_commit(&before, &next).unwrap();
        let captured = db.snapshot_to_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let error = zip_skill_artifacts(&tmp.path().join("out.zip"), &captured).unwrap_err();
        assert!(error.to_string().contains("skillTooLarge"));
    }
}
