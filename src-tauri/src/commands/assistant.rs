use crate::services::assistant::{
    backup, catalog, host, policy::PermissionMode, store, types::*, AssistantHost,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInput {
    pub kind: AssistantKind,
    pub model_id: Option<String>,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub web_search: bool,
}

async fn models(app: &tauri::AppHandle) -> Result<Vec<catalog::Model>, String> {
    let route = crate::ofox_apex::current_apex().to_string();
    let host = app.state::<AssistantHost>();
    if let Some(models) = host
        .models
        .lock()
        .map_err(|_| "assistant_state_unavailable")?
        .get(&route)
        .cloned()
    {
        return Ok(models);
    }
    let models = catalog::fetch_models().await?;
    host.models
        .lock()
        .map_err(|_| "assistant_state_unavailable")?
        .insert(route, models.clone());
    Ok(models)
}

#[tauri::command]
pub async fn assistant_bootstrap(app: tauri::AppHandle) -> Result<Value, String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let (compatible, model_error) = match models(&app).await {
        Ok(m) => (m, None),
        Err(e) => (vec![], Some(e)),
    };
    let models: Vec<Value> = compatible
        .iter()
        .map(|m| serde_json::to_value(m).unwrap_or_default())
        .collect();
    let default = compatible
        .iter()
        .find(|m| m.id == "openai/gpt-6.1-sol")
        .map(|m| m.id.clone());
    let runtime = match host::runtime_paths(&app) {
        Ok(_) => json!({"available":true,"version":"1.4.0"}),
        Err(reason) => json!({"available":false,"reason":reason}),
    };
    let token = host::token(&app).await;
    let reauthorization = token
        .as_ref()
        .is_err_and(|e| e == "assistant_reauthorization_required");
    let capabilities = match token {
        Ok(token) => catalog::capabilities(&token).await,
        Err(_) => json!({"search":{"available":false,"reason":"authorization_required"}}),
    };
    let state = app.state::<crate::store::AppState>();
    let selected = |kind| {
        store::remembered_model(&state.db, &identity.owner, kind)
            .ok()
            .flatten()
            .filter(|m| compatible.iter().any(|v| &v.id == m))
            .or_else(|| default.clone())
    };
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    Ok(
        json!({"owner":identity.owner,"authorityGeneration":authority_generation,"runtime":runtime,"tools":crate::services::assistant::capabilities::tools(),"models":models,"modelUnavailableReason":model_error,"defaultModelId":default,"defaultModelIds":{"general":selected(AssistantKind::General),"configuration":selected(AssistantKind::Configuration)},"search":capabilities["search"],"reauthorizationRequired":reauthorization}),
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_list_sessions(
    app: tauri::AppHandle,
    kind: AssistantKind,
    deleted: bool,
    search: String,
) -> Result<Vec<Session>, String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let state = app.state::<crate::store::AppState>();
    let host = app.state::<AssistantHost>();
    let mut result = store::list(&state.db, &identity.owner, kind, deleted, &search)?;
    for session in &mut result {
        host.decorate(&identity.owner, session);
    }
    Ok(result)
}

#[tauri::command]
pub async fn assistant_create_session(
    app: tauri::AppHandle,
    request: CreateInput,
) -> Result<Session, String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    if request.permission_mode != PermissionMode::Ask {
        return Err("assistant_explicit_permission_required".into());
    }
    let models = models(&app).await?;
    let state = app.state::<crate::store::AppState>();
    let remembered = store::remembered_model(&state.db, &identity.owner, request.kind)?
        .filter(|m| models.iter().any(|v| &v.id == m));
    let selected = request
        .model_id
        .or(remembered)
        .or_else(|| {
            models
                .iter()
                .find(|m| m.id == "openai/gpt-6.1-sol")
                .map(|m| m.id.clone())
        })
        .ok_or("assistant_choose_model")?;
    if !models.iter().any(|m| m.id == selected) {
        return Err("assistant_model_unavailable".into());
    }
    let state = app.state::<crate::store::AppState>();
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    let detail = store::create(
        &state.db,
        &identity.owner,
        request.kind,
        &selected,
        request.web_search,
    )?;
    store::remember_model(&state.db, &identity.owner, request.kind, &selected)?;
    Ok(detail.session)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_get_session(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Detail, String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let state = app.state::<crate::store::AppState>();
    let host = app.state::<AssistantHost>();
    if host::runtime_paths(&app).is_ok() {
        host.recover(&app, &identity, &session_id).await?;
    }
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    let mut detail = store::get(&state.db, &identity.owner, &session_id)?;
    host.decorate(&identity.owner, &mut detail.session);
    detail.pending_approval = host
        .runs
        .lock()
        .map_err(|_| "assistant_state_unavailable")?
        .get(&session_id)
        .filter(|run| run.identity.owner == identity.owner)
        .and_then(|run| {
            run.approvals
                .lock()
                .ok()
                .and_then(|pending| pending.values().next().map(|p| p.details.clone()))
        });
    if detail.session.active_run.is_none() {
        for message in &mut detail.messages {
            if message.status == "streaming" {
                message.status = "stopped".into();
            }
        }
    }
    Ok(detail)
}

#[tauri::command]
pub async fn assistant_update_session(
    app: tauri::AppHandle,
    request: UpdateRequest,
) -> Result<Session, String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
        || request
            .expected_owner
            .as_ref()
            .is_some_and(|owner| owner != &identity.owner)
        || request
            .expected_generation
            .is_some_and(|generation| generation != authority_generation)
    {
        return Err("assistant_identity_changed".into());
    }
    let state = app.state::<crate::store::AppState>();
    let host = app.state::<AssistantHost>();
    let detail = store::get(&state.db, &identity.owner, &request.session_id)?;
    if let Some(mode) = request.permission_mode {
        if detail.session.kind != AssistantKind::Configuration && mode != PermissionMode::Ask {
            return Err("assistant_operation_denied".into());
        }
        if mode == PermissionMode::FullAccess && !request.full_access_acknowledged {
            return Err("assistant_full_access_acknowledgement_required".into());
        }
        host.change_permission(&identity.owner, &request.session_id, mode)?;
    }
    if let Some(model) = &request.model_id {
        if !models(&app).await?.iter().any(|m| &m.id == model) {
            return Err("assistant_model_unavailable".into());
        }
        host.cancel_session(&request.session_id);
    }
    if request.deleted == Some(true) {
        host.cancel_session(&request.session_id);
        host.change_permission(&identity.owner, &request.session_id, PermissionMode::Ask)?;
    }
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    let mut session = store::mutate(&state.db, &identity.owner, &request.session_id, |detail| {
        if host.generation.load(std::sync::atomic::Ordering::SeqCst) != authority_generation {
            return Err("assistant_identity_changed".into());
        }
        if let Some(title) = request.title {
            if title.len() > 512 {
                return Err("assistant_title_too_long".into());
            }
            detail.session.title = crate::redact::scrub(&title).into_owned();
        }
        if let Some(model) = request.model_id {
            detail.session.model_id = model;
        }
        if let Some(web) = request.web_search {
            detail.session.web_search = web;
        }
        if let Some(deleted) = request.deleted {
            detail.session.deleted_at = deleted.then(now);
        }
        if let Some(draft) = request.draft {
            if draft.len() > 128_000 {
                return Err("assistant_message_too_large".into());
            }
            detail.session.draft = crate::redact::scrub(&draft).into_owned();
        }
        if let Some(ids) = request.draft_attachment_ids {
            if ids.len() > 200
                || ids.iter().any(|id| {
                    !detail
                        .attachments
                        .iter()
                        .any(|attachment| &attachment.id == id && attachment.status == "ready")
                })
            {
                return Err("assistant_attachment_not_found".into());
            }
            detail.session.draft_attachment_ids = ids;
        }
        detail.session.permission_mode = PermissionMode::Ask;
        detail.session.updated_at = now();
        Ok(detail.session.clone())
    })?;
    store::remember_model(&state.db, &identity.owner, session.kind, &session.model_id)?;
    host.decorate(&identity.owner, &mut session);
    Ok(session)
}

#[tauri::command]
pub async fn assistant_send(app: tauri::AppHandle, request: SendRequest) -> Result<RunRef, String> {
    validate_id(&request.session_id)?;
    validate_id(&request.run_id)?;
    validate_id(&request.operation_id)?;
    if request.text.len() > 128_000
        || (request.text.trim().is_empty()
            && request.attachment_ids.is_empty()
            && request.regenerate_message_id.is_none())
    {
        return Err("assistant_invalid_message".into());
    }
    app.state::<AssistantHost>().ensure_accepting_runs()?;
    let generation = app
        .state::<AssistantHost>()
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let token = host::token(&app).await?;
    let state = app.state::<crate::store::AppState>();
    let host = app.state::<AssistantHost>();
    host.recover(&app, &identity, &request.session_id).await?;
    let detail = store::get(&state.db, &identity.owner, &request.session_id)?;
    if detail.session.deleted_at.is_some() {
        return Err("assistant_session_deleted".into());
    }
    host::runtime_paths(&app)?;
    let model = models(&app)
        .await?
        .into_iter()
        .find(|m| m.id == detail.session.model_id)
        .ok_or("assistant_model_unavailable")?;
    let root = host::data_root(&identity.owner, &request.session_id)?;
    let native_file = root.join("session.jsonl");
    host::ensure_regular(&native_file)?;
    if !native_file.exists() && !detail.messages.is_empty() {
        return Err("assistant_native_history_missing".into());
    }
    let attachment_root = host::private_subdir(&root, "attachments")?;
    let regenerated_user = request
        .regenerate_message_id
        .as_ref()
        .and_then(|mid| {
            detail
                .messages
                .iter()
                .find(|m| &m.id == mid && m.role == "assistant")
        })
        .and_then(|assistant| assistant.run_id.as_ref())
        .and_then(|rid| {
            detail
                .messages
                .iter()
                .find(|m| m.role == "user" && m.run_id.as_ref() == Some(rid))
        })
        .cloned();
    if request.regenerate_message_id.is_some() && regenerated_user.is_none() {
        return Err("assistant_branch_unavailable".into());
    }
    let attachment_ids = if let Some(user) = &regenerated_user {
        user.attachments
            .iter()
            .map(|attachment| attachment.id.clone())
            .collect()
    } else if request.attachment_ids.is_empty() {
        request
            .edit_message_id
            .as_ref()
            .and_then(|mid| {
                detail
                    .messages
                    .iter()
                    .find(|m| &m.id == mid && m.role == "user")
            })
            .map(|m| m.attachments.iter().map(|a| a.id.clone()).collect())
            .unwrap_or_else(|| request.attachment_ids.clone())
    } else {
        request.attachment_ids.clone()
    };
    let selected: Vec<Attachment> = attachment_ids
        .iter()
        .map(|id| {
            detail
                .attachments
                .iter()
                .find(|a| &a.id == id)
                .cloned()
                .ok_or("assistant_attachment_not_found")
        })
        .collect::<Result<_, _>>()?;
    let edit_entry = request
        .edit_message_id
        .as_ref()
        .map(|mid| {
            detail
                .messages
                .iter()
                .find(|m| &m.id == mid && m.role == "user")
                .and_then(|m| m.native_entry_id.clone())
                .ok_or("assistant_branch_unavailable")
        })
        .transpose()?;
    let regenerate_entry = request
        .regenerate_message_id
        .as_ref()
        .map(|mid| {
            detail
                .messages
                .iter()
                .find(|m| &m.id == mid && m.role == "assistant")
                .and_then(|m| m.native_entry_id.clone())
                .ok_or("assistant_branch_unavailable")
        })
        .transpose()?;
    let caps = if detail.session.web_search {
        catalog::capabilities(&token).await
    } else {
        json!({"search":{"available":false}})
    };
    if detail.session.web_search && caps["search"]["available"] != true {
        return Err("assistant_search_unavailable".into());
    }
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    let epoch = host.current_epoch(&identity.owner, &request.session_id)?;
    let (cancel, _) = tokio::sync::watch::channel(false);
    let control = Arc::new(host::RunControl {
        identity: identity.clone(),
        session_id: request.session_id.clone(),
        run_id: request.run_id.clone(),
        operation_id: request.operation_id.clone(),
        message_id: id(),
        cancel,
        epoch,
        deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(900),
        outcome_unknown: std::sync::atomic::AtomicBool::new(false),
        approvals: Default::default(),
        secret: token.clone(),
        search_capabilities: caps.clone(),
    });
    host.register_run(control.clone(), generation, || {
        store::mutate(&state.db, &identity.owner, &request.session_id, |detail| {
            store::ensure_new_operation(detail, &request.operation_id)?;
            let revision = request
                .edit_message_id
                .as_ref()
                .or_else(|| regenerated_user.as_ref().map(|m| &m.id));
            if let Some(mid) = revision {
                if let Some(index) = detail.messages.iter().position(|m| &m.id == mid) {
                    for m in &mut detail.messages[index..] {
                        m.superseded = true;
                    }
                }
            }
            let user_text = regenerated_user
                .as_ref()
                .map(|u| u.content.as_str())
                .unwrap_or(&request.text);
            let user_attachments = regenerated_user
                .as_ref()
                .map(|u| u.attachments.clone())
                .unwrap_or_else(|| selected.clone());
            detail.messages.push(Message {
                id: id(),
                role: "user".into(),
                content: crate::redact::scrub(user_text).replace(&token, crate::redact::REDACTED),
                created_at: now(),
                reasoning: None,
                status: "completed".into(),
                attachments: user_attachments,
                sources: vec![],
                superseded: false,
                revision_of: revision.cloned(),
                native_entry_id: None,
                run_id: Some(request.run_id.clone()),
            });
            detail.messages.push(Message {
                id: control.message_id.clone(),
                role: "assistant".into(),
                content: String::new(),
                created_at: now(),
                reasoning: None,
                status: "streaming".into(),
                attachments: vec![],
                sources: vec![],
                superseded: false,
                revision_of: request.regenerate_message_id.clone(),
                native_entry_id: None,
                run_id: Some(request.run_id.clone()),
            });
            if detail.session.title.is_empty() {
                detail.session.title = crate::redact::scrub(&request.text)
                    .replace(&token, crate::redact::REDACTED)
                    .chars()
                    .take(48)
                    .collect();
            }
            store::consume_draft(detail, &request);
            detail.session.updated_at = now();
            detail.session.active_run = Some(RunRef {
                run_id: request.run_id.clone(),
                operation_id: request.operation_id.clone(),
            });
            detail.operations.push(
                json!({"id":request.operation_id,"title":"assistant_request","status":"running"}),
            );
            Ok(())
        })
    })?;
    let attachments:Vec<Value>=detail.attachments.iter().map(|a|json!({"id":a.id,"path":attachment_root.join(&a.id),"name":a.name,"mimeType":a.mime_type})).collect();
    let memory = detail
        .memories
        .iter()
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let text = crate::redact::scrub(&request.text).replace(&token, crate::redact::REDACTED);
    let mut allowed_urls = extract_urls(&text);
    for message in detail.messages.iter().filter(|m| !m.superseded) {
        if message.role == "user" {
            allowed_urls.extend(extract_urls(&message.content));
        }
        allowed_urls.extend(
            message
                .sources
                .iter()
                .filter_map(|s| s["url"].as_str().map(str::to_owned)),
        );
    }
    allowed_urls.sort();
    allowed_urls.dedup();
    let permission_mode = {
        let mut session = detail.session.clone();
        host.decorate(&identity.owner, &mut session);
        session.permission_mode
    };
    let params = json!({"sessionId":request.session_id,"runId":request.run_id,"operationId":request.operation_id,"sessionFile":native_file,"kind":detail.session.kind,"model":model,"accessToken":token,"apex":identity.apex,"baseUrl":crate::ofox_apex::gateway_base_for(&identity.apex),"allowLocalGateway":cfg!(debug_assertions) && crate::ofox_apex::gateway_base_for(&identity.apex)=="http://localhost:8088","prompt":text,"originalPrompt":regenerated_user.as_ref().map(|user| user.content.as_str()),"memory":memory,"attachmentRoot":attachment_root,"attachments":attachments,"selectedAttachmentIds":attachment_ids,"editEntryId":edit_entry,"regenerateEntryId":regenerate_entry,"webSearch":detail.session.web_search,"searchCapabilities":caps["search"],"toolCapabilities":crate::services::assistant::capabilities::tools(),"permissionMode":permission_mode,"allowedWebUrls":allowed_urls});
    host::emit(
        &app,
        &control,
        "run_start",
        json!({"messageId":control.message_id}),
    );
    let result = RunRef {
        run_id: request.run_id,
        operation_id: request.operation_id,
    };
    let mut tasks = host
        .tasks
        .lock()
        .map_err(|_| "assistant_state_unavailable")?;
    let task = tauri::async_runtime::spawn(host::run(app.clone(), control, params));
    tasks.insert(result.run_id.clone(), task);
    Ok(result)
}

fn extract_urls(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|part| {
            let trimmed = part.trim_matches(|c: char| {
                matches!(
                    c,
                    '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | ',' | ';'
                )
            });
            let parsed = url::Url::parse(trimmed).ok()?;
            (parsed.scheme() == "https"
                && parsed.username().is_empty()
                && parsed.password().is_none())
            .then(|| parsed.to_string())
        })
        .collect()
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_stop(
    app: tauri::AppHandle,
    session_id: String,
    run_id: String,
    operation_id: String,
) -> Result<(), String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let host = app.state::<AssistantHost>();
    validate_id(&session_id)?;
    validate_id(&run_id)?;
    validate_id(&operation_id)?;
    host.request_stop(
        &app.state::<crate::store::AppState>().db,
        &identity,
        &session_id,
        &run_id,
        &operation_id,
    )
}

#[tauri::command]
pub async fn assistant_respond_approval(
    app: tauri::AppHandle,
    request: ApprovalRequest,
) -> Result<(), String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let host = app.state::<AssistantHost>();
    let run = host
        .runs
        .lock()
        .map_err(|_| "assistant_state_unavailable")?
        .get(&request.session_id)
        .cloned()
        .ok_or("assistant_stale_operation")?;
    if run.identity != identity
        || run.run_id != request.run_id
        || run.operation_id != request.operation_id
    {
        return Err("assistant_stale_operation".into());
    }
    host.validate_run(&run)?;
    if !matches!(
        request.decision.as_str(),
        "allowOnce" | "allowSession" | "deny"
    ) {
        return Err("assistant_invalid_approval".into());
    }
    let tx = run
        .approvals
        .lock()
        .map_err(|_| "assistant_state_unavailable")?
        .remove(&request.approval_id)
        .ok_or("assistant_stale_approval")?;
    if request.decision == "allowSession" {
        let mut grants = host
            .grants
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        let grant = grants
            .get_mut(&format!("{}:{}", identity.owner, request.session_id))
            .ok_or("assistant_stale_approval")?;
        if grant.epoch != run.epoch {
            return Err("assistant_stale_approval".into());
        }
        grant.approved.insert(tx.signature);
    }
    tx.sender
        .send(request.decision != "deny")
        .map_err(|_| "assistant_stale_approval".into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_pick_attachments(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Vec<Attachment>, String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let state = app.state::<crate::store::AppState>();
    store::get(&state.db, &identity.owner, &session_id)?;
    let dialog_app = app.clone();
    let files = tauri::async_runtime::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .add_filter(
                "Documents",
                &[
                    "png", "jpg", "jpeg", "webp", "gif", "txt", "md", "csv", "json", "pdf", "docx",
                    "xlsx", "pptx",
                ],
            )
            .blocking_pick_files()
    })
    .await
    .map_err(|_| "assistant_file_dialog_failed")?
    .unwrap_or_default();
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    let root = host::private_subdir(
        &host::data_root(&identity.owner, &session_id)?,
        "attachments",
    )?;
    if store::get(&state.db, &identity.owner, &session_id)?
        .session
        .deleted_at
        .is_some()
    {
        return Err("assistant_session_deleted".into());
    }
    if files.len() > 10 {
        return Err("assistant_attachment_too_large".into());
    }
    let current = store::get(&state.db, &identity.owner, &session_id)?;
    if current.attachments.len() + files.len() > 200 {
        return Err("assistant_attachment_too_large".into());
    }
    let mut total = current
        .attachments
        .iter()
        .map(|a| a.size_bytes)
        .sum::<u64>();
    let mut txn = crate::config::FileTxn::new();
    let result = (|| -> Result<Vec<Attachment>, String> {
        let mut items = Vec::new();
        for file in files {
            let source = file
                .into_path()
                .map_err(|_| "assistant_attachment_unreadable")?;
            let metadata =
                std::fs::metadata(&source).map_err(|_| "assistant_attachment_unreadable")?;
            if !metadata.is_file() || metadata.len() > 25 * 1024 * 1024 {
                return Err("assistant_attachment_too_large".into());
            }
            let extension = source
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mime = match extension.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "gif" => "image/gif",
                "pdf" => "application/pdf",
                "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "pptx" => {
                    "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                }
                "txt" | "md" | "csv" | "json" => "text/plain",
                _ => return Err("assistant_attachment_unsupported".into()),
            };
            let mut item = Attachment {
                id: id(),
                name: source
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attachment")
                    .into(),
                mime_type: mime.into(),
                size_bytes: metadata.len(),
                status: "ready".into(),
                reason: None,
                preview_url: None,
            };
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(&source)
                .map_err(|_| "assistant_attachment_unreadable")?
                .take(25 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "assistant_attachment_unreadable")?;
            if bytes.len() > 25 * 1024 * 1024 {
                return Err("assistant_attachment_too_large".into());
            }
            item.size_bytes = bytes.len() as u64;
            total += bytes.len() as u64;
            if total > 500 * 1024 * 1024 {
                return Err("assistant_attachment_too_large".into());
            }
            txn.write_with_mode(&root.join(&item.id), &bytes, 0o600)
                .map_err(|_| "assistant_attachment_write_failed")?;
            items.push(item);
        }
        store::mutate(&state.db, &identity.owner, &session_id, |detail| {
            let count = detail.attachments.len() + items.len();
            let actual_total = detail
                .attachments
                .iter()
                .chain(items.iter())
                .map(|item| item.size_bytes)
                .sum::<u64>();
            if detail.session.deleted_at.is_some() {
                return Err("assistant_session_deleted".into());
            }
            if count > 200 || actual_total > 500 * 1024 * 1024 {
                return Err("assistant_attachment_too_large".into());
            }
            detail.attachments.extend(items.clone());
            Ok(())
        })?;
        Ok(items)
    })();
    if result.is_err() {
        txn.rollback()
            .map_err(|_| "assistant_attachment_cleanup_failed")?;
    }
    result
}

#[tauri::command]
pub async fn assistant_save_memory(
    app: tauri::AppHandle,
    request: MemoryRequest,
) -> Result<Memory, String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let state = app.state::<crate::store::AppState>();
    let detail = store::get(&state.db, &identity.owner, &request.session_id)?;
    store::save_memory(
        &state.db,
        &identity.owner,
        detail.session.kind,
        request.id.as_deref(),
        &request.content,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_delete_memory(
    app: tauri::AppHandle,
    session_id: String,
    memory_id: String,
) -> Result<(), String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let state = app.state::<crate::store::AppState>();
    let detail = store::get(&state.db, &identity.owner, &session_id)?;
    store::delete_memory(&state.db, &identity.owner, detail.session.kind, &memory_id)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_export_session(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Value, String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let state = app.state::<crate::store::AppState>();
    let value = store::readable_export(store::get(&state.db, &identity.owner, &session_id)?)?;
    let dialog_app = app.clone();
    let filename = format!("ofox-conversation-{session_id}.json");
    let target = tauri::async_runtime::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .add_filter("JSON", &["json"])
            .set_file_name(filename)
            .blocking_save_file()
    })
    .await
    .map_err(|_| "assistant_file_dialog_failed")?;
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    let Some(target) = target else {
        return Ok(json!({"path":null}));
    };
    let path = target.into_path().map_err(|_| "assistant_export_failed")?;
    crate::config::atomic_write(
        &path,
        &serde_json::to_vec_pretty(&value).map_err(|_| "assistant_export_failed")?,
    )
    .map_err(|_| "assistant_export_failed")?;
    Ok(json!({"path":path.to_string_lossy()}))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_purge_session(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    let state = app.state::<crate::store::AppState>();
    let host = app.state::<AssistantHost>();
    if host
        .runs
        .lock()
        .map_err(|_| "assistant_state_unavailable")?
        .contains_key(&session_id)
    {
        return Err("assistant_session_busy".into());
    }
    let detail = store::get(&state.db, &identity.owner, &session_id)?;
    if detail.session.deleted_at.is_none() {
        return Err("assistant_trash_required".into());
    }
    let root = host::data_root(&identity.owner, &session_id)?;
    std::fs::remove_dir_all(root).map_err(|_| "assistant_purge_failed")?;
    store::purge(&state.db, &identity.owner, &session_id)
}

#[tauri::command]
pub async fn assistant_usage(app: tauri::AppHandle) -> Result<Value, String> {
    let identity = host::identity(&app).await?;
    let assistant_host = app.state::<AssistantHost>();
    let authority_generation = assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let _data = assistant_host.data_gate.read().await;
    if assistant_host
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != authority_generation
        || host::identity(&app).await? != identity
    {
        return Err("assistant_identity_changed".into());
    }
    store::usage(&app.state::<crate::store::AppState>().db, &identity.owner)
}

fn backup_identity_current(
    app: &tauri::AppHandle,
    identity: &host::Identity,
    generation: u64,
) -> Result<(), String> {
    if app
        .state::<AssistantHost>()
        .generation
        .load(std::sync::atomic::Ordering::SeqCst)
        != generation
        || crate::ofox_apex::current_apex() != identity.apex
    {
        return Err("assistant_identity_changed".into());
    }
    let auth = app.state::<crate::commands::ofox_auth::OfoxAuthState>();
    let manager = auth
        .0
        .try_read()
        .map_err(|_| "assistant_identity_changed")?;
    if !manager.is_authenticated() {
        return Err("assistant_identity_changed".into());
    }
    let user = manager
        .get_user_info()
        .ok_or("assistant_identity_changed")?
        .key_owner();
    if store::namespace(
        user.user_id.as_deref().unwrap_or(""),
        user.org_id.as_deref().unwrap_or(""),
    )? != identity.owner
    {
        return Err("assistant_identity_changed".into());
    }
    Ok(())
}
fn backup_idle(host: &AssistantHost, owner: &str) -> Result<(), String> {
    if host
        .runs
        .lock()
        .map_err(|_| "assistant_state_unavailable")?
        .values()
        .any(|run| run.identity.owner == owner)
    {
        return Err("assistant_backup_busy".into());
    }
    Ok(())
}
fn backup_root(owner: &str) -> Result<std::path::PathBuf, String> {
    let parent = host::private_subdir(&crate::config::get_app_config_dir(), "assistant")?;
    host::private_subdir(&parent, owner)
}
#[tauri::command]
pub async fn assistant_backup(app: tauri::AppHandle) -> Result<Value, String> {
    let identity = host::identity(&app).await?;
    let host = app.state::<AssistantHost>();
    backup_idle(&host, &identity.owner)?;
    let dialog_app = app.clone();
    let target = tauri::async_runtime::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .add_filter("Ofox Assistant", &["ofox-assistant"])
            .set_file_name("ofox-assistant-backup.ofox-assistant")
            .blocking_save_file()
    })
    .await
    .map_err(|_| "assistant_file_dialog_failed")?;
    let Some(target) = target else {
        return Ok(json!({"path":null}));
    };
    let target = target.into_path().map_err(|_| "assistant_export_failed")?;
    let _data = host.data_gate.write().await;
    backup_idle(&host, &identity.owner)?;
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    let root = backup_root(&identity.owner)?;
    let worker_app = app.clone();
    let owner = identity.owner.clone();
    let generation = host.generation.load(std::sync::atomic::Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<crate::store::AppState>();
        backup::recover(&state.db, &root, &owner)?;
        let private_root = crate::config::get_app_config_dir()
            .canonicalize()
            .map_err(|_| "assistant_backup_io_failed")?;
        let resolved =
            crate::config::write_target(&target).map_err(|_| "assistant_backup_io_failed")?;
        let parent = resolved
            .parent()
            .ok_or("assistant_backup_invalid")?
            .canonicalize()
            .map_err(|_| "assistant_backup_io_failed")?;
        if parent.starts_with(private_root) {
            return Err("assistant_backup_invalid_target".into());
        }
        backup::create_guarded(&state.db, &owner, &root, &target, || {
            backup_identity_current(&worker_app, &identity, generation)
        })?;
        Ok::<_, String>(json!({"path":target.to_string_lossy()}))
    })
    .await
    .map_err(|_| "assistant_backup_io_failed")?
}
#[tauri::command]
pub async fn assistant_preview_restore(app: tauri::AppHandle) -> Result<Value, String> {
    let identity = host::identity(&app).await?;
    let host = app.state::<AssistantHost>();
    backup_idle(&host, &identity.owner)?;
    let dialog_app = app.clone();
    let source = tauri::async_runtime::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .add_filter("Ofox Assistant", &["ofox-assistant"])
            .blocking_pick_file()
    })
    .await
    .map_err(|_| "assistant_file_dialog_failed")?;
    let Some(source) = source else {
        return Ok(Value::Null);
    };
    let path = source.into_path().map_err(|_| "assistant_backup_invalid")?;
    let _data = host.data_gate.write().await;
    backup_idle(&host, &identity.owner)?;
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    let root = backup_root(&identity.owner)?;
    let owner = identity.owner.clone();
    let worker_app = app.clone();
    let (archive, report) = tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<crate::store::AppState>();
        backup::recover(&state.db, &root, &owner)?;
        let archive = backup::inspect(&path, &owner)?;
        let report = backup::preview(&state.db, &owner, &root, &archive)?;
        Ok::<_, String>((archive, report))
    })
    .await
    .map_err(|_| "assistant_backup_io_failed")??;
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    let receipt = id();
    let mut previews = host
        .restore_previews
        .lock()
        .map_err(|_| "assistant_state_unavailable")?;
    previews.retain(|_, item| item.expires > std::time::Instant::now());
    if previews.len() >= 4 {
        previews.clear();
    }
    previews.insert(
        receipt.clone(),
        backup::RestoreReceipt {
            owner: identity.owner,
            apex: identity.apex,
            expires: std::time::Instant::now() + std::time::Duration::from_secs(900),
            generation: host.generation.load(std::sync::atomic::Ordering::SeqCst),
            report: report.clone(),
            archive,
        },
    );
    let mut report = serde_json::to_value(report).map_err(|_| "assistant_backup_invalid")?;
    report["receipt"] = json!(receipt);
    Ok(report)
}
#[tauri::command]
pub async fn assistant_restore(
    app: tauri::AppHandle,
    receipt: String,
) -> Result<backup::Report, String> {
    let identity = host::identity(&app).await?;
    let host = app.state::<AssistantHost>();
    let _data = host.data_gate.write().await;
    backup_idle(&host, &identity.owner)?;
    let preview = host
        .restore_previews
        .lock()
        .map_err(|_| "assistant_state_unavailable")?
        .remove(&receipt)
        .ok_or("assistant_backup_preview_expired")?;
    if preview.generation != host.generation.load(std::sync::atomic::Ordering::SeqCst)
        || preview.owner != identity.owner
        || preview.apex != identity.apex
        || preview.expires <= std::time::Instant::now()
    {
        return Err("assistant_backup_preview_expired".into());
    }
    let root = backup_root(&identity.owner)?;
    if host::identity(&app).await? != identity {
        return Err("assistant_identity_changed".into());
    }
    // Reset transient permission/recovery caches; restored operations never replay.
    host.cancel_all();
    host.reset_recovery();
    let generation = host.generation.load(std::sync::atomic::Ordering::SeqCst);
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<crate::store::AppState>();
        if backup::preview(&state.db, &identity.owner, &root, &preview.archive)? != preview.report {
            return Err("assistant_backup_local_changed".into());
        }
        backup::apply_guarded(&state.db, &identity.owner, &root, &preview.archive, || {
            backup_identity_current(&worker_app, &identity, generation)
        })
    })
    .await
    .map_err(|_| "assistant_backup_io_failed")?
}
