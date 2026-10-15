//! Shareable resource drafts. Native configuration and machine proof never travel.
use super::{coordinator, skill_artifact as artifacts, skill_resource, InstructionSet};
use crate::{
    app_config::{InstalledSkill, McpServer},
    error::AppError,
    services::webdav_sync::archive,
    Database,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Write},
};

const MAX_MANIFEST: usize = 16 * 1024 * 1024;
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pack {
    schema_version: u32,
    instructions: Vec<Instruction>,
    mcp: Vec<Mcp>,
    skills: Vec<Skill>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Instruction {
    id: String,
    name: String,
    content: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Mcp {
    id: String,
    name: String,
    server: Value,
    omitted_fields: Vec<Vec<String>>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Skill {
    id: String,
    name: String,
    directory: String,
    artifact_hash: String,
}
struct Prepared {
    pack: Pack,
    bundle: archive::PreparedBundle,
    fingerprint: String,
}
fn portable_id(kind: &str, id: &str) -> String {
    if valid_id(id) {
        return id.into();
    }
    format!(
        "portable:{}",
        coordinator::fingerprint(Some(format!("{kind}:{id}").as_bytes()))
    )
}
fn valid_id(id: &str) -> bool {
    id.strip_prefix("portable:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}
fn reference(value: &str) -> bool {
    let Some(name) = value.strip_prefix("${").and_then(|s| s.strip_suffix('}')) else {
        return false;
    };
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b == b'_' || b.is_ascii_alphabetic())
        && bytes.all(|b| b == b'_' || b.is_ascii_alphanumeric())
}
/// Only exact environment references and the root transport enum are portable.
/// Nulls record omitted leaves too, so import can require an exact omission list.
fn skeleton(
    value: &Value,
    path: &mut Vec<String>,
    omitted: &mut Vec<Vec<String>>,
    depth: usize,
) -> Result<Value, AppError> {
    if depth > 32 {
        return Err(error("packInvalid"));
    }
    Ok(match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, v)| {
                    if key.is_empty() || key.len() > 256 || key.chars().any(char::is_control) {
                        return Err(error("packInvalid"));
                    }
                    path.push(key.clone());
                    let safe = skeleton(v, path, omitted, depth + 1)?;
                    path.pop();
                    Ok((key.clone(), safe))
                })
                .collect::<Result<_, AppError>>()?,
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    path.push(i.to_string());
                    let safe = skeleton(v, path, omitted, depth + 1)?;
                    path.pop();
                    Ok(safe)
                })
                .collect::<Result<_, AppError>>()?,
        ),
        Value::String(text)
            if reference(text)
                || (path.as_slice() == ["type"]
                    && ["stdio", "http", "sse"].contains(&text.as_str())) =>
        {
            value.clone()
        }
        _ => {
            omitted.push(path.clone());
            Value::Null
        }
    })
}
fn validate(pack: &Pack) -> Result<(), AppError> {
    if pack.schema_version != 1 {
        return Err(error("packVersionUnsupported"));
    }
    let count = pack
        .instructions
        .len()
        .checked_add(pack.mcp.len())
        .and_then(|n| n.checked_add(pack.skills.len()))
        .ok_or_else(|| error("packTooLarge"))?;
    if count > artifacts::PACKAGE_MAX_OBJECTS {
        return Err(error("packTooLarge"));
    }
    let mut ids = BTreeSet::new();
    let mut item = |kind: &str, id: &str, name: &str| -> Result<(), AppError> {
        if !valid_id(id)
            || name.trim().is_empty()
            || name.len() > 1024
            || !ids.insert((kind.to_string(), id.to_string()))
        {
            return Err(error("packInvalid"));
        }
        Ok(())
    };
    for row in &pack.instructions {
        item("instructions", &row.id, &row.name)?;
        if row.content.len() > 1024 * 1024 || row.content.contains("<!-- OFOX:") {
            return Err(error("invalidInstructions"));
        }
    }
    for row in &pack.mcp {
        item("mcp", &row.id, &row.name)?;
        if !row.server.is_object()
            || serde_json::to_vec(&row.server)
                .map_err(|_| error("packInvalid"))?
                .len()
                > 1024 * 1024
        {
            return Err(error("packInvalid"));
        }
        let mut omitted = vec![];
        let safe = skeleton(&row.server, &mut vec![], &mut omitted, 0)?;
        if safe != row.server || omitted != row.omitted_fields {
            return Err(error("packInvalid"));
        }
    }
    let mut directories = BTreeSet::new();
    for row in &pack.skills {
        item("skills", &row.id, &row.name)?;
        artifacts::safe_leaf(&row.directory)?;
        artifacts::path(&row.artifact_hash)?;
        if !directories.insert(row.directory.to_lowercase()) {
            return Err(error("skillNameConflict"));
        }
    }
    Ok(())
}
fn zip_pack(pack: &Pack, skills: &[u8]) -> Result<Vec<u8>, AppError> {
    let manifest = serde_json::to_vec(pack).map_err(|_| error("packInvalid"))?;
    if manifest.len() > MAX_MANIFEST
        || manifest.len() as u64 + skills.len() as u64 > artifacts::PACKAGE_MAX_BYTES
    {
        return Err(error("packTooLarge"));
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600)
        .last_modified_time(zip::DateTime::default());
    for (name, bytes) in [
        ("resources.json", manifest.as_slice()),
        ("skill-artifacts.zip", skills),
    ] {
        writer
            .start_file(name, options)
            .map_err(|_| error("packInvalid"))?;
        writer
            .write_all(bytes)
            .map_err(|_| error("packWriteFailed"))?;
    }
    let raw = writer
        .finish()
        .map_err(|_| error("packInvalid"))?
        .into_inner();
    if raw.len() as u64 > artifacts::PACKAGE_MAX_BYTES {
        return Err(error("packTooLarge"));
    }
    Ok(raw)
}
pub fn export(db: &Database) -> Result<Vec<u8>, AppError> {
    let _io = crate::config_io::lock();
    let snapshot = db.library_snapshot()?;
    let mut mcp = vec![];
    for row in db.get_all_mcp_servers()?.into_values() {
        let mut omitted_fields = vec![];
        let server = skeleton(&row.server, &mut vec![], &mut omitted_fields, 0)?;
        mcp.push(Mcp {
            id: portable_id("mcp", &row.id),
            name: row.name,
            server,
            omitted_fields,
        });
    }
    let mut skills = vec![];
    for row in db.get_all_installed_skills()?.into_values() {
        skill_resource::artifact_for(db, &row.id)?;
        skills.push(Skill {
            id: portable_id("skills", &row.id),
            name: row.name,
            directory: row.directory,
            artifact_hash: row
                .content_hash
                .ok_or_else(|| error("skillArtifactMissing"))?,
        });
    }
    let pack = Pack {
        schema_version: 1,
        instructions: snapshot
            .definitions
            .instructions
            .into_values()
            .map(|row| Instruction {
                id: portable_id("instructions", &row.id),
                name: row.name,
                content: row.content,
            })
            .collect(),
        mcp,
        skills,
    };
    validate(&pack)?;
    let temp = tempfile::tempdir().map_err(|_| error("packWriteFailed"))?;
    let path = temp.path().join("artifacts.zip");
    {
        let conn = crate::database::lock_conn!(db.conn);
        archive::zip_skill_artifacts(&path, &conn)?;
    }
    let bytes = std::fs::read(path).map_err(|_| error("packWriteFailed"))?;
    zip_pack(&pack, &bytes)
}
fn prepare(raw: &[u8]) -> Result<Prepared, AppError> {
    if raw.len() as u64 > artifacts::PACKAGE_MAX_BYTES {
        return Err(error("packTooLarge"));
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(raw)).map_err(|_| error("packInvalid"))?;
    if zip.len() != 2 {
        return Err(error("packInvalid"));
    }
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|_| error("packInvalid"))?;
        let name = entry.name().to_string();
        if !["resources.json", "skill-artifacts.zip"].contains(&name.as_str())
            || entry.is_dir()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
            || files.contains_key(&name)
        {
            return Err(error("packInvalid"));
        }
        let limit = if name == "resources.json" {
            MAX_MANIFEST as u64
        } else {
            artifacts::PACKAGE_MAX_BYTES
        };
        if entry.size() > limit {
            return Err(error("packTooLarge"));
        }
        let mut bytes = vec![];
        entry
            .by_ref()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error("packInvalid"))?;
        if bytes.len() as u64 > limit {
            return Err(error("packTooLarge"));
        }
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| error("packTooLarge"))?;
        if total > artifacts::PACKAGE_MAX_BYTES {
            return Err(error("packTooLarge"));
        }
        files.insert(name, bytes);
    }
    let pack: Pack = serde_json::from_slice(
        files
            .get("resources.json")
            .ok_or_else(|| error("packInvalid"))?,
    )
    .map_err(|_| error("packInvalid"))?;
    validate(&pack)?;
    let bundle = archive::prepare_zip(
        files
            .get("skill-artifacts.zip")
            .ok_or_else(|| error("packInvalid"))?,
        true,
    )?;
    let references = pack
        .skills
        .iter()
        .map(|row| row.artifact_hash.clone())
        .collect::<BTreeSet<_>>();
    if bundle.objects()?.keys().cloned().collect::<BTreeSet<_>>() != references {
        return Err(error("packInvalid"));
    }
    Ok(Prepared {
        pack,
        bundle,
        fingerprint: coordinator::fingerprint(Some(raw)),
    })
}
fn items(db: &Database, pack: &Pack) -> Result<Vec<Value>, AppError> {
    let snapshot = db.library_snapshot()?;
    let mcp = db.get_all_mcp_servers()?;
    let skills = db.get_all_installed_skills()?;
    let mut result = vec![];
    for row in &pack.instructions {
        let conflict = snapshot.definitions.instructions.contains_key(&row.id);
        result.push(json!({"kind":"instructions","id":row.id,"name":row.name,"conflict":conflict,"omittedFields":0}));
    }
    for row in &pack.mcp {
        let conflict = mcp.contains_key(&row.id);
        result.push(json!({"kind":"mcp","id":row.id,"name":row.name,"conflict":conflict,"omittedFields":row.omitted_fields.len()}));
    }
    for row in &pack.skills {
        let conflict = skills.contains_key(&row.id)
            || skills
                .values()
                .any(|old| old.directory.eq_ignore_ascii_case(&row.directory));
        result.push(json!({"kind":"skills","id":row.id,"name":row.name,"conflict":conflict,"omittedFields":0}));
    }
    Ok(result)
}
pub fn preview(db: &Database, raw: &[u8]) -> Result<Value, AppError> {
    let prepared = prepare(raw)?;
    let _io = crate::config_io::lock();
    let snapshot = db.library_snapshot()?;
    Ok(
        json!({"fingerprint":prepared.fingerprint,"revision":snapshot.definitions.revision,"epoch":snapshot.machine.epoch,"items":items(db,&prepared.pack)?}),
    )
}
fn prefix_bytes(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn copy_name(name: &str, mut exists: impl FnMut(&str) -> bool) -> String {
    for i in 2.. {
        let suffix = format!(" ({i})");
        let candidate = format!("{}{suffix}", prefix_bytes(name, 1024 - suffix.len()));
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}
fn fresh_id(kind: &str) -> String {
    portable_id(kind, &uuid::Uuid::new_v4().to_string())
}
pub fn restore(
    db: &Database,
    raw: &[u8],
    hash: &str,
    revision: u64,
    epoch: u64,
    conflicts: &str,
) -> Result<Value, AppError> {
    let prepared = prepare(raw)?;
    if prepared.fingerprint != hash || !["skip", "copy"].contains(&conflicts) {
        return Err(error("packChanged"));
    }
    let _io = crate::config_io::lock();
    let before = db.library_snapshot()?;
    if before.machine.epoch != epoch {
        return Err(error("epochChanged"));
    }
    if before.definitions.revision != revision {
        return Err(error("revisionChanged"));
    }
    // Invalidate open forms and old native-write operations, even on partial import.
    let mut next = before.clone();
    next.machine.epoch = next
        .machine
        .epoch
        .checked_add(1)
        .ok_or_else(|| error("revisionOverflow"))?;
    db.library_commit(&before, &next)?;
    let mut results = vec![];
    let mut imported = 0;
    for row in prepared.pack.instructions {
        let snapshot = db.library_snapshot()?;
        let collision = snapshot.definitions.instructions.contains_key(&row.id);
        if collision && conflicts == "skip" {
            results.push(json!({"kind":"instructions","name":row.name,"status":"skipped"}));
            continue;
        }
        let name = if collision {
            copy_name(&row.name, |name| {
                snapshot
                    .definitions
                    .instructions
                    .values()
                    .any(|r| r.name == name)
            })
        } else {
            row.name
        };
        let id = if collision {
            fresh_id("instructions")
        } else {
            row.id
        };
        let saved = super::save_instruction(
            db,
            InstructionSet {
                id,
                name: name.clone(),
                content: row.content,
                revision: 0,
            },
            None,
        );
        if saved.is_ok() {
            imported += 1;
        }
        results.push(json!({"kind":"instructions","name":name,"status":if saved.is_ok(){"imported"}else{"failed"},"reason":saved.err().map(|_|"library.packItemFailed")}));
    }
    for row in prepared.pack.mcp {
        let existing = db.get_all_mcp_servers()?;
        let collision = existing.contains_key(&row.id);
        if collision && conflicts == "skip" {
            results.push(json!({"kind":"mcp","name":row.name,"status":"skipped"}));
            continue;
        }
        let name = if collision {
            copy_name(&row.name, |name| existing.values().any(|r| r.name == name))
        } else {
            row.name
        };
        let id = if collision { fresh_id("mcp") } else { row.id };
        let saved = super::mcp_resource::save_draft(
            db,
            McpServer {
                id,
                name: name.clone(),
                server: row.server,
                apps: Default::default(),
                description: None,
                homepage: None,
                docs: None,
                tags: vec![],
            },
            None,
        );
        if saved.is_ok() {
            imported += 1;
        }
        results.push(json!({"kind":"mcp","name":name,"status":if saved.is_ok(){"imported"}else{"failed"},"reason":saved.err().map(|_|"library.packItemFailed")}));
    }
    for row in prepared.pack.skills {
        let existing = db.get_all_installed_skills()?;
        let collision = existing.contains_key(&row.id)
            || existing
                .values()
                .any(|old| old.directory.eq_ignore_ascii_case(&row.directory));
        if collision && conflicts == "skip" {
            results.push(json!({"kind":"skills","name":row.name,"status":"skipped"}));
            continue;
        }
        let name = if collision {
            copy_name(&row.name, |name| existing.values().any(|r| r.name == name))
        } else {
            row.name
        };
        let mut directory = row.directory;
        if existing
            .values()
            .any(|r| r.directory.eq_ignore_ascii_case(&directory))
        {
            directory = (2..)
                .map(|i| {
                    let suffix = format!("-{i}");
                    format!("{}{suffix}", prefix_bytes(&directory, 128 - suffix.len()))
                })
                .find(|d| {
                    !existing
                        .values()
                        .any(|r| r.directory.eq_ignore_ascii_case(d))
                })
                .ok_or_else(|| error("skillNameConflict"))?;
        }
        let id = if collision {
            fresh_id("skills")
        } else {
            row.id
        };
        let (source, execution) = prepared
            .bundle
            .objects()?
            .get(&row.artifact_hash)
            .ok_or_else(|| error("skillArtifactMissing"))?;
        let now = chrono::Utc::now().timestamp();
        let saved = skill_resource::save_draft_with_execution(
            db,
            InstalledSkill {
                id,
                name: name.clone(),
                description: None,
                directory,
                repo_owner: None,
                repo_name: None,
                repo_branch: None,
                readme_url: None,
                apps: Default::default(),
                installed_at: now,
                updated_at: now,
                content_hash: Some(row.artifact_hash),
            },
            source,
            None,
            Some(execution),
        );
        if saved.is_ok() {
            imported += 1;
        }
        results.push(json!({"kind":"skills","name":name,"status":if saved.is_ok(){"imported"}else{"failed"},"reason":saved.err().map(|_|"library.packItemFailed")}));
    }
    Ok(json!({"imported":imported,"items":results}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app_config::{McpApps, McpServer},
        services::library::{self, catalog, skill_resource},
    };
    use serde_json::json;
    use std::{
        fs,
        io::{Read, Write},
    };
    fn fixture() -> Database {
        let db = Database::memory().unwrap();
        let set = catalog::import_instruction(&db, "Shared", "Use careful tests").unwrap();
        let native = library::coordinator::native_instruction_paths().unwrap()["codex"].clone();
        crate::config::atomic_write(&native, b"Personal private native content").unwrap();
        let op = library::preview_instructions(&db, &set.id, &["codex".into()]).unwrap();
        library::apply_operation(&db, &op.id).unwrap();
        library::mcp_resource::save_draft(&db,McpServer{id:"legacy-private-source-id".into(),name:"MCP template".into(),server:json!({"type":"stdio","command":"fixture-server","args":["random-fixture-secret-89273"],"env":{"PRIVATE":"random-fixture-secret-89273","REF":"${MCP_TOKEN}","type":"http","MIXED":"prefix-random-fixture-secret-89273-${MCP_TOKEN}"},"headers":{"Authorization":"random-fixture-secret-89273"},"url":"https://example.invalid/endpoint?token=random-fixture-secret-89273"}),apps:McpApps::default(),description:None,homepage:None,docs:None,tags:vec![]},None).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(
            source.path().join("SKILL.md"),
            "---\nname: sample\ndescription: Sample\n---\nSkill body",
        )
        .unwrap();
        fs::write(source.path().join(".hidden"), "Hidden content").unwrap();
        catalog::import_directory(&db, source.path(), "sample", "Sample").unwrap();
        db
    }
    fn manifest(raw: &[u8]) -> Value {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(raw)).unwrap();
        let mut text = String::new();
        zip.by_name("resources.json")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        serde_json::from_str(&text).unwrap()
    }
    fn with_manifest(raw: &[u8], value: &Value) -> Vec<u8> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(raw)).unwrap();
        let mut skill_zip = Vec::new();
        zip.by_name("skill-artifacts.zip")
            .unwrap()
            .read_to_end(&mut skill_zip)
            .unwrap();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("resources.json", options).unwrap();
        writer
            .write_all(&serde_json::to_vec(value).unwrap())
            .unwrap();
        writer.start_file("skill-artifacts.zip", options).unwrap();
        writer.write_all(&skill_zip).unwrap();
        writer.finish().unwrap().into_inner()
    }
    #[test]
    #[serial_test::serial]
    fn portable_export_excludes_private_mcp_values_native_paths_and_machine_state() {
        let sandbox = crate::test_sandbox::sandbox();
        let db = fixture();
        let bytes = export(&db).unwrap();
        let value = manifest(&bytes);
        let text = value.to_string();
        for private in [
            "random-fixture-secret-89273",
            "Personal private native content",
            "legacy-private-source-id",
            "assignments",
            "receipts",
            "library_machine",
            sandbox.home().to_str().unwrap(),
        ] {
            assert!(!text.contains(private), "leaked {private}");
        }
        assert_eq!(value["mcp"][0]["server"]["env"]["REF"], "${MCP_TOKEN}");
        assert_eq!(value["mcp"][0]["server"]["command"], Value::Null);
        assert_eq!(value["mcp"][0]["server"]["env"]["type"], Value::Null);
        assert_eq!(value["mcp"][0]["server"]["env"]["MIXED"], Value::Null);
        assert!(value["mcp"][0]["omittedFields"].as_array().unwrap().len() >= 4);
    }
    #[test]
    #[serial_test::serial]
    fn restoring_a_pack_imports_only_drafts_and_preserves_native_content_and_hidden_files() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = fixture();
        let raw = export(&source).unwrap();
        let db = Database::memory().unwrap();
        let native = library::coordinator::native_instruction_paths().unwrap()["codex"].clone();
        let previous = fs::read(&native).unwrap();
        let plan = preview(&db, &raw).unwrap();
        let result = restore(
            &db,
            &raw,
            plan["fingerprint"].as_str().unwrap(),
            plan["revision"].as_u64().unwrap(),
            plan["epoch"].as_u64().unwrap(),
            "skip",
        )
        .unwrap();
        assert_eq!(result["imported"], 3);
        let snapshot = db.library_snapshot().unwrap();
        assert!(snapshot.machine.assignments.is_empty());
        assert!(snapshot.machine.receipts.is_empty());
        assert!(snapshot.machine.operations.is_empty());
        assert_eq!(fs::read(native).unwrap(), previous);
        let row = db
            .get_all_installed_skills()
            .unwrap()
            .into_values()
            .next()
            .unwrap();
        assert_eq!(
            fs::read_to_string(
                skill_resource::artifact_for(&db, &row.id)
                    .unwrap()
                    .join(".hidden")
            )
            .unwrap(),
            "Hidden content"
        );
        let plan = preview(&db, &raw).unwrap();
        let again = restore(
            &db,
            &raw,
            plan["fingerprint"].as_str().unwrap(),
            plan["revision"].as_u64().unwrap(),
            plan["epoch"].as_u64().unwrap(),
            "skip",
        )
        .unwrap();
        assert_eq!(again["imported"], 0);
    }
    #[test]
    #[serial_test::serial]
    fn malformed_or_changed_packs_never_write_partial_definitions() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = fixture();
        let raw = export(&source).unwrap();
        let db = Database::memory().unwrap();
        let plan = preview(&db, &raw).unwrap();
        let mut value = manifest(&raw);
        value["mcp"][0]["server"]["command"] = json!("unapproved-private-literal");
        let invalid = with_manifest(&raw, &value);
        assert!(preview(&db, &invalid).is_err());
        let mut value = manifest(&raw);
        value["instructions"][0]["name"] = json!("Changed after preview");
        let changed = with_manifest(&raw, &value);
        assert!(restore(
            &db,
            &changed,
            plan["fingerprint"].as_str().unwrap(),
            0,
            0,
            "skip"
        )
        .is_err());
        assert!(db
            .library_snapshot()
            .unwrap()
            .definitions
            .instructions
            .is_empty());
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        next.machine.epoch += 1;
        db.library_commit(&before, &next).unwrap();
        assert!(restore(
            &db,
            &raw,
            plan["fingerprint"].as_str().unwrap(),
            0,
            0,
            "copy"
        )
        .is_err());
        assert!(db
            .library_snapshot()
            .unwrap()
            .definitions
            .instructions
            .is_empty());
    }
    #[test]
    #[serial_test::serial]
    fn restoring_distinct_same_name_resources_matches_preview_and_keeps_both() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = fixture();
        let mut mcp = source
            .get_all_mcp_servers()
            .unwrap()
            .into_values()
            .next()
            .unwrap();
        mcp.id = "another-mcp".into();
        super::super::mcp_resource::save_draft(&source, mcp, None).unwrap();
        let raw = export(&source).unwrap();
        let db = Database::memory().unwrap();
        let plan = preview(&db, &raw).unwrap();
        assert!(plan["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["conflict"] == false));
        let result = restore(
            &db,
            &raw,
            plan["fingerprint"].as_str().unwrap(),
            plan["revision"].as_u64().unwrap(),
            plan["epoch"].as_u64().unwrap(),
            "skip",
        )
        .unwrap();
        assert_eq!(result["imported"], 4);
        assert_eq!(db.get_all_mcp_servers().unwrap().len(), 2);
    }
    #[test]
    #[serial_test::serial]
    fn copy_preserves_multibyte_directory_and_maximum_name_exportability() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = fixture();
        let mut value = manifest(&export(&source).unwrap());
        value["skills"][0]["directory"] = json!("é".repeat(64));
        value["skills"][0]["name"] = json!("é".repeat(512));
        let raw = with_manifest(&export(&source).unwrap(), &value);
        let db = Database::memory().unwrap();
        for policy in ["skip", "copy"] {
            let plan = preview(&db, &raw).unwrap();
            let result = restore(
                &db,
                &raw,
                plan["fingerprint"].as_str().unwrap(),
                plan["revision"].as_u64().unwrap(),
                plan["epoch"].as_u64().unwrap(),
                policy,
            )
            .unwrap();
            assert_eq!(result["imported"], 3);
        }
        assert_eq!(db.get_all_installed_skills().unwrap().len(), 2);
        export(&db).unwrap();
    }
    #[test]
    #[serial_test::serial]
    fn rejects_missing_and_extra_artifacts_and_misrepresented_omission_lists() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = fixture();
        let raw = export(&source).unwrap();
        let db = Database::memory().unwrap();
        let mut value = manifest(&raw);
        value["skills"] = json!([]);
        assert!(preview(&db, &with_manifest(&raw, &value)).is_err());
        let mut value = manifest(&raw);
        value["skills"][0]["artifactHash"] = json!("a".repeat(64));
        assert!(preview(&db, &with_manifest(&raw, &value)).is_err());
        let mut value = manifest(&raw);
        value["mcp"][0]["omittedFields"] = json!([]);
        assert!(preview(&db, &with_manifest(&raw, &value)).is_err());
        for field in [
            json!("prefix-${MCP_TOKEN}"),
            json!({"type":"http"}),
            json!(true),
            json!(12),
        ] {
            let mut value = manifest(&raw);
            value["mcp"][0]["server"]["newField"] = field;
            assert!(preview(&db, &with_manifest(&raw, &value)).is_err());
        }
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file(
                "../resources.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"{}").unwrap();
        writer
            .start_file(
                "skill-artifacts.zip",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        let malicious = writer.finish().unwrap().into_inner();
        assert!(preview(&db, &malicious).is_err());
        assert!(db
            .library_snapshot()
            .unwrap()
            .definitions
            .instructions
            .is_empty());
    }
    #[test]
    #[serial_test::serial]
    fn partial_database_failure_reports_each_resource_and_preserves_native_files() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = fixture();
        let raw = export(&source).unwrap();
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch("CREATE TRIGGER fail_mcp BEFORE INSERT ON mcp_servers BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        }
        let native = library::coordinator::native_instruction_paths().unwrap()["codex"].clone();
        let old = fs::read(&native).unwrap();
        let plan = preview(&db, &raw).unwrap();
        let result = restore(
            &db,
            &raw,
            plan["fingerprint"].as_str().unwrap(),
            plan["revision"].as_u64().unwrap(),
            plan["epoch"].as_u64().unwrap(),
            "skip",
        )
        .unwrap();
        assert_eq!(result["imported"], 2);
        assert_eq!(result["items"][1]["status"], "failed");
        assert_eq!(fs::read(&native).unwrap(), old);
        assert!(db.get_all_mcp_servers().unwrap().is_empty());
        assert!(db.library_snapshot().unwrap().definitions.mcp.is_empty());
    }
}
