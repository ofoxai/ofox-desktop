use crate::{services::library, AppState};
use serde_json::{json, Value};
use tauri::State;

/// Read-only presence discovery: never start login shells or agent processes.
/// A preview still performs the full current installation/version validation.
async fn list_discovery() -> std::collections::BTreeMap<String, crate::commands::InventoryEntry> {
    #[allow(unused_mut)]
    let mut found: std::collections::BTreeMap<_, _> = library::skill_targets::all()
        .map(|target| {
            let mut detected = library::skill_targets::detected_locally(target.id);
            detected |= crate::commands::misc::locate_cli(target.id);
            if ["codex", "workbuddy"].contains(&target.id) {
                let names = if target.id == "codex" {
                    vec!["Codex.app", "ChatGPT.app"]
                } else {
                    vec!["WorkBuddy.app"]
                };
                let home = crate::config::get_home_dir();
                detected |= [
                    home.join("Applications"),
                    std::path::PathBuf::from("/Applications"),
                ]
                .iter()
                .any(|dir| names.iter().any(|name| dir.join(name).is_dir()))
                    && cfg!(all(target_os = "macos", not(test)));
            }
            (
                target.id.to_owned(),
                crate::commands::InventoryEntry {
                    tool_id: target.id.to_owned(),
                    status: if detected { "installed" } else { "unknown" }.into(),
                    version: None,
                    path: None,
                    error: None,
                    managed_copy: None,
                },
            )
        })
        .collect();
    #[cfg(windows)]
    {
        // Preserve desktop-only Windows installations. These bounded native
        // metadata readers run outside config_io; no CLI/version batch is started.
        let need_codex = found
            .get("codex")
            .is_none_or(|entry| entry.status != "installed");
        let need_workbuddy = found
            .get("workbuddy")
            .is_none_or(|entry| entry.status != "installed");
        let (codex, workbuddy) = tokio::join!(
            async {
                if need_codex {
                    crate::commands::misc::codex_desktop_app().await
                } else {
                    Ok(None)
                }
            },
            async {
                if need_workbuddy {
                    crate::commands::misc::find_workbuddy_app().await
                } else {
                    Ok(None)
                }
            }
        );
        for (tool, result) in [("codex", codex), ("workbuddy", workbuddy)] {
            if let Ok(Some(app)) = result {
                if let Some(entry) = found.get_mut(tool) {
                    entry.status = "installed".into();
                    entry.version = app.version;
                    entry.path = Some(app.path.to_string_lossy().into_owned());
                }
            }
        }
    }
    found
}

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
async fn extension_discovery(
) -> Result<std::collections::BTreeMap<String, crate::commands::InventoryEntry>, String> {
    let tools = crate::commands::get_tool_versions(
        Some(
            library::extension_targets::CLIENTS
                .iter()
                .map(|id| (*id).to_owned())
                .collect(),
        ),
        None,
        Some(false),
    )
    .await?;
    let mut found = tools
        .into_iter()
        .map(|tool| {
            let entry = tool.into_skill_inventory();
            (entry.tool_id.clone(), entry)
        })
        .collect();
    merge_extension_aliases(&mut found);
    Ok(found)
}
/// MCP supports native desktop clients too. ChatGPT and Codex resolve to one target.
async fn mcp_discovery(
) -> Result<std::collections::BTreeMap<String, crate::commands::InventoryEntry>, String> {
    let mut found = extension_discovery().await?;
    for (tool, entry) in &mut found {
        if entry.status == "installed" {
            if let Some(reason) =
                library::mcp_targets::version_reason(tool, entry.version.as_deref())
            {
                entry.status = "unsupported".into();
                entry.error = Some(reason.into());
            }
        }
    }
    Ok(found)
}
/// A desktop-only ChatGPT installation supplies the shared Codex target.
fn merge_extension_aliases(
    found: &mut std::collections::BTreeMap<String, crate::commands::InventoryEntry>,
) {
    if found
        .get("codex")
        .is_none_or(|entry| entry.status != "installed")
    {
        if let Some(mut entry) = found
            .remove("chatgpt")
            .filter(|entry| entry.status == "installed")
        {
            entry.tool_id = library::extension_targets::canonical("chatgpt").into();
            found.insert("codex".into(), entry);
        }
    }
    found.remove("chatgpt");
}
/// Every agent that can take skills, keyed by target id: one whose CLI or app is
/// installed (never under WSL), or whose own folder is on this machine.
async fn skill_discovery(
) -> Result<std::collections::BTreeMap<String, crate::commands::InventoryEntry>, String> {
    let targets: Vec<_> = library::skill_targets::all().collect();
    let mut found = list_discovery().await;
    for target in &targets {
        let installed = found
            .get(target.id)
            .is_some_and(|entry: &crate::commands::InventoryEntry| entry.status == "installed")
            || library::skill_targets::detected_locally(target.id);
        let entry =
            found
                .entry(target.id.to_string())
                .or_insert_with(|| crate::commands::InventoryEntry {
                    tool_id: target.id.to_string(),
                    status: "notInstalled".into(),
                    version: None,
                    path: None,
                    error: None,
                    managed_copy: None,
                });
        if installed {
            entry.status = "installed".into();
        }
    }
    Ok(found)
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
            return Err(discovery
                .get(tool)
                .and_then(|entry| entry.error.clone())
                .filter(|reason| reason.starts_with("library."))
                .unwrap_or_else(|| {
                    if kind == library::ResourceKind::Mcp {
                        "library.nativeMcpClientRequired".into()
                    } else {
                        "library.nativeCliRequired".into()
                    }
                }));
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
    let discovery = list_discovery().await;
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
    let mut content_by_group = std::collections::BTreeMap::new();
    for (tool, path) in &paths {
        let group = &physical[tool];
        content_by_group.entry(group.clone()).or_insert_with(|| {
            library::instruction_content_view(
                path,
                snapshot
                    .machine
                    .receipts
                    .get(&format!("instructions:{group}")),
            )
        });
    }
    let targets: Vec<Value> = paths.iter().map(|(tool,path)| {
        let target = &physical[tool];
        let receipt = snapshot.machine.receipts.get(&format!("instructions:{target}"));
        let own = receipt.filter(|receipt|receipt.owners.contains(tool));
        let desired = snapshot.machine.assignments.get(&format!("instructions:{tool}"));
        let content = &content_by_group[target];
        let native = content["nativeState"].as_str().unwrap_or("unreadable");
        let inspected = receipt.map(|receipt| {
            snapshot.definitions.instructions.get(&receipt.resource_id).map(|definition| library::instruction_status(native, receipt, definition)).unwrap_or("needsReview")
        });
        let status = if receipt.is_none() && ["conflict","unreadable"].contains(&native) { native } else if receipt.is_some() && inspected != Some("applied") { inspected.unwrap_or("needsReview") } else {
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
            "readableResourceId":receipt.filter(|_|native == "intact").map(|receipt|&receipt.resource_id),"nativeState":native,"status":status,
            "userContentLines":content["userContentLines"],"managedContentLines":content["managedContentLines"],"lastAppliedRevision":own.map(|receipt|receipt.resource_revision)})
    }).collect();
    let mut operations: Vec<_> = snapshot
        .machine
        .operations
        .values()
        .filter(|op| !op.id.starts_with(library::batch::CHILD_PREFIX))
        .filter(|op| op.kind == library::ResourceKind::Instructions)
        .collect();
    operations.sort_by_key(|operation| std::cmp::Reverse(operation.created_at));
    Ok(
        json!({"epoch":snapshot.machine.epoch,"instructions":snapshot.definitions.instructions.values().collect::<Vec<_>>(),"targets":targets,
        "detachedTargets":snapshot.machine.receipts.values().filter(|receipt| receipt.kind == library::ResourceKind::Instructions && !physical.values().any(|group|group == &receipt.physical_target)).map(|receipt| json!({"path":receipt.path,"resourceId":receipt.resource_id,"tools":receipt.owners})).collect::<Vec<_>>(),
        "operations":operations.into_iter().take(20).map(library::operation_view).collect::<Vec<_>>()}),
    )
}

#[tauri::command]
pub fn prepare_library_mcp_check(
    state: State<'_, AppState>,
    resource_id: String,
    expected_revision: u64,
    expected_epoch: u64,
) -> Result<Value, String> {
    library::mcp_check::prepare(&state.db, &resource_id, expected_revision, expected_epoch)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn run_library_mcp_check(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Value, String> {
    use tauri::Emitter;
    library::mcp_check::run(&state.db, &operation_id, |stage| {
        let _ = app.emit(
            "library-mcp-check-progress",
            json!({"operationId":operation_id,"stage":stage}),
        );
    })
    .await
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn cancel_library_mcp_check(operation_id: String) -> Result<bool, String> {
    library::mcp_check::cancel(&operation_id).map_err(|e| e.to_string())
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
    let discovery = list_discovery().await;
    library_mcp_from_discovery(&state.db, &discovery)
}

fn library_mcp_from_discovery(
    db: &crate::Database,
    discovery: &std::collections::BTreeMap<String, crate::commands::InventoryEntry>,
) -> Result<Value, String> {
    let _guard = crate::config_io::lock();
    library::mcp_resource::ensure_metadata(db).map_err(|e| e.to_string())?;
    let snapshot = db.library_snapshot().map_err(|e| e.to_string())?;
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
    let native_errors: std::collections::BTreeMap<_, _> = paths
        .iter()
        .filter_map(|(tool, path)| {
            let result = library::coordinator::read(path).and_then(|text| {
                text.map(|text| library::mcp_codec::parse_native(&text, tool).map(|_| ()))
                    .unwrap_or(Ok(()))
            });
            result.err().map(|error| (tool.clone(), error.to_string()))
        })
        .collect();
    let global_targets: Vec<_> = paths.iter().map(|(tool, path)| {
        let discovery_status = discovery.get(tool).map(|entry|entry.status.as_str()).unwrap_or("unknown");
        let available = library::coordinator::target_available(discovery_status, path);
        json!({"toolId":tool,"path":path.to_string_lossy(),"targetGroupId":physical[tool],
            "available":available,"discoveryStatus":discovery_status,
            "pathState":if path.exists(){"exists"}else if path.parent().is_some_and(|parent|parent.is_dir()){"missing"}else{"missingParent"},
            "readers":physical.iter().filter(|(_,id)|*id==&physical[tool]).map(|(tool,_)|tool.clone()).chain(opencode_readers.contains(&physical[tool]).then_some("opencode".to_string())).collect::<std::collections::BTreeSet<_>>(),
            "disabledReason":native_errors.get(tool).cloned().or_else(||discovery.get(tool).and_then(|entry|entry.error.clone()).filter(|reason|reason.starts_with("library."))).or_else(||(!available).then_some("library.nativeMcpClientRequired".to_string()))})
    }).collect();
    let servers = db.get_all_mcp_servers().map_err(|e| e.to_string())?;
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
            let reason=native_errors.get(tool).cloned().or_else(||library::mcp_codec::native_server(tool,&server.id,&server.server).err().map(|e|e.to_string())).or_else(||discovery.get(tool).and_then(|entry|entry.error.clone()).filter(|reason|reason.starts_with("library.")));
            json!({"toolId":tool,"path":path.to_string_lossy(),"targetGroupId":physical[tool],"available":library::coordinator::target_available(discovery.get(tool).map(|entry|entry.status.as_str()).unwrap_or("unknown"),path),
            "discoveryStatus":discovery.get(tool).map(|entry|entry.status.as_str()).unwrap_or("unknown"),
            "pathState":if path.exists(){"exists"}else if path.parent().is_some_and(|parent|parent.is_dir()){"missing"}else{"missingParent"},"desired":desired,"applied":owns&&intact&&current,"status":status,"readers":physical.iter().filter(|(_,id)|*id==&physical[tool]).map(|(tool,_)|tool.clone()).chain(opencode_readers.contains(&physical[tool]).then_some("opencode".to_string())).collect::<std::collections::BTreeSet<_>>(),"disabledReason":reason})
        }).collect();
        let mut view=library::mcp_resource::definition_view(server,&snapshot);view["targets"]=json!(targets);view["serviceHost"]=json!(library::inventory::mcp_service_host(&server.server));view
    }).collect();
    let mut operations: Vec<_> = snapshot
        .machine
        .operations
        .values()
        .filter(|op| !op.id.starts_with(library::batch::CHILD_PREFIX))
        .filter(|op| op.kind == library::ResourceKind::Mcp)
        .collect();
    operations.sort_by_key(|op| std::cmp::Reverse(op.created_at));
    Ok(
        json!({"epoch":snapshot.machine.epoch,"servers":definitions,"targets":global_targets,"operations":operations.into_iter().take(20).map(library::operation_view).collect::<Vec<_>>()}),
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
    let discovery = mcp_discovery().await?;
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
    let discovery = skill_discovery().await?;
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
        .filter(|op| !op.id.starts_with(library::batch::CHILD_PREFIX))
        .filter(|op| op.kind == library::ResourceKind::Skills)
        .collect();
    operations.sort_by_key(|op| std::cmp::Reverse(op.created_at));
    let resolved = library::skill_resource::native_paths().unwrap_or_default();
    let skill_targets: Vec<Value> = library::extension_targets::TARGETS.iter()
        .map(|target| {
            json!({"id":target.id,"label":target.label,"readsShared":target.reads_shared,
                "detected":discovery.get(target.id).is_some_and(|entry| entry.status == "installed"),
                "scopeKnown":resolved.contains_key(target.id)})
        })
        .collect();
    Ok(
        json!({"epoch":snapshot.machine.epoch,"skills":skills,"skillTargets":skill_targets,"operations":operations.into_iter().take(20).map(library::operation_view).collect::<Vec<_>>()}),
    )
}
#[tauri::command]
pub async fn preview_library_skill_distribution(
    state: State<'_, AppState>,
    resource_id: String,
    tools: Vec<String>,
    adopt_tools: Vec<String>,
    expected_revision: u64,
    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = skill_discovery().await?;
    let _io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    check_revision(
        &snapshot,
        library::ResourceKind::Skills,
        &resource_id,
        Some(expected_revision),
    )?;
    verify_selected_tools(
        &snapshot,
        library::ResourceKind::Skills,
        &resource_id,
        &tools,
        &discovery,
    )?;
    library::skill_resource::preview_append(&state.db, &resource_id, &tools, &adopt_tools)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}

/// Explicit choice to replace a personal copy; ordinary distribution still refuses it.
#[tauri::command]
pub async fn preview_library_skill_replacement(
    state: State<'_, AppState>,
    resource_id: String,
    tool: String,
    expected_revision: u64,
    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = skill_discovery().await?;
    let _io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    check_revision(
        &snapshot,
        library::ResourceKind::Skills,
        &resource_id,
        Some(expected_revision),
    )?;
    verify_selected_tools(
        &snapshot,
        library::ResourceKind::Skills,
        &resource_id,
        std::slice::from_ref(&tool),
        &discovery,
    )?;
    library::skill_resource::preview_replacement(&state.db, &resource_id, &tool)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn keep_library_skill_native(
    state: State<'_, AppState>,
    resource_id: String,
    tool: String,
    expected_revision: u64,
    expected_epoch: u64,
) -> Result<bool, String> {
    let _io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    check_revision(
        &snapshot,
        library::ResourceKind::Skills,
        &resource_id,
        Some(expected_revision),
    )?;
    library::skill_resource::keep_native(&state.db, &resource_id, &tool)
        .map(|()| true)
        .map_err(|e| e.to_string())
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
    let discovery = skill_discovery().await?;
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
    expected_revision: Option<u64>,
    expected_epoch: Option<u64>,
    expected_hash: Option<String>,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    library::skill_view::detail_revisioned(
        &state.db,
        &resource_id,
        relative_path.as_deref(),
        expected_revision,
        expected_epoch,
        expected_hash.as_deref(),
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn import_library_skill_directory(
    state: State<'_, AppState>,
    source: String,
    directory: String,
    name: String,
    expected_epoch: Option<u64>,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let _io = crate::config_io::lock();
    if let Some(epoch) = expected_epoch {
        library::catalog::verify_epoch(&state.db, epoch).map_err(|e| e.to_string())?;
    }
    library::catalog::import_directory(&state.db, std::path::Path::new(&source), &directory, &name)
        .and_then(|row| {
            library::skill_resource::collection_receipt(&state.db, &row, expected_epoch)
        })
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
    selection: Option<library::resource_pack::ExportSelection>,
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
        let raw = match selection {
            Some(selection) => library::resource_pack::export_selected(&db, &selection),
            None => library::resource_pack::export(&db),
        }
        .map_err(|e| e.to_string())?;
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
    let discovery = mcp_discovery().await?;
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

#[tauri::command]
pub async fn check_library_skill_update(
    state: State<'_, AppState>,
    resource_id: String,
    expected_revision: u64,
    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    library::skill_actions::check(&state.db, &resource_id, expected_revision, expected_epoch)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn compare_library_skill_update(
    state: State<'_, AppState>,
    ticket_id: String,
    relative_path: String,
) -> Result<Value, String> {
    library::skill_actions::compare(&state.db, &ticket_id, &relative_path)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn confirm_library_skill_update(
    state: State<'_, AppState>,
    ticket_id: String,
    expected_revision: u64,
    expected_epoch: u64,
    mode: String,
    name: Option<String>,
    directory: Option<String>,
) -> Result<Value, String> {
    library::skill_actions::confirm(
        &state.db,
        &ticket_id,
        expected_revision,
        expected_epoch,
        &mode,
        name.as_deref(),
        directory.as_deref(),
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn discard_library_skill_update(ticket_id: String) -> Result<bool, String> {
    library::skill_actions::discard(&ticket_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn scan_library_inventory(
    state: State<'_, AppState>,
    kind: library::ResourceKind,
) -> Result<library::inventory::LibraryInventory, String> {
    crate::agent_env::ready().await;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library::inventory::scan(&db, kind).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "library.inventoryUnavailable".to_string())?
}
#[tauri::command]
pub async fn import_library_inventory(
    state: State<'_, AppState>,
    ticket_id: String,
    entry_ids: Vec<String>,
    expected_epoch: u64,
) -> Result<library::inventory::InventoryImport, String> {
    crate::agent_env::ready().await;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library::inventory::import(&db, &ticket_id, &entry_ids, expected_epoch)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "library.inventoryUnavailable".to_string())?
}
/// Remove a found skill (not in the library) from its agents: links go,
/// folders are set aside in the skill backups.
#[tauri::command]
pub async fn remove_library_found_skill(
    state: State<'_, AppState>,
    ticket_id: String,
    entry_id: String,
    tool_ids: Vec<String>,
    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library::inventory::remove_found(&db, &ticket_id, &entry_id, &tool_ids, expected_epoch)
            .map(|removed| json!({ "removed": removed }))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "library.inventoryUnavailable".to_string())?
}
/// Like `preview_library_skills`, but an identical entry already in one of
/// `adopt_tools`' agents is taken over as it is.
#[tauri::command]
pub async fn preview_library_skill_adoption(
    state: State<'_, AppState>,
    resource_id: String,
    tools: Vec<String>,
    adopt_tools: Vec<String>,
    expected_revision: u64,
    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = skill_discovery().await?;
    let _io = crate::config_io::lock();
    library::catalog::verify_epoch(&state.db, expected_epoch).map_err(|e| e.to_string())?;
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    check_revision(
        &snapshot,
        library::ResourceKind::Skills,
        &resource_id,
        Some(expected_revision),
    )?;
    verify_selected_tools(
        &snapshot,
        library::ResourceKind::Skills,
        &resource_id,
        &tools,
        &discovery,
    )?;
    library::skill_resource::preview_adoption(&state.db, &resource_id, &tools, &adopt_tools)
        .map(|op| library::operation_view(&op))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn preview_library_batch(
    state: State<'_, AppState>,
    kind: library::ResourceKind,
    resources: Vec<library::batch::BatchResource>,
    tools: Vec<String>,
    action: library::batch::BatchAction,
    expected_epoch: u64,
) -> Result<Value, String> {
    crate::agent_env::ready().await;
    let discovery = if kind == library::ResourceKind::Skills {
        skill_discovery().await?
    } else if kind == library::ResourceKind::Mcp {
        mcp_discovery().await?
    } else {
        resource_discovery().await?
    };
    let snapshot = state.db.library_snapshot().map_err(|e| e.to_string())?;
    for resource in &resources {
        let mut additions = resource.add_tools.clone();
        if action == library::batch::BatchAction::Apply {
            additions.extend(tools.iter().cloned());
        }
        verify_selected_tools(
            &snapshot,
            kind.clone(),
            &resource.resource_id,
            &additions,
            &discovery,
        )?;
    }
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library::batch::preview_batch(&db, kind, resources, tools, action, expected_epoch)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "library.failed".to_string())?
}
#[tauri::command]
pub async fn run_library_batch(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Value, String> {
    let db = state.db.clone();
    run_ready_batch(move || {
        library::batch::run_batch(&db, &operation_id).map_err(|e| e.to_string())
    })
    .await
}
#[tauri::command]
pub async fn cancel_library_batch(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Value, String> {
    let db = state.db.clone();
    run_ready_batch(move || {
        library::batch::cancel_batch(&db, &operation_id).map_err(|e| e.to_string())
    })
    .await
}
#[tauri::command]
pub async fn reconcile_library_batch(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Value, String> {
    let db = state.db.clone();
    run_ready_batch(move || {
        library::batch::reconcile_batch(&db, &operation_id).map_err(|e| e.to_string())
    })
    .await
}

async fn run_ready_batch(
    job: impl FnOnce() -> Result<Value, String> + Send + 'static,
) -> Result<Value, String> {
    run_ready_batch_with(crate::agent_env::ready(), job).await
}

async fn run_ready_batch_with(
    readiness: impl std::future::Future<Output = ()>,
    job: impl FnOnce() -> Result<Value, String> + Send + 'static,
) -> Result<Value, String> {
    readiness.await;
    tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|_| "library.failed".to_string())?
}
#[tauri::command]
pub fn list_library_batches(
    state: State<'_, AppState>,
    kind: Option<library::ResourceKind>,
) -> Result<Value, String> {
    library::batch::list_batches(&state.db, kind).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn discard_library_inventory(ticket_id: String) -> bool {
    library::inventory::discard(&ticket_id)
}

#[tauri::command]
pub async fn export_library_selection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    selection: library::resource_pack::ExportSelection,
) -> Result<bool, String> {
    export_library_pack(app, state, Some(selection)).await
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    use crate::{app_config::McpServer, Database};
    use std::sync::Arc;
    use tokio::sync::oneshot;

    #[tokio::test]
    #[serial_test::serial]
    async fn batch_execution_waits_for_pending_environment_before_native_write() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Arc::new(Database::memory().unwrap());
        library::mcp_resource::save_draft(
            &db,
            McpServer {
                id: "ready-fixture".into(),
                name: "Readiness fixture".into(),
                server: json!({"command":"never-executed","args":["fixture"]}),
                apps: Default::default(),
                description: None,
                homepage: None,
                docs: None,
                tags: vec![],
            },
            None,
        )
        .unwrap();
        let native = library::mcp_resource::native_paths().unwrap()["codex"].clone();
        let plan = library::batch::preview_batch(
            &db,
            library::ResourceKind::Mcp,
            vec![library::batch::BatchResource {
                resource_id: "ready-fixture".into(),
                expected_revision: 1,
                add_tools: vec![],
                withdraw_tools: vec![],
            }],
            vec!["codex".into()],
            library::batch::BatchAction::Apply,
            0,
        )
        .unwrap();
        let operation_id = plan["operationId"].as_str().unwrap().to_owned();
        assert!(!native.exists(), "preview must not write the native file");
        let (ready_started_tx, mut ready_started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (worker_started_tx, mut worker_started_rx) = oneshot::channel();
        let readiness = async move {
            let _ = ready_started_tx.send(());
            release_rx.await.unwrap();
        };
        let task = tokio::spawn(run_ready_batch_with(readiness, move || {
            let _ = worker_started_tx.send(());
            library::batch::run_batch(&db, &operation_id).map_err(|e| e.to_string())
        }));
        let waited = tokio::select! {
            ready = &mut ready_started_rx => ready.is_ok(),
            _ = &mut worker_started_rx => false,
        };
        if !waited {
            let _ = release_tx.send(());
            let _ = task.await;
            panic!("native batch callable bypassed unresolved environment readiness");
        }
        assert!(matches!(
            worker_started_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(
            !native.exists(),
            "unresolved environment must block native writes"
        );
        release_tx.send(()).unwrap();
        let result = task.await.unwrap().unwrap();
        assert_eq!(result["status"], "applied");
        let text = std::fs::read_to_string(native).unwrap();
        let parsed = library::mcp_codec::parse_native(&text, "codex").unwrap();
        assert_eq!(
            parsed["mcp_servers"]["ready-fixture"]["command"],
            "never-executed"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn batch_gate_awaits_the_real_environment_ready_entry_point() {
        let _sandbox = crate::test_sandbox::sandbox();
        let before = crate::agent_env::READY_CALLS.with(|calls| calls.get());
        let result = run_ready_batch(|| Ok(json!({"ran":true}))).await.unwrap();
        assert_eq!(result["ran"], true);
        assert_eq!(
            crate::agent_env::READY_CALLS.with(|calls| calls.get()),
            before + 1
        );
    }
}

#[tauri::command]
pub async fn get_library_inventory_source(
    state: State<'_, AppState>,
    ticket_id: String,
    entry_id: String,
    source_ref: String,
    expected_epoch: u64,
) -> Result<library::inventory::InventorySourceDetail, String> {
    crate::agent_env::ready().await;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library::inventory::source_detail(&db, &ticket_id, &entry_id, &source_ref, expected_epoch)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "library.inventoryUnavailable".to_string())?
}

#[tauri::command]
pub async fn reveal_library_inventory_source(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ticket_id: String,
    entry_id: String,
    source_ref: String,
    expected_epoch: u64,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    crate::agent_env::ready().await;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library::inventory::reveal_source(
            &db,
            &ticket_id,
            &entry_id,
            &source_ref,
            expected_epoch,
            |path| {
                app.opener().reveal_item_in_dir(path).map_err(|_| {
                    crate::error::AppError::Config("library.inventoryRevealFailed".into())
                })
            },
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "library.inventoryUnavailable".to_string())?
}

#[tauri::command]
pub async fn compare_library_mcp(
    state: State<'_, AppState>,
    left: library::inventory::McpComparisonSubject,
    right: library::inventory::McpComparisonSubject,
    expected_epoch: u64,
) -> Result<library::inventory::McpComparison, String> {
    crate::agent_env::ready().await;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library::inventory::compare_mcp(&db, &left, &right, expected_epoch)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "library.inventoryUnavailable".to_string())?
}

#[tauri::command]
pub async fn match_library_mcp_recipes(
    state: State<'_, AppState>,
    recipes: Vec<library::inventory::McpRecipe>,
    inventory_ticket_id: Option<String>,
    expected_epoch: u64,
) -> Result<Vec<library::inventory::McpRecipeMatch>, String> {
    crate::agent_env::ready().await;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library::inventory::match_recipes(
            &db,
            &recipes,
            inventory_ticket_id.as_deref(),
            expected_epoch,
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "library.inventoryUnavailable".to_string())?
}

#[cfg(test)]
mod mcp_target_tests {
    use super::*;
    #[test]
    #[serial_test::serial]
    fn empty_mcp_library_still_advertises_real_global_targets() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = crate::Database::memory().unwrap();
        let discovery = [(
            "claude".to_string(),
            crate::commands::InventoryEntry {
                tool_id: "claude".into(),
                status: "installed".into(),
                version: Some("fixture".into()),
                path: None,
                error: None,
                managed_copy: None,
            },
        )]
        .into_iter()
        .collect();
        let view = library_mcp_from_discovery(&db, &discovery).unwrap();
        assert_eq!(view["servers"], json!([]));
        let targets = view["targets"]
            .as_array()
            .expect("first MCP needs backend-proven targets before a resource exists");
        assert_eq!(targets.len(), library::mcp_targets::TOOLS.len());
        for target in targets {
            assert_eq!(target["available"], target["toolId"] == "claude");
            assert!(target["path"]
                .as_str()
                .is_some_and(|path| std::path::Path::new(path).is_absolute()));
            assert!(target["targetGroupId"].as_str().is_some());
            assert!(target.get("desired").is_none());
        }
    }
}

#[tauri::command]
pub async fn match_library_builtin_recipes(
    state: State<'_, AppState>,
    kind: library::ResourceKind,
    recipes: Vec<library::inventory::BuiltinRecipe>,
    inventory_ticket_id: Option<String>,
    expected_epoch: u64,
) -> Result<Vec<library::inventory::McpRecipeMatch>, String> {
    crate::agent_env::ready().await;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library::inventory::match_builtin_recipes(
            &db,
            kind,
            &recipes,
            inventory_ticket_id.as_deref(),
            expected_epoch,
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "library.inventoryUnavailable".to_string())?
}
