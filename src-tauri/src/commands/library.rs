use crate::{services::library, AppState};
use serde_json::{json, Value};
use tauri::State;

async fn resource_discovery(
) -> Result<std::collections::BTreeMap<String, crate::commands::InventoryEntry>, String> {
    let tools = crate::commands::get_tool_versions(
        Some(
            ["claude", "codex", "gemini", "opencode", "hermes"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        ),
        None,
        Some(false),
    )
    .await?;
    Ok(tools
        .into_iter()
        .map(|tool| {
            let entry = tool.into_resource_inventory();
            (entry.tool_id.clone(), entry)
        })
        .collect())
}
async fn verify_selected_tools(tools: &[String]) -> Result<(), String> {
    if tools.is_empty() {
        return Ok(());
    }
    let discovery = resource_discovery().await?;
    for tool in tools {
        if discovery
            .get(tool)
            .is_none_or(|entry| entry.status != "installed")
        {
            return Err("library.nativeCliRequired".into());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_library_instructions(state: State<'_, AppState>) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = resource_discovery().await?;
    let _io = crate::config_io::lock();
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    let paths = library::coordinator::native_instruction_paths().map_err(|e| e.to_string())?;
    let physical: std::collections::BTreeMap<_, _> = paths
        .iter()
        .map(|(tool, path)| {
            Ok((
                tool.clone(),
                library::coordinator::physical_id(path).map_err(|e| e.to_string())?,
            ))
        })
        .collect::<Result<_, String>>()?;
    let targets: Vec<Value> = paths.iter().map(|(tool,path)| {
        let target = &physical[tool];
        let receipt = snapshot.machine.receipts.get(&format!("instructions:{target}"));
        let own = receipt.filter(|receipt|receipt.owners.contains(tool));
        let desired = snapshot.machine.assignments.get(&format!("instructions:{tool}"));
        let native = receipt.map(|receipt|library::instruction_block_health(path,receipt));
        let inspected = receipt.map(|receipt| snapshot.definitions.instructions.get(&receipt.resource_id).map(|definition|library::instruction_status(path,receipt,definition)).unwrap_or("needsReview"));
        let status = if receipt.is_some() && inspected != Some("applied") { inspected.unwrap_or("needsReview") } else {
            match (desired,own) {
                (Some(id),Some(receipt)) if id == &receipt.resource_id => "applied",
                (Some(_),_) => "pending",
                (None,Some(_)) => "needsReview",
                (None,None) if receipt.is_some() => "shared",
                _ => "unassigned",
            }
        };
        json!({"toolId":tool,"path":path.to_string_lossy(),"targetGroupId":target,
            "available":library::coordinator::target_available(discovery.get(tool).map(|entry|entry.status.as_str()).unwrap_or("unknown"),path),
            "discoveryStatus":discovery.get(tool).map(|entry|entry.status.as_str()).unwrap_or("unknown"),
            "pathState":if path.exists(){"exists"}else if path.parent().is_some_and(|parent|parent.is_dir()){"missing"}else{"missingParent"},
            "readers":physical.iter().filter(|(_,id)|*id == target).map(|(tool,_)|tool).collect::<Vec<_>>(),
            "desiredResourceId":desired,"lastAppliedResourceId":own.map(|receipt|&receipt.resource_id),
            "appliedResourceId":own.filter(|_|inspected == Some("applied")).map(|receipt|&receipt.resource_id),
            "readableResourceId":receipt.filter(|_|native == Some("intact")).map(|receipt|&receipt.resource_id),"nativeState":native,"status":status})
    }).collect();
    let mut operations: Vec<_> = snapshot.machine.operations.values().collect();
    operations.sort_by_key(|operation| std::cmp::Reverse(operation.created_at));
    Ok(
        json!({"instructions":snapshot.definitions.instructions.values().collect::<Vec<_>>(),"targets":targets,
        "operations":operations.into_iter().take(20).map(library::operation_view).collect::<Vec<_>>()}),
    )
}

#[tauri::command]
pub fn save_library_instruction(
    state: State<'_, AppState>,
    set: library::InstructionSet,
    expected_revision: Option<u64>,
) -> Result<library::InstructionSet, String> {
    library::save_instruction(&state.db, set, expected_revision).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preview_library_instructions(
    state: State<'_, AppState>,
    resource_id: String,
    tools: Vec<String>,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    verify_selected_tools(&tools).await?;
    library::preview_instructions(&state.db, &resource_id, &tools)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn apply_library_operation(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Value, String> {
    library::apply_operation(&state.db, &operation_id)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn recover_library_operation(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Value, String> {
    library::recover_operation(&state.db, &operation_id)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_library_mcp(state: State<'_, AppState>) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = resource_discovery().await?;
    let _guard = crate::config_io::lock();
    library::mcp_resource::ensure_metadata(&state.db).map_err(|e| e.to_string())?;
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    let paths = library::mcp_resource::native_paths().map_err(|e| e.to_string())?;
    let physical: std::collections::BTreeMap<_, _> = paths
        .iter()
        .map(|(tool, path)| {
            Ok((
                tool.clone(),
                library::coordinator::physical_id(path).map_err(|e| e.to_string())?,
            ))
        })
        .collect::<Result<_, String>>()?;
    let opencode_readers = library::mcp_resource::opencode_reader_paths()
        .map_err(|error| error.to_string())?
        .iter()
        .map(|path| library::coordinator::physical_id(path).map_err(|error| error.to_string()))
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    let servers = state.db.get_all_mcp_servers().map_err(|e| e.to_string())?;
    let definitions:Vec<_>=servers.values().map(|server|{
        let targets:Vec<_>=paths.iter().map(|(tool,path)|{
            let receipt=snapshot.machine.receipts.get(&format!("mcp:{}:{}",physical[tool],server.id));
            let owns=receipt.is_some_and(|r|r.owners.contains(tool));
            let desired=snapshot.machine.assignments.contains_key(&format!("mcp:{}:{tool}",server.id));
            let intact=receipt.is_some_and(|receipt|{
                let Ok(owned)=serde_json::from_str::<library::mcp_codec::OwnedFields>(&receipt.owned_segment)else{return false;};
                let Ok(Some(text))=library::coordinator::read(path)else{return false;};
                library::mcp_codec::verify_owned_text(&text,&owned).is_ok()
            });
            let scope=tool!="opencode"||library::mcp_resource::opencode_scope_clear(&server.id,&physical[tool]).is_ok();
            let intact=intact&&scope;
            let current=receipt.is_some_and(|receipt|snapshot.definitions.mcp.get(&server.id).is_some_and(|m|m.revision==receipt.resource_revision&&m.hash==receipt.definition_hash));
            let status=match(desired,owns,intact,current){(_,true,false,_)=>"conflict",(true,true,true,true)=>"applied",(true,_,_,_)=>"pending",(false,true,_,_)=>"needsReview",(false,false,true,_)=>"shared",_=>"unassigned"};
            let reason=library::mcp_codec::native_spec(tool,&server.server).err().map(|e|e.to_string());
            json!({"toolId":tool,"path":path.to_string_lossy(),"targetGroupId":physical[tool],"available":library::coordinator::target_available(discovery.get(tool).map(|entry|entry.status.as_str()).unwrap_or("unknown"),path),
            "discoveryStatus":discovery.get(tool).map(|entry|entry.status.as_str()).unwrap_or("unknown"),
            "pathState":if path.exists(){"exists"}else if path.parent().is_some_and(|parent|parent.is_dir()){"missing"}else{"missingParent"},"desired":desired,"applied":owns&&intact&&current,"status":status,"readers":physical.iter().filter(|(_,id)|*id==&physical[tool]).map(|(tool,_)|tool.clone()).chain(opencode_readers.contains(&physical[tool]).then_some("opencode".to_string())).collect::<std::collections::BTreeSet<_>>(),"disabledReason":reason})
        }).collect();
        let mut view=library::mcp_resource::definition_view(server,&snapshot);view["targets"]=json!(targets);view
    }).collect();
    let mut operations: Vec<_> = snapshot
        .machine
        .operations
        .values()
        .filter(|op| op.kind == library::ResourceKind::Mcp)
        .collect();
    operations.sort_by_key(|op| std::cmp::Reverse(op.created_at));
    Ok(
        json!({"servers":definitions,"operations":operations.into_iter().take(20).map(library::operation_view).collect::<Vec<_>>()}),
    )
}

#[tauri::command]
pub fn save_library_mcp(
    state: State<'_, AppState>,
    resource_id: String,
    name: String,
    expected_revision: Option<u64>,
    edits: Vec<library::mcp_resource::FieldEdit>,
) -> Result<Value, String> {
    library::mcp_resource::save_edits(&state.db, &resource_id, &name, expected_revision, &edits)
        .map(|_| json!({"saved":true}))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn preview_library_mcp(
    state: State<'_, AppState>,
    resource_id: String,
    tools: Vec<String>,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    verify_selected_tools(&tools).await?;
    library::mcp_resource::preview(&state.db, &resource_id, &tools)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn delete_library_mcp(state: State<'_, AppState>, resource_id: String) -> Result<bool, String> {
    library::mcp_resource::delete_draft(&state.db, &resource_id).map_err(|e| e.to_string())
}
pub(crate) fn import_source_view(
    state: &AppState,
    tool: &str,
    import: fn(&AppState) -> Result<usize, crate::AppError>,
) -> Value {
    let _io = crate::config_io::lock();
    let before = match state.db.get_all_mcp_servers() {
        Ok(rows) => rows,
        Err(_) => {
            return json!({"toolId":tool,"imported":null,"error":"library.importResultUnreadable"})
        }
    };
    let result = import(state);
    let after = match state.db.get_all_mcp_servers() {
        Ok(rows) => rows,
        Err(_) => {
            return json!({"toolId":tool,"imported":null,"error":"library.importResultUnreadable"})
        }
    };
    let created: Vec<_> = after
        .keys()
        .filter(|id| !before.contains_key(*id))
        .cloned()
        .collect();
    json!({"toolId":tool,"imported":created.len(),"createdIds":created,"error":result.err().map(|error|error.to_string())})
}

#[tauri::command]
pub async fn import_library_mcp(state: State<'_, AppState>) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let mut sources = Vec::new();
    for (tool, import) in [
        (
            "claude",
            crate::services::McpService::import_from_claude
                as fn(&AppState) -> Result<usize, crate::AppError>,
        ),
        ("codex", crate::services::McpService::import_from_codex),
        ("gemini", crate::services::McpService::import_from_gemini),
        (
            "opencode",
            crate::services::McpService::import_from_opencode,
        ),
        ("hermes", crate::services::McpService::import_from_hermes),
    ] {
        sources.push(import_source_view(&state, tool, import));
    }
    Ok(json!({"sources":sources}))
}

#[cfg(test)]
mod import_tests {
    #[test]
    #[serial_test::serial]
    fn partial_import_reports_the_definitions_it_actually_created() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state =
            crate::store::AppState::new(std::sync::Arc::new(crate::Database::memory().unwrap()));
        let path = crate::config::get_claude_mcp_path();
        crate::config::atomic_write(
            &path,
            br#"{"mcpServers":{"a":{"command":"server"},"z":[]}}"#,
        )
        .unwrap();
        let result = super::import_source_view(
            &state,
            "claude",
            crate::services::McpService::import_from_claude,
        );
        assert_eq!(result["imported"], 1);
        assert!(result["error"].is_string());
        assert!(state.db.get_all_mcp_servers().unwrap().contains_key("a"));
        assert!(state
            .db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
    }
}
