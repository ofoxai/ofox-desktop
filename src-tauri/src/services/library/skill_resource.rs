//! Skill drafts and directory ownership, independent of model binding.
use super::{
    coordinator as io, skill_artifact as artifacts, Operation, Receipt, ResourceKind,
    ResourceRevision, TargetWrite,
};
use crate::{
    app_config::{InstalledSkill, SkillApps},
    error::AppError,
    Database,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DirectoryPlan {
    pub artifact_hash: String,
    pub method: String,
    pub fallback_copy: bool,
    pub stage_path: String,
    pub backup_path: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DirectoryOwned {
    pub artifact_hash: String,
    pub method: String,
    pub entry_hash: String,
    /// Present when Ofox took over an entry that was already there instead of
    /// writing it; older receipts have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopted: Option<AdoptedEntry>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdoptedEntry {
    /// The link's own text when the entry taken over is a symlink.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

pub fn validate_skill(skill: &InstalledSkill) -> Result<(), AppError> {
    artifacts::safe_leaf(&skill.directory)?;
    if skill.id.is_empty() || skill.id.len() > 256 || skill.name.trim().is_empty() {
        return Err(error("invalidSkill"));
    }
    if let Some(hash) = &skill.content_hash {
        artifacts::path(hash)?;
    }
    Ok(())
}

pub fn spec_hash(skill: &InstalledSkill) -> Result<String, AppError> {
    let mut portable = skill.clone();
    portable.apps = SkillApps::default();
    Ok(io::fingerprint(Some(
        &serde_json::to_vec(&portable).map_err(|_| error("invalidSkill"))?,
    )))
}
pub fn save_draft(
    db: &Database,
    skill: InstalledSkill,
    source: &Path,
    expected: Option<u64>,
) -> Result<InstalledSkill, AppError> {
    save_draft_with_execution(db, skill, source, expected, None)
}
pub fn save_draft_with_execution(
    db: &Database,
    skill: InstalledSkill,
    source: &Path,
    expected: Option<u64>,
    flags: Option<&artifacts::ExecutionMap>,
) -> Result<InstalledSkill, AppError> {
    save_draft_checked_with_execution(db, skill, source, expected, flags, None)
}
pub(crate) fn save_draft_with_execution_at_epoch(
    db: &Database,
    skill: InstalledSkill,
    source: &Path,
    expected: Option<u64>,
    flags: Option<&artifacts::ExecutionMap>,
    expected_epoch: u64,
) -> Result<InstalledSkill, AppError> {
    save_draft_checked_at_epoch(
        db,
        skill,
        source,
        expected,
        flags,
        None,
        Some(expected_epoch),
    )
}
pub(crate) fn save_draft_checked_with_execution(
    db: &Database,
    skill: InstalledSkill,
    source: &Path,
    expected: Option<u64>,
    flags: Option<&artifacts::ExecutionMap>,
    expected_hash: Option<&str>,
) -> Result<InstalledSkill, AppError> {
    save_draft_checked_at_epoch(db, skill, source, expected, flags, expected_hash, None)
}
fn save_draft_checked_at_epoch(
    db: &Database,
    mut skill: InstalledSkill,
    source: &Path,
    expected: Option<u64>,
    flags: Option<&artifacts::ExecutionMap>,
    expected_hash: Option<&str>,
    expected_epoch: Option<u64>,
) -> Result<InstalledSkill, AppError> {
    // Collection is synchronous here. Keep restoration outside the publication
    // and commit interval without retaining a configuration lock across download.
    let _epoch_guard = expected_epoch.map(|_| crate::config_io::lock());
    if let Some(epoch) = expected_epoch {
        if db.library_snapshot()?.machine.epoch != epoch {
            return Err(error("epochChanged"));
        }
    }
    artifacts::safe_leaf(&skill.directory)?;
    if skill.id.is_empty() || skill.id.len() > 256 || skill.name.trim().is_empty() {
        return Err(error("invalidSkill"));
    }
    let artifact = if let Some(hash) = expected_hash {
        artifacts::publish_checked(source, flags, hash)?
    } else {
        artifacts::publish_with_execution(source, flags)?
    };
    let _guard = crate::config_io::lock();
    let before = db.library_snapshot()?;
    if expected_epoch.is_some_and(|epoch| before.machine.epoch != epoch) {
        return Err(error("epochChanged"));
    }
    if before
        .definitions
        .skills
        .get(&skill.id)
        .map(|meta| meta.revision)
        != expected
    {
        return Err(error("revisionChanged"));
    }
    if expected.is_some() {
        artifact_for(db, &skill.id)?;
    }
    let existing = db.get_all_installed_skills()?;
    if existing
        .values()
        .any(|row| row.id != skill.id && row.directory.eq_ignore_ascii_case(&skill.directory))
    {
        return Err(error("skillNameConflict"));
    }
    if expected.is_none() && existing.contains_key(&skill.id) {
        return Err(error("revisionChanged"));
    }
    skill.apps = SkillApps::default();
    skill.content_hash = Some(artifact);
    let hash = spec_hash(&skill)?;
    let mut next = before.clone();
    next.definitions.skills.insert(
        skill.id.clone(),
        ResourceRevision {
            revision: expected
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| error("revisionOverflow"))?,
            hash,
        },
    );
    db.library_commit_skill(&before, &next, Some(&skill), None)?;
    Ok(skill)
}
/// Legacy rows are draft import candidates, never evidence of native ownership.
pub fn ensure_metadata(db: &Database) -> Result<(), AppError> {
    let _guard = crate::config_io::lock();
    let rows = db.get_all_installed_skills()?;
    for row in rows.values() {
        validate_skill(row)?;
        if db
            .library_snapshot()?
            .definitions
            .skills
            .contains_key(&row.id)
        {
            continue;
        }
        artifacts::safe_leaf(&row.directory)?;
        let source = crate::services::skill::SkillService::get_ssot_dir()
            .map_err(|_| error("skillUnreadable"))?
            .join(&row.directory);
        if !source.is_dir() {
            continue;
        }
        let canonical = source
            .canonicalize()
            .map_err(|_| error("skillUnreadable"))?;
        let mut draft = row.clone();
        draft.apps = SkillApps::default();
        draft.content_hash = Some(artifacts::publish(&canonical)?);
        let before = db.library_snapshot()?;
        if before.definitions.skills.contains_key(&row.id) {
            continue;
        }
        let mut next = before.clone();
        next.definitions.skills.insert(
            row.id.clone(),
            ResourceRevision {
                revision: 1,
                hash: spec_hash(&draft)?,
            },
        );
        db.library_commit_skill(&before, &next, Some(&draft), None)?;
    }
    Ok(())
}
pub fn delete_draft(db: &Database, id: &str) -> Result<bool, AppError> {
    let _guard = crate::config_io::lock();
    let before = db.library_snapshot()?;
    if before
        .machine
        .receipts
        .values()
        .any(|receipt| receipt.kind == ResourceKind::Skills && receipt.resource_id == id)
        || before
            .machine
            .assignments
            .iter()
            .any(|(key, resource)| key.starts_with("skills:") && resource == id)
        || before.machine.operations.values().any(|op| {
            op.kind == ResourceKind::Skills
                && op.resource_id == id
                && !["previewed", "applied", "cancelled"].contains(&op.status.as_str())
        })
    {
        return Err(error("withdrawRequired"));
    }
    let exists = db.get_installed_skill(id)?.is_some();
    let mut next = before.clone();
    next.definitions.skills.remove(id);
    for op in next.machine.operations.values_mut().filter(|op| {
        op.kind == ResourceKind::Skills && op.resource_id == id && op.status == "previewed"
    }) {
        op.status = "cancelled".into();
    }
    db.library_commit_skill(&before, &next, None, Some(id))?;
    // Artifacts may still be read by old, unowned links. Deletion is append-only.
    Ok(exists)
}
pub fn artifact_for(db: &Database, id: &str) -> Result<PathBuf, AppError> {
    let snapshot = db.library_snapshot()?;
    let meta = snapshot
        .definitions
        .skills
        .get(id)
        .ok_or_else(|| error("skillNeedsMigration"))?;
    let row = db
        .get_installed_skill(id)?
        .ok_or_else(|| error("resourceMissing"))?;
    validate_skill(&row)?;
    if spec_hash(&row)? != meta.hash {
        return Err(error("revisionChanged"));
    }
    let hash = row
        .content_hash
        .as_deref()
        .ok_or_else(|| error("skillArtifactMissing"))?;
    let path = artifacts::path(hash)?;
    if artifacts::tree_hash(&path)? != hash {
        return Err(error("skillArtifactChanged"));
    }
    Ok(path)
}
pub fn native_paths() -> Result<BTreeMap<String, PathBuf>, AppError> {
    let mut paths = BTreeMap::new();
    for target in super::skill_targets::TARGETS {
        match super::skill_targets::root(target.id) {
            Ok(path) => {
                paths.insert(target.id.to_string(), path);
            }
            // A misplaced folder variable hides only that agent; the five
            // bound tools still fail loudly, as before.
            Err(failure)
                if ["claude", "codex", "gemini", "opencode", "hermes"].contains(&target.id) =>
            {
                return Err(failure)
            }
            Err(_) => {}
        }
    }
    Ok(paths)
}
pub fn target_paths() -> Result<BTreeMap<String, PathBuf>, AppError> {
    let mut paths = native_paths()?;
    paths.insert(
        "shared".into(),
        crate::config::get_home_dir().join(".agents/skills"),
    );
    Ok(paths)
}
fn flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| ["true", "1"].contains(&value.to_ascii_lowercase().as_str()))
}
pub fn reader_paths() -> Result<BTreeMap<String, Vec<PathBuf>>, AppError> {
    let targets = native_paths()?;
    let home = crate::config::get_home_dir();
    let shared = home.join(".agents").join("skills");
    let mut readers: BTreeMap<String, Vec<PathBuf>> = targets
        .into_iter()
        .map(|(tool, path)| (tool, vec![path]))
        .collect();
    // Gemini and OpenCode find the shared folder their own way, below.
    for target in super::skill_targets::TARGETS
        .iter()
        .filter(|target| target.reads_shared && !["gemini", "opencode"].contains(&target.id))
    {
        if let Some(roots) = readers.get_mut(target.id) {
            roots.push(shared.clone());
        }
    }
    // A known default reader, not a supported native write target or proof of loading.
    readers.insert("copilot".into(), vec![shared.clone()]);
    readers.get_mut("gemini").unwrap().push(
        std::env::var_os("GEMINI_CLI_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.clone())
            .join(".agents/skills"),
    );
    let opencode = readers.get_mut("opencode").unwrap();
    opencode.push(crate::opencode_config::get_opencode_dir().join("skill"));
    opencode.push(home.join(".opencode").join("skills"));
    if let Some(extra) = std::env::var_os("OPENCODE_CONFIG_DIR").filter(|value| !value.is_empty()) {
        opencode.push(PathBuf::from(extra).join("skills"));
    }
    if !flag("OPENCODE_DISABLE_EXTERNAL_SKILLS") {
        opencode.push(shared);
        if !flag("OPENCODE_DISABLE_CLAUDE_CODE") && !flag("OPENCODE_DISABLE_CLAUDE_CODE_SKILLS") {
            opencode.push(home.join(".claude").join("skills"));
        }
    }
    let mut config = if crate::opencode_config::has_global_config() {
        crate::opencode_config::read_opencode_config()?
    } else {
        serde_json::json!({})
    };
    let mut extra_configs = Vec::new();
    if let Some(raw) = std::env::var_os("OPENCODE_CONFIG").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(raw);
        if !path.is_absolute() {
            return Err(error("skillScopeUnsupported"));
        }
        extra_configs.push(path);
    }
    let mut dirs = vec![home.join(".opencode")];
    if let Some(raw) = std::env::var_os("OPENCODE_CONFIG_DIR").filter(|value| !value.is_empty()) {
        let dir = PathBuf::from(raw);
        if !dir.is_absolute() {
            return Err(error("skillScopeUnsupported"));
        }
        dirs.push(dir);
    }
    for dir in dirs {
        opencode.push(dir.join("skill"));
        opencode.push(dir.join("skills"));
        for name in ["opencode.json", "opencode.jsonc"] {
            extra_configs.push(dir.join(name));
        }
    }
    for path in extra_configs {
        if let Some(text) = io::read(&path)? {
            let value = super::native_json::parse(&text, true, true, false)?;
            crate::opencode_config::merge_global_value(&mut config, value);
        }
    }
    if let Ok(text) = std::env::var("OPENCODE_CONFIG_CONTENT") {
        if !text.is_empty() {
            if text.len() > 2 * 1024 * 1024 {
                return Err(error("targetTooLarge"));
            }
            let value = super::native_json::parse(&text, true, true, false)?;
            crate::opencode_config::merge_global_value(&mut config, value);
        }
    }
    if let Some(paths) = config.pointer("/skills/paths") {
        for path in paths.as_array().ok_or_else(|| error("nativeInvalid"))? {
            let raw = path.as_str().ok_or_else(|| error("nativeInvalid"))?;
            if raw.contains("{env:") || raw.contains("{file:") {
                return Err(error("skillScopeUnsupported"));
            }
            let path = if let Some(tail) = raw.strip_prefix("~/") {
                home.join(tail)
            } else {
                PathBuf::from(raw)
            };
            if !path.is_absolute() {
                return Err(error("skillScopeUnsupported"));
            }
            opencode.push(path);
        }
    }
    super::native_json::validate_routing_keys(&config, &["skills"])?;
    let hermes_path = crate::hermes_config::get_hermes_config_path();
    if hermes_path.exists() {
        let text = io::read(&hermes_path)?.ok_or_else(|| error("nativeInvalid"))?;
        let config: serde_json::Value =
            serde_yaml::from_str(&text).map_err(|_| error("nativeInvalid"))?;
        if let Some(root) = config.pointer("/skills/create_dir") {
            let raw = root.as_str().ok_or_else(|| error("nativeInvalid"))?;
            if !raw.trim().is_empty() {
                readers.get_mut("hermes").unwrap().push(hermes_reader(raw)?);
            }
        }
        if let Some(extra) = config.pointer("/skills/external_dirs") {
            let values = if extra.is_string() {
                vec![extra.clone()]
            } else {
                extra
                    .as_array()
                    .ok_or_else(|| error("nativeInvalid"))?
                    .clone()
            };
            for root in values {
                let raw = root.as_str().ok_or_else(|| error("nativeInvalid"))?;
                if !raw.trim().is_empty() {
                    readers.get_mut("hermes").unwrap().push(hermes_reader(raw)?);
                }
            }
        }
    }
    for roots in readers.values_mut() {
        roots.sort();
        roots.dedup();
    }
    Ok(readers)
}
fn hermes_reader(raw: &str) -> Result<PathBuf, AppError> {
    let mut expanded = String::new();
    let mut rest = raw.trim();
    while let Some(start) = rest.find('$') {
        expanded.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        let (name, used) = if let Some(tail) = tail.strip_prefix('{') {
            let end = tail
                .find('}')
                .ok_or_else(|| error("skillScopeUnsupported"))?;
            (&tail[..end], end + 3)
        } else {
            let end = tail
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                .count();
            (&tail[..end], end + 1)
        };
        if name.is_empty() {
            return Err(error("skillScopeUnsupported"));
        }
        let value = std::env::var(name).map_err(|_| error("skillScopeUnsupported"))?;
        expanded.push_str(&value);
        rest = &rest[start + used..];
    }
    expanded.push_str(rest);
    let path = if expanded == "~" {
        crate::config::get_home_dir()
    } else if let Some(tail) = expanded.strip_prefix("~/") {
        crate::config::get_home_dir().join(tail)
    } else {
        PathBuf::from(expanded)
    };
    Ok(if path.is_absolute() {
        path
    } else {
        crate::hermes_config::get_hermes_dir().join(path)
    })
}
pub fn reads_entry(
    tool: &str,
    root: &Path,
    entry: &Path,
    directory: &str,
) -> Result<bool, AppError> {
    if ["opencode", "hermes"].contains(&tool) {
        let root = io::physical(root)?;
        Ok(entry.starts_with(root))
    } else {
        Ok(entry_id(&root.join(directory))? == entry.to_string_lossy())
    }
}
pub fn resolver_paths() -> Result<BTreeMap<String, PathBuf>, AppError> {
    let mut paths = target_paths()?;
    for (tool, roots) in reader_paths()? {
        for (index, path) in roots.into_iter().enumerate() {
            paths.insert(format!("reader:{tool}:{index}"), path);
        }
    }
    Ok(paths)
}
/// Follow aliases of the parent, never the leaf link pointing at an artifact.
pub fn entry_id(path: &Path) -> Result<String, AppError> {
    let parent = path.parent().ok_or_else(|| error("invalidSkillPath"))?;
    let leaf = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| error("invalidSkillPath"))?;
    artifacts::safe_leaf(leaf)?;
    io::physical(parent)?
        .join(leaf)
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| error("invalidSkillPath"))
}
pub fn entry_hash_with(path: &Path, artifact: Option<&str>) -> Result<String, AppError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok("missing".into()),
        Err(_) => return Err(error("skillUnreadable")),
    };
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(path).map_err(|_| error("skillUnreadable"))?;
        let source = if link.is_absolute() {
            link.clone()
        } else {
            path.parent()
                .ok_or_else(|| error("skillUnreadable"))?
                .join(&link)
        };
        let tree = artifacts::tree_hash(&source)?;
        return Ok(io::fingerprint(Some(
            format!("symlink:{}:{tree}", link.to_string_lossy()).as_bytes(),
        )));
    }
    if !metadata.is_dir() {
        return Err(error("skillEntryUnsupported"));
    }
    let hash = if let Some(artifact) = artifact {
        artifacts::hash_with_execution(path, &artifacts::manifest(artifact)?)?
    } else {
        artifacts::tree_hash(path)?
    };
    Ok(io::fingerprint(Some(format!("copy:{hash}").as_bytes())))
}
fn predicted(hash: &str, method: &str) -> Result<String, AppError> {
    if method == "withdraw" {
        return Ok("missing".into());
    }
    let text = if method == "symlink" {
        format!(
            "symlink:{}:{hash}",
            artifacts::path(hash)?.to_string_lossy()
        )
    } else {
        format!("copy:{hash}")
    };
    Ok(io::fingerprint(Some(text.as_bytes())))
}
/// A taken-over entry is proven by what it is, not by Ofox's own writing
/// rules: a link by its own text (it may be relative, so it is not followed),
/// a folder by its files whatever their modes.
fn adopted_hash(path: &Path) -> Result<String, AppError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok("missing".into()),
        Err(_) => return Err(error("skillUnreadable")),
    };
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(path).map_err(|_| error("skillUnreadable"))?;
        return Ok(adopted_link_hash(&link.to_string_lossy()));
    }
    if !metadata.is_dir() {
        return Err(error("skillEntryUnsupported"));
    }
    Ok(io::fingerprint(Some(
        format!("copy:{}", artifacts::tree_hash(path)?).as_bytes(),
    )))
}
fn adopted_link_hash(link: &str) -> String {
    io::fingerprint(Some(format!("adopted-link:{link}").as_bytes()))
}
/// The skill's files behind an entry, through a link if it is one.
pub fn content_hash(path: &Path) -> Result<String, AppError> {
    let real = fs::canonicalize(path).map_err(|_| error("skillUnreadable"))?;
    artifacts::tree_hash(&real)
}
fn link_text(path: &Path) -> Result<Option<String>, AppError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| error("skillUnreadable"))?;
    if !metadata.file_type().is_symlink() {
        return Ok(None);
    }
    let link = fs::read_link(path).map_err(|_| error("skillUnreadable"))?;
    Ok(Some(link.to_string_lossy().into_owned()))
}
/// How the entry a receipt covers is checked: taken over, written by Ofox, or unowned.
pub fn owned_hash(path: &Path, owned: Option<&DirectoryOwned>) -> Result<String, AppError> {
    match owned {
        Some(owned) if owned.adopted.is_some() => adopted_hash(path),
        Some(owned) => entry_hash_with(path, Some(&owned.artifact_hash)),
        None => entry_hash_with(path, None),
    }
}
fn adopted_owned(receipt: Option<&Receipt>) -> Option<DirectoryOwned> {
    receipt
        .and_then(|receipt| serde_json::from_str::<DirectoryOwned>(&receipt.owned_segment).ok())
        .filter(|owned| owned.adopted.is_some())
}
/// The hash a target's entry has once it is done.
pub fn after_entry_hash(target: &TargetWrite) -> Result<String, AppError> {
    let path = Path::new(&target.path);
    if let Some(plan) = &target.directory {
        return entry_hash_with(path, Some(&plan.artifact_hash));
    }
    if adopted_owned(target.receipt.as_ref()).is_some() {
        return adopted_hash(path);
    }
    entry_hash_with(path, None)
}
pub fn validate_receipt(receipt: &Receipt) -> Result<(), AppError> {
    let owned: DirectoryOwned =
        serde_json::from_str(&receipt.owned_segment).map_err(|_| error("stateUnreadable"))?;
    artifacts::path(&owned.artifact_hash)?;
    let expected = match (&owned.adopted, owned.method.as_str()) {
        (Some(AdoptedEntry { link: Some(link) }), "symlink") => adopted_link_hash(link),
        (Some(AdoptedEntry { link: None }), "copy") => predicted(&owned.artifact_hash, "copy")?,
        (None, "copy" | "symlink") => predicted(&owned.artifact_hash, &owned.method)?,
        _ => return Err(error("stateUnreadable")),
    };
    if owned.entry_hash != expected {
        return Err(error("stateUnreadable"));
    }
    Ok(())
}
fn recovery_paths(target: &str, op_id: &str) -> Result<(PathBuf, PathBuf), AppError> {
    uuid::Uuid::parse_str(op_id).map_err(|_| error("stateUnreadable"))?;
    let parent = Path::new(target)
        .parent()
        .ok_or_else(|| error("stateUnreadable"))?;
    let slug = io::fingerprint(Some(target.as_bytes()));
    let recovery = parent.join(format!(".ofox-skill-{op_id}-{}", &slug[..12]));
    Ok((recovery.join("next"), recovery.join("previous")))
}
pub fn validate_target(target: &TargetWrite, op: &Operation) -> Result<(), AppError> {
    let valid_hash = |hash: &str| {
        hash == "missing" || (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    if let Some(summary) = &target.transfer {
        artifacts::path(&summary.artifact_hash)?;
        if !["copy", "symlink", "withdraw"].contains(&summary.method.as_str())
            || summary
                .planned_method
                .as_deref()
                .is_some_and(|method| !["copy", "symlink", "withdraw"].contains(&method))
            || target.after_hash != predicted(&summary.artifact_hash, &summary.method)?
            || target.directory.as_ref().is_some_and(|plan| {
                plan.artifact_hash != summary.artifact_hash
                    || plan.method != summary.method
                    || plan.fallback_copy != summary.fallback_copy
            })
        {
            return Err(error("stateUnreadable"));
        }
    }
    if !target.after_text.is_empty()
        || (!target.before_hash.is_empty() && !valid_hash(&target.before_hash))
        || (!target.after_hash.is_empty() && !valid_hash(&target.after_hash))
    {
        return Err(error("stateUnreadable"));
    }
    if let Some(plan) = &target.directory {
        let (stage, backup) = recovery_paths(&target.physical_target, &op.id)?;
        artifacts::path(&plan.artifact_hash)?;
        if !["copy", "symlink", "withdraw"].contains(&plan.method.as_str())
            || stage.to_string_lossy() != plan.stage_path
            || backup.to_string_lossy() != plan.backup_path
            || target.after_hash != predicted(&plan.artifact_hash, &plan.method)?
            || target.before_hash.is_empty()
            || (plan.method == "withdraw") != target.receipt.is_none()
        {
            return Err(error("stateUnreadable"));
        }
        if let Some(receipt) = &target.receipt {
            let owned: DirectoryOwned = serde_json::from_str(&receipt.owned_segment)
                .map_err(|_| error("stateUnreadable"))?;
            if owned.artifact_hash != plan.artifact_hash
                || owned.method != plan.method
                || owned.entry_hash != target.after_hash
            {
                return Err(error("stateUnreadable"));
            }
        }
    } else if adopted_owned(target.receipt.as_ref()).is_some() {
        // A take-over writes nothing: it only proves the entry and records it.
        if target.previous_receipt.is_some()
            || target.transfer.is_some()
            || !valid_hash(&target.before_hash)
            || !valid_hash(&target.after_hash)
            || target.before_hash == "missing"
        {
            return Err(error("stateUnreadable"));
        }
    } else if target.status == "pending"
        || (target.status == "applied"
            && (target.after_hash.is_empty() || target.before_hash.is_empty()))
    {
        return Err(error("stateUnreadable"));
    }
    if let Some(receipt) = &target.receipt {
        let owned: DirectoryOwned =
            serde_json::from_str(&receipt.owned_segment).map_err(|_| error("stateUnreadable"))?;
        if owned.entry_hash != target.after_hash {
            return Err(error("stateUnreadable"));
        }
    }
    for receipt in [&target.previous_receipt, &target.receipt]
        .into_iter()
        .flatten()
    {
        if receipt.kind != ResourceKind::Skills
            || receipt.physical_target != target.physical_target
            || format!("skills:{}", receipt.physical_target) != target.receipt_key
        {
            return Err(error("stateUnreadable"));
        }
        validate_receipt(receipt)?;
    }
    Ok(())
}
fn assignment(id: &str, tool: &str) -> String {
    format!("skills:{id}:{tool}")
}
fn selected_roots(tools: &[String]) -> Result<Vec<(String, PathBuf)>, AppError> {
    let paths = target_paths()?;
    tools
        .iter()
        .map(|tool| {
            Ok((
                tool.clone(),
                paths
                    .get(tool)
                    .ok_or_else(|| error("targetUnsupported"))?
                    .clone(),
            ))
        })
        .collect()
}
pub fn preview(db: &Database, id: &str, tools: &[String]) -> Result<Operation, AppError> {
    ensure_metadata(db)?;
    let selected = selected_roots(tools)?;
    preview_at(db, id, &selected, Some(resolver_paths()?))
}
/// Like `preview`, but where an entry of the same skill is already in one of
/// `adopt`'s agents, take it over as it is instead of refusing it.
pub fn preview_adoption(
    db: &Database,
    id: &str,
    tools: &[String],
    adopt: &[String],
) -> Result<Operation, AppError> {
    if adopt.is_empty() || adopt.iter().any(|tool| !tools.contains(tool)) {
        return Err(error("targetUnsupported"));
    }
    ensure_metadata(db)?;
    let selected = selected_roots(tools)?;
    preview_at_adopting(
        db,
        id,
        &selected,
        Some(resolver_paths()?),
        &adopt.iter().cloned().collect(),
    )
}
pub(crate) fn preview_at(
    db: &Database,
    id: &str,
    roots: &[(String, PathBuf)],
    resolvers: Option<BTreeMap<String, PathBuf>>,
) -> Result<Operation, AppError> {
    preview_at_adopting(db, id, roots, resolvers, &BTreeSet::new())
}
pub(crate) fn preview_at_adopting(
    db: &Database,
    id: &str,
    roots: &[(String, PathBuf)],
    resolvers: Option<BTreeMap<String, PathBuf>>,
    adopt: &BTreeSet<String>,
) -> Result<Operation, AppError> {
    let _guard = crate::config_io::lock();
    let before = db.library_snapshot()?;
    let meta = before
        .definitions
        .skills
        .get(id)
        .ok_or_else(|| error("resourceMissing"))?;
    let row = db
        .get_installed_skill(id)?
        .ok_or_else(|| error("resourceMissing"))?;
    validate_skill(&row)?;
    if spec_hash(&row)? != meta.hash {
        return Err(error("revisionChanged"));
    }
    let artifact = artifact_for(db, id)?;
    let hash = artifacts::tree_hash(&artifact)?;
    let selected: BTreeSet<_> = roots.iter().map(|(tool, _)| tool.clone()).collect();
    if selected.len() != roots.len() {
        return Err(error("targetDuplicate"));
    }
    let desired_updates: BTreeMap<_, _> = selected
        .iter()
        .map(|tool| (assignment(id, tool), Some(id.to_owned())))
        .chain(
            before
                .machine
                .assignments
                .iter()
                .filter(|(key, resource)| {
                    resource.as_str() == id
                        && key
                            .strip_prefix(&format!("skills:{id}:"))
                            .is_some_and(|tool| !selected.contains(tool))
                })
                .map(|(key, _)| (key.clone(), None)),
        )
        .collect();
    let previous_assignments = desired_updates
        .keys()
        .map(|key| (key.clone(), before.machine.assignments.get(key).cloned()))
        .collect();
    let mut groups: BTreeMap<String, (PathBuf, BTreeSet<String>)> = BTreeMap::new();
    for (tool, root) in roots {
        let path = root.join(&row.directory);
        let group = groups
            .entry(entry_id(&path)?)
            .or_insert((path, BTreeSet::new()));
        group.1.insert(tool.clone());
    }
    for receipt in before
        .machine
        .receipts
        .values()
        .filter(|receipt| receipt.kind == ResourceKind::Skills && receipt.resource_id == id)
    {
        groups
            .entry(receipt.physical_target.clone())
            .or_insert((PathBuf::from(&receipt.path), BTreeSet::new()));
    }
    let resolved_paths: BTreeMap<String, String> = resolvers
        .unwrap_or_default()
        .into_iter()
        .map(|(key, path)| (key, path.to_string_lossy().into_owned()))
        .collect();
    let resolved_physical = resolved_paths
        .iter()
        .map(|(key, path)| Ok((key.clone(), io::physical_id(Path::new(path))?)))
        .collect::<Result<_, AppError>>()?;
    let reader_roots = if resolved_paths.is_empty() {
        BTreeMap::new()
    } else {
        reader_paths()?
    };
    let op_id = uuid::Uuid::new_v4().to_string();
    let mut targets = Vec::new();
    for (physical, (path, chosen)) in groups {
        let key = format!("skills:{physical}");
        let old = before.machine.receipts.get(&key).cloned();
        let mut owners = chosen.clone();
        if let Some(receipt) = &old {
            if receipt.resource_id == id {
                owners.extend(
                    receipt
                        .owners
                        .iter()
                        .filter(|tool| !desired_updates.contains_key(&assignment(id, tool)))
                        .cloned(),
                );
            }
        }
        // A taken-over entry that is still wanted, unchanged and intact stays
        // exactly as it is: rewriting it would set the user's folder aside.
        if old.as_ref().is_some_and(|receipt| {
            receipt.resource_id == id
                && adopted_owned(Some(receipt)).is_some()
                && !owners.is_empty()
                && receipt.owners.iter().cloned().collect::<BTreeSet<_>>() == owners
                && receipt.resource_revision == meta.revision
                && receipt.definition_hash == meta.hash
                && receipt_intact(receipt)
        }) {
            continue;
        }
        let mut readers = owners
            .iter()
            .filter(|tool| tool.as_str() != "shared")
            .cloned()
            .collect::<BTreeSet<_>>();
        for (tool, roots) in &reader_roots {
            for root in roots {
                if reads_entry(tool, root, Path::new(&physical), &row.directory)? {
                    readers.insert(tool.clone());
                }
            }
        }
        let updates = desired_updates
            .iter()
            .filter(|(key, _)| {
                chosen.iter().any(|tool| assignment(id, tool) == **key)
                    || old.as_ref().is_some_and(|receipt| {
                        receipt
                            .owners
                            .iter()
                            .any(|tool| assignment(id, tool) == **key)
                    })
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let mut target = TargetWrite {
            directory: None,
            transfer: None,
            tools: owners.iter().cloned().collect(),
            readers: readers.into_iter().collect(),
            path: path.to_string_lossy().into_owned(),
            physical_target: physical.clone(),
            receipt_key: key,
            previous_receipt: old.clone(),
            assignment_updates: updates,
            source_fingerprints: BTreeMap::new(),
            before_hash: String::new(),
            after_hash: String::new(),
            after_text: String::new(),
            private_values: false,
            receipt: None,
            status: "pending".into(),
            reason: None,
        };
        let plan = (|| {
            let previous_owned = old
                .as_ref()
                .map(|receipt| {
                    serde_json::from_str::<DirectoryOwned>(&receipt.owned_segment)
                        .map_err(|_| error("stateUnreadable"))
                })
                .transpose()?;
            let current = owned_hash(&path, previous_owned.as_ref())?;
            target.before_hash = current.clone();
            if let Some(receipt) = &old {
                validate_receipt(receipt)?;
                let owned: DirectoryOwned = serde_json::from_str(&receipt.owned_segment)
                    .map_err(|_| error("stateUnreadable"))?;
                if receipt.resource_id != id {
                    return Err(error("sharedTargetConflict"));
                }
                if current != owned.entry_hash && !(current == "missing" && owners.is_empty()) {
                    return Err(error("skillEntryChanged"));
                }
            } else if current != "missing" {
                // Only the same skill, in an agent asked to take it over, is kept as it is.
                if !owners.iter().any(|tool| adopt.contains(tool)) || content_hash(&path)? != hash {
                    return Err(error("skillNameConflict"));
                }
                let link = link_text(&path)?;
                target.before_hash = entry_hash_with(&path, None)?;
                target.after_hash = adopted_hash(&path)?;
                target.reason = Some("library.adoptionPreview".into());
                target.receipt = Some(Receipt {
                    kind: ResourceKind::Skills,
                    resource_id: id.into(),
                    resource_revision: meta.revision,
                    definition_hash: meta.hash.clone(),
                    physical_target: physical,
                    path: target.path.clone(),
                    owners: owners.into_iter().collect(),
                    owned_segment: serde_json::to_string(&DirectoryOwned {
                        artifact_hash: hash.clone(),
                        method: if link.is_some() { "symlink" } else { "copy" }.into(),
                        entry_hash: target.after_hash.clone(),
                        adopted: Some(AdoptedEntry { link }),
                    })
                    .map_err(|_| error("invalidSkill"))?,
                });
                return Ok(());
            }
            let sync = crate::settings::get_skill_sync_method();
            let method = if owners.is_empty() {
                "withdraw"
            } else if sync == crate::services::skill::SyncMethod::Copy
                || (cfg!(windows) && sync == crate::services::skill::SyncMethod::Auto)
            {
                "copy"
            } else {
                "symlink"
            };
            let after = predicted(&hash, method)?;
            let (stage, backup) = recovery_paths(&physical, &op_id)?;
            target.after_hash = after.clone();
            target.transfer = Some(super::SkillTransfer {
                artifact_hash: hash.clone(),
                planned_method: Some(method.into()),
                method: method.into(),
                fallback_copy: sync == crate::services::skill::SyncMethod::Auto,
            });
            target.directory = Some(DirectoryPlan {
                artifact_hash: hash.clone(),
                method: method.into(),
                fallback_copy: sync == crate::services::skill::SyncMethod::Auto,
                stage_path: stage.to_string_lossy().into_owned(),
                backup_path: backup.to_string_lossy().into_owned(),
            });
            if !owners.is_empty() {
                target.receipt = Some(Receipt {
                    kind: ResourceKind::Skills,
                    resource_id: id.into(),
                    resource_revision: meta.revision,
                    definition_hash: meta.hash.clone(),
                    physical_target: physical,
                    path: target.path.clone(),
                    owners: owners.into_iter().collect(),
                    owned_segment: serde_json::to_string(&DirectoryOwned {
                        artifact_hash: hash.clone(),
                        method: method.into(),
                        entry_hash: after,
                        adopted: None,
                    })
                    .map_err(|_| error("invalidSkill"))?,
                });
            }
            Ok(())
        })();
        if let Err(failure) = plan {
            target.status = "conflict".into();
            target.reason = Some(failure.to_string());
            target.directory = None;
            target.transfer = None;
            target.after_hash.clear();
            target.receipt = None;
        }
        targets.push(target);
    }
    let op = Operation {
        kind: ResourceKind::Skills,
        id: op_id,
        epoch: before.machine.epoch,
        resource_id: id.into(),
        resource_revision: meta.revision,
        resource_hash: meta.hash.clone(),
        created_at: chrono::Utc::now().timestamp_millis(),
        status: "previewed".into(),
        targets,
        desired_updates,
        previous_assignments,
        resolved_paths,
        resolved_physical,
        routing: None,
    };
    let mut next = before.clone();
    next.machine.operations.insert(op.id.clone(), op.clone());
    io::prune_history(&mut next, &op.id);
    db.library_commit(&before, &next)?;
    Ok(op)
}
fn unlink(path: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| error("skillWriteFailed"))?;
    if metadata.file_type().is_symlink() {
        #[cfg(unix)]
        fs::remove_file(path).map_err(|_| error("skillWriteFailed"))?;
        #[cfg(windows)]
        fs::remove_dir(path).map_err(|_| error("skillWriteFailed"))?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|_| error("skillWriteFailed"))?;
    } else {
        return Err(error("skillEntryUnsupported"));
    }
    Ok(())
}
pub fn stage(target: &mut TargetWrite) -> Result<(), AppError> {
    stage_with(target, |source, path| {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(source, path)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(source, path)
        }
    })
}
fn stage_with(
    target: &mut TargetWrite,
    link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), AppError> {
    let old = target
        .previous_receipt
        .as_ref()
        .map(|receipt| {
            serde_json::from_str::<DirectoryOwned>(&receipt.owned_segment)
                .map_err(|_| error("stateUnreadable"))
        })
        .transpose()?;
    if entry_id(Path::new(&target.path))? != target.physical_target
        || owned_hash(Path::new(&target.path), old.as_ref())? != target.before_hash
    {
        return Err(error("targetChanged"));
    }
    let plan = target
        .directory
        .as_mut()
        .ok_or_else(|| error("stateUnreadable"))?;
    let artifact = artifacts::path(&plan.artifact_hash)?;
    if artifacts::tree_hash(&artifact)? != plan.artifact_hash {
        return Err(error("skillArtifactChanged"));
    }
    let path = Path::new(&plan.stage_path);
    let parent = path.parent().ok_or_else(|| error("stateUnreadable"))?;
    if fs::symlink_metadata(parent).is_ok() {
        if plan.method == "withdraw" && fs::symlink_metadata(&plan.backup_path).is_err() {
            return Ok(());
        }
        if entry_hash_with(path, Some(&plan.artifact_hash))? == target.after_hash {
            return Ok(());
        }
        return Err(error("operationNeedsReview"));
    }
    fs::create_dir_all(parent.parent().ok_or_else(|| error("stateUnreadable"))?)
        .map_err(|_| error("skillWriteFailed"))?;
    fs::create_dir(parent).map_err(|_| error("skillWriteFailed"))?;
    if plan.method == "withdraw" {
        return Ok(());
    }
    if plan.method == "symlink" {
        let result = link(&artifact, path);
        if result.is_err() {
            if !plan.fallback_copy {
                return Err(error("skillLinkFailed"));
            }
            plan.method = "copy".into();
            target
                .transfer
                .get_or_insert_with(|| super::SkillTransfer {
                    artifact_hash: plan.artifact_hash.clone(),
                    planned_method: None,
                    method: "copy".into(),
                    fallback_copy: plan.fallback_copy,
                })
                .method = "copy".into();
            target.after_hash = predicted(&plan.artifact_hash, "copy")?;
            if let Some(receipt) = &mut target.receipt {
                receipt.owned_segment = serde_json::to_string(&DirectoryOwned {
                    artifact_hash: plan.artifact_hash.clone(),
                    method: "copy".into(),
                    entry_hash: target.after_hash.clone(),
                    adopted: None,
                })
                .map_err(|_| error("stateUnreadable"))?;
            }
            artifacts::copy_verified(&artifact, path)?;
        }
    } else {
        artifacts::copy_verified(&artifact, path)?;
    }
    if entry_hash_with(path, Some(&plan.artifact_hash))? != target.after_hash {
        return Err(error("skillChanged"));
    }
    Ok(())
}
/// Taking over writes nothing: the entry must still be exactly what was previewed.
fn verify_adoption(target: &TargetWrite) -> Result<(), AppError> {
    let owned = adopted_owned(target.receipt.as_ref()).ok_or_else(|| error("stateUnreadable"))?;
    let path = Path::new(&target.path);
    if target.previous_receipt.is_some()
        || entry_id(path)? != target.physical_target
        || entry_hash_with(path, None)? != target.before_hash
        || adopted_hash(path)? != target.after_hash
        || content_hash(path)? != owned.artifact_hash
    {
        return Err(error("targetChanged"));
    }
    Ok(())
}
pub fn write(target: &TargetWrite) -> Result<(), AppError> {
    let Some(plan) = target.directory.as_ref() else {
        return verify_adoption(target);
    };
    let path = Path::new(&target.path);
    let previous = target
        .previous_receipt
        .as_ref()
        .map(|receipt| {
            serde_json::from_str::<DirectoryOwned>(&receipt.owned_segment)
                .map_err(|_| error("stateUnreadable"))
        })
        .transpose()?;
    if entry_id(path)? != target.physical_target
        || owned_hash(path, previous.as_ref())? != target.before_hash
    {
        return Err(error("targetChanged"));
    }
    if fs::symlink_metadata(&plan.backup_path).is_ok() {
        return Err(error("operationNeedsReview"));
    }
    if target.before_hash != "missing" {
        fs::rename(path, &plan.backup_path).map_err(|_| error("skillWriteFailed"))?;
    }
    if plan.method != "withdraw" {
        if let Err(_failure) = fs::rename(&plan.stage_path, path) {
            if target.before_hash != "missing" && fs::symlink_metadata(path).is_err() {
                let _ = fs::rename(&plan.backup_path, path);
            }
            return Err(error("skillWriteFailed"));
        }
    }
    if entry_hash_with(path, Some(&plan.artifact_hash))? != target.after_hash {
        return Err(error("verifyFailed"));
    }
    Ok(())
}
pub fn cleanup(target: &TargetWrite) -> Result<(), AppError> {
    let Some(plan) = &target.directory else {
        return Ok(());
    };
    let backup = Path::new(&plan.backup_path);
    if fs::symlink_metadata(backup).is_ok() {
        let previous = target
            .previous_receipt
            .as_ref()
            .ok_or_else(|| error("operationNeedsReview"))?;
        let owned: DirectoryOwned =
            serde_json::from_str(&previous.owned_segment).map_err(|_| error("stateUnreadable"))?;
        if owned_hash(backup, Some(&owned))? != target.before_hash {
            return Err(error("operationNeedsReview"));
        }
        let user_folder = owned.adopted.is_some()
            && !fs::symlink_metadata(backup)
                .map_err(|_| error("operationNeedsReview"))?
                .file_type()
                .is_symlink();
        if user_folder {
            // The user's own folder: set it aside where it can be restored, never deleted.
            let leaf = Path::new(&target.path)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| error("stateUnreadable"))?;
            let origin = if plan.method == "withdraw" {
                "libraryWithdraw"
            } else {
                "libraryUpdate"
            };
            crate::services::skill::SkillService::archive_native_entry(backup, leaf, origin)
                .map_err(|_| error("operationNeedsReview"))?;
        } else {
            unlink(backup)?;
        }
    }
    let stage = Path::new(&plan.stage_path);
    if fs::symlink_metadata(stage).is_ok() {
        return Err(error("operationNeedsReview"));
    }
    if let Some(parent) = stage.parent() {
        if parent.is_dir() {
            fs::remove_dir(parent).map_err(|_| error("operationNeedsReview"))?;
        }
    }
    Ok(())
}
/// Explicit cancellation may discard a proven new stage, never the old backup
/// or native entry. A backup may be the only intact pre-operation entry.
pub fn discard_staged(target: &mut TargetWrite) -> Result<(), AppError> {
    let Some(plan) = target.directory.as_ref() else {
        return Ok(());
    };
    if entry_id(Path::new(&target.path))? != target.physical_target {
        return Err(error("cleanupNeedsReview"));
    }
    match fs::symlink_metadata(&plan.backup_path) {
        Ok(_) => return Err(error("cleanupNeedsReview")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(error("cleanupNeedsReview")),
    }
    let stage = Path::new(&plan.stage_path);
    let parent = stage.parent().ok_or_else(|| error("stateUnreadable"))?;
    match fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err(error("cleanupNeedsReview")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            target.directory = None;
            return Ok(());
        }
        Err(_) => return Err(error("cleanupNeedsReview")),
    }
    match fs::symlink_metadata(stage) {
        Ok(_) => {
            if entry_hash_with(stage, Some(&plan.artifact_hash))? != target.after_hash {
                return Err(error("cleanupNeedsReview"));
            }
            unlink(stage)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(error("cleanupNeedsReview")),
    }
    fs::remove_dir(parent).map_err(|_| error("cleanupNeedsReview"))?;
    target.directory = None;
    Ok(())
}
pub fn receipt_intact(receipt: &Receipt) -> bool {
    let path = Path::new(&receipt.path);
    validate_receipt(receipt).is_ok()
        && entry_id(path).ok().as_deref() == Some(receipt.physical_target.as_str())
        && serde_json::from_str::<DirectoryOwned>(&receipt.owned_segment)
            .ok()
            .is_some_and(|owned| {
                owned_hash(path, Some(&owned)).ok().as_ref() == Some(&owned.entry_hash)
                    // A taken-over link is checked by its text; its files must match too.
                    && (owned.adopted.is_none()
                        || content_hash(path).ok().as_ref() == Some(&owned.artifact_hash))
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn skill_draft_epoch_guard_rejects_an_old_download_before_publication() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = tempfile::tempdir().unwrap();
        fs::write(
            source.path().join("SKILL.md"),
            "---\nname: epoch-guard\ndescription: Sandbox\n---\nSynthetic fixture",
        )
        .unwrap();
        let hash = artifacts::tree_hash(source.path()).unwrap();
        let artifact = artifacts::path(&hash).unwrap();
        assert!(!artifact.exists());
        let db = Database::memory().unwrap();
        let before = db.library_snapshot().unwrap();
        let mut restored = before.clone();
        restored.machine.epoch += 1;
        let restored = db.library_commit(&before, &restored).unwrap();
        let writes_before: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get(0))
            .unwrap();
        let row = InstalledSkill {
            id: "epoch:fixture".into(),
            name: "Epoch fixture".into(),
            description: None,
            directory: "epoch-guard".into(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps::default(),
            installed_at: 0,
            updated_at: 0,
            content_hash: None,
        };
        let result = save_draft_with_execution_at_epoch(
            &db,
            row,
            source.path(),
            None,
            None,
            before.machine.epoch,
        );
        assert!(
            result.is_err(),
            "a restore epoch must invalidate the original download intent"
        );
        assert_eq!(db.library_snapshot().unwrap(), restored);
        assert!(db.get_all_installed_skills().unwrap().is_empty());
        assert!(
            !artifact.exists(),
            "an already-stale intent must not publish its artifact"
        );
        let writes_after: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            writes_after, writes_before,
            "a stale draft intent cannot write metadata"
        );
    }

    #[test]
    #[serial_test::serial]
    fn skill_draft_epoch_guard_same_epoch_creates_only_an_unassigned_draft() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = tempfile::tempdir().unwrap();
        fs::write(
            source.path().join("SKILL.md"),
            "---\nname: epoch-guard\ndescription: Sandbox\n---\nSynthetic fixture",
        )
        .unwrap();
        let db = Database::memory().unwrap();
        let before = db.library_snapshot().unwrap();
        let row = InstalledSkill {
            id: "epoch:fixture".into(),
            name: "Epoch fixture".into(),
            description: None,
            directory: "epoch-guard".into(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps {
                claude: true,
                ..Default::default()
            },
            installed_at: 0,
            updated_at: 0,
            content_hash: None,
        };
        let saved = save_draft_with_execution_at_epoch(
            &db,
            row,
            source.path(),
            None,
            None,
            before.machine.epoch,
        )
        .unwrap();
        assert_eq!(saved.apps, SkillApps::default());
        let after = db.library_snapshot().unwrap();
        assert!(after.machine.assignments.is_empty());
        assert!(after.machine.receipts.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn cancelling_discards_only_a_verified_stage_and_preserves_native_and_backup_entries() {
        let _sandbox = crate::test_sandbox::sandbox();
        copy_mode();
        let db = Database::memory().unwrap();
        let row = draft(&db, "Skill body");
        let mut first = preview(&db, &row.id, &["codex".into()]).unwrap();
        stage(&mut first.targets[0]).unwrap();
        let plan = first.targets[0].directory.clone().unwrap();
        assert!(Path::new(&plan.stage_path).exists());
        let cancelled = io::cancel_operation(&db, &first.id).unwrap();
        assert_eq!(cancelled.status, "cancelled");
        assert!(!Path::new(&plan.stage_path).exists());
        assert!(!Path::new(&first.targets[0].path).exists());
        assert!(artifact_for(&db, &row.id)
            .unwrap()
            .join("SKILL.md")
            .exists());
        let mut changed = preview(&db, &row.id, &["codex".into()]).unwrap();
        stage(&mut changed.targets[0]).unwrap();
        let plan = changed.targets[0].directory.clone().unwrap();
        fs::write(
            Path::new(&plan.stage_path).join("SKILL.md"),
            "User-edited stage",
        )
        .unwrap();
        assert!(io::cancel_operation(&db, &changed.id).is_err());
        assert_eq!(
            fs::read_to_string(Path::new(&plan.stage_path).join("SKILL.md")).unwrap(),
            "User-edited stage"
        );
        let mut backup = preview(&db, &row.id, &["codex".into()]).unwrap();
        stage(&mut backup.targets[0]).unwrap();
        let plan = backup.targets[0].directory.clone().unwrap();
        fs::create_dir(&plan.backup_path).unwrap();
        fs::write(
            Path::new(&plan.backup_path).join("untouched"),
            "Previous data",
        )
        .unwrap();
        assert!(io::cancel_operation(&db, &backup.id).is_err());
        assert!(Path::new(&plan.stage_path).exists());
        assert_eq!(
            fs::read_to_string(Path::new(&plan.backup_path).join("untouched")).unwrap(),
            "Previous data"
        );
    }

    #[test]
    #[cfg(unix)]
    #[serial_test::serial]
    fn a_receipt_stops_proving_ownership_when_its_parent_alias_moves() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let row = draft(&db, "Old");
        let roots = tempfile::tempdir().unwrap();
        let real = roots.path().join("real");
        let other = roots.path().join("other");
        let alias = roots.path().join("alias");
        fs::create_dir(&real).unwrap();
        fs::create_dir(&other).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let op = preview_at(&db, &row.id, &[("codex".into(), alias.clone())], None).unwrap();
        io::apply_operation(&db, &op.id).unwrap();
        let receipt = db
            .library_snapshot()
            .unwrap()
            .machine
            .receipts
            .into_values()
            .next()
            .unwrap();
        assert!(receipt_intact(&receipt));
        std::os::unix::fs::symlink(
            artifacts::path(row.content_hash.as_deref().unwrap()).unwrap(),
            other.join("fixture"),
        )
        .unwrap();
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(other, &alias).unwrap();
        assert!(!receipt_intact(&receipt));
    }

    #[test]
    #[serial_test::serial]
    fn damaged_directory_receipts_are_rejected_before_staging() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let skill = draft(&db, "draft");
        let native = tempfile::tempdir().unwrap();
        let op = preview_at(
            &db,
            &skill.id,
            &[("codex".into(), native.path().join("skills"))],
            None,
        )
        .unwrap();
        let before = db.library_snapshot().unwrap();
        for case in 0..7 {
            let mut next = before.clone();
            let target = &mut next.machine.operations.get_mut(&op.id).unwrap().targets[0];
            let receipt = target.receipt.as_mut().unwrap();
            match case {
                0 => receipt.owners.clear(),
                1 => receipt.resource_revision = 0,
                2 => receipt.path = native.path().join("other").to_string_lossy().into_owned(),
                3 => receipt.definition_hash = "0".repeat(64),
                4 => receipt.resource_id = "other".into(),
                5 => {
                    let mut owned: DirectoryOwned =
                        serde_json::from_str(&receipt.owned_segment).unwrap();
                    owned.artifact_hash = "0".repeat(64);
                    owned.entry_hash = predicted(&owned.artifact_hash, &owned.method).unwrap();
                    receipt.owned_segment = serde_json::to_string(&owned).unwrap();
                }
                _ => target.before_hash = "invalid".into(),
            }
            assert!(db.library_commit(&before, &next).is_err(), "case {case}");
            assert_eq!(db.library_snapshot().unwrap(), before);
            assert!(!Path::new(&op.targets[0].path).exists());
            assert!(!Path::new(&op.targets[0].directory.as_ref().unwrap().stage_path).exists());
        }
    }

    #[test]
    #[serial_test::serial]
    fn opencode_skill_path_references_require_explicit_supported_resolution() {
        let _sandbox = crate::test_sandbox::sandbox();
        crate::config::atomic_write(
            &crate::opencode_config::get_opencode_config_path(),
            br#"{"skills":{"paths":["/tmp/{env:USER}/skills"]}}"#,
        )
        .unwrap();
        assert!(reader_paths().is_err());
        crate::config::atomic_write(
            &crate::opencode_config::get_opencode_config_path(),
            br#"{"skills":{"{env:FIELD}":["/tmp/skills"]}}"#,
        )
        .unwrap();
        assert!(reader_paths().is_err());
    }

    #[test]
    #[serial_test::serial]
    fn hermes_bare_home_and_trimmed_paths_are_readers() {
        let sandbox = crate::test_sandbox::sandbox();
        crate::config::atomic_write(
            &crate::hermes_config::get_hermes_config_path(),
            b"skills:\n  external_dirs: ['  ~  ', ' ', ' ~/shared-skills  ']\n",
        )
        .unwrap();
        let readers = reader_paths().unwrap();
        assert!(readers["hermes"].contains(&sandbox.home().to_path_buf()));
        assert!(readers["hermes"].contains(&sandbox.home().join("shared-skills")));
        assert!(!readers["hermes"].contains(&crate::hermes_config::get_hermes_dir().join(" ")));
    }

    #[test]
    #[serial_test::serial]
    fn native_skill_roots_follow_cli_home_xdg_and_all_global_readers() {
        let sandbox = crate::test_sandbox::sandbox();
        let gemini = sandbox.home().join("gemini-home");
        let xdg = sandbox.home().join("xdg");
        let extra = sandbox.home().join("opencode-extra");
        let custom = sandbox.home().join("recursive-reader");
        std::env::set_var("GEMINI_CLI_HOME", &gemini);
        std::env::set_var("XDG_CONFIG_HOME", &xdg);
        std::env::set_var("OPENCODE_CONFIG_DIR", &extra);
        let config = sandbox.home().join("opencode-env.jsonc");
        crate::config::atomic_write(
            &config,
            serde_json::to_vec(&serde_json::json!({"skills":{"paths":[custom]}}))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        std::env::set_var("OPENCODE_CONFIG", &config);
        let native = native_paths().unwrap();
        assert_eq!(native["gemini"], gemini.join(".gemini/skills"));
        assert_eq!(native["opencode"], xdg.join("opencode/skills"));
        let readers = reader_paths().unwrap();
        assert!(readers["gemini"].contains(&gemini.join(".agents/skills")));
        assert!(readers["opencode"].contains(&extra.join("skill")));
        assert!(readers["opencode"].contains(&sandbox.home().join(".opencode/skill")));
        assert!(readers["opencode"].contains(&custom));
    }

    #[test]
    #[serial_test::serial]
    fn shared_skills_disclose_copilot_without_offering_a_native_write_target() {
        let sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let skill = draft(&db, "shared reader fixture");
        let review = preview(&db, &skill.id, &["shared".into()]).unwrap();
        assert!(review
            .targets
            .iter()
            .any(|target| target.readers.contains(&"copilot".into())));
        assert!(!native_paths().unwrap().contains_key("copilot"));
        assert!(!target_paths().unwrap().contains_key("copilot"));
        assert!(preview(&db, &skill.id, &["copilot".into()]).is_err());
        assert_eq!(
            reader_paths().unwrap()["copilot"],
            vec![sandbox.home().join(".agents/skills")]
        );
        io::apply_operation(&db, &review.id).unwrap();
        let state = db.library_snapshot().unwrap();
        assert!(!state
            .machine
            .assignments
            .contains_key(&assignment(&skill.id, "copilot")));
        assert!(!sandbox.home().join(".copilot").exists());
    }

    #[test]
    #[serial_test::serial]
    fn recursive_readers_are_disclosed_and_new_roots_invalidate_preview() {
        let sandbox = crate::test_sandbox::sandbox();
        std::env::set_var("OPENCODE_DISABLE_EXTERNAL_SKILLS", "true");
        let db = Database::memory().unwrap();
        let old = draft(&db, "old");
        let claude = crate::config::get_claude_config_dir();
        let config = crate::opencode_config::get_opencode_config_path();
        crate::config::atomic_write(
            &config,
            &serde_json::to_vec(&serde_json::json!({"skills":{"paths":[claude]}})).unwrap(),
        )
        .unwrap();
        let visible = preview(&db, &old.id, &["claude".into()]).unwrap();
        assert!(visible
            .targets
            .iter()
            .any(|target| target.readers.contains(&"opencode".into())));
        crate::config::atomic_write(&config, b"{}").unwrap();
        let stale = preview(&db, &old.id, &["codex".into()]).unwrap();
        let extra = sandbox.home().join("z-reader");
        crate::config::atomic_write(
            &config,
            &serde_json::to_vec(&serde_json::json!({"skills":{"paths":[extra]}})).unwrap(),
        )
        .unwrap();
        assert!(io::apply_operation(&db, &stale.id).is_err());
        assert!(!crate::codex_config::get_codex_config_dir()
            .join("skills/fixture")
            .exists());
    }

    #[test]
    #[serial_test::serial]
    fn a_reader_appended_after_preview_also_requires_reconfirmation() {
        let sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let old = draft(&db, "old");
        let config = crate::opencode_config::get_opencode_config_path();
        crate::config::atomic_write(&config, b"{}").unwrap();
        let stale = preview(&db, &old.id, &["codex".into()]).unwrap();
        let extra = sandbox.home().join("z-reader");
        crate::config::atomic_write(
            &config,
            &serde_json::to_vec(&serde_json::json!({"skills":{"paths":[extra]}})).unwrap(),
        )
        .unwrap();
        assert!(io::apply_operation(&db, &stale.id).is_err());
        assert!(db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn hermes_global_reader_paths_expand_home_and_home_relative_strings() {
        let sandbox = crate::test_sandbox::sandbox();
        let config = crate::hermes_config::get_hermes_config_path();
        crate::config::atomic_write(
            &config,
            b"skills:\n  create_dir: generated\n  external_dirs: '~/shared-skills'\n",
        )
        .unwrap();
        let readers = reader_paths().unwrap();
        assert!(
            readers["hermes"].contains(&crate::hermes_config::get_hermes_dir().join("generated"))
        );
        assert!(readers["hermes"].contains(&sandbox.home().join("shared-skills")));
    }
    fn source(body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("SKILL.md"),
            format!("---\nname: fixture\ndescription: Fixture\n---\n{body}"),
        )
        .unwrap();
        fs::write(dir.path().join(".hidden"), body).unwrap();
        dir
    }
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
            apps: SkillApps::default(),
            installed_at: 0,
            content_hash: None,
            updated_at: 0,
        }
    }
    fn draft(db: &Database, body: &str) -> InstalledSkill {
        let source = source(body);
        save_draft(db, row(), source.path(), None).unwrap()
    }
    fn copy_mode() {
        let mut settings = crate::settings::get_settings();
        settings.skill_sync_method = crate::services::skill::SyncMethod::Copy;
        crate::settings::update_settings(settings).unwrap();
    }
    #[test]
    #[serial_test::serial]
    fn skills_reach_every_registered_agent_and_withdraw_cleanly() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let skill = draft(&db, "body");
        for tool in ["openclaw", "pi", "dsh", "workbuddy", "cursor", "codebuddy"] {
            let entry = super::super::skill_targets::root(tool)
                .unwrap()
                .join(&skill.directory);
            let op = preview(&db, &skill.id, &[tool.to_string()]).unwrap();
            assert_eq!(
                io::apply_operation(&db, &op.id).unwrap().status,
                "applied",
                "{tool}"
            );
            assert!(entry.join("SKILL.md").exists(), "{tool}");
            let op = preview(&db, &skill.id, &[]).unwrap();
            assert_eq!(
                io::apply_operation(&db, &op.id).unwrap().status,
                "applied",
                "{tool}"
            );
            assert!(!entry.exists(), "{tool}");
        }
    }
    #[test]
    #[serial_test::serial]
    fn dsh_reads_the_shared_folder_and_pi_only_its_own() {
        let _sandbox = crate::test_sandbox::sandbox();
        let shared = crate::config::get_home_dir().join(".agents/skills");
        let readers = reader_paths().unwrap();
        assert!(readers["dsh"].contains(&shared));
        assert!(!readers["pi"].contains(&shared));
        assert_eq!(
            readers["cursor"],
            vec![super::super::skill_targets::root("cursor").unwrap()]
        );
    }
    /// The fixture skill's files, written by hand with the user's own modes.
    #[cfg(unix)]
    fn user_folder(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: fixture\ndescription: Fixture\n---\n{body}"),
        )
        .unwrap();
        fs::write(path.join(".hidden"), body).unwrap();
        for file in ["SKILL.md", ".hidden"] {
            fs::set_permissions(path.join(file), fs::Permissions::from_mode(0o600)).unwrap();
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn skill_target(db: &Database, skill: &InstalledSkill, tool: &str) -> serde_json::Value {
        let view = super::super::skill_view::definition_view(db, skill).unwrap();
        view["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["toolId"] == tool)
            .unwrap()
            .clone()
    }
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn an_identical_folder_already_in_an_agent_is_taken_over_as_it_is() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let skill = draft(&db, "body");
        let native = super::super::skill_targets::root("claude")
            .unwrap()
            .join("fixture");
        user_folder(&native, "body");
        let inode = fs::metadata(native.join("SKILL.md")).unwrap().ino();
        let plain = preview(&db, &skill.id, &["claude".into()]).unwrap();
        assert_eq!(plain.targets[0].status, "conflict");
        let op = preview_adoption(&db, &skill.id, &["claude".into()], &["claude".into()]).unwrap();
        assert_eq!(op.targets[0].status, "pending");
        assert_eq!(
            op.targets[0].reason.as_deref(),
            Some("library.adoptionPreview")
        );
        assert_eq!(io::apply_operation(&db, &op.id).unwrap().status, "applied");
        assert_eq!(fs::metadata(native.join("SKILL.md")).unwrap().ino(), inode);
        assert_eq!(
            fs::metadata(native.join("SKILL.md"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(skill_target(&db, &skill, "claude")["status"], "applied");
    }
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn a_different_folder_with_the_same_name_is_never_taken_over() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let skill = draft(&db, "library");
        let native = super::super::skill_targets::root("codex")
            .unwrap()
            .join("fixture");
        user_folder(&native, "the user's own");
        let op = preview_adoption(&db, &skill.id, &["codex".into()], &["codex".into()]).unwrap();
        assert_eq!(op.targets[0].status, "conflict");
        assert!(op.targets[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("skillNameConflict"));
        assert_eq!(
            fs::read_to_string(native.join(".hidden")).unwrap(),
            "the user's own"
        );
    }
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn a_link_is_let_go_alone_and_a_taken_over_folder_is_archived_not_deleted() {
        let sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let skill = draft(&db, "body");
        let shared = target_paths().unwrap()["shared"].join("fixture");
        user_folder(&shared, "body");
        let claude = super::super::skill_targets::root("claude").unwrap();
        fs::create_dir_all(&claude).unwrap();
        // ~/.claude/skills/fixture -> ../../.agents/skills/fixture, as these links are made
        std::os::unix::fs::symlink("../../.agents/skills/fixture", claude.join("fixture")).unwrap();
        let both = vec!["claude".to_string(), "shared".to_string()];
        let op = preview_adoption(&db, &skill.id, &both, &both).unwrap();
        assert!(op.targets.iter().all(|target| target.status == "pending"));
        assert_eq!(io::apply_operation(&db, &op.id).unwrap().status, "applied");
        assert_eq!(skill_target(&db, &skill, "claude")["status"], "applied");
        assert_eq!(skill_target(&db, &skill, "shared")["status"], "applied");
        assert_eq!(skill_target(&db, &skill, "codex")["status"], "shared");
        let op = preview(&db, &skill.id, &["shared".into()]).unwrap();
        assert_eq!(io::apply_operation(&db, &op.id).unwrap().status, "applied");
        assert!(fs::symlink_metadata(claude.join("fixture")).is_err());
        assert!(shared.join("SKILL.md").exists());
        let op = preview(&db, &skill.id, &[]).unwrap();
        assert_eq!(io::apply_operation(&db, &op.id).unwrap().status, "applied");
        assert!(fs::symlink_metadata(&shared).is_err());
        let backups = crate::services::skill::SkillService::list_backups().unwrap();
        let archived = backups
            .iter()
            .find(|entry| entry.skill.directory == "fixture")
            .unwrap();
        assert_eq!(
            fs::read_to_string(Path::new(&archived.backup_path).join("skill/.hidden")).unwrap(),
            "body"
        );
        assert!(Path::new(&archived.backup_path).starts_with(sandbox.home()));
    }
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn an_identical_unowned_entry_is_offered_for_take_over() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let skill = draft(&db, "body");
        let codex = super::super::skill_targets::root("codex")
            .unwrap()
            .join("fixture");
        user_folder(&codex, "body");
        let gemini = super::super::skill_targets::root("gemini")
            .unwrap()
            .join("fixture");
        user_folder(&gemini, "different");
        let target = skill_target(&db, &skill, "codex");
        assert_eq!(target["adoptable"], true);
        assert_eq!(target["entryKind"], "directory");
        assert_eq!(skill_target(&db, &skill, "gemini")["adoptable"], false);
        assert_eq!(
            skill_target(&db, &skill, "claude")["entryKind"],
            serde_json::Value::Null
        );
        let op = preview_adoption(&db, &skill.id, &["codex".into()], &["codex".into()]).unwrap();
        io::apply_operation(&db, &op.id).unwrap();
        let target = skill_target(&db, &skill, "codex");
        assert_eq!(target["adopted"], true);
        assert_eq!(target["adoptable"], false);
    }
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn giving_a_skill_to_another_agent_leaves_taken_over_entries_as_they_are() {
        use std::os::unix::fs::MetadataExt;
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let skill = draft(&db, "body");
        let claude = super::super::skill_targets::root("claude")
            .unwrap()
            .join("fixture");
        user_folder(&claude, "body");
        let inode = fs::metadata(claude.join("SKILL.md")).unwrap().ino();
        let op = preview_adoption(&db, &skill.id, &["claude".into()], &["claude".into()]).unwrap();
        io::apply_operation(&db, &op.id).unwrap();
        let op = preview(&db, &skill.id, &["claude".into(), "pi".into()]).unwrap();
        assert_eq!(io::apply_operation(&db, &op.id).unwrap().status, "applied");
        assert!(!fs::symlink_metadata(&claude)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::metadata(claude.join("SKILL.md")).unwrap().ino(), inode);
        assert_eq!(skill_target(&db, &skill, "claude")["adopted"], true);
        assert_eq!(skill_target(&db, &skill, "claude")["status"], "applied");
        assert_eq!(skill_target(&db, &skill, "pi")["status"], "applied");
    }
    #[test]
    fn receipts_written_before_take_over_existed_still_read() {
        let old = r#"{"artifactHash":"a","method":"copy","entryHash":"b"}"#;
        let owned: DirectoryOwned = serde_json::from_str(old).unwrap();
        assert!(owned.adopted.is_none());
        assert_eq!(serde_json::to_string(&owned).unwrap(), old);
    }
    #[test]
    #[serial_test::serial]
    fn imported_row_paths_cannot_escape_the_native_root() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let mut skill = draft(&db, "old");
        skill.directory = "../escape/fixture".into();
        db.save_skill(&skill).unwrap();
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        next.definitions.skills.get_mut(&skill.id).unwrap().hash = spec_hash(&skill).unwrap();
        db.library_commit(&before, &next).unwrap();
        let roots = tempfile::tempdir().unwrap();
        fs::create_dir(roots.path().join("native")).unwrap();
        fs::create_dir(roots.path().join("escape")).unwrap();
        assert!(preview_at(
            &db,
            &skill.id,
            &[("codex".into(), roots.path().join("native"))],
            None
        )
        .is_err());
    }

    #[test]
    #[cfg(unix)]
    #[serial_test::serial]
    fn a_private_artifact_root_cannot_alias_native_skills() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = source("draft");
        let native = crate::services::skill::SkillService::get_app_skills_dir(
            &crate::app_config::AppType::Claude,
        )
        .unwrap();
        fs::create_dir_all(&native).unwrap();
        let private = artifacts::root();
        fs::create_dir_all(private.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&native, &private).unwrap();
        let db = Database::memory().unwrap();
        assert!(save_draft(&db, row(), source.path(), None).is_err());
        assert_eq!(fs::read_dir(native).unwrap().count(), 0);
        assert!(db.get_all_installed_skills().unwrap().is_empty());
    }
    #[test]
    #[serial_test::serial]
    fn saving_an_update_refuses_a_modified_row_or_old_artifact() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let original = draft(&db, "old");
        let new_source = source("new");
        let mut changed = original.clone();
        changed.name = "External change".into();
        db.save_skill(&changed).unwrap();
        let before = db.library_snapshot().unwrap();
        assert!(save_draft(&db, original.clone(), new_source.path(), Some(1)).is_err());
        assert_eq!(db.library_snapshot().unwrap(), before);
        db.save_skill(&original).unwrap();
        let artifact = artifacts::path(original.content_hash.as_deref().unwrap()).unwrap();
        fs::write(artifact.join(".hidden"), "user edit").unwrap();
        assert!(save_draft(&db, original, new_source.path(), Some(1)).is_err());
        assert_eq!(db.library_snapshot().unwrap(), before);
    }
    #[test]
    #[serial_test::serial]
    fn a_failed_link_and_copy_fallback_keeps_the_journal_loadable() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        draft(&db, "old");
        let roots = tempfile::tempdir().unwrap();
        let mut op = preview_at(
            &db,
            "local:fixture",
            &[("codex".into(), roots.path().join("native"))],
            None,
        )
        .unwrap();
        let target = &mut op.targets[0];
        target.directory.as_mut().unwrap().method = "symlink".into();
        target.directory.as_mut().unwrap().fallback_copy = true;
        target.after_hash =
            predicted(&target.directory.as_ref().unwrap().artifact_hash, "symlink").unwrap();
        let owned = DirectoryOwned {
            artifact_hash: target.directory.as_ref().unwrap().artifact_hash.clone(),
            method: "symlink".into(),
            entry_hash: target.after_hash.clone(),
            adopted: None,
        };
        target.receipt.as_mut().unwrap().owned_segment = serde_json::to_string(&owned).unwrap();
        let failure = stage_with(target, |_, dest| {
            fs::write(dest, "block copy").unwrap();
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "test link refused",
            ))
        });
        assert!(failure.is_err());
        target.status = "failed".into();
        op.status = "partial".into();
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        next.machine.operations.insert(op.id.clone(), op.clone());
        db.library_commit(&before, &next).unwrap();
        assert!(db.library_snapshot().is_ok());
        assert!(!Path::new(&op.targets[0].path).exists());
    }
    #[test]
    #[serial_test::serial]
    fn immutable_updates_keep_old_native_contents_until_apply() {
        let _sandbox = crate::test_sandbox::sandbox();
        copy_mode();
        let db = Database::memory().unwrap();
        let old = draft(&db, "old");
        let roots = tempfile::tempdir().unwrap();
        let native = roots.path().join("native");
        let first = preview_at(&db, &old.id, &[("codex".into(), native.clone())], None).unwrap();
        assert_eq!(
            super::super::operation_view(&first)["targets"][0]["method"],
            "copy"
        );
        let applied = io::apply_operation(&db, &first.id).unwrap();
        assert_eq!(
            super::super::operation_view(&applied)["targets"][0]["method"],
            "copy"
        );
        assert_eq!(
            io::apply_operation(&db, &first.id).unwrap().status,
            "applied"
        );
        let new_source = source("new");
        save_draft(&db, old.clone(), new_source.path(), Some(1)).unwrap();
        assert_eq!(
            fs::read_to_string(native.join("fixture/.hidden")).unwrap(),
            "old"
        );
        let next = preview_at(&db, &old.id, &[("codex".into(), native.clone())], None).unwrap();
        assert_eq!(
            io::apply_operation(&db, &next.id).unwrap().status,
            "applied"
        );
        assert_eq!(
            fs::read_to_string(native.join("fixture/.hidden")).unwrap(),
            "new"
        );
        let withdraw = preview_at(&db, &old.id, &[], None).unwrap();
        assert_eq!(
            io::apply_operation(&db, &withdraw.id).unwrap().status,
            "applied"
        );
        assert!(!native.join("fixture").exists());
        assert!(artifacts::path(old.content_hash.as_deref().unwrap())
            .unwrap()
            .exists());
    }
    #[test]
    #[serial_test::serial]
    fn user_changes_to_owned_copies_block_withdrawal() {
        let _sandbox = crate::test_sandbox::sandbox();
        copy_mode();
        let db = Database::memory().unwrap();
        draft(&db, "old");
        let roots = tempfile::tempdir().unwrap();
        let native = roots.path().join("native");
        let op = preview_at(
            &db,
            "local:fixture",
            &[("gemini".into(), native.clone())],
            None,
        )
        .unwrap();
        io::apply_operation(&db, &op.id).unwrap();
        fs::write(native.join("fixture/.hidden"), "user").unwrap();
        let withdraw = preview_at(&db, "local:fixture", &[], None).unwrap();
        assert_eq!(withdraw.targets[0].status, "conflict");
        assert_eq!(
            io::apply_operation(&db, &withdraw.id).unwrap().status,
            "partial"
        );
        assert_eq!(
            fs::read_to_string(native.join("fixture/.hidden")).unwrap(),
            "user"
        );
        assert!(delete_draft(&db, "local:fixture").is_err());
    }
    #[test]
    #[serial_test::serial]
    fn recovery_only_claims_verified_writes_and_keeps_interrupted_swaps() {
        let _sandbox = crate::test_sandbox::sandbox();
        copy_mode();
        let db = Database::memory().unwrap();
        let old = draft(&db, "old");
        let roots = tempfile::tempdir().unwrap();
        let native = roots.path().join("native");
        let mut op = preview_at(&db, &old.id, &[("codex".into(), native.clone())], None).unwrap();
        op.status = "prepared".into();
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        next.machine
            .assignments
            .insert(assignment(&old.id, "codex"), old.id.clone());
        next.machine.operations.insert(op.id.clone(), op.clone());
        db.library_commit(&before, &next).unwrap();
        stage(&mut op.targets[0]).unwrap();
        write(&op.targets[0]).unwrap();
        assert_eq!(
            io::recover_operation(&db, &op.id).unwrap().status,
            "applied"
        );
        let new_source = source("new");
        save_draft(&db, old.clone(), new_source.path(), Some(1)).unwrap();
        let mut update =
            preview_at(&db, &old.id, &[("codex".into(), native.clone())], None).unwrap();
        update.status = "prepared".into();
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        next.machine
            .operations
            .insert(update.id.clone(), update.clone());
        db.library_commit(&before, &next).unwrap();
        stage(&mut update.targets[0]).unwrap();
        let plan = update.targets[0].directory.as_ref().unwrap();
        fs::rename(native.join("fixture"), &plan.backup_path).unwrap();
        assert_eq!(
            io::recover_operation(&db, &update.id).unwrap().status,
            "needsReview"
        );
        assert!(!native.join("fixture").exists());
        assert!(Path::new(&plan.backup_path).exists());
        assert!(Path::new(&plan.stage_path).exists());
    }
    #[test]
    #[serial_test::serial]
    fn partial_retry_does_not_touch_an_already_applied_peer() {
        let _sandbox = crate::test_sandbox::sandbox();
        copy_mode();
        let db = Database::memory().unwrap();
        draft(&db, "old");
        let roots = tempfile::tempdir().unwrap();
        let good = roots.path().join("good");
        let blocked = roots.path().join("blocked");
        let op = preview_at(
            &db,
            "local:fixture",
            &[
                ("claude".into(), good.clone()),
                ("gemini".into(), blocked.clone()),
            ],
            None,
        )
        .unwrap();
        fs::write(&blocked, "block directory").unwrap();
        assert_eq!(io::apply_operation(&db, &op.id).unwrap().status, "partial");
        let modified = fs::metadata(good.join("fixture/SKILL.md"))
            .unwrap()
            .modified()
            .unwrap();
        fs::remove_file(&blocked).unwrap();
        fs::create_dir(&blocked).unwrap();
        assert_eq!(io::apply_operation(&db, &op.id).unwrap().status, "applied");
        assert_eq!(
            fs::metadata(good.join("fixture/SKILL.md"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
    }
    #[test]
    #[cfg(unix)]
    #[serial_test::serial]
    fn shared_parent_aliases_group_but_independent_leaf_links_do_not() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let old = draft(&db, "old");
        let roots = tempfile::tempdir().unwrap();
        let real = roots.path().join("real");
        fs::create_dir(&real).unwrap();
        let alias = roots.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let shared = preview_at(
            &db,
            &old.id,
            &[("codex".into(), real.clone()), ("gemini".into(), alias)],
            None,
        )
        .unwrap();
        assert_eq!(shared.targets.len(), 1);
        io::apply_operation(&db, &shared.id).unwrap();
        let retained = preview_at(&db, &old.id, &[("codex".into(), real.clone())], None).unwrap();
        io::apply_operation(&db, &retained.id).unwrap();
        assert!(real.join("fixture").exists());
        let other = roots.path().join("other");
        let independent = preview_at(
            &db,
            &old.id,
            &[
                ("codex".into(), real.clone()),
                ("gemini".into(), other.clone()),
            ],
            None,
        )
        .unwrap();
        assert_eq!(independent.targets.len(), 2);
        io::apply_operation(&db, &independent.id).unwrap();
        assert_ne!(
            entry_id(&real.join("fixture")).unwrap(),
            entry_id(&other.join("fixture")).unwrap()
        );
        let withdraw = preview_at(&db, &old.id, &[("gemini".into(), other.clone())], None).unwrap();
        io::apply_operation(&db, &withdraw.id).unwrap();
        assert!(!real.join("fixture").exists());
        assert!(other.join("fixture").exists());
        assert!(artifacts::path(old.content_hash.as_deref().unwrap())
            .unwrap()
            .exists());
    }
}
