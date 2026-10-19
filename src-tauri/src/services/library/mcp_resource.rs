//! MCP definitions remain in the v11 table; revisions and local ownership use library state.
use super::{
    coordinator as io, mcp_codec, LibrarySnapshot, Operation, Receipt, ResourceKind,
    ResourceRevision, TargetWrite,
};
use crate::{
    app_config::{McpApps, McpServer},
    error::AppError,
    Database,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}
pub(crate) fn spec_hash(spec: &Value) -> Result<String, AppError> {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, value)| (key.clone(), sorted(value)))
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            Value::Array(values) => Value::Array(values.iter().map(sorted).collect()),
            _ => value.clone(),
        }
    }
    Ok(io::fingerprint(Some(
        serde_json::to_string(&sorted(spec))
            .map_err(|_| error("invalidMcp"))?
            .as_bytes(),
    )))
}
/// Compares definitions by meaning; `spec_hash` stays the exact stored-row hash.
pub(crate) fn match_hash(spec: &Value) -> Result<String, AppError> {
    spec_hash(&mcp_codec::normalize_spec(spec))
}

pub fn save_draft(
    db: &Database,
    mut server: McpServer,
    expected_revision: Option<u64>,
) -> Result<ResourceRevision, AppError> {
    let _guard = crate::config_io::lock();
    if server.id.is_empty()
        || server.id.len() > 256
        || server.name.trim().is_empty()
        || !server.server.is_object()
        || serde_json::to_vec(&server.server)
            .map_err(|_| error("invalidMcp"))?
            .len()
            > 1024 * 1024
    {
        return Err(error("invalidMcp"));
    }
    let before = db.library_snapshot()?;
    if before
        .definitions
        .mcp
        .get(&server.id)
        .map(|meta| meta.revision)
        != expected_revision
    {
        return Err(error("revisionChanged"));
    }
    let meta = ResourceRevision {
        revision: expected_revision
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| error("revisionOverflow"))?,
        hash: spec_hash(&server.server)?,
    };
    server.apps = McpApps::default();
    let mut next = before.clone();
    next.definitions.mcp.insert(server.id.clone(), meta.clone());
    db.library_commit_mcp(&before, &next, Some(&server))?;
    Ok(meta)
}
fn bound(snapshot: &LibrarySnapshot, id: &str) -> bool {
    snapshot
        .machine
        .receipts
        .values()
        .any(|r| r.kind == ResourceKind::Mcp && r.resource_id == id)
        || snapshot
            .machine
            .assignments
            .keys()
            .any(|key| key.starts_with(&format!("mcp:{id}:")))
}
pub fn delete_draft(db: &Database, id: &str) -> Result<bool, AppError> {
    let _guard = crate::config_io::lock();
    let before = db.library_snapshot()?;
    if bound(&before, id) {
        return Err(error("withdrawRequired"));
    }
    if !db.get_all_mcp_servers()?.contains_key(id) {
        return Ok(false);
    }
    let mut next = before.clone();
    next.definitions.mcp.remove(id);
    db.library_commit_resources(&before, &next, None, Some(id))?;
    Ok(true)
}

pub fn ensure_metadata(db: &Database) -> Result<(), AppError> {
    let _guard = crate::config_io::lock();
    for server in db.get_all_mcp_servers()?.into_values() {
        if !db
            .library_snapshot()?
            .definitions
            .mcp
            .contains_key(&server.id)
        {
            save_draft(db, server, None)?;
        }
    }
    drop_generated_duplicates(db)
}
/// Ids an import generated beside an existing name: `name--<16 hex>` or `name-import-<8 hex>`.
fn generated_alias_of(id: &str, base: &str) -> bool {
    let hex = |text: &str, len: usize| {
        text.len() == len && text.bytes().all(|byte| byte.is_ascii_hexdigit())
    };
    id.strip_prefix(base).is_some_and(|rest| {
        rest.strip_prefix("--").is_some_and(|tail| hex(tail, 16))
            || rest
                .strip_prefix("-import-")
                .is_some_and(|tail| hex(tail, 8))
    })
}
/// Imports used to keep a second copy when only the spelling differed (`"env": {}`).
/// Such a copy goes, unless a tool assignment or receipt still refers to it.
fn drop_generated_duplicates(db: &Database) -> Result<(), AppError> {
    let rows = db.get_all_mcp_servers()?;
    let snapshot = db.library_snapshot()?;
    for row in rows.values() {
        let Ok(hash) = match_hash(&row.server) else {
            continue;
        };
        let copy = rows.values().any(|other| {
            generated_alias_of(&row.id, &other.id)
                && other.name.eq_ignore_ascii_case(&row.name)
                && match_hash(&other.server).ok().as_ref() == Some(&hash)
        });
        if copy && !bound(&snapshot, &row.id) {
            delete_draft(db, &row.id)?;
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldEdit {
    pub path: Vec<String>,
    #[serde(default)]
    pub field_ref: Option<String>,
    pub action: String,
    pub value: Option<Value>,
}

pub fn save_edits(
    db: &Database,
    id: &str,
    name: &str,
    revision: Option<u64>,
    edits: &[FieldEdit],
) -> Result<ResourceRevision, AppError> {
    let _guard = crate::config_io::lock();
    let existing = db.get_all_mcp_servers()?.shift_remove(id);
    if existing.is_some() && revision.is_none() {
        return Err(error("revisionChanged"));
    }
    let mut server = existing.unwrap_or_else(|| McpServer {
        id: id.into(),
        name: name.into(),
        server: json!({}),
        apps: McpApps::default(),
        description: None,
        homepage: None,
        docs: None,
        tags: vec![],
    });
    if edits.len() > 64 {
        return Err(error("invalidMcp"));
    }
    for (index, edit) in edits.iter().enumerate() {
        if edits[..index]
            .iter()
            .any(|prior| prior.path.starts_with(&edit.path) || edit.path.starts_with(&prior.path))
        {
            return Err(error("overlappingEdits"));
        }
    }
    for edit in edits {
        if edit
            .field_ref
            .as_ref()
            .is_some_and(|reference| *reference != field_ref(id, revision, &edit.path))
        {
            return Err(error("fieldReferenceChanged"));
        }
    }
    let mut seen = BTreeSet::new();
    for edit in edits {
        if edit.path.is_empty()
            || edit.path.len() > 4
            || edit.path.iter().any(|s| s.is_empty() || s.len() > 256)
            || !seen.insert(&edit.path)
        {
            return Err(error("invalidMcp"));
        }
        if edit.action == "keep" {
            if edit.value.is_some() {
                return Err(error("invalidMcp"));
            }
            continue;
        }
        edit_field(
            &mut server.server,
            &edit.path,
            &edit.action,
            edit.value.as_ref(),
        )?;
    }
    server.name = name.into();
    save_draft(db, server, revision)
}

fn edit_field(
    root: &mut Value,
    path: &[String],
    action: &str,
    new: Option<&Value>,
) -> Result<(), AppError> {
    let map = root.as_object_mut().ok_or_else(|| error("invalidMcp"))?;
    let key = &path[0];
    if path.len() > 1 {
        if !map.contains_key(key) {
            if action == "clear" {
                return Ok(());
            }
            map.insert(key.clone(), json!({}));
        }
        return edit_field(
            map.get_mut(key).expect("key inserted"),
            &path[1..],
            action,
            new,
        );
    }
    match action {
        "set" => {
            let new = new.cloned().ok_or_else(|| error("invalidMcp"))?;
            let serialized = serde_json::to_string(&new).map_err(|_| error("invalidMcp"))?;
            if serialized.contains("[REDACTED]") || serialized.contains(crate::redact::REDACTED) {
                return Err(error("maskedValueRejected"));
            }
            map.insert(key.clone(), new);
        }
        "clear" if new.is_none() => {
            map.remove(key);
        }
        _ => return Err(error("invalidMcp")),
    }
    Ok(())
}

pub fn native_paths() -> Result<BTreeMap<String, PathBuf>, AppError> {
    let dir = crate::opencode_config::get_opencode_dir();
    let opencode = ["opencode.jsonc", "opencode.json", "config.json"]
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.exists())
        .unwrap_or_else(|| dir.join("opencode.json"));
    Ok(BTreeMap::from([
        ("claude".into(), crate::config::get_claude_mcp_path()),
        ("codex".into(), crate::codex_config::get_codex_config_path()),
        (
            "gemini".into(),
            crate::gemini_config::get_gemini_settings_path(),
        ),
        ("opencode".into(), opencode),
        (
            "hermes".into(),
            crate::hermes_config::get_hermes_config_path(),
        ),
    ]))
}
fn assignment(id: &str, tool: &str) -> String {
    format!("mcp:{id}:{tool}")
}
/// Global layers only. Project/session overrides remain explicitly outside this scope.
pub fn opencode_reader_paths() -> Result<Vec<PathBuf>, AppError> {
    let mut paths = crate::opencode_config::global_config_paths();
    if let Ok(raw) = std::env::var("OPENCODE_CONFIG") {
        if !raw.is_empty() {
            let path = PathBuf::from(raw);
            if !path.is_absolute() {
                return Err(error("mcpScopeUnsupported"));
            }
            paths.push(path);
        }
    }
    let mut dirs = vec![crate::config::get_home_dir().join(".opencode")];
    if let Ok(raw) = std::env::var("OPENCODE_CONFIG_DIR") {
        if !raw.is_empty() {
            let path = PathBuf::from(raw);
            if !path.is_absolute() {
                return Err(error("mcpScopeUnsupported"));
            }
            dirs.push(path);
        }
    }
    for dir in dirs {
        for file in ["opencode.json", "opencode.jsonc"] {
            paths.push(dir.join(file));
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}
pub fn routing_dependencies() -> Result<super::RoutingDependencies, AppError> {
    let paths = opencode_reader_paths()?
        .into_iter()
        .map(|path| Ok((path.to_string_lossy().into_owned(), io::physical_id(&path)?)))
        .collect::<Result<_, AppError>>()?;
    let environment: BTreeMap<_, _> = [
        "OPENCODE_CONFIG",
        "OPENCODE_CONFIG_DIR",
        "OPENCODE_CONFIG_CONTENT",
    ]
    .into_iter()
    .map(|key| (key, std::env::var(key).ok()))
    .collect();
    Ok(super::RoutingDependencies {
        version: 1,
        paths,
        environment_hash: io::fingerprint(Some(
            &serde_json::to_vec(&environment).map_err(|_| error("nativeInvalid"))?,
        )),
    })
}
fn check_inline_opencode_server(id: &str) -> Result<(), AppError> {
    let Ok(text) = std::env::var("OPENCODE_CONFIG_CONTENT") else {
        return Ok(());
    };
    if text.is_empty() {
        return Ok(());
    }
    if text.len() > 2 * 1024 * 1024 {
        return Err(error("targetTooLarge"));
    }
    let root = mcp_codec::parse_native(&text, "opencode")?;
    super::native_json::validate_routing_keys(&root, &["mcp"])?;
    if root
        .get("mcp")
        .and_then(|servers| servers.get(id))
        .is_some()
    {
        return Err(error("mcpNameConflict"));
    }
    Ok(())
}
pub fn opencode_scope_clear(id: &str, target: &str) -> Result<(), AppError> {
    check_inline_opencode_server(id)?;
    for path in opencode_reader_paths()? {
        if let Some(text) = io::read(&path)? {
            let root = mcp_codec::parse_native(&text, "opencode")?;
            super::native_json::validate_routing_keys(&root, &["mcp"])?;
            if io::physical_id(&path)? != target
                && root
                    .get("mcp")
                    .and_then(|servers| servers.get(id))
                    .is_some()
            {
                return Err(error("mcpNameConflict"));
            }
        }
    }
    Ok(())
}
#[derive(Default)]
struct Group {
    path: PathBuf,
    selected: BTreeSet<String>,
    old: Option<Receipt>,
}
pub fn preview(db: &Database, id: &str, tools: &[String]) -> Result<Operation, AppError> {
    let _guard = crate::config_io::lock();
    let paths = native_paths()?;
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
    preview_at(db, id, &selected, Some(paths))
}
pub(crate) fn preview_at(
    db: &Database,
    id: &str,
    selected: &[(String, PathBuf)],
    paths: Option<BTreeMap<String, PathBuf>>,
) -> Result<Operation, AppError> {
    preview_at_adoption(db, id, selected, paths, &[])
}
pub(crate) fn preview_at_adoption(
    db: &Database,
    id: &str,
    selected: &[(String, PathBuf)],
    paths: Option<BTreeMap<String, PathBuf>>,
    adopt: &[String],
) -> Result<Operation, AppError> {
    if adopt
        .iter()
        .any(|tool| !selected.iter().any(|(selected, _)| selected == tool))
        || adopt.iter().collect::<BTreeSet<_>>().len() != adopt.len()
    {
        return Err(error("targetUnsupported"));
    }
    let _guard = crate::config_io::lock();
    let before = db.library_snapshot()?;
    let meta = before
        .definitions
        .mcp
        .get(id)
        .ok_or_else(|| error("resourceMissing"))?;
    let server = db
        .get_all_mcp_servers()?
        .shift_remove(id)
        .ok_or_else(|| error("resourceMissing"))?;
    let hash = spec_hash(&server.server)?;
    if hash != meta.hash {
        return Err(error("revisionChanged"));
    }
    let tools: BTreeSet<_> = selected.iter().map(|(tool, _)| tool.clone()).collect();
    if tools.len() != selected.len() {
        return Err(error("targetDuplicate"));
    }
    let desired_updates: BTreeMap<_, _> = tools
        .iter()
        .map(|tool| (assignment(id, tool), Some(id.to_string())))
        .chain(
            before
                .machine
                .assignments
                .iter()
                .filter(|(key, value)| {
                    *value == id
                        && key
                            .strip_prefix(&format!("mcp:{id}:"))
                            .is_some_and(|tool| !tools.contains(tool))
                })
                .map(|(key, _)| (key.clone(), None)),
        )
        .collect();
    let previous_assignments = desired_updates
        .keys()
        .map(|key| (key.clone(), before.machine.assignments.get(key).cloned()))
        .collect();
    let paths = paths.unwrap_or_default();
    let resolved_paths = paths
        .iter()
        .map(|(tool, path)| (tool.clone(), path.to_string_lossy().into_owned()))
        .collect();
    let resolved_physical: BTreeMap<_, _> = paths
        .iter()
        .map(|(tool, path)| Ok((tool.clone(), io::physical_id(path)?)))
        .collect::<Result<_, AppError>>()?;
    let global_paths = if paths.contains_key("opencode") {
        opencode_reader_paths()?
    } else {
        Vec::new()
    };
    let routing = if paths.contains_key("opencode") {
        Some(routing_dependencies()?)
    } else {
        None
    };
    let layered_readers = global_paths
        .iter()
        .map(|path| io::physical_id(path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for (tool, path) in selected {
        let physical = io::physical_id(path)?;
        let group = groups.entry(physical).or_default();
        group.path = path.clone();
        group.selected.insert(tool.clone());
    }
    for receipt in before
        .machine
        .receipts
        .values()
        .filter(|receipt| receipt.kind == ResourceKind::Mcp && receipt.resource_id == id)
    {
        let group = groups.entry(receipt.physical_target.clone()).or_default();
        if group.path.as_os_str().is_empty() {
            group.path = receipt.path.clone().into();
        }
        group.old = Some(receipt.clone());
    }
    let mut targets = Vec::new();
    for (physical, group) in groups {
        let old = group.old;
        let owners: Vec<_> = group.selected.iter().cloned().collect();
        let mut updates: BTreeMap<_, _> = owners
            .iter()
            .map(|tool| (assignment(id, tool), Some(id.to_string())))
            .collect();
        if let Some(old) = &old {
            for tool in &old.owners {
                if !group.selected.contains(tool)
                    && desired_updates.contains_key(&assignment(id, tool))
                {
                    updates.insert(
                        assignment(id, tool),
                        desired_updates[&assignment(id, tool)].clone(),
                    );
                }
            }
        }
        let readers: Vec<String> = owners
            .iter()
            .cloned()
            .chain(
                resolved_physical
                    .iter()
                    .filter(|(_, path)| *path == &physical)
                    .map(|(tool, _)| tool.clone()),
            )
            .chain(
                layered_readers
                    .iter()
                    .filter(|path| *path == &physical)
                    .map(|_| "opencode".to_string()),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut write = TargetWrite {
            directory: None,
            transfer: None,
            tools: owners.clone(),
            readers,
            path: group.path.to_string_lossy().into_owned(),
            physical_target: physical.clone(),
            receipt_key: format!("mcp:{physical}:{id}"),
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
        let result = (|| {
            if io::physical_id(&group.path)? != physical {
                return Err(error("targetChanged"));
            }
            if owners.iter().any(|tool| tool == "opencode") {
                opencode_scope_clear(id, &physical)?;
            }
            for source_path in &global_paths {
                let source_physical = io::physical_id(source_path)?;
                let selected_opencode = owners.iter().any(|tool| tool == "opencode");
                let source = if selected_opencode {
                    io::read(source_path)?
                } else {
                    None
                };
                write.source_fingerprints.insert(
                    source_path.to_string_lossy().into_owned(),
                    super::SourceFingerprint {
                        physical_target: source_physical.clone(),
                        hash: if selected_opencode {
                            io::fingerprint(source.as_deref().map(str::as_bytes))
                        } else {
                            String::new()
                        },
                    },
                );
                if selected_opencode && source_physical != physical {
                    if let Some(source) = source {
                        let root = mcp_codec::parse_native(&source, "opencode")?;
                        super::native_json::validate_routing_keys(&root, &["mcp"])?;
                        if root
                            .get("mcp")
                            .and_then(|servers| servers.get(id))
                            .is_some()
                        {
                            return Err(error("mcpNameConflict"));
                        }
                    }
                }
            }
            let text = io::read(&group.path)?;
            write.before_hash = io::fingerprint(text.as_deref().map(str::as_bytes));
            let owned = old
                .as_ref()
                .map(|receipt| {
                    serde_json::from_str::<mcp_codec::OwnedFields>(&receipt.owned_segment)
                        .map_err(|_| error("stateUnreadable"))
                })
                .transpose()?;
            let formatter = owned
                .as_ref()
                .map(|owned| owned.tool.as_str())
                .or_else(|| owners.first().map(String::as_str))
                .ok_or_else(|| error("stateUnreadable"))?;
            let native = if owners.is_empty() {
                None
            } else {
                let mut native = mcp_codec::native_spec(formatter, &server.server)?;
                if formatter == "claude" {
                    crate::claude_mcp::prepare_mcp_spec_for_path(&mut native, &group.path);
                }
                for tool in &owners {
                    let mut sibling = mcp_codec::native_spec(tool, &server.server)?;
                    if tool == "claude" {
                        crate::claude_mcp::prepare_mcp_spec_for_path(&mut sibling, &group.path);
                    }
                    if mcp_codec::section(tool) != mcp_codec::section(formatter)
                        || sibling != native
                    {
                        return Err(error("sharedFormatConflict"));
                    }
                }
                Some(native)
            };
            let (after, receipt) =
                if owned.is_none() && owners.iter().any(|tool| adopt.contains(tool)) {
                    let text = text.as_deref().ok_or_else(|| error("mcpNameConflict"))?;
                    let spec = native.as_ref().ok_or_else(|| error("invalidMcp"))?;
                    let adopted = mcp_codec::adopt_server(text, formatter, id, spec)?;
                    write.reason = Some("library.adoptionPreview".into());
                    (text.to_string(), Some(adopted))
                } else {
                    mcp_codec::patch_server(
                        text.as_deref(),
                        formatter,
                        id,
                        owned.as_ref(),
                        native.as_ref(),
                    )?
                };
            for reader in &write.readers {
                if mcp_codec::section(reader) != mcp_codec::section(formatter)
                    || mcp_codec::parse_native(&after, reader).is_err()
                {
                    return Err(error("sharedFormatConflict"));
                }
                if let Some(native) = &native {
                    let mut expected = mcp_codec::native_spec(reader, &server.server)
                        .map_err(|_| error("sharedFormatConflict"))?;
                    if reader == "claude" {
                        crate::claude_mcp::prepare_mcp_spec_for_path(&mut expected, &group.path);
                    }
                    if &expected != native {
                        return Err(error("sharedFormatConflict"));
                    }
                }
            }
            if after.len() > 2 * 1024 * 1024 {
                return Err(error("targetTooLarge"));
            }
            write.after_hash = io::fingerprint(Some(after.as_bytes()));
            write.after_text = after;
            write.receipt = receipt
                .map(|owned| {
                    Ok::<Receipt, AppError>(Receipt {
                        kind: ResourceKind::Mcp,
                        resource_id: id.into(),
                        resource_revision: meta.revision,
                        definition_hash: hash.clone(),
                        physical_target: physical,
                        path: write.path.clone(),
                        owners,
                        owned_segment: serde_json::to_string(&owned)
                            .map_err(|_| error("stateUnreadable"))?,
                    })
                })
                .transpose()?;
            Ok(())
        })();
        if let Err(reason) = result {
            write.status = "conflict".into();
            write.reason = Some(reason.to_string());
        }
        targets.push(write);
    }
    let op = Operation {
        kind: ResourceKind::Mcp,
        id: uuid::Uuid::new_v4().to_string(),
        epoch: before.machine.epoch,
        resource_id: id.into(),
        resource_revision: meta.revision,
        resource_hash: hash,
        created_at: chrono::Utc::now().timestamp_millis(),
        status: "previewed".into(),
        targets,
        desired_updates,
        previous_assignments,
        resolved_paths,
        resolved_physical,
        routing,
    };
    let mut next = before.clone();
    next.machine.operations.insert(op.id.clone(), op.clone());
    io::prune_history(&mut next, &op.id);
    db.library_commit(&before, &next)?;
    Ok(op)
}

/// Private values are represented by paths, never by a mask that can be written back.
fn field_ref(id: &str, revision: Option<u64>, path: &[String]) -> String {
    io::fingerprint(Some(
        serde_json::to_string(&(id, revision, path))
            .expect("serializable field reference")
            .as_bytes(),
    ))
}
fn field_views(
    server: &McpServer,
    revision: Option<u64>,
    path: Vec<String>,
    value: &Value,
    result: &mut Vec<Value>,
) {
    if let Some(map) = value.as_object().filter(|map| !map.is_empty()) {
        for (key, value) in map {
            let mut child = path.clone();
            child.push(key.clone());
            field_views(server, revision, child, value, result);
        }
    } else if !path.is_empty() {
        let value_type = match value {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        };
        result.push(json!({"path":path,"fieldRef":field_ref(&server.id,revision,&path),"valueType":value_type,"exists":true}));
    }
}
/// Values never leave the backend. Field references are bound to identity,
/// path and revision so a stale form cannot keep/clear a different definition.
pub fn definition_view(server: &McpServer, snapshot: &LibrarySnapshot) -> Value {
    let revision = snapshot.definitions.mcp.get(&server.id).map(|m| m.revision);
    let mut fields = Vec::new();
    field_views(server, revision, vec![], &server.server, &mut fields);
    let transport = match server.server.get("type") {
        Some(Value::String(value)) if ["stdio", "http", "sse"].contains(&value.as_str()) => {
            value.as_str()
        }
        Some(_) => "invalid",
        None if server.server.get("url").is_some() => "http",
        None => "stdio",
    };
    let credential_state = if ["claude", "codex", "gemini", "opencode", "hermes"]
        .iter()
        .any(|tool| mcp_codec::native_spec(tool, &server.server).is_ok())
    {
        "nativeEnvironmentRequired"
    } else {
        "needsMigration"
    };
    let (display_summary, icon_id) = super::display::mcp_display(&server.server);
    json!({"id":server.id,"name":server.name,"revision":revision,"transport":transport,"privateFields":server.server.as_object().map(|m|m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),"fields":fields,"credentialState":credential_state,"appOwned":mcp_codec::app_owned(&server.server),"displaySummary":display_summary,"iconId":icon_id})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn list_safe_summary_exposes_only_verified_runner_package_and_icon() {
        let db = Database::memory().unwrap();
        let snapshot = db.library_snapshot().unwrap();
        let mut server = McpServer {
            id: "custom".into(),
            name: "Not an identity proof".into(),
            server: json!({"command":"npx","args":["-y","chrome-devtools-mcp@latest","--token","private-value"],"env":{"TOKEN":"private-value"}}),
            apps: McpApps::default(),
            description: None,
            homepage: None,
            docs: None,
            tags: vec![],
        };
        let view = definition_view(&server, &snapshot);
        assert_eq!(view["displaySummary"], "npx · chrome-devtools-mcp");
        assert_eq!(view["iconId"], "chrome-devtools");
        assert!(!view.to_string().contains("private-value"));
        server.server = json!({"command":"/private/person/tool","args":["private-value"]});
        let view = definition_view(&server, &snapshot);
        assert!(view["displaySummary"].is_null());
        assert!(view["iconId"].is_null());
        assert!(!view.to_string().contains("/private/person"));
        server.server = json!({"type":"http","url":"https://mcp.context7.com/private-value?key=private-value","headers":{"Authorization":"private-value"}});
        let view = definition_view(&server, &snapshot);
        assert_eq!(view["displaySummary"], "mcp.context7.com");
        assert_eq!(view["iconId"], "context7");
        assert!(!view.to_string().contains("private-value"));
        server.server = json!({"command":"sh","args":["-c","npx chrome-devtools-mcp"]});
        assert!(definition_view(&server, &snapshot)["displaySummary"].is_null());
    }
    #[test]
    #[serial_test::serial]
    fn recovery_does_not_claim_writes_when_inline_routing_changed() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        fixture(&db);
        let mut op = preview(&db, "demo", &["opencode".into()]).unwrap();
        op.status = "prepared".into();
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        for (key, value) in &op.desired_updates {
            if let Some(value) = value {
                next.machine.assignments.insert(key.clone(), value.clone());
            }
        }
        next.machine.operations.insert(op.id.clone(), op.clone());
        db.library_commit(&before, &next).unwrap();
        crate::config::atomic_write(
            std::path::Path::new(&op.targets[0].path),
            op.targets[0].after_text.as_bytes(),
        )
        .unwrap();
        std::env::set_var(
            "OPENCODE_CONFIG_CONTENT",
            r#"{"mcp":{"demo":{"enabled":false}}}"#,
        );
        let recovered = super::super::recover_operation(&db, &op.id).unwrap();
        assert_eq!(recovered.status, "needsReview");
        assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
    }
    #[test]
    #[serial_test::serial]
    fn opencode_late_layers_and_inline_overrides_cannot_claim_applied() {
        let sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        fixture(&db);
        let late = sandbox.home().join(".opencode/opencode.json");
        crate::config::atomic_write(&late, br#"{"mcp":{"demo":{"enabled":false}}}"#).unwrap();
        let op = preview(&db, "demo", &["opencode".into()]).unwrap();
        assert_eq!(op.targets[0].status, "conflict");
        std::fs::remove_file(late).unwrap();
        std::env::set_var(
            "OPENCODE_CONFIG_CONTENT",
            r#"{"mcp":{"demo":{"enabled":false}}}"#,
        );
        let op = preview(&db, "demo", &["opencode".into()]).unwrap();
        assert_eq!(op.targets[0].status, "conflict");
        assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn opencode_dynamic_routing_keys_are_not_treated_as_literal_names() {
        let sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        fixture(&db);
        std::env::set_var(
            "OPENCODE_CONFIG_CONTENT",
            r#"{"mcp":{"{env:MCP_NAME}":{"enabled":false}}}"#,
        );
        assert_eq!(
            preview(&db, "demo", &["opencode".into()]).unwrap().targets[0].status,
            "conflict"
        );
        std::env::remove_var("OPENCODE_CONFIG_CONTENT");
        crate::config::atomic_write(
            &sandbox.home().join(".opencode/opencode.json"),
            br#"{"mcp":{"{file:name.txt}":{"enabled":false}}}"#,
        )
        .unwrap();
        assert_eq!(
            preview(&db, "demo", &["opencode".into()]).unwrap().targets[0].status,
            "conflict"
        );
    }

    #[test]
    #[serial_test::serial]
    fn new_opencode_late_layers_and_environment_require_reconfirmation() {
        let sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        fixture(&db);
        for case in 0..2 {
            let op = preview(&db, "demo", &["opencode".into()]).unwrap();
            if case == 0 {
                crate::config::atomic_write(
                    &sandbox.home().join(".opencode/opencode.json"),
                    br#"{"mcp":{"demo":{"enabled":false}}}"#,
                )
                .unwrap();
            } else {
                std::env::set_var(
                    "OPENCODE_CONFIG_CONTENT",
                    r#"{"mcp":{"demo":{"enabled":false}}}"#,
                );
            }
            let result = super::super::apply_operation(&db, &op.id);
            assert!(
                result.is_err() || result.unwrap().status != "applied",
                "case {case}"
            );
            assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
            assert!(!crate::opencode_config::get_opencode_config_path().exists());
            let _ = std::fs::remove_file(sandbox.home().join(".opencode/opencode.json"));
        }
    }
    fn fixture(db: &Database) -> McpServer {
        let server = McpServer {
            id: "demo".into(),
            name: "Demo".into(),
            server: json!({"command":"server","env":{"TOKEN":"${TOKEN}"}}),
            apps: McpApps::default(),
            description: None,
            homepage: None,
            docs: None,
            tags: vec![],
        };
        save_draft(db, server.clone(), None).unwrap();
        server
    }
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn opencode_reads_its_shared_lower_layer_even_with_another_active_file() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        fixture(&db);
        let dir = crate::opencode_config::get_opencode_dir();
        let gemini = crate::gemini_config::get_gemini_settings_path();
        crate::config::atomic_write(&gemini, b"{}").unwrap();
        crate::config::atomic_write(&dir.join("opencode.jsonc"), b"{}").unwrap();
        std::os::unix::fs::symlink(&gemini, dir.join("config.json")).unwrap();
        let op = preview(&db, "demo", &["gemini".into()]).unwrap();
        assert!(op.targets[0]
            .readers
            .iter()
            .any(|reader| reader == "opencode"));
        assert_eq!(op.targets[0].status, "conflict");
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn a_new_opencode_lower_layer_reader_invalidates_an_old_preview() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        fixture(&db);
        let dir = crate::opencode_config::get_opencode_dir();
        let gemini = crate::gemini_config::get_gemini_settings_path();
        crate::config::atomic_write(&gemini, b"{}").unwrap();
        crate::config::atomic_write(&dir.join("opencode.jsonc"), b"{}").unwrap();
        let op = preview(&db, "demo", &["gemini".into()]).unwrap();
        std::os::unix::fs::symlink(&gemini, dir.join("config.json")).unwrap();
        assert!(super::super::apply_operation(&db, &op.id).is_err());
        assert_eq!(std::fs::read_to_string(gemini).unwrap(), "{}");
    }

    #[test]
    #[serial_test::serial]
    fn a_shared_target_must_be_valid_for_unselected_readers() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        fixture(&db);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared");
        let paths = BTreeMap::from([
            ("hermes".into(), path.clone()),
            ("gemini".into(), path.clone()),
        ]);
        let op = preview_at(&db, "demo", &[("hermes".into(), path)], Some(paths)).unwrap();
        assert_eq!(op.targets[0].status, "conflict");
        assert_eq!(
            op.targets[0].reason.as_deref(),
            Some("配置错误: library.sharedFormatConflict")
        );
    }

    #[test]
    fn private_field_views_support_editing_without_returning_values() {
        let db = Database::memory().unwrap();
        let mut server = fixture(&db);
        server.server["env"]["TOKEN"] = json!("private-sentinel");
        save_draft(&db, server.clone(), Some(1)).unwrap();
        let view = definition_view(&server, &db.library_snapshot().unwrap());
        assert!(!view.to_string().contains("private-sentinel"));
        let fields = view["fields"].as_array().expect("typed field references");
        let token = fields
            .iter()
            .find(|field| field["path"] == json!(["env", "TOKEN"]))
            .unwrap();
        assert_eq!(token["valueType"], "string");
        assert!(token["fieldRef"].as_str().unwrap().len() >= 32);
        server.server["type"] = json!(17);
        assert_eq!(
            definition_view(&server, &db.library_snapshot().unwrap())["transport"],
            "invalid"
        );
    }

    #[test]
    #[serial_test::serial]
    fn opencode_checks_every_global_layer_before_assigning() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        fixture(&db);
        let dir = crate::opencode_config::get_opencode_dir();
        crate::config::atomic_write(
            &dir.join("config.json"),
            br#"{"mcp":{"demo":{"type":"local","command":["personal"]}}}"#,
        )
        .unwrap();
        crate::config::atomic_write(&dir.join("opencode.jsonc"), b"{}").unwrap();
        let op = preview(&db, "demo", &["opencode".into()]).unwrap();
        assert_eq!(op.targets[0].status, "conflict");
        assert_eq!(
            std::fs::read_to_string(dir.join("opencode.jsonc")).unwrap(),
            "{}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn opencode_lower_layer_changes_invalidate_confirmation() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        fixture(&db);
        let dir = crate::opencode_config::get_opencode_dir();
        crate::config::atomic_write(&dir.join("opencode.jsonc"), b"{}").unwrap();
        let op = preview(&db, "demo", &["opencode".into()]).unwrap();
        assert_eq!(op.targets[0].status, "pending");
        crate::config::atomic_write(
            &dir.join("config.json"),
            br#"{"mcp":{"demo":{"type":"local","command":["personal"]}}}"#,
        )
        .unwrap();
        let applied = super::super::apply_operation(&db, &op.id).unwrap();
        assert_eq!(applied.targets[0].status, "conflict");
        assert_eq!(
            std::fs::read_to_string(dir.join("opencode.jsonc")).unwrap(),
            "{}"
        );
        assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
    }

    #[test]
    fn actual_redaction_markers_and_invalid_transport_never_cross_edit_view() {
        let db = Database::memory().unwrap();
        fixture(&db);
        let before = db.library_snapshot().unwrap();
        for value in [
            json!(crate::redact::REDACTED),
            json!([crate::redact::REDACTED]),
            json!({"nested":crate::redact::REDACTED}),
        ] {
            assert!(save_edits(
                &db,
                "demo",
                "bad",
                Some(1),
                &[FieldEdit {
                    field_ref: None,
                    path: vec!["args".into()],
                    action: "set".into(),
                    value: Some(value)
                }]
            )
            .is_err());
            assert_eq!(db.library_snapshot().unwrap(), before);
        }
    }
    #[test]
    fn invalid_transport_values_stay_private() {
        let db = Database::memory().unwrap();
        let mut server = fixture(&db);
        server.server["type"] = json!("private-transport-sentinel");
        assert!(!definition_view(&server, &db.library_snapshot().unwrap())
            .to_string()
            .contains("private-transport-sentinel"));
    }

    #[test]
    fn overlapping_keep_set_clear_and_redacted_values_are_rejected() {
        let db = Database::memory().unwrap();
        fixture(&db);
        let before = db.library_snapshot().unwrap();
        let secret = db.get_all_mcp_servers().unwrap()["demo"].server.clone();
        for action in ["set", "clear"] {
            for reverse in [false, true] {
                let mut edits = vec![
                    FieldEdit {
                        field_ref: None,
                        path: vec!["env".into(), "TOKEN".into()],
                        action: "keep".into(),
                        value: None,
                    },
                    FieldEdit {
                        field_ref: None,
                        path: vec!["env".into()],
                        action: action.into(),
                        value: (action == "set").then(|| json!({})),
                    },
                ];
                if reverse {
                    edits.reverse();
                }
                assert!(save_edits(&db, "demo", "bad", Some(1), &edits).is_err());
                assert_eq!(db.library_snapshot().unwrap(), before);
                assert_eq!(db.get_all_mcp_servers().unwrap()["demo"].server, secret);
            }
        }
        for value in [
            json!(crate::redact::REDACTED),
            json!([crate::redact::REDACTED]),
            json!({"nested":crate::redact::REDACTED}),
        ] {
            assert!(save_edits(
                &db,
                "demo",
                "bad",
                Some(1),
                &[FieldEdit {
                    field_ref: None,
                    path: vec!["args".into()],
                    action: "set".into(),
                    value: Some(value)
                }]
            )
            .is_err());
            assert_eq!(db.library_snapshot().unwrap(), before);
        }
        let mut server = db
            .get_all_mcp_servers()
            .unwrap()
            .shift_remove("demo")
            .unwrap();
        server.server["type"] = json!("private-transport-sentinel");
        assert!(!definition_view(&server, &before)
            .to_string()
            .contains("private-transport-sentinel"));
    }

    #[test]
    fn private_edits_keep_set_and_clear_have_explicit_revisioned_semantics() {
        let db = Database::memory().unwrap();
        let mut server = fixture(&db);
        server.server =
            json!({"command":"server","env":{"TOKEN":"old-private"},"TOKEN":"personal-root"});
        save_draft(&db, server, Some(1)).unwrap();
        save_edits(
            &db,
            "demo",
            "Renamed",
            Some(2),
            &[FieldEdit {
                field_ref: None,
                path: vec!["env".into(), "TOKEN".into()],
                action: "keep".into(),
                value: None,
            }],
        )
        .unwrap();
        assert_eq!(
            db.get_all_mcp_servers().unwrap()["demo"].server["env"]["TOKEN"],
            "old-private"
        );
        assert!(save_edits(&db, "demo", "Stale", Some(2), &[]).is_err());
        save_edits(
            &db,
            "demo",
            "Renamed",
            Some(3),
            &[FieldEdit {
                field_ref: None,
                path: vec!["env".into(), "TOKEN".into()],
                action: "set".into(),
                value: Some(json!("${TOKEN}")),
            }],
        )
        .unwrap();
        save_edits(
            &db,
            "demo",
            "Renamed",
            Some(4),
            &[FieldEdit {
                field_ref: None,
                path: vec!["env".into()],
                action: "clear".into(),
                value: None,
            }],
        )
        .unwrap();
        save_edits(
            &db,
            "demo",
            "Renamed",
            Some(5),
            &[FieldEdit {
                field_ref: None,
                path: vec!["env".into(), "TOKEN".into()],
                action: "clear".into(),
                value: None,
            }],
        )
        .unwrap();
        assert_eq!(
            db.get_all_mcp_servers().unwrap()["demo"].server["TOKEN"],
            "personal-root"
        );
    }

    #[test]
    fn moving_an_owner_uses_consistent_desired_for_old_and_new_targets() {
        let db = Database::memory().unwrap();
        fixture(&db);
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.json");
        let b = dir.path().join("b.json");
        let first = preview_at(&db, "demo", &[("claude".into(), a.clone())], None).unwrap();
        super::super::apply_operation(&db, &first.id).unwrap();
        let moved = preview_at(&db, "demo", &[("claude".into(), b.clone())], None).unwrap();
        let old = moved
            .targets
            .iter()
            .find(|target| target.path == a.to_string_lossy())
            .unwrap();
        assert_eq!(
            old.assignment_updates.get("mcp:demo:claude"),
            moved.desired_updates.get("mcp:demo:claude")
        );
    }

    #[test]
    fn preview_apply_withdraw_and_private_projection_use_durable_state() {
        let db = Database::memory().unwrap();
        fixture(&db);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"model":"personal"}"#).unwrap();
        let op = preview_at(&db, "demo", &[("claude".into(), path.clone())], None).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"model":"personal"}"#
        );
        let result = super::super::apply_operation(&db, &op.id).unwrap();
        assert_eq!(result.status, "applied");
        assert_eq!(db.library_snapshot().unwrap().machine.receipts.len(), 1);
        let op = preview_at(&db, "demo", &[], None).unwrap();
        assert_eq!(
            super::super::apply_operation(&db, &op.id).unwrap().status,
            "applied"
        );
        assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
        let root =
            mcp_codec::parse_native(&std::fs::read_to_string(&path).unwrap(), "claude").unwrap();
        assert_eq!(root["model"], "personal");
        assert!(root["mcpServers"]["demo"].get("command").is_none());
        let mut server = db
            .get_all_mcp_servers()
            .unwrap()
            .shift_remove("demo")
            .unwrap();
        server.server = json!({"command":"private-command","args":["private-arg"],"env":{"SECRET":"private-env"},"url":"https://private.example?key=private-url","custom":"private-custom"});
        let dto = definition_view(&server, &db.library_snapshot().unwrap()).to_string();
        for value in [
            "private-command",
            "private-arg",
            "private-env",
            "private-url",
            "private-custom",
        ] {
            assert!(!dto.contains(value));
        }
    }
    #[test]
    fn stale_preview_and_unowned_native_name_never_write() {
        let db = Database::memory().unwrap();
        let mut server = fixture(&db);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"mcpServers":{"demo":{"command":"user"}}}"#).unwrap();
        let original = std::fs::read_to_string(&path).unwrap();
        let op = preview_at(&db, "demo", &[("claude".into(), path.clone())], None).unwrap();
        assert_eq!(op.targets[0].status, "conflict");
        assert_eq!(
            super::super::apply_operation(&db, &op.id).unwrap().status,
            "partial"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        server.server["command"] = json!("new");
        save_draft(&db, server, Some(1)).unwrap();
        assert!(super::super::apply_operation(&db, &op.id).is_err());
    }
    fn row(id: &str, name: &str, server: Value) -> McpServer {
        McpServer {
            id: id.into(),
            name: name.into(),
            server,
            apps: McpApps::default(),
            description: None,
            homepage: None,
            docs: None,
            tags: vec![],
        }
    }
    #[test]
    fn match_hash_ignores_empty_collections_and_an_implied_type() {
        let stored = json!({"command":"codegraph","args":["serve","--mcp"],"env":{},"headers":{}});
        let native = json!({"type":"stdio","command":"codegraph","args":["serve","--mcp"]});
        assert_eq!(match_hash(&stored).unwrap(), match_hash(&native).unwrap());
        assert_ne!(
            match_hash(&stored).unwrap(),
            match_hash(&json!({"type":"stdio","command":"codegraph"})).unwrap()
        );
        assert_eq!(
            match_hash(&json!({"url":"https://x.test/mcp","headers":{}})).unwrap(),
            match_hash(&json!({"type":"http","url":"https://x.test/mcp"})).unwrap()
        );
    }
    #[test]
    fn a_stored_enabled_switch_does_not_block_distribution() {
        let db = Database::memory().unwrap();
        let snapshot = db.library_snapshot().unwrap();
        let server = row(
            "local",
            "local",
            json!({"type":"stdio","command":"server","enabled":false}),
        );
        assert_eq!(
            definition_view(&server, &snapshot)["credentialState"],
            "nativeEnvironmentRequired"
        );
        assert!(mcp_codec::native_spec("claude", &server.server)
            .unwrap()
            .get("enabled")
            .is_none());
        assert_eq!(
            match_hash(&server.server).unwrap(),
            match_hash(&json!({"type":"stdio","command":"server"})).unwrap()
        );
    }
    #[test]
    fn rows_an_app_writes_for_itself_are_marked_app_owned() {
        let db = Database::memory().unwrap();
        let snapshot = db.library_snapshot().unwrap();
        for (command, owned) in [
            (
                "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl",
                true,
            ),
            (
                "./Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient",
                true,
            ),
            (
                r"C:\Program Files\WindowsApps\OpenAI.Codex\app\resources\node_repl.exe",
                true,
            ),
            ("npx", false),
            ("/usr/local/bin/codegraph", false),
        ] {
            let server = row("x", "x", json!({"type":"stdio","command":command}));
            assert_eq!(definition_view(&server, &snapshot)["appOwned"], owned, "{command}");
        }
        let remote = row(
            "r",
            "r",
            json!({"type":"http","url":"https://x.app/contents/mcp"}),
        );
        assert_eq!(definition_view(&remote, &snapshot)["appOwned"], false);
    }
    #[test]
    fn ensure_metadata_drops_an_identical_generated_duplicate() {
        let db = Database::memory().unwrap();
        let config = json!({"type":"stdio","command":"codegraph","args":["serve","--mcp"]});
        let mut with_empty_env = config.clone();
        with_empty_env["env"] = json!({});
        save_draft(&db, row("codegraph", "codegraph", with_empty_env), None).unwrap();
        save_draft(
            &db,
            row("codegraph--e92a6b623361891f", "codegraph", config.clone()),
            None,
        )
        .unwrap();
        save_draft(&db, row("codegraph-copy", "codegraph", config), None).unwrap();
        ensure_metadata(&db).unwrap();
        let ids = db
            .get_all_mcp_servers()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["codegraph", "codegraph-copy"]);
        assert!(!db
            .library_snapshot()
            .unwrap()
            .definitions
            .mcp
            .contains_key("codegraph--e92a6b623361891f"));
    }
    #[test]
    fn ensure_metadata_keeps_duplicates_that_differ_or_are_assigned() {
        let db = Database::memory().unwrap();
        let config = json!({"type":"stdio","command":"codegraph","args":["serve","--mcp"]});
        save_draft(&db, row("codegraph", "codegraph", config.clone()), None).unwrap();
        save_draft(
            &db,
            row("codegraph--0123456789abcdef", "codegraph", config.clone()),
            None,
        )
        .unwrap();
        save_draft(
            &db,
            row(
                "codegraph--fedcba9876543210",
                "codegraph",
                json!({"type":"stdio","command":"codegraph","args":["serve"]}),
            ),
            None,
        )
        .unwrap();
        let before = db.library_snapshot().unwrap();
        let mut next = before.clone();
        next.machine.assignments.insert(
            assignment("codegraph--0123456789abcdef", "codex"),
            "desired".into(),
        );
        db.library_commit(&before, &next).unwrap();
        ensure_metadata(&db).unwrap();
        assert_eq!(db.get_all_mcp_servers().unwrap().len(), 3);
    }
}
