//! The builtin assistant is a resource consumer, never an account/key binding.
use super::{coordinator as io, skill_artifact, skill_resource, Receipt, ResourceKind};
use crate::{app_config::InstalledSkill, error::AppError, Database};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[path = "assistant_target_state.rs"]
mod state;
pub use state::{
    agent_created, read_agent_skill_origins, set_skill_mode, skill_modes, AgentSkillOrigin,
};

pub const ID: &str = "ofox-assistant";
pub fn assistant_dir() -> PathBuf {
    crate::config::get_app_config_dir().join("assistant")
}
pub fn skills_dir() -> PathBuf {
    assistant_dir().join("skills")
}
pub fn mcp_config_path() -> PathBuf {
    assistant_dir().join("mcp.json")
}
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillMode {
    #[default]
    Auto,
    Manual,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillOrigin {
    Reviewed,
    Unreviewed,
    AssistantCreated,
    Local,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantSkill {
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
    pub mode: SkillMode,
    pub has_scripts: bool,
    pub reviewed: bool,
    pub broken: Option<String>,
    pub origin: Option<SkillOrigin>,
}

#[derive(Clone, Debug, PartialEq)]
enum Identity {
    Managed {
        resource_id: String,
        revision: super::ResourceRevision,
        receipt: Box<Receipt>,
        artifact_hash: String,
    },
    Local {
        tree_hash: String,
    },
}
/// Opaque, run-local authority. Never serialize or accept it from an IPC caller.
#[derive(Clone, Debug, PartialEq)]
pub struct AssistantSkillProof {
    name: String,
    lexical_root: PathBuf,
    canonical_root: PathBuf,
    root_identity: String,
    lexical_target: PathBuf,
    canonical_dir: PathBuf,
    directory_identity: String,
    mode: SkillMode,
    epoch: u64,
    identity: Identity,
}
impl AssistantSkillProof {
    pub fn lexical_target(&self) -> &Path {
        &self.lexical_target
    }
    pub fn canonical_dir(&self) -> &Path {
        &self.canonical_dir
    }
}
pub struct AssistantSkillSnapshot {
    pub skills: Vec<AssistantSkill>,
    proofs: BTreeMap<String, AssistantSkillProof>,
}
impl AssistantSkillSnapshot {
    pub fn proof(&self, name: &str) -> Option<&AssistantSkillProof> {
        self.proofs.get(name)
    }
}
#[derive(Clone, Copy)]
pub enum SkillAccess {
    AutoUse,
    UserSelected,
}
pub struct VerifiedSkillAccess<'a> {
    proof: &'a AssistantSkillProof,
}
impl VerifiedSkillAccess<'_> {
    pub fn dir(&self) -> &Path {
        self.proof.canonical_dir()
    }
    pub fn target(&self) -> &Path {
        self.proof.lexical_target()
    }
}

#[derive(Deserialize)]
struct Recommended {
    id: String,
    owner: String,
    repo: String,
    directory: String,
}
#[derive(Deserialize)]
struct Popular {
    source: String,
    #[serde(rename = "skillId")]
    skill_id: String,
    official: bool,
}
#[derive(Deserialize)]
struct PopularCatalog {
    skills: Vec<Popular>,
}
fn recommended() -> Vec<Recommended> {
    serde_json::from_str(include_str!(
        "../../../../src/components/library/recommendedSkills.json"
    ))
    .expect("bundled recommended metadata")
}
fn popular() -> Vec<Popular> {
    serde_json::from_str::<PopularCatalog>(include_str!(
        "../../../../src/components/library/popularSkills.json"
    ))
    .expect("bundled popular metadata")
    .skills
}
pub fn reviewed_names() -> BTreeSet<String> {
    recommended()
        .into_iter()
        .map(|r| r.id)
        .chain(
            popular()
                .into_iter()
                .filter(|r| r.official)
                .map(|r| r.skill_id),
        )
        .collect()
}

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.contains("..")
        && !name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "/\\<>:\"|?*".contains(c))
        && skill_artifact::safe_leaf(name).is_ok()
}

/// Strict metadata from the actual document; discovery/display titles are not identity.
pub(super) fn metadata(dir: &Path) -> Result<(String, String), AppError> {
    let dir = fs::canonicalize(dir).map_err(|_| error("assistantSkillUnreadable"))?;
    let bytes = state::read_regular(&dir.join("SKILL.md"), 1024 * 1024)?
        .ok_or_else(|| error("assistantSkillUnreadable"))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| error("assistantSkillBinary"))?;
    if text
        .chars()
        .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
    {
        return Err(error("assistantSkillBinary"));
    }
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next() != Some("---") {
        return Err(error("assistantSkillFrontmatter"));
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        return Err(error("assistantSkillFrontmatter"));
    }
    #[derive(Deserialize)]
    struct Frontmatter {
        name: String,
        description: Option<String>,
    }
    let data: Frontmatter =
        serde_yaml::from_str(&yaml).map_err(|_| error("assistantSkillFrontmatter"))?;
    if !valid_name(&data.name) {
        return Err(error("assistantSkillNameInvalid"));
    }
    let description = data
        .description
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| error("assistantSkillDescriptionMissing"))?;
    Ok((data.name, description))
}

fn has_scripts(root: &Path) -> Result<bool, AppError> {
    let mut stack = vec![(root.to_path_buf(), 0)];
    let mut count = 0;
    let mut scripts = false;
    while let Some((dir, depth)) = stack.pop() {
        if depth > 32 {
            return Err(error("skillTooLarge"));
        }
        for entry in fs::read_dir(dir).map_err(|_| error("assistantSkillUnreadable"))? {
            let entry = entry.map_err(|_| error("assistantSkillUnreadable"))?;
            count += 1;
            if count > 10_000 {
                return Err(error("skillTooLarge"));
            }
            let kind = entry
                .file_type()
                .map_err(|_| error("assistantSkillUnreadable"))?;
            if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) {
                return Err(error("skillLinkUnsupported"));
            }
            if kind.is_dir() {
                scripts |= entry.file_name() == "scripts";
                stack.push((entry.path(), depth + 1));
            } else {
                scripts |= entry
                    .path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| ["py", "sh", "js", "ts"].contains(&s));
            }
        }
    }
    Ok(scripts)
}

/// Catalog origin classification, not an execution permission or a safety review.
pub fn origin_for_resource(db: &Database, row: &InstalledSkill) -> Result<SkillOrigin, AppError> {
    origin_with_candidates(db, row, &reviewed_names())
}
fn origin_with_candidates(
    db: &Database,
    row: &InstalledSkill,
    candidates: &BTreeSet<String>,
) -> Result<SkillOrigin, AppError> {
    let _io = crate::config_io::lock();
    let names = agent_created()?;
    let origins = read_agent_skill_origins()?;
    let path = skill_resource::artifact_for(db, &row.id);
    let source = super::skill_actions::provenance_view(row);
    let fallback = if source["sourceKind"] == "localOrArchive" {
        SkillOrigin::Local
    } else {
        SkillOrigin::Unreviewed
    };
    let Ok(path) = path else {
        return Ok(fallback);
    };
    let Ok((name, _)) = metadata(&path) else {
        return Ok(fallback);
    };
    if names.contains(&name) && state::verified_created_origin(db, row, origins.get(&row.id))? {
        return Ok(SkillOrigin::AssistantCreated);
    }
    if source["commitSha"].as_str().is_none() || source["sourceKind"] != "repository" {
        return Ok(fallback);
    }
    let recipes: Vec<_> = recommended()
        .into_iter()
        .filter(|r| candidates.contains(&r.id))
        .map(|r| super::inventory::BuiltinRecipe {
            recipe_id: r.id,
            content: None,
            repo_owner: Some(r.owner),
            repo_name: Some(r.repo),
            directory: Some(r.directory),
        })
        .collect();
    if recipes
        .iter()
        .any(|recipe| super::inventory::fixed_skill_matches(db, recipe, row))
    {
        return Ok(SkillOrigin::Reviewed);
    }
    let official = popular().iter().any(|p| {
        p.official
            && candidates.contains(&p.skill_id)
            && p.skill_id == name
            && source["repository"]
                .as_str()
                .is_some_and(|repo| p.source.eq_ignore_ascii_case(repo))
    });
    Ok(if official {
        SkillOrigin::Reviewed
    } else {
        fallback
    })
}

pub fn list_assistant_skills(
    db: &Database,
    names: &BTreeSet<String>,
) -> Result<Vec<AssistantSkill>, AppError> {
    Ok(assistant_skill_snapshot(db, names)?.skills)
}
pub fn assistant_skill_snapshot(
    db: &Database,
    names: &BTreeSet<String>,
) -> Result<AssistantSkillSnapshot, AppError> {
    let _io = crate::config_io::lock();
    let modes = skill_modes()?;
    agent_created()?;
    read_agent_skill_origins()?;
    let root = skills_dir();
    state::check_directory(&root)?;
    let canonical_root = io::physical(&root)?;
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AssistantSkillSnapshot {
                skills: vec![],
                proofs: BTreeMap::new(),
            })
        }
        Err(_) => return Err(error("assistantSkillUnreadable")),
    };
    let snapshot = db.library_snapshot()?;
    let root_identity = directory_identity(&canonical_root)?;
    let rows = db.get_all_installed_skills()?;
    let mut skills = Vec::new();
    let mut candidates = Vec::new();
    for (index, entry) in entries.enumerate() {
        let entry = entry.map_err(|_| error("assistantSkillUnreadable"))?;
        if index >= 1000 {
            return Err(error("skillTooLarge"));
        }
        let leaf = entry.file_name().to_string_lossy().into_owned();
        // Library's adjacent swap/backup folders are operation material, not skills.
        if leaf.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let mut skill = AssistantSkill {
            name: leaf.clone(),
            description: String::new(),
            dir: path.clone(),
            mode: SkillMode::Auto,
            has_scripts: false,
            reviewed: false,
            broken: None,
            origin: None,
        };
        let result = (|| {
            skill_artifact::safe_leaf(&leaf)?;
            let canonical_dir =
                fs::canonicalize(&path).map_err(|_| error("assistantSkillUnreadable"))?;
            let identity_before = directory_identity(&canonical_dir)?;
            let entry_id = skill_resource::entry_id(&path)?;
            let row = rows
                .values()
                .find(|r| r.directory.eq_ignore_ascii_case(&leaf));
            let receipt = snapshot.machine.receipts.get(&format!("skills:{entry_id}"));
            let identity = if let Some(row) = row {
                let revision = snapshot
                    .definitions
                    .skills
                    .get(&row.id)
                    .ok_or_else(|| error("skillNeedsMigration"))?;
                if snapshot
                    .machine
                    .assignments
                    .get(&format!("skills:{}:{ID}", row.id))
                    != Some(&row.id)
                {
                    return Err(error("assistantSkillUnavailable"));
                }
                let receipt = receipt
                    .filter(|r| {
                        r.kind == ResourceKind::Skills
                            && r.resource_id == row.id
                            && r.resource_revision == revision.revision
                            && r.definition_hash == revision.hash
                            && r.owners.iter().any(|o| o == ID)
                            && r.path == path.to_string_lossy()
                            && r.physical_target == entry_id
                    })
                    .ok_or_else(|| error("assistantSkillUnavailable"))?;
                let artifact = skill_resource::artifact_for(db, &row.id)?;
                let owned: skill_resource::DirectoryOwned =
                    serde_json::from_str(&receipt.owned_segment)
                        .map_err(|_| error("stateUnreadable"))?;
                if row.content_hash.as_ref() != Some(&owned.artifact_hash)
                    || !skill_resource::receipt_intact(receipt)
                {
                    return Err(error("skillEntryChanged"));
                }
                if fs::symlink_metadata(&path)
                    .map_err(|_| error("assistantSkillUnreadable"))?
                    .file_type()
                    .is_symlink()
                    && canonical_dir
                        != fs::canonicalize(artifact)
                            .map_err(|_| error("assistantSkillUnreadable"))?
                {
                    return Err(error("skillLinkUnsupported"));
                }
                Identity::Managed {
                    resource_id: row.id.clone(),
                    revision: revision.clone(),
                    receipt: Box::new(receipt.clone()),
                    artifact_hash: owned.artifact_hash,
                }
            } else {
                if receipt.is_some()
                    || fs::symlink_metadata(&path)
                        .map_err(|_| error("assistantSkillUnreadable"))?
                        .file_type()
                        .is_symlink()
                    || canonical_dir.parent() != Some(canonical_root.as_path())
                {
                    return Err(error("skillLinkUnsupported"));
                }
                // Parse first to preserve the precise broken-document reason.
                metadata(&canonical_dir)?;
                Identity::Local {
                    tree_hash: skill_artifact::tree_hash(&canonical_dir)?,
                }
            };
            let (name, description) = metadata(&canonical_dir)?;
            skill.name = name;
            skill.description = description;
            skill.mode = modes.get(&skill.name).copied().unwrap_or_default();
            skill.has_scripts = has_scripts(&canonical_dir)?;
            let origin = row
                .map(|r| origin_with_candidates(db, r, names))
                .transpose()?
                .unwrap_or(SkillOrigin::Local);
            skill.origin = Some(origin);
            skill.reviewed = origin == SkillOrigin::Reviewed;
            if directory_identity(&canonical_dir)? != identity_before
                || directory_identity(&canonical_root)? != root_identity
            {
                return Err(error("assistantSkillUnavailable"));
            }
            Ok(AssistantSkillProof {
                name: skill.name.clone(),
                lexical_root: root.clone(),
                canonical_root: canonical_root.clone(),
                root_identity: root_identity.clone(),
                lexical_target: path,
                canonical_dir,
                directory_identity: identity_before,
                mode: skill.mode,
                epoch: snapshot.machine.epoch,
                identity,
            })
        })();
        match result {
            Ok(proof) => candidates.push(proof),
            Err(e) => {
                skill.broken = Some(e.to_string());
                skill.reviewed = false;
                skill.origin = None;
            }
        }
        skills.push(skill);
    }
    let mut counts = BTreeMap::new();
    for skill in &skills {
        *counts.entry(skill.name.to_lowercase()).or_insert(0usize) += 1;
    }
    for skill in &mut skills {
        if counts[&skill.name.to_lowercase()] > 1 {
            skill.broken = Some("library.assistantSkillDuplicate".into());
            skill.reviewed = false;
            skill.origin = None;
        }
    }
    let proofs = candidates
        .into_iter()
        .filter(|p| counts[&p.name.to_lowercase()] == 1)
        .map(|p| (p.name.clone(), p))
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.dir.cmp(&b.dir)));
    Ok(AssistantSkillSnapshot { skills, proofs })
}

fn directory_identity(path: &Path) -> Result<String, AppError> {
    crate::session_manager::source_cache::SourceStamp::at(path)
        .map(|stamp| stamp.physical_identity)
        .map_err(|_| error("assistantSkillUnreadable"))
}

/// Bounded synchronous reads only; never hold this callback across approval/network/process waits.
pub fn with_revalidated_skill<T>(
    db: &Database,
    proof: &AssistantSkillProof,
    access: SkillAccess,
    read: impl FnOnce(&VerifiedSkillAccess<'_>) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let _io = crate::config_io::lock();
    let verify = || {
        let current = assistant_skill_snapshot(db, &reviewed_names())?;
        if current.proof(&proof.name) != Some(proof)
            || (matches!(access, SkillAccess::AutoUse) && proof.mode != SkillMode::Auto)
        {
            return Err(error("assistantSkillUnavailable"));
        }
        Ok(())
    };
    verify()?;
    let result = read(&VerifiedSkillAccess { proof })?;
    verify()?;
    Ok(result)
}

/// Validate all sidecar input before a library operation changes assignments/files.
pub(super) fn validate_operation(op: &super::Operation) -> Result<(), AppError> {
    if op.kind == ResourceKind::Skills
        && op
            .desired_updates
            .keys()
            .any(|k| k.ends_with(&format!(":{ID}")))
    {
        skill_modes()?;
        agent_created()?;
        read_agent_skill_origins()?;
    }
    Ok(())
}
/// A newly enabled entry starts Auto. State write failure prevents target mutation.
pub(super) fn write_target(target: &super::TargetWrite, enabling: bool) -> Result<(), AppError> {
    if !enabling {
        return skill_resource::write(target);
    }
    let receipt = target
        .receipt
        .as_ref()
        .ok_or_else(|| error("stateUnreadable"))?;
    let owned: skill_resource::DirectoryOwned =
        serde_json::from_str(&receipt.owned_segment).map_err(|_| error("stateUnreadable"))?;
    let (name, _) = metadata(&skill_artifact::path(&owned.artifact_hash)?)?;
    let previous = skill_modes()?;
    set_skill_mode(&name, SkillMode::Auto)?;
    match skill_resource::write(target) {
        Ok(()) => Ok(()),
        Err(failure) => {
            state::write_modes(&previous)?;
            Err(failure)
        }
    }
}

#[cfg(test)]
#[path = "assistant_target_tests.rs"]
mod tests;
