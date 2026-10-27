//! What each `assistant_*` command does. The commands only find the host and
//! call these, so tests drive exactly what the renderer drives.
use super::{
    attachments, catalog,
    host::{blocking, local_gateway_allowed, AssistantHost, Identity, RunControl},
    store::{self, SessionList, SessionRecord},
    types::*,
    web,
};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_MESSAGE_BYTES: usize = 128_000;
const TITLE_CHARS: usize = 48;

impl AssistantHost {
    /// Everything the assistant page needs to open. Never mints a key.
    pub async fn bootstrap(&self) -> Result<Value, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let (models, model_error) = match self.models(&identity.apex).await {
            Ok(models) => (models, None),
            Err(error) => (vec![], Some(error)),
        };
        // A damaged preferences file only loses the remembered choice.
        let remembered = blocking({
            let store = self.store(&identity.owner)?;
            move || store.remembered_model()
        })
        .await
        .unwrap_or_default();
        let default_model_id = catalog::default_model_id(&models, remembered.as_deref());
        let search = search_status(&models);
        let runtime = match self.runtime() {
            Ok(runtime) => json!({"available": true, "reason": null, "version": runtime.version}),
            Err(reason) => json!({"available": false, "reason": reason, "version": null}),
        };
        let key = if self.env.key_ready(&identity).await {
            "ready"
        } else {
            "missing"
        };
        self.ensure_current(&identity, generation).await?;
        Ok(json!({
            "owner": identity.owner,
            "authorityGeneration": generation,
            "runtime": runtime,
            "models": models,
            "defaultModelId": default_model_id,
            "modelUnavailableReason": model_error,
            "key": {"status": key},
            "search": search,
        }))
    }

    pub async fn list_sessions(&self, deleted: bool, search: &str) -> Result<SessionList, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let mut list = blocking({
            let store = self.store(&identity.owner)?;
            let search = search.to_owned();
            move || store.list(deleted, &search)
        })
        .await?;
        self.ensure_current(&identity, generation).await?;
        for session in &mut list.sessions {
            self.decorate(&identity.owner, session);
        }
        Ok(list)
    }

    pub async fn create_session(&self, request: CreateRequest) -> Result<Session, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let store = self.store(&identity.owner)?;
        let selected = match request.model_id {
            Some(model) => self.model(&identity.apex, &model).await?.id,
            None => {
                let models = self.models(&identity.apex).await?;
                let remembered = blocking({
                    let store = self.store(&identity.owner)?;
                    move || store.remembered_model()
                })
                .await
                .unwrap_or_default();
                catalog::default_model_id(&models, remembered.as_deref())
                    .ok_or("assistant_choose_model")?
            }
        };
        self.ensure_current(&identity, generation).await?;
        let detail = blocking(move || {
            let detail = store.create(&selected, request.web_search)?;
            store.remember_model(&selected)?;
            Ok(detail)
        })
        .await?;
        Ok(detail.session)
    }

    /// One conversation. A run the previous process left behind is settled
    /// first; if that fails the saved state is shown as it is.
    pub async fn get_session(&self, sid: &str) -> Result<Detail, String> {
        validate_id(sid)?;
        let identity = self.identity().await?;
        let generation = self.generation();
        if let Err(error) = self.recover(&identity, sid).await {
            log::warn!("[Assistant] could not recover session {sid}: {error}");
        }
        let mut detail = blocking({
            let store = self.store(&identity.owner)?;
            let sid = sid.to_owned();
            move || store.load_detail(&sid)
        })
        .await?;
        self.ensure_current(&identity, generation).await?;
        self.decorate(&identity.owner, &mut detail.session);
        if detail.session.active_run.is_none() {
            for message in &mut detail.messages {
                if message.status == "streaming" {
                    message.status = "stopped".into();
                }
            }
        }
        Ok(detail)
    }

    /// Title, model, draft, web search and trash state. Changing the model or
    /// trashing the session stops its run, once the change is saved.
    pub async fn update_session(
        self: &Arc<Self>,
        mut request: UpdateRequest,
    ) -> Result<Session, String> {
        validate_id(&request.session_id)?;
        let identity = self.identity().await?;
        let generation = self.generation();
        if request
            .expected_owner
            .as_ref()
            .is_some_and(|owner| *owner != identity.owner)
            || request
                .expected_generation
                .is_some_and(|expected| expected != generation)
        {
            return Err(super::host::IDENTITY_CHANGED.into());
        }
        if let Some(model) = &request.model_id {
            self.model(&identity.apex, model).await?;
        }
        // The user's own words: only a key this process handed out goes.
        let keys = self.known_keys();
        for text in [&mut request.title, &mut request.draft]
            .into_iter()
            .flatten()
        {
            *text = super::host::scrub_secrets(text, &keys);
        }
        let stops_run = request.model_id.is_some() || request.deleted == Some(true);
        let sid = request.session_id.clone();
        self.ensure_current(&identity, generation).await?;
        let (mut session, remembered) = blocking({
            let store = self.store(&identity.owner)?;
            let host = self.clone();
            move || {
                let session = store.update(&request, || {
                    if host.generation() != generation {
                        return Err(super::host::IDENTITY_CHANGED.into());
                    }
                    Ok(())
                })?;
                let remembered = match &request.model_id {
                    Some(_) => store.remember_model(&session.model_id),
                    None => Ok(()),
                };
                Ok((session, remembered))
            }
        })
        .await?;
        // The change is saved: the old model's run stops even if remembering
        // the choice failed.
        if stops_run {
            self.cancel_session(&sid);
        }
        remembered?;
        self.decorate(&identity.owner, &mut session);
        Ok(session)
    }

    /// Send a message, or edit / regenerate an earlier turn. Returns once the
    /// run is registered; the reply arrives as `assistant:event`s.
    pub async fn send(self: &Arc<Self>, request: SendRequest) -> Result<RunRef, String> {
        validate_id(&request.session_id)?;
        validate_id(&request.run_id)?;
        validate_id(&request.operation_id)?;
        check_message(&request)?;
        self.ensure_accepting_runs()?;
        let generation = self.generation();
        let identity = self.identity().await?;
        self.recover(&identity, &request.session_id).await?;
        let (record, native_file, attachment_root) = blocking({
            let store = self.store(&identity.owner)?;
            let sid = request.session_id.clone();
            move || {
                Ok((
                    store.load_record(&sid)?,
                    store.native_history_path(&sid)?,
                    store.attachment_root(&sid)?,
                ))
            }
        })
        .await?;
        if record.session.deleted_at.is_some() {
            return Err("assistant_session_deleted".into());
        }
        self.runtime()?;
        let model = self.model(&identity.apex, &record.session.model_id).await?;
        // Native history is the conversation's context: once a turn reached
        // the runtime, its history must still be there.
        if !native_file.is_file() && record.messages.iter().any(|m| m.native_entry_id.is_some()) {
            return Err("assistant_native_history_missing".into());
        }
        let branch = Branch::resolve(&record, &request)?;
        let selected = selection(&record, &request, &branch)?;
        // Search only at a price the catalog gives; the run keeps it.
        let search_price = if record.session.web_search {
            catalog::search_price(&self.models(&identity.apex).await?)
        } else {
            None
        };
        let base_url = self.env.gateway_base(&identity.apex)?;
        let proxy = blocking({
            let env = self.env.clone();
            move || Ok(env.proxy())
        })
        .await?;
        // Checked again under the key lock: a logout that revokes keys while
        // this send waits for the lock leaves it with no key.
        self.ensure_current(&identity, generation).await?;
        let admit = move || self.admit(generation);
        let key = self.env.api_key(&identity, None, &admit).await?;
        self.ensure_current(&identity, generation).await?;

        self.remember_key(&key);
        let control = Arc::new(RunControl::new(identity.clone(), &request, key.clone()));
        let text = super::host::scrub_secrets(&request.text, std::slice::from_ref(&key));
        blocking({
            let host = self.clone();
            let store = self.store(&identity.owner)?;
            let control = control.clone();
            let request = request.clone();
            let branch = branch.clone();
            let text = text.clone();
            let selected = selected.clone();
            let search_price = search_price.clone();
            move || {
                host.register_run(control.clone(), generation, || {
                    store.mutate(&request.session_id, |record| {
                        publish_turn(
                            record,
                            &control,
                            &request,
                            &branch,
                            &text,
                            &selected,
                            search_price,
                        )
                    })
                })
            }
        })
        .await?;

        let mut params = json!({
            "sessionId": request.session_id,
            "runId": request.run_id,
            "operationId": request.operation_id,
            "sessionFile": native_file,
            "model": model,
            "apiKey": key,
            "apex": identity.apex,
            "baseUrl": base_url,
            "allowLocalGateway": local_gateway_allowed(&base_url),
            "proxy": proxy,
            "prompt": text,
            "editEntryId": branch.edit_entry,
            "regenerateEntryId": branch.regenerate_entry,
        });
        if let Some(params) = params.as_object_mut() {
            params.extend(attachment_params(
                &record,
                &attachment_root,
                &selected,
                &branch,
            ));
            let prompt = branch.resent_text.as_deref().unwrap_or(&text);
            params.extend(web_params(&record, &branch, prompt, search_price.as_ref()));
        }
        self.emit_start(&control);
        self.spawn_run(control, params);
        Ok(RunRef {
            run_id: request.run_id,
            operation_id: request.operation_id,
        })
    }

    /// Let the user pick files and import them into a conversation. `pick`
    /// shows the file dialog and returns the chosen paths (none when it was
    /// cancelled); it blocks, so it runs off the async workers. The account,
    /// the authority and the session are checked before the dialog opens and
    /// again before anything is saved.
    pub async fn pick_attachments(
        self: &Arc<Self>,
        sid: &str,
        pick: impl FnOnce() -> Result<Vec<PathBuf>, String> + Send + 'static,
    ) -> Result<Vec<Attachment>, String> {
        validate_id(sid)?;
        let identity = self.identity().await?;
        let generation = self.generation();
        let record = blocking({
            let store = self.store(&identity.owner)?;
            let sid = sid.to_owned();
            move || store.load_record(&sid)
        })
        .await?;
        if record.session.deleted_at.is_some() {
            return Err("assistant_session_deleted".into());
        }
        self.ensure_current(&identity, generation).await?;
        let sources = tokio::task::spawn_blocking(pick)
            .await
            .map_err(|_| attachments::DIALOG_FAILED.to_string())??;
        if sources.is_empty() {
            return Ok(vec![]);
        }
        self.ensure_current(&identity, generation).await?;
        let store = self.store(&identity.owner)?;
        let host = self.clone();
        let sid = sid.to_owned();
        blocking(move || {
            attachments::import(&store, &sid, &sources, || {
                if host.generation() != generation {
                    return Err(super::host::IDENTITY_CHANGED.into());
                }
                Ok(())
            })
        })
        .await
    }

    pub async fn stop(self: &Arc<Self>, sid: &str, rid: &str, op: &str) -> Result<(), String> {
        validate_id(sid)?;
        validate_id(rid)?;
        validate_id(op)?;
        let identity: Identity = self.identity().await?;
        let host = self.clone();
        let store = self.store(&identity.owner)?;
        let (sid, rid, op) = (sid.to_owned(), rid.to_owned(), op.to_owned());
        blocking(move || host.request_stop(&store, &identity, &sid, &rid, &op)).await
    }
}

/// The checks a send needs no state for: the text's size, and text or a file
/// unless it regenerates (which sends its turn again).
fn check_message(request: &SendRequest) -> Result<(), String> {
    if request.text.len() > MAX_MESSAGE_BYTES {
        return Err("assistant_message_too_large".into());
    }
    if (request.text.trim().is_empty()
        && request.attachment_ids.is_empty()
        && request.regenerate_message_id.is_none())
        || (request.edit_message_id.is_some() && request.regenerate_message_id.is_some())
    {
        return Err("assistant_invalid_message".into());
    }
    // No conversation holds more files than this.
    if request.attachment_ids.len() > attachments::MAX_PER_SESSION {
        return Err("assistant_attachment_not_found".into());
    }
    Ok(())
}

/// Which earlier turn a send rewrites, if any.
#[derive(Clone, Default)]
struct Branch {
    /// The user message an edit or a regenerate replaces from.
    revision_of: Option<String>,
    /// For a regenerate: the user text it sends again.
    resent_text: Option<String>,
    /// The files the replaced user message sent.
    replaced_attachments: Vec<String>,
    edit_entry: Option<String>,
    regenerate_entry: Option<String>,
}

impl Branch {
    fn resolve(record: &SessionRecord, request: &SendRequest) -> Result<Self, String> {
        const UNAVAILABLE: &str = "assistant_branch_unavailable";
        let find = |id: &str, role: &str| {
            record
                .messages
                .iter()
                .find(|m| m.id == id && m.role == role)
        };
        let sent = |user: &Message| user.attachments.iter().map(|a| a.id.clone()).collect();
        if let Some(mid) = &request.edit_message_id {
            let user = find(mid, "user").ok_or(UNAVAILABLE)?;
            return Ok(Self {
                revision_of: Some(user.id.clone()),
                replaced_attachments: sent(user),
                edit_entry: Some(user.native_entry_id.clone().ok_or(UNAVAILABLE)?),
                ..Self::default()
            });
        }
        if let Some(mid) = &request.regenerate_message_id {
            let assistant = find(mid, "assistant").ok_or(UNAVAILABLE)?;
            let entry = assistant.native_entry_id.clone().ok_or(UNAVAILABLE)?;
            let run = assistant.run_id.as_deref().ok_or(UNAVAILABLE)?;
            let user = record
                .messages
                .iter()
                .find(|m| m.role == "user" && m.run_id.as_deref() == Some(run))
                .ok_or(UNAVAILABLE)?;
            return Ok(Self {
                revision_of: Some(user.id.clone()),
                resent_text: Some(user.content.clone()),
                replaced_attachments: sent(user),
                regenerate_entry: Some(entry),
                ..Self::default()
            });
        }
        Ok(Self::default())
    }
}

/// The files a send carries, in order and each once: a regenerate sends its
/// turn's files again, an edit without a selection of its own keeps the
/// edited turn's, and any other send takes `attachmentIds` (a regenerate
/// ignores them, as it ignores its text). Every one must be a selectable
/// file of the conversation. The copies are `ready`: this turn's outcome
/// arrives with its run.
fn selection(
    record: &SessionRecord,
    request: &SendRequest,
    branch: &Branch,
) -> Result<Vec<Attachment>, String> {
    let keeps_turn = request.regenerate_message_id.is_some()
        || (request.edit_message_id.is_some() && request.attachment_ids.is_empty());
    let ids = if keeps_turn {
        &branch.replaced_attachments
    } else {
        &request.attachment_ids
    };
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(id.as_str()))
        .map(|id| {
            record
                .attachments
                .iter()
                .find(|a| &a.id == id && a.selectable())
                .map(|a| Attachment {
                    status: "ready".into(),
                    reason: None,
                    details: None,
                    ..a.clone()
                })
                .ok_or_else(|| "assistant_attachment_not_found".to_string())
        })
        .collect()
}

/// The attachment part of a run's parameters (assistant-runtime README,
/// "Attachments"): every file of the conversation, kept as `<root>/<id>`, and
/// this turn's selection, always given since the runtime's default is none.
/// A regenerate also gives the user's own text to rebuild its turn from.
fn attachment_params(
    record: &SessionRecord,
    root: &Path,
    selected: &[Attachment],
    branch: &Branch,
) -> Map<String, Value> {
    let files: Vec<Value> = record
        .attachments
        .iter()
        .map(|a| {
            json!({
                "id": a.id, "name": a.name, "mimeType": a.mime_type,
                "sizeBytes": a.size_bytes, "path": a.id,
            })
        })
        .collect();
    let selected: Vec<&str> = selected.iter().map(|a| a.id.as_str()).collect();
    let mut params = Map::new();
    params.insert("attachmentRoot".into(), json!(root));
    params.insert("attachments".into(), json!(files));
    params.insert("selectedAttachmentIds".into(), json!(selected));
    if let Some(text) = &branch.resent_text {
        params.insert("originalPrompt".into(), json!(text));
    }
    params
}

/// The web search part of a run's parameters: `webSearch` only with a search
/// price (the session's switch is on and the catalog prices the search
/// model), and then `allowedWebUrls` (see [`web::allowed_web_urls`]) from
/// the prompt and the turns before it on the conversation's current branch.
fn web_params(
    record: &SessionRecord,
    branch: &Branch,
    prompt: &str,
    search: Option<&catalog::SearchPrice>,
) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("webSearch".into(), json!(search.is_some()));
    if search.is_none() {
        return params;
    }
    // An edit or regenerate starts again from the turn it replaces.
    let end = branch
        .revision_of
        .as_ref()
        .and_then(|mid| record.messages.iter().position(|m| &m.id == mid))
        .unwrap_or(record.messages.len());
    let earlier = record.messages[..end]
        .iter()
        .rev()
        .filter(|m| !m.superseded);
    let urls = web::allowed_web_urls(prompt, earlier);
    params.insert("allowedWebUrls".into(), json!(urls));
    params
}

/// Bootstrap's `search`: whether runs can search, and the price of one
/// search (the search model's tokens come on top).
fn search_status(models: &[catalog::Model]) -> Value {
    let price = catalog::search_price(models);
    json!({
        "available": price.is_some(),
        "unitPriceUsd": price.map(|p| p.per_search.normalize().to_string()),
        "model": catalog::SEARCH_MODEL,
    })
}

/// Save the new turn as the run registers: the user message with the files
/// it sends, the empty assistant message the stream fills, the title, and
/// the running operation with the search price the run was given. An edit or
/// regenerate marks the replaced turns superseded.
fn publish_turn(
    record: &mut SessionRecord,
    control: &RunControl,
    request: &SendRequest,
    branch: &Branch,
    text: &str,
    selected: &[Attachment],
    search_price: Option<catalog::SearchPrice>,
) -> Result<(), String> {
    store::ensure_new_operation(record, &request.operation_id)?;
    if let Some(mid) = &branch.revision_of {
        if let Some(index) = record.messages.iter().position(|m| &m.id == mid) {
            for message in &mut record.messages[index..] {
                message.superseded = true;
            }
        }
    }
    let at = now();
    let user_text = branch.resent_text.as_deref().unwrap_or(text);
    record.messages.push(Message {
        id: id(),
        role: "user".into(),
        content: user_text.into(),
        created_at: at,
        reasoning: None,
        status: "completed".into(),
        attachments: selected.to_vec(),
        sources: vec![],
        superseded: false,
        revision_of: branch.revision_of.clone(),
        native_entry_id: None,
        run_id: Some(request.run_id.clone()),
    });
    record.messages.push(Message {
        id: control.message_id.clone(),
        role: "assistant".into(),
        content: String::new(),
        created_at: at,
        reasoning: None,
        status: "streaming".into(),
        attachments: vec![],
        sources: vec![],
        superseded: false,
        revision_of: request.regenerate_message_id.clone(),
        native_entry_id: None,
        run_id: Some(request.run_id.clone()),
    });
    if record.session.title.is_empty() {
        // A file sent alone names the conversation.
        let title = match user_text.trim() {
            "" => selected.first().map_or("", |a| a.name.as_str()),
            text => text,
        };
        record.session.title = title.chars().take(TITLE_CHARS).collect();
    }
    store::consume_draft(&mut record.session, request);
    record.session.updated_at = at;
    record.session.active_run = Some(RunRef {
        run_id: request.run_id.clone(),
        operation_id: request.operation_id.clone(),
    });
    record.operations.push(Operation {
        id: request.operation_id.clone(),
        run_id: request.run_id.clone(),
        status: OperationStatus::Running,
        search_price,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::assistant::host::Identity;

    fn attachment(name: &str, status: &str) -> Attachment {
        serde_json::from_value(json!({
            "id": id(), "name": name, "mimeType": "text/plain", "sizeBytes": 3,
            "status": status, "reason": "assistant_attachment_partial",
            "details": {"format": "text"},
        }))
        .unwrap()
    }

    fn message(role: &str, run: &str, attachments: &[&Attachment]) -> Message {
        serde_json::from_value(json!({
            "id": id(), "role": role, "content": format!("{role} text"), "createdAt": 1,
            "reasoning": null, "status": "completed", "attachments": attachments,
            "revisionOf": null, "nativeEntryId": id(), "runId": run,
        }))
        .unwrap()
    }

    /// One earlier turn that sent `a`; `a`, `b` and an unfinished `pending`
    /// are in the conversation.
    fn fixture() -> (SessionRecord, [Attachment; 3], Message, Message) {
        let a = attachment("a.txt", "partial");
        let b = attachment("b.md", "ready");
        let pending = attachment("c.txt", "processing");
        let run = id();
        let user = message("user", &run, &[&a]);
        let reply = message("assistant", &run, &[]);
        let record = SessionRecord {
            session: serde_json::from_value(json!({
                "id": id(), "title": "", "modelId": "m", "createdAt": 1, "updatedAt": 1,
            }))
            .unwrap(),
            messages: vec![user.clone(), reply.clone()],
            attachments: vec![a.clone(), b.clone(), pending.clone()],
            usage: json!({}),
            operations: vec![],
        };
        (record, [a, b, pending], user, reply)
    }

    fn send(record: &SessionRecord, text: &str, extra: Value) -> SendRequest {
        let mut value = json!({
            "sessionId": record.session.id, "runId": id(), "operationId": id(), "text": text,
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    fn ids(selection: &[Attachment]) -> Vec<&str> {
        selection.iter().map(|a| a.id.as_str()).collect()
    }

    fn select(record: &SessionRecord, request: &SendRequest) -> Result<Vec<Attachment>, String> {
        selection(record, request, &Branch::resolve(record, request)?)
    }

    #[test]
    fn a_new_turn_sends_each_file_it_selected_once_in_order() {
        let (record, [a, b, pending], _, _) = fixture();
        let request = send(&record, "x", json!({"attachmentIds": [b.id, a.id, b.id]}));
        let selected = select(&record, &request).unwrap();
        assert_eq!(ids(&selected), [b.id.as_str(), a.id.as_str()]);
        // The new turn's own outcome is not known yet.
        for attachment in &selected {
            assert_eq!(attachment.status, "ready");
            assert!(attachment.reason.is_none() && attachment.details.is_none());
        }
        assert_eq!(selected[1].name, "a.txt");

        for unknown in [id(), pending.id.clone()] {
            let request = send(&record, "x", json!({"attachmentIds": [a.id, unknown]}));
            assert_eq!(
                select(&record, &request).unwrap_err(),
                "assistant_attachment_not_found"
            );
        }
        assert!(select(&record, &send(&record, "x", json!({})))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_regenerate_resends_its_turns_files_and_an_edit_keeps_them_unless_it_picks_others() {
        let (record, [a, b, _], user, reply) = fixture();
        let regenerate = send(
            &record,
            "",
            json!({"regenerateMessageId": reply.id, "attachmentIds": [b.id]}),
        );
        assert_eq!(ids(&select(&record, &regenerate).unwrap()), [a.id.as_str()]);

        let keep = send(&record, "Edited", json!({"editMessageId": user.id}));
        assert_eq!(ids(&select(&record, &keep).unwrap()), [a.id.as_str()]);
        let replace = send(
            &record,
            "Edited",
            json!({"editMessageId": user.id, "attachmentIds": [b.id]}),
        );
        assert_eq!(ids(&select(&record, &replace).unwrap()), [b.id.as_str()]);
    }

    #[test]
    fn the_runtime_gets_every_file_of_the_conversation_and_the_turns_selection() {
        let (record, [a, b, _], _, reply) = fixture();
        let root = Path::new("/config/assistant/owner/sid/attachments");
        let request = send(&record, "x", json!({"attachmentIds": [b.id]}));
        let branch = Branch::resolve(&record, &request).unwrap();
        let selected = selection(&record, &request, &branch).unwrap();
        let params = attachment_params(&record, root, &selected, &branch);
        assert_eq!(params["attachmentRoot"], json!(root));
        assert_eq!(
            params["attachments"],
            json!(record
                .attachments
                .iter()
                .map(|a| json!({
                    "id": a.id, "name": a.name, "mimeType": a.mime_type,
                    "sizeBytes": a.size_bytes, "path": a.id,
                }))
                .collect::<Vec<_>>())
        );
        assert_eq!(params["selectedAttachmentIds"], json!([b.id]));
        assert!(!params.contains_key("originalPrompt"));

        // Always a selection, even an empty one: the runtime's default is none.
        let plain = send(&record, "x", json!({}));
        let branch = Branch::resolve(&record, &plain).unwrap();
        let params = attachment_params(&record, root, &[], &branch);
        assert_eq!(params["selectedAttachmentIds"], json!([]));

        // A regenerate rebuilds its turn from the user's own words.
        let regenerate = send(&record, "", json!({"regenerateMessageId": reply.id}));
        let branch = Branch::resolve(&record, &regenerate).unwrap();
        let selected = selection(&record, &regenerate, &branch).unwrap();
        let params = attachment_params(&record, root, &selected, &branch);
        assert_eq!(params["originalPrompt"], "user text");
        assert_eq!(params["selectedAttachmentIds"], json!([a.id]));
    }

    #[test]
    fn the_new_user_message_keeps_its_selection_and_a_file_alone_names_the_session() {
        let (mut record, [a, b, _], _, _) = fixture();
        let request = send(&record, "  ", json!({"attachmentIds": [b.id, a.id]}));
        let branch = Branch::resolve(&record, &request).unwrap();
        let selected = selection(&record, &request, &branch).unwrap();
        let identity = Identity {
            owner: "ab".repeat(32),
            apex: "ofox.ai".into(),
        };
        let control = RunControl::new(identity, &request, "test-key-0123".into());
        publish_turn(
            &mut record,
            &control,
            &request,
            &branch,
            "  ",
            &selected,
            None,
        )
        .unwrap();

        let user = &record.messages[2];
        assert_eq!(user.role, "user");
        assert_eq!(ids(&user.attachments), [b.id.as_str(), a.id.as_str()]);
        assert_eq!(user.attachments[1].status, "ready");
        assert_eq!(record.session.title, "b.md");
        // The conversation's own list keeps the last outcomes.
        assert_eq!(record.attachments[0].status, "partial");
    }

    fn sourced(message: &mut Message, urls: &[&str]) {
        message.sources = urls
            .iter()
            .map(|url| json!({"id": id(), "title": "t", "url": url, "excerpt": ""}))
            .collect();
    }

    fn price() -> catalog::SearchPrice {
        serde_json::from_value(json!({
            "perSearch": "0.014", "perInputToken": "0", "perOutputToken": "0",
        }))
        .unwrap()
    }

    #[test]
    fn the_allowlist_holds_the_prompts_links_earlier_questions_and_earlier_sources() {
        let (mut record, _, mut user, mut reply) = fixture();
        user.content = "Compare https://asked.example.com/".into();
        reply.content = "Answer citing https://answer-text.example.com/".into();
        sourced(
            &mut reply,
            &[
                "https://source.example.com/1",
                "http://insecure.example.com/",
            ],
        );
        let mut old = message("user", &id(), &[]);
        old.content = "https://old-branch.example.com/".into();
        old.superseded = true;
        record.messages = vec![old, user.clone(), reply.clone()];

        let request = send(&record, "x", json!({}));
        let branch = Branch::resolve(&record, &request).unwrap();
        let params = web_params(
            &record,
            &branch,
            "Now https://new.example.com/?q=1",
            Some(&price()),
        );
        assert_eq!(params["webSearch"], true);
        assert_eq!(
            params["allowedWebUrls"],
            json!([
                "https://asked.example.com/",
                "https://new.example.com/?q=1",
                "https://source.example.com/1",
            ])
        );

        // An edit keeps only the turns before the edited one.
        let edit = send(&record, "x", json!({"editMessageId": user.id}));
        let branch = Branch::resolve(&record, &edit).unwrap();
        let params = web_params(
            &record,
            &branch,
            "https://edited.example.com/",
            Some(&price()),
        );
        assert_eq!(
            params["allowedWebUrls"],
            json!(["https://edited.example.com/"])
        );

        // Without search the run gets neither the tools nor the list.
        let off = web_params(&record, &branch, "https://edited.example.com/", None);
        assert_eq!(off["webSearch"], false);
        assert!(!off.contains_key("allowedWebUrls"));
    }

    #[test]
    fn the_allowlist_keeps_the_prompt_and_the_newest_pages_when_it_is_full() {
        let (mut record, _, _, mut reply) = fixture();
        let old: Vec<String> = (0..600)
            .map(|i| format!("https://old.example.com/{i:03}"))
            .collect();
        sourced(
            &mut reply,
            &old.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let mut newer = message("assistant", &id(), &[]);
        sourced(&mut newer, &["https://zz-newest.example.com/"]);
        record.messages = vec![reply, newer];
        let request = send(&record, "x", json!({}));
        let branch = Branch::resolve(&record, &request).unwrap();
        let params = web_params(
            &record,
            &branch,
            "https://zz-prompt.example.com/",
            Some(&price()),
        );
        let urls: Vec<&str> = params["allowedWebUrls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u.as_str().unwrap())
            .collect();
        assert_eq!(urls.len(), web::MAX_ALLOWED_WEB_URLS);
        let mut sorted = urls.clone();
        sorted.sort();
        assert_eq!(urls, sorted);
        assert!(urls.contains(&"https://zz-prompt.example.com/"));
        assert!(urls.contains(&"https://zz-newest.example.com/"));
    }

    #[test]
    fn the_runs_ledger_row_keeps_the_search_price_it_was_given() {
        let (mut record, _, _, _) = fixture();
        let request = send(&record, "Search this", json!({}));
        let branch = Branch::resolve(&record, &request).unwrap();
        let identity = Identity {
            owner: "ab".repeat(32),
            apex: "ofox.ai".into(),
        };
        let control = RunControl::new(identity, &request, "test-key-0123".into());
        publish_turn(
            &mut record,
            &control,
            &request,
            &branch,
            "Search this",
            &[],
            Some(price()),
        )
        .unwrap();
        let row = record.operations.last().unwrap();
        assert_eq!(row.run_id, request.run_id);
        assert_eq!(row.search_price, Some(price()));
    }

    #[test]
    fn bootstrap_offers_search_only_with_a_priced_search_model() {
        let row = |pricing: Value| -> catalog::Model {
            serde_json::from_value(json!({
                "id": catalog::SEARCH_MODEL, "name": "GPT-5 Nano", "contextWindow": 128000,
                "maxOutputTokens": 8192, "supportsImages": false, "inputModalities": ["text"],
                "supportedEndpoints": ["/v1/responses"], "inputPricePerMillion": null,
                "outputPricePerMillion": null, "pricing": pricing, "cost": {},
                "pricingKnown": true,
            }))
            .unwrap()
        };
        let priced =
            row(json!({"prompt": "0.00000005", "completion": "0.0000004", "web_search": "0.0140"}));
        assert_eq!(
            search_status(&[priced]),
            json!({"available": true, "unitPriceUsd": "0.014", "model": "openai/gpt-5-nano"})
        );
        let unpriced = row(json!({"prompt": "0.00000005", "completion": "0.0000004"}));
        assert_eq!(
            search_status(&[unpriced]),
            json!({"available": false, "unitPriceUsd": null, "model": "openai/gpt-5-nano"})
        );
        assert_eq!(search_status(&[])["available"], false);
    }

    #[test]
    fn a_message_needs_text_or_a_file() {
        let (record, [a, _, _], _, _) = fixture();
        assert_eq!(
            check_message(&send(&record, " \n", json!({}))).unwrap_err(),
            "assistant_invalid_message"
        );
        assert!(check_message(&send(&record, "", json!({"attachmentIds": [a.id]}))).is_ok());
        let too_many = vec![a.id.clone(); attachments::MAX_PER_SESSION + 1];
        assert_eq!(
            check_message(&send(&record, "x", json!({ "attachmentIds": too_many }))).unwrap_err(),
            "assistant_attachment_not_found"
        );
    }
}
