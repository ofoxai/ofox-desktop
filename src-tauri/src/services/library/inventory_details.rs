//! Ticket-scoped source inspection. Values stay private even in comparison responses.
use super::*;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

#[derive(Clone, Debug)]
pub(super) enum SourceLocation {
    File(PathBuf),
    Directory(PathBuf),
    Inline,
    Legacy,
}
impl SourceLocation {
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Self::File(_) => "file",
            Self::Directory(_) => "directory",
            Self::Inline => "inline",
            Self::Legacy => "legacy",
        }
    }
    pub(super) fn short_path(&self) -> Option<String> {
        match self {
            Self::File(path) | Self::Directory(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Self::Inline | Self::Legacy => None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldShape {
    pub path: Vec<String>,
    pub value_type: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventorySourceDetail {
    pub ticket_id: String,
    pub entry_id: String,
    pub source_ref: String,
    pub tool_id: String,
    pub label: String,
    pub location_kind: String,
    pub display_path: Option<String>,
    pub logical_path: Option<String>,
    pub canonical_path: Option<String>,
    pub is_symlink: bool,
    pub observed_at_ms: i64,
    pub can_reveal: bool,
    pub matched_resource_ids: Vec<String>,
    pub fields: Vec<FieldShape>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum McpComparisonSubject {
    Native {
        ticket_id: String,
        entry_id: String,
    },
    Resource {
        resource_id: String,
        expected_revision: u64,
    },
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparedField {
    pub path: Vec<String>,
    pub left_type: Option<String>,
    pub right_type: Option<String>,
    pub state: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpComparison {
    pub fields: Vec<ComparedField>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpRecipe {
    pub recipe_id: String,
    pub name: Option<String>,
    pub definition: Value,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRecipeMatch {
    pub recipe_id: String,
    pub exact_resource_ids: Vec<String>,
    pub native_entry_ids: Vec<String>,
    pub similar_resource_ids: Vec<String>,
    pub similar_native_entry_ids: Vec<String>,
}

pub(super) fn verify_context(
    db: &Database,
    ticket: &Ticket,
    expected_epoch: u64,
) -> Result<(), AppError> {
    if ticket.owner != db as *const Database as usize
        || ticket.root != crate::config::get_app_config_dir()
    {
        return Err(error("inventoryExpired"));
    }
    if ticket.epoch != expected_epoch {
        return Err(error("epochChanged"));
    }
    if ticket.definitions_revision != db.library_snapshot()?.definitions.revision
        || readers::routing_signature(&ticket.kind).map_err(|_| error("inventoryChanged"))?
            != ticket.routing
    {
        return Err(error("inventoryChanged"));
    }
    Ok(())
}
fn ticket_candidate(
    db: &Database,
    ticket_id: &str,
    entry_id: &str,
    expected_epoch: u64,
) -> Result<(Candidate, Vec<InventorySource>, i64), AppError> {
    super::super::catalog::verify_epoch(db, expected_epoch)?;
    let (candidate, sources, observed) = {
        let mut tickets = pending()
            .lock()
            .map_err(|_| error("inventoryUnavailable"))?;
        prune(&mut tickets);
        let ticket = tickets
            .get(ticket_id)
            .ok_or_else(|| error("inventoryExpired"))?;
        verify_context(db, ticket, expected_epoch)?;
        let candidate = ticket
            .candidates
            .get(entry_id)
            .ok_or_else(|| error("inventorySelectionInvalid"))?
            .clone();
        (candidate, ticket.sources.clone(), ticket.observed_at_ms)
    };
    verify(db, &candidate).map_err(|_| error("inventoryChanged"))?;
    Ok((candidate, sources, observed))
}
fn source_path(candidate: &Candidate, source: &InventorySource) -> Option<PathBuf> {
    match source.location.as_ref()? {
        SourceLocation::File(path) => Some(path.clone()),
        SourceLocation::Directory(root) => candidate.proofs.iter().find_map(|proof| match proof {
            Proof::Tree { path, .. } if path.starts_with(root) => Some(path.clone()),
            _ => None,
        }),
        SourceLocation::Inline | SourceLocation::Legacy => None,
    }
}
fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
fn fields<'a>(
    value: &'a Value,
    prefix: Vec<String>,
    output: &mut BTreeMap<Vec<String>, &'a Value>,
    depth: usize,
) -> Result<(), AppError> {
    if depth > 32 || output.len() > 4096 {
        return Err(error("inventoryLimit"));
    }
    if let Some(map) = value.as_object().filter(|map| !map.is_empty()) {
        if map.len() > 4096 {
            return Err(error("inventoryLimit"));
        }
        for (key, value) in map {
            if key.len() > 1024 {
                return Err(error("inventoryLimit"));
            }
            let mut path = prefix.clone();
            path.push(key.clone());
            fields(value, path, output, depth + 1)?;
        }
    } else if !prefix.is_empty() {
        output.insert(prefix, value);
    }
    Ok(())
}
fn shapes(candidate: &Candidate) -> Result<Vec<FieldShape>, AppError> {
    let Definition::Mcp { spec } = &candidate.definition else {
        return Ok(vec![]);
    };
    let mut rows = BTreeMap::new();
    fields(spec, vec![], &mut rows, 0)?;
    if rows.len() > 4096 {
        return Err(error("inventoryLimit"));
    }
    Ok(rows
        .into_iter()
        .map(|(path, value)| FieldShape {
            path,
            value_type: value_type(value).into(),
        })
        .collect())
}
fn matched_ids(db: &Database, candidate: &Candidate) -> Result<Vec<String>, AppError> {
    let snapshot = db.library_snapshot()?;
    Ok(match &candidate.definition {
        Definition::Instructions { content } => snapshot
            .definitions
            .instructions
            .values()
            .filter(|row| row.content == *content)
            .map(|row| row.id.clone())
            .collect(),
        Definition::Mcp { spec } => {
            let hash = super::super::mcp_resource::match_hash(spec)?;
            db.get_all_mcp_servers()?
                .values()
                .filter(|row| {
                    mcp_alias_matches(&row.id, &candidate.view.name, &candidate.hash)
                        && super::super::mcp_resource::match_hash(&row.server)
                            .ok()
                            .as_ref()
                            == Some(&hash)
                })
                .map(|row| row.id.clone())
                .collect()
        }
        Definition::Skill { hash, .. } => db
            .get_all_installed_skills()?
            .values()
            .filter(|row| row.content_hash.as_ref() == Some(hash))
            .map(|row| row.id.clone())
            .collect(),
    })
}
/// Explicit detail is the only inventory response which includes complete filesystem locations.
pub fn source_detail(
    db: &Database,
    ticket_id: &str,
    entry_id: &str,
    source_ref: &str,
    expected_epoch: u64,
) -> Result<InventorySourceDetail, AppError> {
    let _io = crate::config_io::lock();
    let (candidate, sources, observed_at_ms) =
        ticket_candidate(db, ticket_id, entry_id, expected_epoch)?;
    let source = sources
        .iter()
        .find(|source| {
            source.source_ref.as_deref() == Some(source_ref)
                && candidate.view.sources.iter().any(|entry| {
                    entry.source_id == source.source_id && entry.tool_id == source.tool_id
                })
        })
        .ok_or_else(|| error("inventorySelectionInvalid"))?;
    let location = source
        .location
        .as_ref()
        .ok_or_else(|| error("inventorySourceUnsupported"))?;
    let path = source_path(&candidate, source);
    let canonical = path
        .as_ref()
        .map(|path| path.canonicalize().map_err(|_| error("inventoryChanged")))
        .transpose()?;
    let is_symlink = path
        .as_ref()
        .zip(canonical.as_ref())
        .is_some_and(|(path, canonical)| path != canonical);
    Ok(InventorySourceDetail {
        ticket_id: ticket_id.into(),
        entry_id: entry_id.into(),
        source_ref: source_ref.into(),
        tool_id: source.tool_id.clone(),
        label: source.label.clone(),
        location_kind: location.kind().into(),
        display_path: source.display_path.clone(),
        logical_path: path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        canonical_path: canonical
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        is_symlink,
        observed_at_ms,
        can_reveal: path.is_some(),
        matched_resource_ids: matched_ids(db, &candidate)?,
        fields: shapes(&candidate)?,
    })
}
/// Run a caller-owned reveal operation while source validation still holds the IO barrier.
pub fn reveal_source<F>(
    db: &Database,
    ticket_id: &str,
    entry_id: &str,
    source_ref: &str,
    expected_epoch: u64,
    reveal: F,
) -> Result<(), AppError>
where
    F: FnOnce(&Path) -> Result<(), AppError>,
{
    let _io = crate::config_io::lock();
    let detail = source_detail(db, ticket_id, entry_id, source_ref, expected_epoch)?;
    if !detail.can_reveal {
        return Err(error("inventorySourceUnsupported"));
    }
    let path = detail
        .canonical_path
        .ok_or_else(|| error("inventorySourceUnsupported"))?;
    reveal(Path::new(&path))
}
fn comparison_spec(
    db: &Database,
    subject: &McpComparisonSubject,
    epoch: u64,
) -> Result<Value, AppError> {
    match subject {
        McpComparisonSubject::Native {
            ticket_id,
            entry_id,
        } => {
            let (candidate, _, _) = ticket_candidate(db, ticket_id, entry_id, epoch)?;
            match candidate.definition {
                Definition::Mcp { spec } => Ok(spec),
                _ => Err(error("inventorySelectionInvalid")),
            }
        }
        McpComparisonSubject::Resource {
            resource_id,
            expected_revision,
        } => {
            let snapshot = db.library_snapshot()?;
            if snapshot
                .definitions
                .mcp
                .get(resource_id)
                .map(|row| row.revision)
                != Some(*expected_revision)
            {
                return Err(error("revisionChanged"));
            }
            let rows = db.get_all_mcp_servers()?;
            let row = rows
                .get(resource_id)
                .ok_or_else(|| error("revisionChanged"))?;
            let meta = snapshot
                .definitions
                .mcp
                .get(resource_id)
                .ok_or_else(|| error("revisionChanged"))?;
            if super::super::mcp_resource::spec_hash(&row.server)? != meta.hash {
                return Err(error("revisionChanged"));
            }
            Ok(row.server.clone())
        }
    }
}
pub fn compare_mcp(
    db: &Database,
    left: &McpComparisonSubject,
    right: &McpComparisonSubject,
    expected_epoch: u64,
) -> Result<McpComparison, AppError> {
    let _io = crate::config_io::lock();
    super::super::catalog::verify_epoch(db, expected_epoch)?;
    let left = comparison_spec(db, left, expected_epoch)?;
    let right = comparison_spec(db, right, expected_epoch)?;
    let mut l = BTreeMap::new();
    fields(&left, vec![], &mut l, 0)?;
    let mut r = BTreeMap::new();
    fields(&right, vec![], &mut r, 0)?;
    let paths: BTreeSet<_> = l.keys().chain(r.keys()).cloned().collect();
    if paths.len() > 4096 {
        return Err(error("inventoryLimit"));
    }
    Ok(McpComparison {
        fields: paths
            .into_iter()
            .map(|path| {
                let left = l.get(&path);
                let right = r.get(&path);
                let state = match (left, right) {
                    (Some(l), Some(r)) if l == r => "equal",
                    (Some(_), Some(_)) => "different",
                    (None, Some(_)) => "missingLeft",
                    _ => "missingRight",
                };
                ComparedField {
                    path,
                    left_type: left.map(|v| value_type(v).into()),
                    right_type: right.map(|v| value_type(v).into()),
                    state: state.into(),
                }
            })
            .collect(),
    })
}
pub fn mcp_service_host(value: &Value) -> Option<String> {
    let url = url::Url::parse(value.get("url")?.as_str()?).ok()?;
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.host_str()?.to_ascii_lowercase())
}
fn similar(recipe: &McpRecipe, name: &str, id: &str, spec: &Value) -> bool {
    recipe
        .name
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(name))
        || recipe.recipe_id.eq_ignore_ascii_case(id)
        || mcp_service_host(&recipe.definition)
            .zip(mcp_service_host(spec))
            .is_some_and(|(left, right)| left == right)
}
pub fn match_recipes(
    db: &Database,
    recipes: &[McpRecipe],
    inventory_ticket_id: Option<&str>,
    expected_epoch: u64,
) -> Result<Vec<McpRecipeMatch>, AppError> {
    let _io = crate::config_io::lock();
    super::super::catalog::verify_epoch(db, expected_epoch)?;
    if recipes.is_empty()
        || recipes.len() > 32
        || recipes
            .iter()
            .map(|recipe| &recipe.recipe_id)
            .collect::<BTreeSet<_>>()
            .len()
            != recipes.len()
    {
        return Err(error("inventorySelectionInvalid"));
    }
    let mut recipe_bytes = 0usize;
    for recipe in recipes {
        recipe_bytes = recipe_bytes.saturating_add(
            serde_json::to_vec(&recipe.definition)
                .map_err(|_| error("invalidMcp"))?
                .len(),
        );
        if recipe_bytes > 4 * 1024 * 1024
            || recipe.name.as_ref().is_some_and(|name| name.len() > 512)
            || recipe.recipe_id.is_empty()
            || recipe.recipe_id.len() > 128
            || !recipe.definition.is_object()
            || serde_json::to_vec(&recipe.definition)
                .map_err(|_| error("invalidMcp"))?
                .len()
                > 1024 * 1024
        {
            return Err(error("invalidMcp"));
        }
    }
    let (native, source_proofs) = if let Some(id) = inventory_ticket_id {
        let mut tickets = pending()
            .lock()
            .map_err(|_| error("inventoryUnavailable"))?;
        prune(&mut tickets);
        let ticket = tickets.get(id).ok_or_else(|| error("inventoryExpired"))?;
        verify_context(db, ticket, expected_epoch)?;
        if ticket.kind != ResourceKind::Mcp {
            return Err(error("inventorySelectionInvalid"));
        }
        if ticket
            .sources
            .iter()
            .any(|source| ["unreadable", "unsupported"].contains(&source.state.as_str()))
        {
            return Err(error("inventorySourceUnreadable"));
        }
        (
            ticket.candidates.values().cloned().collect::<Vec<_>>(),
            ticket.source_proofs.clone(),
        )
    } else {
        (vec![], vec![])
    };
    // Include empty and missing sources, then reread each physical proof only once.
    let proofs: BTreeSet<_> = source_proofs
        .iter()
        .chain(native.iter().flat_map(|candidate| candidate.proofs.iter()))
        .collect();
    for proof in proofs {
        verify_proof(db, proof).map_err(|_| error("inventoryChanged"))?;
    }
    let library = db.get_all_mcp_servers()?;
    recipes
        .iter()
        .map(|recipe| {
            let hash = super::super::mcp_resource::match_hash(&recipe.definition)?;
            let mut result = McpRecipeMatch {
                recipe_id: recipe.recipe_id.clone(),
                exact_resource_ids: vec![],
                native_entry_ids: vec![],
                similar_resource_ids: vec![],
                similar_native_entry_ids: vec![],
            };
            for row in library.values() {
                if super::super::mcp_resource::match_hash(&row.server)? == hash {
                    result.exact_resource_ids.push(row.id.clone());
                } else if similar(recipe, &row.name, &row.id, &row.server) {
                    result.similar_resource_ids.push(row.id.clone());
                }
            }
            for candidate in &native {
                if let Definition::Mcp { spec } = &candidate.definition {
                    if candidate.hash == hash {
                        result
                            .native_entry_ids
                            .push(candidate.view.entry_id.clone());
                    } else if similar(recipe, &candidate.view.name, &candidate.view.name, spec) {
                        result
                            .similar_native_entry_ids
                            .push(candidate.view.entry_id.clone());
                    }
                }
            }
            Ok(result)
        })
        .collect()
}
