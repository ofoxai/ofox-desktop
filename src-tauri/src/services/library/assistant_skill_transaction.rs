//! One approved skill mutation. The journal is an authority fence, not a replay log.
use super::{
    assistant_target as target, skill_artifact as artifacts, skill_resource as skills,
    LibrarySnapshot, ResourceRevision, TargetWrite,
};
use crate::{app_config::InstalledSkill, error::AppError, Database};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

const JOURNAL: &str = "skill-transaction-v1.json";
const FILES: [&str; 3] = [
    "skill-modes.json",
    "agent-skills.json",
    "agent-skill-origins.json",
];
fn error(s: impl ToString) -> AppError {
    AppError::Message(s.to_string())
}
fn checked_path(name: &str) -> Result<PathBuf, AppError> {
    target::state::private_path(&target::assistant_dir().join(name))
}
fn raw(name: &str) -> Result<Option<Vec<u8>>, AppError> {
    target::state::read_regular(&checked_path(name)?, 1024 * 1024)
}
fn equal<T: Serialize>(a: &T, b: &T) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

pub struct Prepared {
    pub name: String,
    pub details: Value,
    source: PathBuf,
    execution: artifacts::ExecutionMap,
    hash: String,
    row: InstalledSkill,
    before: LibrarySnapshot,
    old_row: Option<InstalledSkill>,
    old_hash: String,
    physical: String,
    root_identity: Option<String>,
    proof: Option<target::AssistantSkillProof>,
    entry_identity: Option<String>,
    sidecars: Vec<Option<Vec<u8>>>,
    created: bool,
    reuse: bool,
    // Keep downloaded or generated staging alive through approval without publishing it.
    _source_owner: Box<dyn Send + Sync>,
}

#[derive(Clone, Debug)]
pub struct Success {
    pub name: String,
    row: InstalledSkill,
    revision: ResourceRevision,
    receipt: super::Receipt,
}
/// A cached commit receipt never becomes authority for a different resource.
pub fn verify_success(db: &Database, success: &Success) -> Result<(), AppError> {
    let _io = crate::config_io::lock();
    recover(db)?;
    let snapshot = db.library_snapshot()?;
    if !equal(
        &db.get_installed_skill(&success.row.id)?,
        &Some(success.row.clone()),
    ) || snapshot.definitions.skills.get(&success.row.id) != Some(&success.revision)
    {
        return Err(error("library.revisionChanged"));
    }
    skills::artifact_for(db, &success.row.id)?;
    let assignment = format!("skills:{}:{}", success.row.id, target::ID);
    let receipt_key = format!("skills:{}", success.receipt.physical_target);
    if snapshot.machine.assignments.get(&assignment) == Some(&success.row.id) {
        if snapshot.machine.receipts.get(&receipt_key) != Some(&success.receipt)
            || !skills::receipt_intact(&success.receipt)
        {
            return Err(error("library.targetChanged"));
        }
    } else if snapshot.machine.assignments.contains_key(&assignment)
        || snapshot
            .machine
            .receipts
            .values()
            .any(|r| r.resource_id == success.row.id && r.owners.iter().any(|o| o == target::ID))
        || !matches!(fs::symlink_metadata(target::skills_dir().join(&success.row.directory)),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
    {
        return Err(error("library.targetChanged"));
    }
    Ok(())
}

pub fn validate_save(name: &str, description: &str, body: &str) -> Result<(), AppError> {
    if name.is_empty()
        || name.len() > 64
        || name.split('-').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
        || description.trim().is_empty()
        || description.chars().count() > 500
        || body.chars().count() > 60000
    {
        return Err(error("invalid_skill"));
    }
    Ok(())
}
pub fn prepare_save(
    db: &Database,
    name: &str,
    description: &str,
    body: &str,
) -> Result<Prepared, AppError> {
    validate_save(name, description, body)?;
    let tmp = tempfile::tempdir().map_err(error)?;
    let header =
        serde_yaml::to_string(&json!({"name":name,"description":description})).map_err(error)?;
    fs::write(
        tmp.path().join("SKILL.md"),
        format!("---\n{header}---\n{body}"),
    )
    .map_err(error)?;
    target::metadata(tmp.path())?;
    let row = InstalledSkill {
        id: format!("assistant:{}", uuid::Uuid::new_v4()),
        name: name.into(),
        description: Some(description.into()),
        directory: name.into(),
        repo_owner: None,
        repo_name: None,
        repo_branch: None,
        readme_url: None,
        apps: Default::default(),
        installed_at: chrono::Utc::now().timestamp(),
        content_hash: None,
        updated_at: 0,
    };
    let path = tmp.path().to_owned();
    freeze(
        db,
        row,
        path,
        BTreeMap::from([("SKILL.md".into(), false)]),
        Box::new(tmp),
        Purpose::Save,
        json!({"name":name,"description":description,"body":body}),
    )
}

/// Reuse only a verified fixed resource from the exact repository and source directory.
pub fn prepare_existing(
    db: &Database,
    repository: &str,
    name: &str,
    directory: Option<&str>,
    mut details: Value,
) -> Result<Option<Prepared>, AppError> {
    let _io = crate::config_io::lock();
    recover(db)?;
    let mut matches = vec![];
    for row in db.get_all_installed_skills()?.into_values() {
        let source = super::skill_actions::provenance_view(&row);
        if source["repository"]
            .as_str()
            .is_none_or(|r| !r.eq_ignore_ascii_case(repository))
        {
            continue;
        }
        let Some(sha) = source["commitSha"].as_str() else {
            continue;
        };
        let url = url::Url::parse(row.readme_url.as_deref().unwrap_or_default()).map_err(error)?;
        let decoded = super::skill_actions::decode(url.path())?;
        let prefix = format!(
            "/{}/blob/{sha}/",
            source["repository"]
                .as_str()
                .ok_or_else(|| error("source_ambiguous"))?
        );
        let Some(doc) = decoded.strip_prefix(&prefix) else {
            continue;
        };
        let path = if doc == "SKILL.md" {
            ""
        } else {
            doc.strip_suffix("/SKILL.md")
                .ok_or_else(|| error("source_ambiguous"))?
        };
        if directory.is_some_and(|d| d != path) {
            continue;
        }
        let artifact = skills::artifact_for(db, &row.id)?;
        if target::metadata(&artifact)?.0 != name {
            continue;
        }
        let snapshot = db.library_snapshot()?;
        if snapshot
            .machine
            .receipts
            .values()
            .filter(|r| r.resource_id == row.id)
            .any(|r| !skills::receipt_intact(r))
        {
            return Err(error("library.skillTargetModified"));
        }
        matches.push((row, path.to_owned(), sha.to_owned(), artifact));
    }
    if matches.len() > 1 {
        return Err(error("source_ambiguous"));
    }
    let Some((row, path, sha, source)) = matches.pop() else {
        return Ok(None);
    };
    details["directory"] = json!(path);
    details["commitSha"] = json!(sha);
    let execution = artifacts::manifest(
        row.content_hash
            .as_deref()
            .ok_or_else(|| error("library.skillArtifactMissing"))?,
    )?;
    freeze(
        db,
        row,
        source,
        execution,
        Box::new(()),
        Purpose::Existing,
        details,
    )
    .map(Some)
}

pub fn prepare_download(
    db: &Database,
    row: InstalledSkill,
    download: artifacts::ExtractedDir,
    directory: &str,
    details: Value,
) -> Result<Prepared, AppError> {
    let source = download.path().join(directory);
    let execution = download.execution_for(&source)?;
    freeze(
        db,
        row,
        source,
        execution,
        Box::new(download),
        Purpose::Install,
        details,
    )
}
enum Purpose {
    Save,
    Install,
    Existing,
}
fn freeze(
    db: &Database,
    mut row: InstalledSkill,
    source: PathBuf,
    execution: artifacts::ExecutionMap,
    owner: Box<dyn Send + Sync>,
    purpose: Purpose,
    mut details: Value,
) -> Result<Prepared, AppError> {
    let created = matches!(purpose, Purpose::Save);
    let reuse = matches!(purpose, Purpose::Existing);
    let _io = crate::config_io::lock();
    recover(db)?;
    let snapshot = target::assistant_skill_snapshot(db, &target::reviewed_names())?;
    target::skill_modes()?;
    target::agent_created()?;
    target::read_agent_skill_origins()?;
    let (name, description) = target::metadata(&source)?;
    details["name"] = json!(name);
    details["description"] = json!(description.chars().take(500).collect::<String>());
    skills::validate_skill(&row)?;
    if !reuse && name != row.name {
        return Err(error("source_ambiguous"));
    }
    if !reuse {
        row.description = Some(description);
    }
    let before = db.library_snapshot()?;
    let rows = db.get_all_installed_skills()?;
    let old_row = rows
        .values()
        .find(|r| r.directory.eq_ignore_ascii_case(&row.directory))
        .cloned();
    let path = target::skills_dir().join(&row.directory);
    let physical = skills::entry_id(&path)?;
    let receipt = before.machine.receipts.get(&format!("skills:{physical}"));
    if let Some(old) = &old_row {
        if !reuse
            && (!created
                || target::origin_for_resource(db, old)? != target::SkillOrigin::AssistantCreated
                || !can_replace_created(db, old, &snapshot, &path, &name)?)
        {
            return Err(error("not_agent_created"));
        }
        if reuse && old.id != row.id {
            return Err(error("library.skillNameConflict"));
        }
        row.id = old.id.clone();
        row.installed_at = old.installed_at;
    } else if fs::symlink_metadata(&path).is_ok()
        || receipt.is_some()
        || snapshot
            .skills
            .iter()
            .any(|s| s.name.eq_ignore_ascii_case(&name))
    {
        return Err(error("not_agent_created"));
    }
    let owned = receipt
        .map(|r| serde_json::from_str::<skills::DirectoryOwned>(&r.owned_segment))
        .transpose()
        .map_err(error)?;
    let old_hash = skills::owned_hash(&path, owned.as_ref())?;
    let hash = artifacts::source_hash(&source, &execution)?;
    let sidecars = FILES.iter().map(|s| raw(s)).collect::<Result<_, _>>()?;
    let entry_identity = entry_identity(&path)?;
    let proof = snapshot.proof(&name).cloned();
    let root_identity = root_identity()?;
    Ok(Prepared {
        entry_identity,
        proof,
        root_identity,
        name,
        details,
        source,
        execution,
        hash,
        row,
        before,
        old_row,
        old_hash,
        physical,
        sidecars,
        created,
        reuse,
        _source_owner: owner,
    })
}

fn entry_identity(path: &std::path::Path) -> Result<Option<String>, AppError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(e)),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(Some(format!("{}:{}", meta.dev(), meta.ino())))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        let _ = meta;
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)
            .map_err(error)?;
        Ok(Some(
            crate::session_manager::source_cache::SourceStamp::of(&file)?.physical_identity,
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
        Err(error("unsupported_entry_identity"))
    }
}

fn root_identity() -> Result<Option<String>, AppError> {
    let root = target::state::private_path(&target::skills_dir())?;
    match fs::symlink_metadata(&root) {
        Ok(meta) if meta.is_dir() => Ok(Some(
            crate::session_manager::source_cache::SourceStamp::at(&root)?.physical_identity,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        _ => Err(error("library.targetChanged")),
    }
}

fn can_replace_created(
    db: &Database,
    row: &InstalledSkill,
    snapshot: &target::AssistantSkillSnapshot,
    path: &std::path::Path,
    name: &str,
) -> Result<bool, AppError> {
    let state = db.library_snapshot()?;
    if fs::symlink_metadata(path).is_ok() {
        let origins = target::read_agent_skill_origins()?;
        return Ok(snapshot
            .proof(name)
            .is_some_and(|p| p.lexical_target() == path)
            && target::state::verified_created_origin(db, row, origins.get(&row.id))?);
    }
    // A withdrawn target grants no overwrite permission: require strict absence,
    // and no surviving assignment or receipt for this assistant resource.
    if !matches!(fs::symlink_metadata(path),Err(e) if e.kind()==std::io::ErrorKind::NotFound) {
        return Ok(false);
    }
    if snapshot
        .skills
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case(name))
    {
        return Ok(false);
    }
    Ok(!state
        .machine
        .assignments
        .contains_key(&format!("skills:{}:{}", row.id, target::ID))
        && !state
            .machine
            .receipts
            .values()
            .any(|r| r.resource_id == row.id && r.owners.iter().any(|o| o == target::ID)))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateFile {
    name: String,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    mode: Option<u32>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Intent {
    version: u8,
    operation_id: String,
    root_identity: String,
    before_entry_identity: Option<String>,
    after_entry_identity: String,
    epoch: u64,
    row: InstalledSkill,
    old_row: Option<InstalledSkill>,
    old_revision: Option<ResourceRevision>,
    revision: ResourceRevision,
    old_assignment: Option<String>,
    target: TargetWrite,
    files: Vec<StateFile>,
}
fn state_valid() -> Result<(), AppError> {
    target::skill_modes()?;
    target::agent_created()?;
    target::read_agent_skill_origins()?;
    Ok(())
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, AppError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(error)?;
    if bytes.len() > 1024 * 1024 {
        return Err(error("skill_state_too_large"));
    }
    Ok(bytes)
}

pub fn commit(
    db: &Database,
    prepared: &Prepared,
    admit: impl Fn() -> Result<(), AppError>,
) -> Result<Success, AppError> {
    commit_checked(db, prepared, admit, |_| Ok(()), skills::stage)
}
fn commit_checked(
    db: &Database,
    prepared: &Prepared,
    admit: impl Fn() -> Result<(), AppError>,
    checkpoint: impl Fn(&str) -> Result<(), AppError>,
    stage: impl Fn(&mut TargetWrite) -> Result<(), AppError>,
) -> Result<Success, AppError> {
    let _io = crate::config_io::lock();
    recover(db)?;
    admit()?;
    state_valid()?;
    let current = db.library_snapshot()?;
    if current != prepared.before {
        return Err(error("library.epochOrRevisionChanged"));
    }
    if !equal(
        &db.get_installed_skill(&prepared.row.id)?,
        &prepared.old_row,
    ) {
        return Err(error("library.revisionChanged"));
    }
    for (index, name) in FILES.iter().enumerate() {
        if raw(name)? != prepared.sidecars[index] {
            return Err(error("library.sourceChanged"));
        }
    }
    let path = target::skills_dir().join(&prepared.row.directory);
    if skills::entry_id(&path)? != prepared.physical
        || entry_identity(&path)? != prepared.entry_identity
    {
        return Err(error("library.targetChanged"));
    }
    let previous = current
        .machine
        .receipts
        .get(&format!("skills:{}", prepared.physical));
    let owned = previous
        .map(|r| serde_json::from_str::<skills::DirectoryOwned>(&r.owned_segment))
        .transpose()
        .map_err(error)?;
    if skills::owned_hash(&path, owned.as_ref())? != prepared.old_hash {
        return Err(error("library.targetChanged"));
    }
    if let Some(old) = &prepared.old_row {
        if prepared.created
            && target::origin_for_resource(db, old)? != target::SkillOrigin::AssistantCreated
        {
            return Err(error("not_agent_created"));
        }
    }
    // Opaque proof revalidation happens before the journal exists. Never recurse
    // into snapshot while an in-progress commit owns an unresolved intent.
    let snapshot = target::assistant_skill_snapshot(db, &target::reviewed_names())?;
    if snapshot.proof(&prepared.name) != prepared.proof.as_ref()
        || root_identity()? != prepared.root_identity
    {
        return Err(error("library.targetChanged"));
    }
    if let Some(proof) = &prepared.proof {
        target::with_revalidated_skill(db, proof, target::SkillAccess::UserSelected, |_| Ok(()))?;
    }
    artifacts::publish_checked(&prepared.source, Some(&prepared.execution), &prepared.hash)?;
    let mut row = prepared.row.clone();
    row.content_hash = Some(prepared.hash.clone());
    if !prepared.reuse {
        row.updated_at = chrono::Utc::now().timestamp();
    }
    let old_revision = current.definitions.skills.get(&row.id).cloned();
    let revision = if prepared.reuse {
        old_revision
            .clone()
            .ok_or_else(|| error("library.resourceMissing"))?
    } else {
        ResourceRevision {
            revision: old_revision
                .as_ref()
                .map_or(0, |r| r.revision)
                .checked_add(1)
                .ok_or_else(|| error("library.revisionOverflow"))?,
            hash: skills::spec_hash(&row)?,
        }
    };
    let mut op = skills::plan_proposed(
        &current,
        (&row, &revision),
        &[(target::ID.into(), target::skills_dir())],
        None,
        &BTreeSet::new(),
        true,
        &BTreeSet::new(),
    )?;
    if op.targets.len() != 1
        || op.targets[0].reason.is_some()
        || op.targets[0].tools != [target::ID]
    {
        return Err(error("library.skillNameConflict"));
    }
    let mut target = op.targets.remove(0);
    crate::config::create_private_dir_all(&target::skills_dir()).map_err(error)?;
    stage(&mut target)?;
    let receipt = target
        .receipt
        .clone()
        .ok_or_else(|| error("library.stateUnreadable"))?;
    let mut modes = target::skill_modes()?;
    modes.insert(prepared.name.clone(), target::SkillMode::Auto);
    let mut names = target::agent_created()?;
    let mut origins = target::read_agent_skill_origins()?;
    if prepared.created {
        names.insert(prepared.name.clone());
        let creation_id = origins
            .get(&row.id)
            .map(|o| o.creation_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        origins.insert(
            row.id.clone(),
            target::AgentSkillOrigin {
                version: 1,
                creation_id,
                revision: revision.clone(),
                artifact_hash: prepared.hash.clone(),
                receipt: receipt.clone(),
            },
        );
    }
    let after = [encode(&modes)?, encode(&names)?, encode(&origins)?];
    let files = FILES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            Ok(StateFile {
                name: (*name).into(),
                before: prepared.sidecars[i].clone(),
                after: after[i].clone(),
                mode: crate::config::file_mode(&checked_path(name)?),
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let assignment = format!("skills:{}:{}", row.id, target::ID);
    let intent = Intent {
        version: 1,
        operation_id: op.id.clone(),
        root_identity: root_identity()?.ok_or_else(|| error("library.targetChanged"))?,
        before_entry_identity: prepared.entry_identity.clone(),
        after_entry_identity: entry_identity(std::path::Path::new(
            &target
                .directory
                .as_ref()
                .ok_or_else(|| error("library.stateUnreadable"))?
                .stage_path,
        ))?
        .ok_or_else(|| error("library.targetChanged"))?,
        epoch: current.machine.epoch,
        old_row: prepared.old_row.clone(),
        old_revision,
        revision: revision.clone(),
        old_assignment: current.machine.assignments.get(&assignment).cloned(),
        row: row.clone(),
        target,
        files,
    };
    let bytes = serde_json::to_vec(&intent).map_err(error)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(error("skill_state_too_large"));
    }
    crate::config::atomic_write_with(
        &checked_path(JOURNAL)?,
        &bytes,
        crate::config::AtomicOpts { mode: Some(0o600) },
    )?;
    let result = (|| {
        admit()?;
        checkpoint("journal")?;
        let mut txn = crate::config::FileTxn::new();
        for file in &intent.files {
            txn.write_with_mode(&checked_path(&file.name)?, &file.after, 0o600)?;
            checkpoint(&file.name)?;
        }
        skills::write(&intent.target)?;
        checkpoint("swap")?;
        if !skills::receipt_intact(&receipt) {
            return Err(error("library.verifyFailed"));
        }
        state_valid()?;
        admit()?;
        checkpoint("cas")?;
        let mut next = current.clone();
        next.definitions.skills.insert(row.id.clone(), revision);
        next.machine.assignments.insert(assignment, row.id.clone());
        next.machine
            .receipts
            .insert(intent.target.receipt_key.clone(), receipt);
        db.library_commit_skill(&current, &next, Some(&row), None)?;
        checkpoint("committed")?;
        Ok(())
    })();
    let success = Success {
        name: prepared.name.clone(),
        row: intent.row.clone(),
        revision: intent.revision.clone(),
        receipt: intent
            .target
            .receipt
            .clone()
            .ok_or_else(|| error("library.stateUnreadable"))?,
    };
    match result {
        Ok(()) => {
            // A cleanup problem preserves committed truth and blocks subsequent snapshots.
            let _ = recover(db);
            Ok(success)
        }
        Err(failure) => {
            // An error after CAS is still a committed operation, never a cancellation.
            let current = db.library_snapshot()?;
            let row = db.get_installed_skill(&intent.row.id)?;
            if committed_state(&current, &row, &intent) {
                let _ = recover(db);
                Ok(success)
            } else {
                recover(db)?;
                Err(failure)
            }
        }
    }
}

fn committed_state(
    current: &LibrarySnapshot,
    row: &Option<InstalledSkill>,
    intent: &Intent,
) -> bool {
    equal(row, &Some(intent.row.clone()))
        && current.definitions.skills.get(&intent.row.id) == Some(&intent.revision)
        && current
            .machine
            .assignments
            .get(&format!("skills:{}:{}", intent.row.id, target::ID))
            == Some(&intent.row.id)
        && current.machine.receipts.get(&intent.target.receipt_key)
            == intent.target.receipt.as_ref()
}

/// Called before every skill snapshot and mutation. No snapshots, forward writes or replay here.
pub fn recover(db: &Database) -> Result<(), AppError> {
    let _io = crate::config_io::lock();
    let path = checked_path(JOURNAL)?;
    let Some(bytes) = target::state::read_regular(&path, 16 * 1024 * 1024)? else {
        return Ok(());
    };
    let intent: Intent =
        serde_json::from_slice(&bytes).map_err(|_| error("skill_transaction_needs_review"))?;
    let refuse = || error("skill_transaction_needs_review");
    if intent.version != 1
        || intent.files.len() != 3
        || intent.files.iter().enumerate().any(|(i, f)| {
            f.name != FILES[i]
                || f.after.len() > 1024 * 1024
                || f.before.as_ref().is_some_and(|b| b.len() > 1024 * 1024)
        })
    {
        return Err(refuse());
    }
    skills::validate_skill(&intent.row)?;
    let target_path = target::skills_dir().join(&intent.row.directory);
    if root_identity()?.as_deref() != Some(&intent.root_identity) {
        return Err(refuse());
    }
    let actual_entry = entry_identity(&target_path)?;
    if actual_entry != intent.before_entry_identity
        && actual_entry.as_deref() != Some(&intent.after_entry_identity)
        && actual_entry.is_some()
    {
        return Err(refuse());
    }

    if intent.target.path != target_path.to_string_lossy()
        || skills::entry_id(&target_path)? != intent.target.physical_target
    {
        return Err(refuse());
    }
    // Validate full existing library plan shape before consuming any journal paths.
    let op = super::Operation {
        id: intent.operation_id.clone(),
        kind: super::ResourceKind::Skills,
        epoch: intent.epoch,
        resource_id: intent.row.id.clone(),
        resource_revision: intent.revision.revision,
        resource_hash: intent.revision.hash.clone(),
        created_at: 0,
        status: "prepared".into(),
        targets: vec![intent.target.clone()],
        desired_updates: BTreeMap::new(),
        previous_assignments: BTreeMap::new(),
        resolved_paths: BTreeMap::new(),
        resolved_physical: BTreeMap::new(),
        routing: None,
    };
    skills::validate_target(&intent.target, &op)?;
    if let Some(plan) = &intent.target.directory {
        let staged = entry_identity(std::path::Path::new(&plan.stage_path))?;
        if staged.is_some() && staged.as_deref() != Some(&intent.after_entry_identity) {
            return Err(refuse());
        }
        let backup = entry_identity(std::path::Path::new(&plan.backup_path))?;
        if backup.is_some() && backup != intent.before_entry_identity {
            return Err(refuse());
        }
    }

    let current = db.library_snapshot()?;
    if current.machine.epoch != intent.epoch {
        return Err(refuse());
    }
    let assignment = format!("skills:{}:{}", intent.row.id, target::ID);
    let row = db.get_installed_skill(&intent.row.id)?;
    let committed = committed_state(&current, &row, &intent);
    let before = equal(&row, &intent.old_row)
        && current.definitions.skills.get(&intent.row.id) == intent.old_revision.as_ref()
        && current.machine.assignments.get(&assignment) == intent.old_assignment.as_ref()
        && current.machine.receipts.get(&intent.target.receipt_key)
            == intent.target.previous_receipt.as_ref();
    if !committed && !before {
        return Err(refuse());
    }
    for file in &intent.files {
        let current = raw(&file.name)?;
        if committed {
            if current.as_ref() != Some(&file.after) {
                return Err(refuse());
            }
        } else if current != file.before && current.as_ref() != Some(&file.after) {
            return Err(refuse());
        }
    }
    if committed {
        if actual_entry.as_deref() != Some(&intent.after_entry_identity) {
            return Err(refuse());
        }
        if !intent
            .target
            .receipt
            .as_ref()
            .is_some_and(skills::receipt_intact)
        {
            return Err(refuse());
        }
        skills::cleanup(&intent.target)?;
    } else {
        skills::rollback_written(&intent.target)?;
        for file in intent.files.iter().rev() {
            let path = checked_path(&file.name)?;
            match &file.before {
                Some(bytes) => crate::config::atomic_write_with(
                    &path,
                    bytes,
                    crate::config::AtomicOpts { mode: file.mode },
                )?,
                None => {
                    if path.exists() {
                        fs::remove_file(path).map_err(error)?;
                    }
                }
            }
        }
    }
    // The journal itself is never silently replaced or removed on an uncertain outcome.
    if target::state::read_regular(&path, 16 * 1024 * 1024)?.as_deref() != Some(bytes.as_slice()) {
        return Err(refuse());
    }
    fs::remove_file(path).map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    #[test]
    #[serial_test::serial]
    fn skill_tools_save_commit_and_off_update_require_actual_origin() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let draft = prepare_save(&db, "fixture", "A fixture", "Original").unwrap();
        commit(&db, &draft, || Ok(())).unwrap();
        let rows = target::assistant_skill_snapshot(&db, &BTreeSet::new()).unwrap();
        assert_eq!(
            rows.skills[0].origin,
            Some(target::SkillOrigin::AssistantCreated)
        );
        let row = db
            .get_all_installed_skills()
            .unwrap()
            .into_values()
            .next()
            .unwrap();
        let artifact = skills::artifact_for(&db, &row.id).unwrap();
        let original = fs::read(artifact.join("SKILL.md")).unwrap();
        let off = skills::preview(&db, &row.id, &[]).unwrap();
        super::super::apply_operation(&db, &off.id).unwrap();
        assert_eq!(
            target::origin_for_resource(&db, &row).unwrap(),
            target::SkillOrigin::AssistantCreated
        );
        let own = prepare_save(&db, "fixture", "Updated fixture", "Updated").unwrap();
        commit(&db, &own, || Ok(())).unwrap();
        assert_eq!(
            fs::read(artifact.join("SKILL.md")).unwrap(),
            original,
            "immutable old artifact"
        );
        assert!(
            fs::read_to_string(target::skills_dir().join("fixture/SKILL.md"))
                .unwrap()
                .contains("Updated")
        );
        let off = skills::preview(&db, &row.id, &[]).unwrap();
        super::super::apply_operation(&db, &off.id).unwrap();
        fs::create_dir_all(target::skills_dir().join("fixture")).unwrap();
        fs::write(target::skills_dir().join("fixture/SKILL.md"), "foreign").unwrap();
        assert!(prepare_save(&db, "fixture", "Another", "No overwrite").is_err());
        assert_eq!(
            fs::read_to_string(target::skills_dir().join("fixture/SKILL.md")).unwrap(),
            "foreign"
        );
    }
    #[test]
    #[serial_test::serial]
    fn skill_tools_restart_recovers_only_proven_bytes() {
        for phase in [
            "journal",
            "skill-modes.json",
            "agent-skills.json",
            "agent-skill-origins.json",
            "swap",
            "cas",
            "committed",
        ] {
            for overwrite in [false, true] {
                let _sandbox = crate::test_sandbox::sandbox();
                let db = Database::memory().unwrap();
                if overwrite {
                    let initial = prepare_save(&db, "fixture", "Initial", "Initial").unwrap();
                    commit(&db, &initial, || Ok(())).unwrap();
                }
                let before = db.library_snapshot().unwrap();
                let files = FILES.map(|s| raw(s).unwrap());
                let original = fs::read(target::skills_dir().join("fixture/SKILL.md")).ok();
                let draft = prepare_save(&db, "fixture", "Replacement", "Replacement").unwrap();
                assert!(catch_unwind(AssertUnwindSafe(|| commit_checked(
                    &db,
                    &draft,
                    || Ok(()),
                    |p| {
                        if p == phase {
                            panic!("simulated process loss")
                        };
                        Ok(())
                    },
                    skills::stage
                )))
                .is_err());
                assert!(checked_path(JOURNAL).unwrap().exists());
                // Snapshot is the actual restart authority entry, not an installer replay.
                let snapshot = target::assistant_skill_snapshot(&db, &BTreeSet::new()).unwrap();
                assert!(!checked_path(JOURNAL).unwrap().exists(), "{phase}");
                if phase == "committed" {
                    assert_eq!(snapshot.skills.len(), 1);
                    assert!(
                        fs::read_to_string(target::skills_dir().join("fixture/SKILL.md"))
                            .unwrap()
                            .contains("Replacement")
                    );
                } else {
                    assert_eq!(db.library_snapshot().unwrap(), before, "{phase}");
                    assert_eq!(FILES.map(|s| raw(s).unwrap()), files, "{phase}");
                    assert_eq!(
                        fs::read(target::skills_dir().join("fixture/SKILL.md")).ok(),
                        original,
                        "{phase}"
                    );
                }
            }
        }
    }
    #[test]
    #[serial_test::serial]
    fn skill_tools_rollback_conflict_keeps_evidence_and_blocks_local_fallback() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let draft = prepare_save(&db, "fixture", "Fixture", "Body").unwrap();
        let before = db.library_snapshot().unwrap();
        assert!(catch_unwind(AssertUnwindSafe(|| commit_checked(
            &db,
            &draft,
            || Ok(()),
            |p| {
                if p == "swap" {
                    panic!("crash")
                };
                Ok(())
            },
            skills::stage
        )))
        .is_err());
        skills::unlink(&target::skills_dir().join("fixture")).unwrap();
        fs::create_dir_all(target::skills_dir().join("fixture")).unwrap();
        fs::write(
            target::skills_dir().join("fixture/SKILL.md"),
            "---\nname: fixture\ndescription: Foreign\n---\nForeign",
        )
        .unwrap();
        assert!(target::assistant_skill_snapshot(&db, &BTreeSet::new()).is_err());
        assert!(checked_path(JOURNAL).unwrap().exists());
        assert_eq!(db.library_snapshot().unwrap(), before);
        assert!(
            fs::read_to_string(target::skills_dir().join("fixture/SKILL.md"))
                .unwrap()
                .contains("Foreign")
        );
    }
    #[test]
    #[serial_test::serial]
    fn skill_tools_fallback_copy_journal_uses_final_receipt_and_reopens_database() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::init().unwrap();
        let draft = prepare_save(&db, "fixture", "Fixture", "Body").unwrap();
        let stage = |t: &mut TargetWrite| {
            skills::stage_with(t, |_, _| {
                Err(std::io::Error::other("simulated missing symlink privilege"))
            })
        };
        assert!(catch_unwind(AssertUnwindSafe(|| commit_checked(
            &db,
            &draft,
            || Ok(()),
            |p| {
                if p == "swap" {
                    panic!("crash")
                };
                Ok(())
            },
            stage
        )))
        .is_err());
        let intent: Intent =
            serde_json::from_slice(&fs::read(checked_path(JOURNAL).unwrap()).unwrap()).unwrap();
        assert_eq!(intent.target.directory.as_ref().unwrap().method, "copy");
        assert!(!fs::symlink_metadata(target::skills_dir().join("fixture"))
            .unwrap()
            .file_type()
            .is_symlink());
        drop(db);
        let db = Database::init().unwrap();
        assert!(target::assistant_skill_snapshot(&db, &BTreeSet::new())
            .unwrap()
            .skills
            .is_empty());
        assert!(!target::skills_dir().join("fixture").exists());
        let draft = prepare_save(&db, "fixture", "Fixture", "Body").unwrap();
        commit_checked(&db, &draft, || Ok(()), |_| Ok(()), stage).unwrap();
        let origin = target::read_agent_skill_origins()
            .unwrap()
            .into_values()
            .next()
            .unwrap();
        let owned: skills::DirectoryOwned =
            serde_json::from_str(&origin.receipt.owned_segment).unwrap();
        assert_eq!(owned.method, "copy");
        drop(db);
        let db = Database::init().unwrap();
        assert_eq!(
            target::assistant_skill_snapshot(&db, &BTreeSet::new())
                .unwrap()
                .skills[0]
                .origin,
            Some(target::SkillOrigin::AssistantCreated)
        );
    }
    #[test]
    #[serial_test::serial]
    fn skill_tools_prepared_source_target_identity_and_state_changes_fail_closed() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let draft = prepare_save(&db, "fixture", "Fixture", "Before").unwrap();
        fs::write(draft.source.join("SKILL.md"), "changed source").unwrap();
        assert!(commit(&db, &draft, || Ok(())).is_err());
        assert!(db.get_all_installed_skills().unwrap().is_empty());
        let draft = prepare_save(&db, "fixture", "Fixture", "Before").unwrap();
        let stage = |t: &mut TargetWrite| {
            skills::stage_with(t, |_, _| Err(std::io::Error::other("copy fixture")))
        };
        commit_checked(&db, &draft, || Ok(()), |_| Ok(()), stage).unwrap();
        let draft = prepare_save(&db, "fixture", "Fixture", "After").unwrap();
        let row = db.get_installed_skill(&draft.row.id).unwrap().unwrap();
        let artifact = skills::artifact_for(&db, &row.id).unwrap();
        let path = target::skills_dir().join("fixture");
        let before = db.library_snapshot().unwrap();
        let old = path.with_file_name(".external-old");
        fs::rename(&path, &old).unwrap();
        artifacts::copy_verified(&artifact, &path).unwrap();
        assert!(skills::receipt_intact(
            before
                .machine
                .receipts
                .values()
                .find(|r| r.resource_id == row.id)
                .unwrap()
        ));
        assert!(
            commit(&db, &draft, || Ok(())).is_err(),
            "identical content in a changed directory is not the approved proof"
        );
        assert_eq!(db.library_snapshot().unwrap(), before);
        let draft = prepare_save(&db, "fixture", "Fixture", "After").unwrap();
        fs::write(checked_path("skill-modes.json").unwrap(), "{bad").unwrap();
        assert!(commit(&db, &draft, || Ok(())).is_err());
        assert_eq!(
            fs::read_to_string(checked_path("skill-modes.json").unwrap()).unwrap(),
            "{bad"
        );
    }
    #[test]
    #[serial_test::serial]
    fn skill_tools_recovery_does_not_remove_identical_replacement_directory() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let draft = prepare_save(&db, "fixture", "Fixture", "Body").unwrap();
        assert!(catch_unwind(AssertUnwindSafe(|| commit_checked(
            &db,
            &draft,
            || Ok(()),
            |p| {
                if p == "swap" {
                    panic!("crash")
                };
                Ok(())
            },
            |t| skills::stage_with(t, |_, _| Err(std::io::Error::other("copy fixture")))
        )))
        .is_err());
        let path = target::skills_dir().join("fixture");
        let original = path.with_file_name(".original-pending");
        fs::rename(&path, &original).unwrap();
        artifacts::copy_verified(&artifacts::path(&draft.hash).unwrap(), &path).unwrap();
        assert!(target::assistant_skill_snapshot(&db, &BTreeSet::new()).is_err());
        assert!(path.join("SKILL.md").is_file());
        assert!(checked_path(JOURNAL).unwrap().exists());
        assert!(db.get_all_installed_skills().unwrap().is_empty());
    }
    #[test]
    #[serial_test::serial]
    fn skill_tools_late_cancel_rolls_back_and_epoch_conflict_preserves_unrelated_state() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let draft = prepare_save(&db, "fixture", "Fixture", "Body").unwrap();
        let before = db.library_snapshot().unwrap();
        let calls = std::cell::Cell::new(0);
        assert!(commit(&db, &draft, || {
            calls.set(calls.get() + 1);
            if calls.get() == 3 {
                Err(error("cancelled"))
            } else {
                Ok(())
            }
        })
        .is_err());
        assert_eq!(db.library_snapshot().unwrap(), before);
        assert!(!target::skills_dir().join("fixture").exists());
        let draft = prepare_save(&db, "fixture", "Fixture", "Body").unwrap();
        assert!(catch_unwind(AssertUnwindSafe(|| commit_checked(
            &db,
            &draft,
            || Ok(()),
            |p| {
                if p == "swap" {
                    panic!("crash")
                };
                Ok(())
            },
            skills::stage
        )))
        .is_err());
        let before = db.library_snapshot().unwrap();
        let mut changed = before.clone();
        changed.machine.epoch += 1;
        changed.definitions.skills.insert(
            "unrelated-user-resource".into(),
            ResourceRevision {
                revision: 1,
                hash: "b".repeat(64),
            },
        );
        db.library_commit(&before, &changed).unwrap();
        let current = db.library_snapshot().unwrap();
        let journal = fs::read(checked_path(JOURNAL).unwrap()).unwrap();
        assert!(!String::from_utf8_lossy(&journal).contains("unrelated-user-resource"));
        assert!(target::assistant_skill_snapshot(&db, &BTreeSet::new()).is_err());
        assert_eq!(db.library_snapshot().unwrap(), current);
        assert_eq!(fs::read(checked_path(JOURNAL).unwrap()).unwrap(), journal);
    }
    #[test]
    #[serial_test::serial]
    fn skill_tools_drift_and_cancel_never_mutate() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let draft = prepare_save(&db, "fixture", "Fixture", "Body").unwrap();
        assert!(commit(&db, &draft, || Err(error("cancelled"))).is_err());
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        next.machine.epoch += 1;
        db.library_commit(&before, &next).unwrap();
        let next = db.library_snapshot().unwrap();
        assert!(commit(&db, &draft, || Ok(())).is_err());
        assert_eq!(db.library_snapshot().unwrap(), next);
        assert!(!target::skills_dir().join("fixture").exists());
        assert!(validate_save("fixture", &"😀".repeat(500), &"😀".repeat(60000)).is_ok());
        assert!(validate_save("fixture", &"😀".repeat(501), "").is_err());
        assert!(validate_save("fixture", "Description", &"😀".repeat(60001)).is_err());
    }
}
