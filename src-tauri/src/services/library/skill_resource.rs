//! Skill drafts and directory ownership, independent of model binding.
use super::{
    coordinator as io, skill_artifact as artifacts, Operation, Receipt, ResourceKind,
    ResourceRevision, TargetWrite,
};
use crate::{
    app_config::{AppType, InstalledSkill, SkillApps},
    error::AppError,
    Database,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    str::FromStr,
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
pub(crate) fn save_draft_checked_with_execution(
    db: &Database,
    mut skill: InstalledSkill,
    source: &Path,
    expected: Option<u64>,
    flags: Option<&artifacts::ExecutionMap>,
    expected_hash: Option<&str>,
) -> Result<InstalledSkill, AppError> {
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
    ["claude", "codex", "gemini", "opencode", "hermes"]
        .into_iter()
        .map(|tool| {
            let app = AppType::from_str(tool)?;
            let path = crate::services::skill::SkillService::get_app_skills_dir(&app)
                .map_err(|_| error("targetUnreadable"))?;
            if !path.is_absolute() {
                return Err(error("skillScopeUnsupported"));
            }
            Ok((tool.to_string(), path))
        })
        .collect()
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
    readers.get_mut("codex").unwrap().push(shared.clone());
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
pub fn validate_receipt(receipt: &Receipt) -> Result<(), AppError> {
    let owned: DirectoryOwned =
        serde_json::from_str(&receipt.owned_segment).map_err(|_| error("stateUnreadable"))?;
    artifacts::path(&owned.artifact_hash)?;
    if !["copy", "symlink"].contains(&owned.method.as_str())
        || owned.entry_hash != predicted(&owned.artifact_hash, &owned.method)?
    {
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
pub fn preview(db: &Database, id: &str, tools: &[String]) -> Result<Operation, AppError> {
    ensure_metadata(db)?;
    let paths = target_paths()?;
    let selected = tools
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
        .collect::<Result<Vec<_>, AppError>>()?;
    preview_at(db, id, &selected, Some(resolver_paths()?))
}
pub(crate) fn preview_at(
    db: &Database,
    id: &str,
    roots: &[(String, PathBuf)],
    resolvers: Option<BTreeMap<String, PathBuf>>,
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
            let current = entry_hash_with(
                &path,
                previous_owned
                    .as_ref()
                    .map(|owned| owned.artifact_hash.as_str()),
            )?;
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
                return Err(error("skillNameConflict"));
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
        || entry_hash_with(
            Path::new(&target.path),
            old.as_ref().map(|owned| owned.artifact_hash.as_str()),
        )? != target.before_hash
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
pub fn write(target: &TargetWrite) -> Result<(), AppError> {
    let plan = target
        .directory
        .as_ref()
        .ok_or_else(|| error("stateUnreadable"))?;
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
        || entry_hash_with(
            path,
            previous.as_ref().map(|owned| owned.artifact_hash.as_str()),
        )? != target.before_hash
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
        if entry_hash_with(backup, Some(&owned.artifact_hash))? != target.before_hash {
            return Err(error("operationNeedsReview"));
        }
        unlink(backup)?;
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
    validate_receipt(receipt).is_ok()
        && entry_id(Path::new(&receipt.path)).ok().as_deref()
            == Some(receipt.physical_target.as_str())
        && serde_json::from_str::<DirectoryOwned>(&receipt.owned_segment)
            .ok()
            .is_some_and(|owned| {
                entry_hash_with(Path::new(&receipt.path), Some(&owned.artifact_hash))
                    .ok()
                    .as_ref()
                    == Some(&owned.entry_hash)
            })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let native =
            crate::services::skill::SkillService::get_app_skills_dir(&AppType::Claude).unwrap();
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
