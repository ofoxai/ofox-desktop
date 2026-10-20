use super::*;
use crate::services::library::{mcp_codec, mcp_resource, skill_artifact, skill_resource};
use serde_json::{json, Value};
use std::{fs, path::Path};

pub(super) struct ScanData {
    pub sources: Vec<InventorySource>,
    pub candidates: Vec<Candidate>,
    pub routing: String,
}
#[derive(Clone)]
enum Location {
    File(PathBuf),
    Inline(String),
    Legacy,
}
#[derive(Clone)]
struct Route {
    tool: String,
    label: String,
    location: Location,
}
fn route(tool: &str, label: &str, path: PathBuf) -> Route {
    Route {
        tool: tool.into(),
        label: label.into(),
        location: Location::File(path),
    }
}
fn resolve(kind: &ResourceKind) -> Result<(Vec<Route>, Vec<String>), AppError> {
    let mut routes = vec![];
    let mut uncertain = vec![];
    match kind {
        ResourceKind::Instructions => {
            for (tool, path) in io::native_instruction_paths()? {
                if tool == "codex" {
                    routes.push(route(
                        &tool,
                        "nativeInstructionsOverride",
                        path.with_file_name("AGENTS.override.md"),
                    ));
                }
                routes.push(route(&tool, "nativeInstructions", path));
                routes.push(Route {
                    tool,
                    label: "legacyPrompts".into(),
                    location: Location::Legacy,
                });
            }
        }
        ResourceKind::Mcp => {
            for (tool, path) in mcp_resource::native_paths()? {
                if tool != "opencode" {
                    routes.push(route(&tool, "nativeMcp", path));
                }
            }
            // Preserve native global precedence. A later layer merges keys, not whole servers.
            let mut paths = crate::opencode_config::global_config_paths();
            if let Some(raw) = std::env::var_os("OPENCODE_CONFIG").filter(|value| !value.is_empty())
            {
                let path = PathBuf::from(raw);
                if path.is_absolute() {
                    paths.push(path);
                } else {
                    uncertain.push("opencode".into());
                }
            }
            let mut dirs = vec![crate::config::get_home_dir().join(".opencode")];
            if let Some(raw) =
                std::env::var_os("OPENCODE_CONFIG_DIR").filter(|value| !value.is_empty())
            {
                let path = PathBuf::from(raw);
                if path.is_absolute() {
                    dirs.push(path);
                } else {
                    uncertain.push("opencode".into());
                }
            }
            for dir in dirs {
                paths.extend([dir.join("opencode.json"), dir.join("opencode.jsonc")]);
            }
            let mut seen = BTreeSet::new();
            for path in paths {
                if seen.insert(path.clone()) {
                    routes.push(route("opencode", "nativeMcp", path));
                }
            }
            if let Ok(raw) = std::env::var("OPENCODE_CONFIG_CONTENT") {
                if !raw.is_empty() {
                    routes.push(Route {
                        tool: "opencode".into(),
                        label: "nativeMcpInline".into(),
                        location: Location::Inline(raw),
                    });
                }
            }
        }
        ResourceKind::Skills => {
            match skill_resource::reader_paths() {
                Ok(readers) => {
                    for (tool, roots) in readers {
                        for path in roots {
                            routes.push(route(&tool, "nativeSkills", path));
                        }
                    }
                }
                Err(_) => {
                    // The five native roots remain discoverable even if an optional reader layer is invalid.
                    // Do not pretend the invalid custom OpenCode/Hermes scopes were fully scanned.
                    for (tool, path) in skill_resource::native_paths()? {
                        routes.push(route(&tool, "nativeSkills", path));
                    }
                    let shared = crate::config::get_home_dir().join(".agents/skills");
                    for tool in ["codex", "copilot"] {
                        routes.push(route(tool, "nativeSkills", shared.clone()));
                    }
                    uncertain.extend(["opencode".into(), "hermes".into()]);
                }
            }
        }
    }
    routes.retain(|route| {
        if matches!(&route.location, Location::File(path) if !path.is_absolute()) {
            uncertain.push(route.tool.clone());
            false
        } else {
            true
        }
    });
    uncertain.sort();
    uncertain.dedup();
    Ok((routes, uncertain))
}
pub(super) fn routing_signature(kind: &ResourceKind) -> Result<String, AppError> {
    let (routes, uncertain) = resolve(kind)?;
    let routes: Vec<_> = routes
        .iter()
        .map(|r| {
            (
                r.tool.clone(),
                r.label.clone(),
                match &r.location {
                    Location::File(path) => path.to_string_lossy().into_owned(),
                    Location::Inline(raw) => io::fingerprint(Some(raw.as_bytes())),
                    Location::Legacy => "legacy".into(),
                },
            )
        })
        .collect();
    let env: BTreeMap<_, _> = crate::config::AGENT_DIR_VARS
        .iter()
        .map(|key| {
            (
                *key,
                std::env::var_os(key).map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect();
    Ok(io::fingerprint(Some(
        &serde_json::to_vec(&(routes, uncertain, env))
            .map_err(|_| error("inventoryUnavailable"))?,
    )))
}
fn source(r: &Route) -> InventorySource {
    let location = match &r.location {
        Location::File(path) if r.label == "nativeSkills" => {
            SourceLocation::Directory(path.clone())
        }
        Location::File(path) => SourceLocation::File(path.clone()),
        Location::Inline(_) => SourceLocation::Inline,
        Location::Legacy => SourceLocation::Legacy,
    };
    let source_id = uuid::Uuid::new_v4().to_string();
    InventorySource {
        source_ref: Some(source_id.clone()),
        observed_at_ms: Some(chrono::Utc::now().timestamp_millis()),
        location_kind: Some(location.kind().into()),
        display_path: location.short_path(),
        location: Some(location),
        proofs: vec![],
        source_id,
        tool_id: r.tool.clone(),
        label: r.label.clone(),
        state: "ready".into(),
        reason: None,
        count: 0,
    }
}
fn reference(source: &InventorySource) -> EntrySource {
    EntrySource {
        source_id: source.source_id.clone(),
        tool_id: source.tool_id.clone(),
        disabled: false,
    }
}
fn fail(source: &mut InventorySource, key: &str) {
    source.state =
        if key == "inventorySourceUnsupported" || key == "inventoryTransportUndetermined" {
            "unsupported"
        } else {
            "unreadable"
        }
        .into();
    source.reason = Some(format!("library.{key}"));
}
fn add(
    data: &mut ScanData,
    source: &InventorySource,
    name: String,
    definition: Definition,
    proofs: Vec<Proof>,
    hash: String,
    directory: Option<String>,
) {
    // Content-identical physical aliases are one candidate, with all actual native sources.
    if let Some(existing) = data.candidates.iter_mut().find(|entry| {
        entry.hash == hash
            && (matches!(definition, Definition::Skill { .. }) || entry.view.name == name)
    }) {
        if !existing
            .view
            .sources
            .iter()
            .any(|s| s.source_id == source.source_id)
        {
            existing.view.sources.push(reference(source));
        }
        existing.proofs.extend(proofs);
        return;
    }
    let service_host = match &definition {
        Definition::Mcp { spec } => mcp_service_host(spec),
        _ => None,
    };
    let (display_summary, icon_id) = match &definition {
        Definition::Mcp { spec } => super::super::display::mcp_display(spec),
        _ => (None, None),
    };
    data.candidates.push(Candidate {
        view: InventoryEntry {
            entry_id: uuid::Uuid::new_v4().to_string(),
            kind: data_kind(&definition),
            name,
            summary: None,
            display_summary,
            icon_id,
            service_host,
            sources: vec![reference(source)],
            matched_resource_ids: vec![],
            locations: vec![],
            conflict: false,
            variant: None,
            directory,
        },
        definition,
        proofs,
        hash,
    });
}
fn data_kind(definition: &Definition) -> ResourceKind {
    match definition {
        Definition::Instructions { .. } => ResourceKind::Instructions,
        Definition::Mcp { .. } => ResourceKind::Mcp,
        Definition::Skill { .. } => ResourceKind::Skills,
    }
}
fn text_file(path: &Path) -> Result<Option<(String, Proof)>, AppError> {
    match fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Err(error("inventorySourceUnreadable")),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok() {
                return Err(self::error("inventorySourceUnreadable"));
            }
            return Ok(None);
        }
        Err(_) => return Err(error("inventorySourceUnreadable")),
    }
    let Some(text) = io::read(path)? else {
        return Ok(None);
    };
    let physical = io::physical_id(path)?;
    let hash = io::fingerprint(Some(text.as_bytes()));
    Ok(Some((
        text,
        Proof::File {
            path: path.into(),
            physical,
            hash,
        },
    )))
}
fn user_instructions(text: &str) -> Result<String, AppError> {
    let mut text = text.to_owned();
    for (begin, end) in [
        (
            "<!-- OFOX:BEGIN instructions -->",
            "<!-- OFOX:END instructions -->",
        ),
        (
            "<!-- magpie:begin · written by magpie from its Library -->",
            "<!-- magpie:end -->",
        ),
    ] {
        let begins = text.matches(begin).count();
        let ends = text.matches(end).count();
        if begins != ends || begins > 1 {
            return Err(error("instructionsConflict"));
        }
        if begins == 0 {
            continue;
        }
        let start = text.find(begin).expect("counted marker");
        let end_at = text.find(end).expect("counted marker");
        if end_at < start {
            return Err(error("instructionsConflict"));
        }
        let mut finish = end_at + end.len();
        if text[finish..].starts_with("\r\n") {
            finish += 2;
        } else if text[finish..].starts_with('\n') {
            finish += 1;
        }
        text.replace_range(start..finish, "");
    }
    if text.contains("<!-- OFOX:") || text.contains("<!-- magpie:") {
        return Err(error("instructionsConflict"));
    }
    if text.len() > 1024 * 1024 {
        return Err(error("inventoryLimit"));
    }
    Ok(text)
}
fn instructions(db: &Database, route: &Route, source: &mut InventorySource, data: &mut ScanData) {
    match &route.location {
        Location::File(path) => match text_file(path) {
            Ok(Some((text, proof))) => {
                source.proofs.push(proof.clone());
                match user_instructions(&text) {
                    Ok(content) if !content.trim_matches('\u{feff}').trim().is_empty() => {
                        let hash = io::fingerprint(Some(content.as_bytes()));
                        let name = if route.label == "nativeInstructionsOverride" {
                            "Codex (AGENTS.override.md)"
                        } else {
                            match route.tool.as_str() {
                                "claude" => "Claude Code",
                                "codex" => "Codex",
                                "gemini" => "Gemini CLI",
                                "opencode" => "OpenCode",
                                "hermes" => "Hermes",
                                _ => &route.tool,
                            }
                        };
                        add(
                            data,
                            source,
                            name.into(),
                            Definition::Instructions { content },
                            vec![proof],
                            hash,
                            None,
                        );
                        source.count += 1;
                    }
                    Ok(_) => source.reason = Some("library.inventoryManagedOnly".into()),
                    Err(_) => fail(source, "instructionsConflict"),
                }
            }
            Ok(None) => source.state = "missing".into(),
            Err(_) => fail(source, "inventorySourceUnreadable"),
        },
        Location::Legacy => match db.get_prompts(&route.tool) {
            Ok(rows) => {
                for row in rows.values() {
                    if row.content.trim().is_empty() {
                        continue;
                    }
                    if row.content.len() > 1024 * 1024 || row.content.contains("<!-- OFOX:") {
                        fail(source, "inventorySourceUnreadable");
                        continue;
                    }
                    let Ok(bytes) = serde_json::to_vec(row) else {
                        fail(source, "inventorySourceUnreadable");
                        continue;
                    };
                    let proof = Proof::Legacy {
                        tool: route.tool.clone(),
                        id: row.id.clone(),
                        hash: io::fingerprint(Some(&bytes)),
                    };
                    let hash = io::fingerprint(Some(row.content.as_bytes()));
                    add(
                        data,
                        source,
                        row.name.clone(),
                        Definition::Instructions {
                            content: row.content.clone(),
                        },
                        vec![proof],
                        hash,
                        None,
                    );
                    source.count += 1;
                }
            }
            Err(_) => fail(source, "inventorySourceUnreadable"),
        },
        Location::Inline(_) => fail(source, "inventorySourceUnsupported"),
    }
}
fn mcp_single(
    route: &Route,
    source: &mut InventorySource,
) -> Result<Option<(Value, Option<Proof>)>, AppError> {
    let read = match &route.location {
        Location::File(path) => match text_file(path)? {
            Some((text, proof)) => {
                source.proofs.push(proof.clone());
                Some((text, Some(proof)))
            }
            None => {
                // Negative scan evidence is required even if this source yields no entries.
                source.proofs.push(Proof::File {
                    path: path.clone(),
                    physical: io::physical_id(path)?,
                    hash: io::fingerprint(None),
                });
                None
            }
        },
        Location::Inline(raw) if raw.len() <= 2 * 1024 * 1024 => Some((raw.clone(), None)),
        _ => return Err(error("inventorySourceUnsupported")),
    };
    let Some((text, proof)) = read else {
        source.state = "missing".into();
        return Ok(None);
    };
    let root = mcp_codec::parse_native(&text, &route.tool)?;
    if route.tool == "opencode" {
        crate::services::library::native_json::validate_routing_keys(&root, &["mcp"])?;
    }
    let servers = root
        .get(mcp_codec::section(&route.tool))
        .cloned()
        .unwrap_or_else(|| json!({}));
    Ok(Some((servers, proof)))
}
fn valid_spec(tool: &str, native: &Value) -> Result<Value, AppError> {
    // Gemini CLI 0.64.0 tries HTTP then SSE for `url` without `type`.
    // A portable fixed-transport definition cannot preserve that behavior.
    if tool == "gemini"
        && native.get("url").is_some()
        && native.get("httpUrl").is_none()
        && native.get("type").is_none()
    {
        return Err(error("inventoryTransportUndetermined"));
    }
    let spec = mcp_codec::canonical_spec(tool, native)?;
    if spec.get("command").is_some() && spec.get("url").is_some() {
        return Err(error("nativeInvalid"));
    }
    match spec.get("type").and_then(Value::as_str) {
        Some("stdio")
            if spec
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty()) => {}
        Some("http" | "sse")
            if spec
                .get("url")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty()) => {}
        _ => return Err(error("nativeInvalid")),
    }
    for (key, array) in [("args", true), ("env", false), ("headers", false)] {
        if let Some(value) = spec.get(key) {
            let valid = if array {
                value
                    .as_array()
                    .is_some_and(|values| values.iter().all(Value::is_string))
            } else {
                value
                    .as_object()
                    .is_some_and(|values| values.values().all(Value::is_string))
            };
            if !valid {
                return Err(error("nativeInvalid"));
            }
        }
    }
    Ok(spec)
}
/// The tool's own on/off switch rides on the source, outside the compared definition.
fn add_mcp(
    data: &mut ScanData,
    source: &InventorySource,
    name: &str,
    native: &Value,
    spec: Value,
    proofs: Vec<Proof>,
) {
    let hash = mcp_resource::match_hash(&spec).unwrap_or_default();
    add(
        data,
        source,
        name.to_string(),
        Definition::Mcp { spec },
        proofs,
        hash.clone(),
        None,
    );
    if native.get("enabled").and_then(Value::as_bool) != Some(false) {
        return;
    }
    let entry = data
        .candidates
        .iter_mut()
        .find(|entry| entry.hash == hash && entry.view.name == name);
    for reference in entry
        .into_iter()
        .flat_map(|entry| entry.view.sources.iter_mut())
        .filter(|reference| reference.source_id == source.source_id)
    {
        reference.disabled = true;
    }
}
fn mcp(routes: &[Route], data: &mut ScanData, uncertain: &[String]) {
    let mut open = json!({});
    let mut open_sources = BTreeMap::<String, Vec<(InventorySource, Option<Proof>)>>::new();
    let mut open_proofs = vec![];
    let mut open_failed = uncertain.iter().any(|tool| tool == "opencode");
    for route in routes {
        let mut source = source(route);
        match mcp_single(route, &mut source) {
            Ok(Some((servers, proof))) => {
                if route.tool == "opencode" {
                    open_proofs.extend(proof.clone());
                    for name in servers.as_object().into_iter().flat_map(|map| map.keys()) {
                        open_sources
                            .entry(name.clone())
                            .or_default()
                            .push((source.clone(), proof.clone()));
                    }
                    source.count = servers.as_object().map_or(0, |map| map.len());
                    crate::opencode_config::merge_global_value(&mut open, servers);
                } else if let Some(servers) = servers.as_object() {
                    for (name, native) in servers {
                        if name.is_empty() || name.len() > 256 {
                            fail(&mut source, "inventorySourceUnreadable");
                            continue;
                        }
                        match valid_spec(&route.tool, native) {
                            Ok(spec) if mcp_codec::app_owned(&spec) => {}
                            Ok(spec) => {
                                add_mcp(
                                    data,
                                    &source,
                                    name,
                                    native,
                                    spec,
                                    proof.clone().into_iter().collect(),
                                );
                                source.count += 1;
                            }
                            Err(failure) => fail(
                                &mut source,
                                if failure
                                    .to_string()
                                    .contains("inventoryTransportUndetermined")
                                {
                                    "inventoryTransportUndetermined"
                                } else {
                                    "inventorySourceUnsupported"
                                },
                            ),
                        }
                    }
                }
            }
            Ok(None) => {
                if route.tool == "opencode" {
                    if let Location::File(path) = &route.location {
                        match io::physical_id(path) {
                            Ok(physical) => open_proofs.push(Proof::File {
                                path: path.clone(),
                                physical,
                                hash: "missing".into(),
                            }),
                            Err(_) => {
                                open_failed = true;
                                fail(&mut source, "inventorySourceUnreadable");
                            }
                        }
                    }
                }
            }
            Err(_) => {
                fail(&mut source, "inventorySourceUnreadable");
                if route.tool == "opencode" {
                    open_failed = true;
                }
            }
        }
        data.sources.push(source);
    }
    if !open_failed {
        for (name, native) in open.as_object().into_iter().flat_map(|map| map.iter()) {
            let Some(sources) = open_sources.get(name) else {
                continue;
            };
            match valid_spec("opencode", native) {
                Ok(spec) if mcp_codec::app_owned(&spec) => {}
                Ok(spec) => {
                    for (source, _) in sources {
                        add_mcp(
                            data,
                            source,
                            name,
                            native,
                            spec.clone(),
                            open_proofs.clone(),
                        );
                    }
                }
                Err(_) => {
                    for (source, _) in sources {
                        if let Some(row) = data
                            .sources
                            .iter_mut()
                            .find(|row| row.source_id == source.source_id)
                        {
                            fail(row, "inventorySourceUnsupported");
                        }
                    }
                }
            }
        }
    } else {
        for source in data
            .sources
            .iter_mut()
            .filter(|source| source.tool_id == "opencode" && source.state == "ready")
        {
            source.state = "unsupported".into();
            source.reason = Some("library.inventorySourceUnsupported".into());
        }
    }
}
fn skill_size(
    root: &Path,
    total: &mut u64,
    entries: &mut usize,
    depth: usize,
) -> Result<(), AppError> {
    if depth > 32 {
        return Err(error("inventoryLimit"));
    }
    for entry in fs::read_dir(root).map_err(|_| error("skillUnreadable"))? {
        let entry = entry.map_err(|_| error("skillUnreadable"))?;
        *entries += 1;
        if *entries > 10_000 {
            return Err(error("inventoryLimit"));
        }
        let meta = fs::symlink_metadata(entry.path()).map_err(|_| error("skillUnreadable"))?;
        if meta.file_type().is_symlink() {
            return Err(error("skillLinkUnsupported"));
        }
        if meta.is_dir() {
            skill_size(&entry.path(), total, entries, depth + 1)?;
        } else if meta.is_file() {
            *total = total.saturating_add(meta.len());
            if *total > 128 * 1024 * 1024 {
                return Err(error("inventoryLimit"));
            }
        } else {
            return Err(error("skillUnreadable"));
        }
    }
    Ok(())
}
fn skill_walk(
    path: &Path,
    source: &mut InventorySource,
    data: &mut ScanData,
    visited: &mut BTreeSet<PathBuf>,
    budget: &mut (u64, usize),
    depth: usize,
) -> Result<(), AppError> {
    if depth > 8 || data.candidates.len() >= MAX_ENTRIES {
        return Err(error("inventoryLimit"));
    }
    let physical = path.canonicalize().map_err(|_| error("skillUnreadable"))?;
    if !visited.insert(physical.clone()) {
        return Ok(());
    }
    let metadata = fs::metadata(&physical).map_err(|_| error("skillUnreadable"))?;
    if !metadata.is_dir() {
        return Err(error("skillUnreadable"));
    }
    match fs::symlink_metadata(physical.join("SKILL.md")) {
        Ok(_) => {
            // Shared reader aliases neither reread bytes nor consume the scan budget again.
            if let Some(existing) = data.candidates.iter().find(|candidate| candidate.proofs.iter().any(|proof| matches!(proof, Proof::Tree { physical: known, .. } if known == physical.to_string_lossy().as_ref()))).cloned() {
                let proof = Proof::Tree { path: path.into(), physical: physical.to_string_lossy().into_owned(), hash: existing.hash.clone() };
                add(data, source, existing.view.name, existing.definition, vec![proof], existing.hash, existing.view.directory);
                source.count += 1;
                return Ok(());
            }
            skill_size(&physical, &mut budget.0, &mut budget.1, 0)?;
            let hash = skill_artifact::tree_hash(&physical)?;
            let directory = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| error("invalidSkillPath"))?
                .to_owned();
            skill_artifact::safe_leaf(&directory)?;
            let (name, description) = crate::services::skill::SkillService::read_skill_name_desc(
                &physical.join("SKILL.md"),
                &directory,
            );
            if name.trim().is_empty() || name.len() > 512 {
                return Err(error("invalidSkill"));
            }
            let proof = Proof::Tree {
                path: path.into(),
                physical: physical.to_string_lossy().into_owned(),
                hash: hash.clone(),
            };
            add(
                data,
                source,
                name,
                Definition::Skill {
                    path: path.into(),
                    hash: hash.clone(),
                    directory: directory.clone(),
                    description,
                },
                vec![proof],
                hash,
                Some(directory),
            );
            source.count += 1;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut children: Vec<_> = fs::read_dir(&physical)
                .map_err(|_| self::error("skillUnreadable"))?
                .collect::<Result<_, _>>()
                .map_err(|_| self::error("skillUnreadable"))?;
            children.sort_by_key(|entry| entry.file_name());
            for child in children {
                if child
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with('.'))
                {
                    continue;
                }
                budget.1 += 1;
                if budget.1 > 10_000 {
                    return Err(self::error("inventoryLimit"));
                }
                let meta =
                    fs::metadata(child.path()).map_err(|_| self::error("skillUnreadable"))?;
                if meta.is_dir()
                    && skill_walk(&child.path(), source, data, visited, budget, depth + 1).is_err()
                {
                    fail(source, "inventorySourceUnreadable");
                }
            }
        }
        Err(_) => return Err(error("skillUnreadable")),
    }
    Ok(())
}
pub(super) fn collect(db: &Database, kind: ResourceKind) -> Result<ScanData, AppError> {
    let routing = routing_signature(&kind)?;
    let (routes, uncertain) = resolve(&kind)?;
    let mut data = ScanData {
        sources: vec![],
        candidates: vec![],
        routing,
    };
    match kind {
        ResourceKind::Instructions => {
            for route in &routes {
                let mut source = source(route);
                instructions(db, route, &mut source, &mut data);
                data.sources.push(source);
            }
        }
        ResourceKind::Mcp => mcp(&routes, &mut data, &uncertain),
        ResourceKind::Skills => {
            let mut budget = (0, 0);
            for route in &routes {
                let mut source = source(route);
                if let Location::File(path) = &route.location {
                    match fs::metadata(path) {
                        Ok(_) => {
                            if skill_walk(
                                path,
                                &mut source,
                                &mut data,
                                &mut BTreeSet::new(),
                                &mut budget,
                                0,
                            )
                            .is_err()
                            {
                                fail(&mut source, "inventorySourceUnreadable");
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            if fs::symlink_metadata(path).is_ok() {
                                fail(&mut source, "inventorySourceUnreadable");
                            } else {
                                source.state = "missing".into();
                            }
                        }
                        Err(_) => fail(&mut source, "inventorySourceUnreadable"),
                    }
                }
                data.sources.push(source);
            }
        }
    }
    for tool in uncertain {
        data.sources.push(InventorySource {
            source_id: uuid::Uuid::new_v4().to_string(),
            tool_id: tool,
            label: "nativeCustomScope".into(),
            state: "unsupported".into(),
            reason: Some("library.inventorySourceUnsupported".into()),
            count: 0,
            observed_at_ms: Some(chrono::Utc::now().timestamp_millis()),
            location_kind: None,
            display_path: None,
            source_ref: None,
            location: None,
            proofs: vec![],
        });
    }
    Ok(data)
}

pub(super) fn scan_signature(data: &ScanData) -> Result<String, AppError> {
    let source_key = |source: &InventorySource| {
        let physical = match source.location.as_ref() {
            Some(SourceLocation::File(path) | SourceLocation::Directory(path)) => {
                io::physical_id(path).ok()
            }
            _ => None,
        };
        format!(
            "{:?}",
            (
                &source.tool_id,
                &source.label,
                &source.state,
                &source.reason,
                source.count,
                &source.location,
                physical,
                source.proofs.iter().collect::<BTreeSet<_>>()
            )
        )
    };
    let mut sources: Vec<_> = data.sources.iter().map(&source_key).collect();
    sources.sort();
    let source_keys: BTreeMap<_, _> = data
        .sources
        .iter()
        .map(|source| (source.source_id.as_str(), source_key(source)))
        .collect();
    let mut entries: Vec<_> = data
        .candidates
        .iter()
        .map(|candidate| {
            let origins: BTreeSet<_> = candidate
                .view
                .sources
                .iter()
                .filter_map(|source| source_keys.get(source.source_id.as_str()))
                .collect();
            format!(
                "{:?}",
                (
                    &candidate.view.name,
                    &candidate.hash,
                    &candidate.view.directory,
                    candidate.proofs.iter().collect::<BTreeSet<_>>(),
                    origins
                )
            )
        })
        .collect();
    entries.sort();
    Ok(io::fingerprint(Some(
        &serde_json::to_vec(&(sources, entries, &data.routing))
            .map_err(|_| error("inventoryUnavailable"))?,
    )))
}
