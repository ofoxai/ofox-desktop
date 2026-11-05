//! Strict private state. Missing is empty; malformed/unreadable is never empty.
use super::*;
use serde::de::{DeserializeOwned, MapAccess, Visitor};
#[cfg(not(unix))]
use std::io::Read;
use std::{fmt, marker::PhantomData};

const MAX_STATE: u64 = 1024 * 1024;

fn linked(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
/// Resolve the configured app-root alias, then refuse links within private state.
pub(crate) fn private_path(path: &Path) -> Result<PathBuf, AppError> {
    let base = crate::config::get_app_config_dir();
    let relative = path
        .strip_prefix(&base)
        .map_err(|_| error("assistantSkillStateInvalid"))?;
    let mut actual = io::physical(&base)?;
    for part in relative.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err(error("assistantSkillStateInvalid"));
        }
        actual.push(part);
        match fs::symlink_metadata(&actual) {
            Ok(meta) if linked(&meta) => return Err(error("assistantSkillStateInvalid")),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(error("assistantSkillStateInvalid")),
        }
    }
    Ok(actual)
}
pub(super) fn check_directory(path: &Path) -> Result<(), AppError> {
    let actual = private_path(path)?;
    match fs::symlink_metadata(actual) {
        Ok(meta) if !meta.is_dir() => Err(error("assistantSkillStateInvalid")),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(error("assistantSkillStateInvalid")),
    }
}
pub(crate) fn read_regular(path: &Path, max: u64) -> Result<Option<Vec<u8>>, AppError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(error("assistantSkillUnreadable")),
    };
    if !meta.is_file() || linked(&meta) {
        return Err(error("assistantSkillUnreadable"));
    }
    if meta.len() > max {
        return Err(error("assistantSkillTooLarge"));
    }
    #[cfg(unix)]
    {
        crate::config::read_private_with(path, max, || {})
            .map_err(|_| error("assistantSkillUnreadable"))
    }
    #[cfg(not(unix))]
    {
        let mut file = fs::File::open(path).map_err(|_| error("assistantSkillUnreadable"))?;
        let before = file
            .metadata()
            .map_err(|_| error("assistantSkillUnreadable"))?;
        if before.len() > max {
            return Err(error("assistantSkillTooLarge"));
        }
        let opened_identity = crate::session_manager::source_cache::SourceStamp::of(&file)
            .map_err(|_| error("assistantSkillUnreadable"))?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error("assistantSkillUnreadable"))?;
        if bytes.len() as u64 > max {
            return Err(error("assistantSkillTooLarge"));
        }
        let after = fs::symlink_metadata(path).map_err(|_| error("assistantSkillUnreadable"))?;
        if linked(&after)
            || !after.is_file()
            || bytes.len() as u64 != before.len()
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || crate::session_manager::source_cache::SourceStamp::at(path)
                .map_err(|_| error("assistantSkillUnreadable"))?
                != opened_identity
        {
            return Err(error("assistantSkillUnreadable"));
        }
        Ok(Some(bytes))
    }
}
fn read_state<T: DeserializeOwned + Default>(file: &str) -> Result<T, AppError> {
    let path = private_path(&assistant_dir().join(file))?;
    let Some(bytes) =
        read_regular(&path, MAX_STATE).map_err(|_| error("assistantSkillStateInvalid"))?
    else {
        return Ok(T::default());
    };
    serde_json::from_slice(&bytes).map_err(|_| error("assistantSkillStateInvalid"))
}

struct UniqueMap<T>(BTreeMap<String, T>);
impl<T> Default for UniqueMap<T> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for UniqueMap<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct MapVisitor<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for MapVisitor<T> {
            type Value = UniqueMap<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a map with unique keys")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut result = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, T>()? {
                    if result.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate key"));
                    }
                }
                Ok(UniqueMap(result))
            }
        }
        d.deserialize_map(MapVisitor(PhantomData))
    }
}
pub fn skill_modes() -> Result<BTreeMap<String, SkillMode>, AppError> {
    let _io = crate::config_io::lock();
    let modes = read_state::<UniqueMap<SkillMode>>("skill-modes.json")?.0;
    if modes.keys().any(|name| !valid_name(name)) {
        return Err(error("assistantSkillStateInvalid"));
    }
    Ok(modes)
}
pub fn agent_created() -> Result<BTreeSet<String>, AppError> {
    let _io = crate::config_io::lock();
    let names: Vec<String> = read_state("agent-skills.json")?;
    let set: BTreeSet<_> = names.iter().cloned().collect();
    if names.len() != set.len() || names.iter().any(|name| !valid_name(name)) {
        return Err(error("assistantSkillStateInvalid"));
    }
    Ok(set)
}

/// Task6 writes these with its resource transaction. Names alone grant nothing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSkillOrigin {
    pub version: u8,
    pub creation_id: String,
    pub revision: super::super::ResourceRevision,
    pub artifact_hash: String,
    pub receipt: Receipt,
}
pub fn read_agent_skill_origins() -> Result<BTreeMap<String, AgentSkillOrigin>, AppError> {
    let _io = crate::config_io::lock();
    let origins = read_state::<UniqueMap<AgentSkillOrigin>>("agent-skill-origins.json")?.0;
    for (id, origin) in &origins {
        if id.is_empty()
            || id.len() > 256
            || origin.version != 1
            || uuid::Uuid::parse_str(&origin.creation_id).is_err()
            || origin.receipt.kind != ResourceKind::Skills
            || origin.receipt.resource_id != *id
            || origin.receipt.resource_revision != origin.revision.revision
            || origin.receipt.definition_hash != origin.revision.hash
            || !origin.receipt.owners.iter().any(|s| s == ID)
        {
            return Err(error("assistantSkillStateInvalid"));
        }
        skill_artifact::path(&origin.artifact_hash)
            .map_err(|_| error("assistantSkillStateInvalid"))?;
        skill_resource::validate_receipt(&origin.receipt)
            .map_err(|_| error("assistantSkillStateInvalid"))?;
        let owned: skill_resource::DirectoryOwned =
            serde_json::from_str(&origin.receipt.owned_segment)
                .map_err(|_| error("assistantSkillStateInvalid"))?;
        if owned.artifact_hash != origin.artifact_hash {
            return Err(error("assistantSkillStateInvalid"));
        }
    }
    Ok(origins)
}
/// Durable provenance does not depend on whether the assistant target is enabled.
pub(crate) fn verified_creation_source(
    db: &Database,
    row: &InstalledSkill,
    origin: Option<&AgentSkillOrigin>,
) -> Result<bool, AppError> {
    let Some(origin) = origin else {
        return Ok(false);
    };
    let snapshot = db.library_snapshot()?;
    Ok(
        snapshot.definitions.skills.get(&row.id) == Some(&origin.revision)
            && row.content_hash.as_ref() == Some(&origin.artifact_hash)
            && origin.receipt.resource_id == row.id
            && skill_resource::artifact_for(db, &row.id).is_ok(),
    )
}
pub(crate) fn verified_created_origin(
    db: &Database,
    row: &InstalledSkill,
    origin: Option<&AgentSkillOrigin>,
) -> Result<bool, AppError> {
    let Some(origin) = origin else {
        return Ok(false);
    };
    let snapshot = db.library_snapshot()?;
    let target = skills_dir().join(&row.directory);
    let physical = skill_resource::entry_id(&target)?;
    Ok(verified_creation_source(db, row, Some(origin))?
        && snapshot
            .machine
            .assignments
            .get(&format!("skills:{}:{ID}", row.id))
            == Some(&row.id)
        && origin.receipt.path == target.to_string_lossy()
        && origin.receipt.physical_target == physical
        && snapshot.machine.receipts.get(&format!("skills:{physical}")) == Some(&origin.receipt)
        && skill_resource::artifact_for(db, &row.id).is_ok()
        && skill_resource::receipt_intact(&origin.receipt))
}
pub(super) fn write_modes(modes: &BTreeMap<String, SkillMode>) -> Result<(), AppError> {
    let path = private_path(&assistant_dir().join("skill-modes.json"))?;
    crate::config::create_private_dir_all(
        path.parent()
            .ok_or_else(|| error("assistantSkillStateInvalid"))?,
    )
    .map_err(|_| error("assistantSkillStateInvalid"))?;
    let bytes =
        serde_json::to_vec_pretty(modes).map_err(|_| error("assistantSkillStateInvalid"))?;
    #[cfg(unix)]
    {
        crate::config::atomic_write_private_with(&path, &bytes, || {})
            .map_err(|_| error("assistantSkillStateInvalid"))
    }
    #[cfg(not(unix))]
    {
        crate::config::atomic_write_with(
            &path,
            &bytes,
            crate::config::AtomicOpts { mode: Some(0o600) },
        )
        .map_err(|_| error("assistantSkillStateInvalid"))
    }
}
pub fn set_skill_mode(name: &str, mode: SkillMode) -> Result<(), AppError> {
    let _io = crate::config_io::lock();
    if !valid_name(name) {
        return Err(error("assistantSkillNameInvalid"));
    }
    let mut modes = skill_modes()?;
    agent_created()?;
    read_agent_skill_origins()?;
    modes.insert(name.into(), mode);
    write_modes(&modes)
}
