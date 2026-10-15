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
fn verify_selected_tools(
    snapshot: &library::LibrarySnapshot,
    kind: library::ResourceKind,
    id: &str,
    tools: &[String],
    discovery: &std::collections::BTreeMap<String, crate::commands::InventoryEntry>,
) -> Result<(), String> {
    for tool in tools {
        let key = match kind {
            library::ResourceKind::Instructions => format!("instructions:{tool}"),
            library::ResourceKind::Mcp => format!("mcp:{id}:{tool}"),
            library::ResourceKind::Skills => format!("skills:{id}:{tool}"),
        };
        if snapshot
            .machine
            .assignments
            .get(&key)
            .is_some_and(|value| value == id)
            || (tool == "shared" && kind == library::ResourceKind::Skills)
        {
            continue;
        }
        if discovery
            .get(tool)
            .is_none_or(|entry| entry.status != "installed")
        {
            return Err("library.nativeCliRequired".into());
        }
    }
    Ok(())
}
fn check_revision(
    snapshot: &library::LibrarySnapshot,
    kind: library::ResourceKind,
    id: &str,
    expected: Option<u64>,
) -> Result<(), String> {
    let revision = match kind {
        library::ResourceKind::Instructions => snapshot
            .definitions
            .instructions
            .get(id)
            .map(|row| row.revision),
        library::ResourceKind::Mcp => snapshot.definitions.mcp.get(id).map(|row| row.revision),
        library::ResourceKind::Skills => {
            snapshot.definitions.skills.get(id).map(|row| row.revision)
        }
    };
    if expected.is_some() && revision != expected {
        return Err("library.revisionChanged".into());
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
    let mut operations: Vec<_> = snapshot
        .machine
        .operations
        .values()
        .filter(|op| op.kind == library::ResourceKind::Instructions)
        .collect();
    operations.sort_by_key(|operation| std::cmp::Reverse(operation.created_at));
    Ok(
        json!({"epoch":snapshot.machine.epoch,"instructions":snapshot.definitions.instructions.values().collect::<Vec<_>>(),"targets":targets,
        "operations":operations.into_iter().take(20).map(library::operation_view).collect::<Vec<_>>()}),
    )
}

#[tauri::command]
pub fn save_library_instruction(
    state: State<'_, AppState>,
    set: library::InstructionSet,
    expected_revision: Option<u64>,

    expected_epoch: u64,
) -> Result<library::InstructionSet, String> {
    let _epoch_io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    library::save_instruction(&state.db, set, expected_revision).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preview_library_instructions(
    state: State<'_, AppState>,
    resource_id: String,
    tools: Vec<String>,
    expected_revision: Option<u64>,

    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = resource_discovery().await?;
    let _io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    check_revision(
        &snapshot,
        library::ResourceKind::Instructions,
        &resource_id,
        expected_revision,
    )?;
    verify_selected_tools(
        &snapshot,
        library::ResourceKind::Instructions,
        &resource_id,
        &tools,
        &discovery,
    )?;
    library::preview_instructions(&state.db, &resource_id, &tools)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn apply_library_operation(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    library::apply_operation(&state.db, &operation_id)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recover_library_operation(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    library::recover_operation(&state.db, &operation_id)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn cancel_library_operation(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Value, String> {
    library::coordinator::cancel_operation(&state.db, &operation_id)
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
        json!({"epoch":snapshot.machine.epoch,"servers":definitions,"operations":operations.into_iter().take(20).map(library::operation_view).collect::<Vec<_>>()}),
    )
}

#[tauri::command]
pub fn save_library_mcp(
    state: State<'_, AppState>,
    resource_id: String,
    name: String,
    expected_revision: Option<u64>,
    edits: Vec<library::mcp_resource::FieldEdit>,

    expected_epoch: u64,
) -> Result<Value, String> {
    let _epoch_io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    library::mcp_resource::save_edits(&state.db, &resource_id, &name, expected_revision, &edits)
        .map(|_| json!({"saved":true}))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn preview_library_mcp(
    state: State<'_, AppState>,
    resource_id: String,
    tools: Vec<String>,
    expected_revision: Option<u64>,

    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = resource_discovery().await?;
    let _io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    check_revision(
        &snapshot,
        library::ResourceKind::Mcp,
        &resource_id,
        expected_revision,
    )?;
    verify_selected_tools(
        &snapshot,
        library::ResourceKind::Mcp,
        &resource_id,
        &tools,
        &discovery,
    )?;
    library::mcp_resource::preview(&state.db, &resource_id, &tools)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn delete_library_mcp(
    state: State<'_, AppState>,
    resource_id: String,
    expected_revision: u64,
    expected_epoch: u64,
) -> Result<bool, String> {
    let _io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    library::catalog::verify_mcp_delete_revision(&state.db, &resource_id, expected_revision)
        .map_err(|e| e.to_string())?;
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

#[tauri::command]
pub fn import_library_instruction(
    state: State<'_, AppState>,
    name: String,
    content: String,
) -> Result<library::InstructionSet, String> {
    library::catalog::import_instruction(&state.db, &name, &content).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn delete_library_instruction(
    state: State<'_, AppState>,
    resource_id: String,
    expected_revision: u64,
    expected_epoch: u64,
) -> Result<bool, String> {
    let _epoch_io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    library::catalog::delete_instruction(&state.db, &resource_id, expected_revision)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn get_library_summary(state: State<'_, AppState>) -> Result<Value, String> {
    let _io = crate::config_io::lock();
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    Ok(
        json!({"instructions":snapshot.definitions.instructions.len(),"mcp":state.db.get_all_mcp_servers().map_err(|e|e.to_string())?.len(),"skills":state.db.get_all_installed_skills().map_err(|e|e.to_string())?.len()}),
    )
}
#[tauri::command]
pub async fn get_library_skills(state: State<'_, AppState>) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = resource_discovery().await?;
    let _io = crate::config_io::lock();
    library::skill_resource::ensure_metadata(&state.db).map_err(|e| e.to_string())?;
    let mut skills = Vec::new();
    for row in state
        .db
        .get_all_installed_skills()
        .map_err(|e| e.to_string())?
        .values()
    {
        let mut view =
            library::skill_view::definition_view(&state.db, row).map_err(|e| e.to_string())?;
        if let Some(targets) = view["targets"].as_array_mut() {
            for target in targets {
                let tool = target["toolId"].as_str().unwrap_or_default().to_owned();
                let installed = if tool == "shared" {
                    target["readers"].as_array().is_some_and(|readers| {
                        readers.iter().filter_map(Value::as_str).any(|reader| {
                            discovery
                                .get(reader)
                                .is_some_and(|entry| entry.status == "installed")
                        })
                    })
                } else {
                    discovery
                        .get(&tool)
                        .is_some_and(|entry| entry.status == "installed")
                };
                target["available"] = json!(installed && target["scopeKnown"] == true);
                target["discoveryStatus"] = json!(if installed { "installed" } else { "unknown" });
            }
        }
        skills.push(view);
    }
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    let mut operations: Vec<_> = snapshot
        .machine
        .operations
        .values()
        .filter(|op| op.kind == library::ResourceKind::Skills)
        .collect();
    operations.sort_by_key(|op| std::cmp::Reverse(op.created_at));
    Ok(
        json!({"epoch":snapshot.machine.epoch,"skills":skills,"operations":operations.into_iter().take(20).map(library::operation_view).collect::<Vec<_>>()}),
    )
}
#[tauri::command]
pub async fn preview_library_skills(
    state: State<'_, AppState>,
    resource_id: String,
    tools: Vec<String>,
    expected_revision: Option<u64>,
    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = resource_discovery().await?;
    let _io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    check_revision(
        &snapshot,
        library::ResourceKind::Skills,
        &resource_id,
        expected_revision,
    )?;
    verify_selected_tools(
        &snapshot,
        library::ResourceKind::Skills,
        &resource_id,
        &tools,
        &discovery,
    )?;
    if tools.iter().any(|tool| tool == "shared")
        && !snapshot
            .machine
            .assignments
            .contains_key(&format!("skills:{resource_id}:shared"))
    {
        let readers = library::skill_resource::reader_paths().map_err(|e| e.to_string())?;
        let shared =
            library::skill_resource::target_paths().map_err(|e| e.to_string())?["shared"].clone();
        let row = state
            .db
            .get_installed_skill(&resource_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "library.resourceMissing".to_owned())?;
        let physical = library::skill_resource::entry_id(&shared.join(&row.directory))
            .map_err(|e| e.to_string())?;
        let visible = readers.iter().any(|(tool, roots)| {
            discovery
                .get(tool)
                .is_some_and(|entry| entry.status == "installed")
                && roots.iter().any(|root| {
                    library::skill_resource::reads_entry(
                        tool,
                        root,
                        std::path::Path::new(&physical),
                        &row.directory,
                    )
                    .unwrap_or(false)
                })
        });
        if !visible {
            return Err("library.nativeCliRequired".into());
        }
    }
    library::skill_resource::preview(&state.db, &resource_id, &tools)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn get_library_skill_detail(
    state: State<'_, AppState>,
    resource_id: String,
    relative_path: Option<String>,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    library::skill_view::detail(&state.db, &resource_id, relative_path.as_deref())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn import_library_skill_directory(
    state: State<'_, AppState>,
    source: String,
    directory: String,
    name: String,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    library::catalog::import_directory(&state.db, std::path::Path::new(&source), &directory, &name)
        .map(|row| json!({"id":row.id}))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn delete_library_skill(
    state: State<'_, AppState>,
    resource_id: String,
    expected_revision: u64,
    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let _io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    library::catalog::verify_skill_delete_revision(&state.db, &resource_id, expected_revision)
        .map_err(|e| e.to_string())?;
    crate::services::skill::SkillService::uninstall(&state.db, &resource_id)
        .map(|_| json!({"removed":true,"backupKept":true}))
        .map_err(|_| "library.failed".into())
}

#[cfg(test)]
mod import_tests {
    #[test]
    fn only_new_native_assignments_require_a_fresh_installation() {
        use crate::services::library::{LibrarySnapshot, ResourceKind};
        let discovery = Default::default();
        for kind in [
            ResourceKind::Instructions,
            ResourceKind::Mcp,
            ResourceKind::Skills,
        ] {
            let mut snapshot = LibrarySnapshot::default();
            let key = match kind {
                ResourceKind::Instructions => "instructions:codex",
                ResourceKind::Mcp => "mcp:fixture:codex",
                ResourceKind::Skills => "skills:fixture:codex",
            };
            snapshot
                .machine
                .assignments
                .insert(key.into(), "fixture".into());
            assert!(super::verify_selected_tools(
                &snapshot,
                kind.clone(),
                "fixture",
                &["codex".into()],
                &discovery
            )
            .is_ok());
            assert!(super::verify_selected_tools(
                &snapshot,
                kind,
                "fixture",
                &["gemini".into()],
                &discovery
            )
            .is_err());
        }
        assert!(super::verify_selected_tools(
            &LibrarySnapshot::default(),
            ResourceKind::Mcp,
            "fixture",
            &["shared".into()],
            &discovery
        )
        .is_err());
    }
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

fn read_resource_pack(path: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let limit = library::skill_artifact::PACKAGE_MAX_BYTES;
    let file = std::fs::File::open(path).map_err(|_| "library.packReadFailed")?;
    if !file
        .metadata()
        .map_err(|_| "library.packReadFailed")?
        .is_file()
    {
        return Err("library.packInvalid".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "library.packReadFailed")?;
    if bytes.len() as u64 > limit {
        return Err("library.packTooLarge".into());
    }
    Ok(bytes)
}
#[tauri::command]
pub async fn pick_library_pack(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("ZIP", &["zip"])
            .blocking_pick_file()
    })
    .await
    .map_err(|_| "library.packReadFailed")?;
    selected
        .map(|file| {
            file.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|_| "library.packReadFailed".into())
        })
        .transpose()
}
#[tauri::command]
pub async fn export_library_pack(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_file_name("ofox-resources.zip")
            .add_filter("ZIP", &["zip"])
            .blocking_save_file()
    })
    .await
    .map_err(|_| "library.packWriteFailed")?;
    let Some(file) = selected else {
        return Ok(false);
    };
    let path = file.into_path().map_err(|_| "library.packWriteFailed")?;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let raw = library::resource_pack::export(&db).map_err(|e| e.to_string())?;
        crate::config::atomic_write_with(
            &path,
            &raw,
            crate::config::AtomicOpts { mode: Some(0o600) },
        )
        .map_err(|_| "library.packWriteFailed".to_string())?;
        Ok(true)
    })
    .await
    .map_err(|_| "library.packWriteFailed".to_string())?
}
#[tauri::command]
pub async fn preview_library_pack(
    state: State<'_, AppState>,
    path: String,
) -> Result<Value, String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let raw = read_resource_pack(&path)?;
        library::resource_pack::preview(&db, &raw).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "library.packInvalid".to_string())?
}
#[tauri::command]
pub async fn restore_library_pack(
    state: State<'_, AppState>,
    path: String,
    expected_fingerprint: String,
    expected_revision: u64,
    expected_epoch: u64,
    conflicts: String,
) -> Result<Value, String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let raw = read_resource_pack(&path)?;
        library::resource_pack::restore(
            &db,
            &raw,
            &expected_fingerprint,
            expected_revision,
            expected_epoch,
            &conflicts,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "library.packInvalid".to_string())?
}

#[tauri::command]
pub fn duplicate_library_mcp(
    state: State<'_, AppState>,
    source_id: String,
    expected_revision: u64,
    expected_epoch: u64,
    new_id: String,
    name: String,
) -> Result<Value, String> {
    library::mcp_actions::duplicate(
        &state.db,
        &source_id,
        expected_revision,
        expected_epoch,
        &new_id,
        &name,
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn preview_library_mcp_adoption(
    state: State<'_, AppState>,
    resource_id: String,
    tools: Vec<String>,
    adopt_tools: Vec<String>,
    expected_revision: u64,
    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = resource_discovery().await?;
    let _io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    check_revision(
        &snapshot,
        library::ResourceKind::Mcp,
        &resource_id,
        Some(expected_revision),
    )?;
    verify_selected_tools(
        &snapshot,
        library::ResourceKind::Mcp,
        &resource_id,
        &tools,
        &discovery,
    )?;
    library::mcp_actions::preview_adoption(&state.db, &resource_id, &tools, &adopt_tools)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}
