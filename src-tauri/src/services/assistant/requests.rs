//! What each `assistant_*` command does. The commands only find the host and
//! call these, so tests drive exactly what the renderer drives.
use super::{
    attachments, catalog,
    host::{
        blocking, local_gateway_allowed, navigate_native, read_optional, restore, session_key,
        AssistantHost, Identity, RunControl,
    },
    projection::merge_native,
    store::{self, SessionList, SessionRecord},
    thinking,
    types::*,
    web,
};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_MESSAGE_BYTES: usize = 128_000;
/// The turn to show is not a user turn of the conversation's native history.
const VERSION_UNAVAILABLE: &str = "assistant_version_unavailable";

impl AssistantHost {
    /// Everything the assistant page needs to open. Never mints a key.
    pub async fn bootstrap(&self) -> Result<Value, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        // The user opened the page, retried or refreshed: a source that
        // failed moments ago is asked again (its last rows still stand in).
        let (merged, model_error) = match self.page_models(&identity.apex).await {
            Ok(merged) => (merged, None),
            Err(error) => (catalog::Catalog::default(), Some(error)),
        };
        // Conversations use chat models only, until media generation lands.
        let models = merged.conversation_models();
        let default_model_id = self.default_model(&identity.owner, &merged).await?;
        let search = search_status(&merged.models);
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
            // Catalog sources that could not be read: some models are missing.
            "partial": merged.partial,
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

    /// Every saved conversation of this account, including the trash.
    pub async fn usage_summary(&self) -> Result<Value, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let store = self.store(&identity.owner)?;
        let summary = blocking(move || store.usage_summary()).await?;
        self.ensure_current(&identity, generation).await?;
        Ok(summary)
    }

    /// The account's assistant settings; the defaults while its preferences
    /// file is damaged.
    pub async fn get_preferences(&self) -> Result<Preferences, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let store = self.store(&identity.owner)?;
        let preferences = blocking(move || Ok(store.preferences_or_default())).await?;
        self.ensure_current(&identity, generation).await?;
        Ok(preferences)
    }

    /// Save every assistant setting for the account and authority the page
    /// shows. The instructions apply from the next message on.
    pub async fn set_preferences(
        &self,
        request: SetPreferencesRequest,
    ) -> Result<Preferences, String> {
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
        let mut preferences = request.preferences;
        // The user's own words: only a key this process handed out goes.
        preferences.instructions =
            super::host::scrub_secrets(&preferences.instructions, &self.known_keys());
        let store = self.store(&identity.owner)?;
        self.ensure_current(&identity, generation).await?;
        blocking(move || store.save_preferences(&preferences)).await
    }

    /// The model a new conversation of `owner` starts with in `merged` (see
    /// [`catalog::default_model_id`]): what the page shows, and what a create
    /// without a model takes.
    async fn default_model(
        &self,
        owner: &str,
        merged: &catalog::Catalog,
    ) -> Result<Option<String>, String> {
        // A damaged preferences file only loses the remembered choice.
        let remembered = blocking({
            let store = self.store(owner)?;
            move || store.remembered_model()
        })
        .await
        .unwrap_or_default();
        Ok(catalog::default_model_id(
            &merged.models,
            remembered.as_deref(),
        ))
    }

    pub async fn create_session(&self, request: CreateRequest) -> Result<Session, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let store = self.store(&identity.owner)?;
        let merged = self.models(&identity.apex).await?;
        let default = self.default_model(&identity.owner, &merged).await?;
        let selected = match request.model_id {
            Some(model) => self.model(&identity.apex, &model).await?.id,
            None => default.clone().ok_or("assistant_choose_model")?,
        };
        // A model the user picked is remembered. One they did not pick is
        // only what the page showed: it replaces the remembered model only if
        // it is still the default, and every source was read (while one
        // cannot be, the default stands in for what is missing). A page that
        // opened during an outage may show a default that is stale by now.
        let remember = request.model_picked
            || (merged.partial.is_empty() && default.as_deref() == Some(selected.as_str()));
        self.ensure_current(&identity, generation).await?;
        let detail = blocking(move || {
            let detail = store.create(&selected, request.web_search)?;
            if remember {
                store.remember_model(&selected)?;
            }
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

    /// Title, model, thinking level, draft, web search, pin and trash state.
    /// Changing the model or trashing the session stops its run, once the
    /// change is saved; changing the thinking level does not.
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
        let new_model = match &request.model_id {
            Some(model) => Some(self.model(&identity.apex, model).await?),
            None => None,
        };
        let levels = self
            .update_thinking_levels(&identity, &request, new_model)
            .await?;
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
                let session = store.update_checked(&request, levels.as_ref(), || {
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

    /// The thinking levels an update's level is checked against, under the
    /// lock that saves it (see [`store::AssistantStore::update_checked`]):
    /// needed when the update sets a level or changes the model. The new
    /// model's come from the row just checked; the others from the catalog.
    async fn update_thinking_levels(
        &self,
        identity: &Identity,
        request: &UpdateRequest,
        new_model: Option<catalog::Model>,
    ) -> Result<Option<store::ThinkingLevels>, String> {
        let sets_level = request
            .thinking_level
            .as_deref()
            .is_some_and(|level| level != thinking::DEFAULT);
        if !sets_level && new_model.is_none() {
            return Ok(None);
        }
        let mut levels: store::ThinkingLevels = self
            .models(&identity.apex)
            .await?
            .conversation_models()
            .into_iter()
            .map(|model| (model.id, model.thinking_levels))
            .collect();
        if let Some(model) = new_model {
            levels.insert(model.id, model.thinking_levels);
        }
        Ok(Some(levels))
    }

    pub async fn delete_session(self: &Arc<Self>, request: DeleteRequest) -> Result<(), String> {
        validate_id(&request.session_id)?;
        let identity = self.identity().await?;
        let generation = self.generation();
        if request.expected_owner != identity.owner || request.expected_generation != generation {
            return Err(super::host::IDENTITY_CHANGED.into());
        }
        self.delete_trashed(&identity, generation, &request.session_id)
            .await
    }

    /// Delete every trashed conversation for good, one at a time the way
    /// [`Self::delete_session`] does. One that cannot be removed (still
    /// replying, or some files could not be deleted) is counted in `failed`
    /// and does not stop the others; one still replying stays in the trash,
    /// and one restored or removed meanwhile is not counted.
    pub async fn empty_trash(
        self: &Arc<Self>,
        request: EmptyTrashRequest,
    ) -> Result<EmptyTrashReport, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        if request.expected_owner != identity.owner || request.expected_generation != generation {
            return Err(super::host::IDENTITY_CHANGED.into());
        }
        let trashed = blocking({
            let store = self.store(&identity.owner)?;
            move || store.list(true, "")
        })
        .await?
        .sessions;
        self.ensure_current(&identity, generation).await?;
        let mut report = EmptyTrashReport::default();
        for session in trashed {
            match self
                .delete_trashed(&identity, generation, &session.id)
                .await
            {
                Ok(()) => report.deleted += 1,
                // The account changed: every other one would be refused too.
                Err(error) if error == super::host::IDENTITY_CHANGED => return Err(error),
                // Deleted meanwhile (another window, or startup cleanup), or
                // restored meanwhile (Undo): neither is a failure.
                Err(error)
                    if error == "assistant_session_not_found"
                        || error == "assistant_session_not_trashed" => {}
                Err(error) => {
                    log::warn!(
                        "[Assistant] could not delete trashed session {}: {error}",
                        session.id
                    );
                    report.failed += 1;
                }
            }
        }
        Ok(report)
    }

    /// The permanent deletion of one conversation, once the caller confirmed
    /// the authority: refused unless it is trashed and idle.
    async fn delete_trashed(
        self: &Arc<Self>,
        identity: &Identity,
        generation: u64,
        sid: &str,
    ) -> Result<(), String> {
        // A previous process may have died while stopping this trashed run.
        // A missing session can be a tombstone whose disk cleanup is retried.
        if let Err(error) = self.recover(identity, sid).await {
            if error != "assistant_session_not_found" {
                return Err(error);
            }
        }
        self.ensure_current(identity, generation).await?;
        blocking({
            let store = self.store(&identity.owner)?;
            let host = self.clone();
            let sid = sid.to_owned();
            move || {
                store.delete_trashed(&sid, || {
                    if host.generation() != generation {
                        return Err(super::host::IDENTITY_CHANGED.into());
                    }
                    Ok(())
                })
            }
        })
        .await
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
        // A retry asks again from what native history holds of its turn,
        // which a run that failed may have left unsaved.
        if request.retry_message_id.is_some() {
            self.sync_native(&identity, &request.session_id).await?;
        }
        let (record, native_file, attachment_root, preferences) = blocking({
            let store = self.store(&identity.owner)?;
            let sid = request.session_id.clone();
            move || {
                Ok((
                    store.load_record(&sid)?,
                    store.native_history_path(&sid)?,
                    store.attachment_root(&sid)?,
                    store.preferences_or_default(),
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
        // Search only at a price the catalog gives, for a model that takes
        // tools; the run keeps the price.
        let search_price = if record.session.web_search && model.tools {
            catalog::search_price(&self.models(&identity.apex).await?.models)
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
            let model_id = model.id.clone();
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
                            &model_id,
                        )
                    })
                })
            }
        })
        .await?;

        let thinking_level = run_thinking_level(&record.session, &model);
        let mut params = json!({
            "sessionId": request.session_id,
            "runId": request.run_id,
            "operationId": request.operation_id,
            "sessionFile": native_file,
            "model": model.for_runtime(thinking_level.is_some()),
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
            // At the default a run's parameters stay as they were before
            // thinking levels existed.
            if let Some(level) = thinking_level {
                params.insert("thinkingLevel".into(), json!(level));
            }
            params.extend(instruction_params(&preferences.instructions));
        }
        self.emit_start(&control, search_price.is_some(), thinking_level);
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

    /// Hold the files the user dropped on the window until the renderer asks
    /// for them with the returned token, and how many there are. The token is
    /// the account's and the authority generation's as of now; only the
    /// newest drop is kept. `None` when nothing was dropped. The paths stay
    /// in the host: they are never logged and never sent to the renderer.
    pub async fn accept_drop(
        &self,
        paths: Vec<PathBuf>,
    ) -> Result<Option<(String, usize)>, String> {
        if paths.is_empty() {
            return Ok(None);
        }
        let identity = self.identity().await?;
        let generation = self.generation();
        self.ensure_current(&identity, generation).await?;
        let held = self
            .drops
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .put(
                &identity.owner,
                generation,
                paths,
                std::time::Instant::now(),
            );
        Ok(Some(held))
    }

    /// Import the files `request.token` stands for into a conversation, with
    /// the checks of a picked file (the host reads the paths it kept; the
    /// renderer names none). The token is single use: it is spent when the
    /// import starts, but not by a request refused for its authority or its
    /// conversation, which can be repeated.
    pub async fn import_dropped(
        self: &Arc<Self>,
        request: ImportDropRequest,
    ) -> Result<Vec<Attachment>, String> {
        let authority = DeleteRequest {
            session_id: request.session_id,
            expected_owner: request.expected_owner,
            expected_generation: request.expected_generation,
        };
        let (identity, generation) = self.begin_import(&authority).await?;
        let paths = self
            .drops
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .take(
                &request.token,
                &identity.owner,
                generation,
                std::time::Instant::now(),
            )?;
        let sources = paths.into_iter().map(attachments::Source::Path).collect();
        self.import_checked(&identity, generation, &authority.session_id, sources)
            .await
    }

    /// Import a file pasted into the conversation's page. Its type comes
    /// from its bytes and its name only as a hint (see
    /// [`attachments::Source::Bytes`]); the limits are a picked file's.
    pub async fn import_pasted(
        self: &Arc<Self>,
        authority: DeleteRequest,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<Vec<Attachment>, String> {
        let (identity, generation) = self.begin_import(&authority).await?;
        let source = attachments::Source::Bytes { name, bytes };
        self.import_checked(&identity, generation, &authority.session_id, vec![source])
            .await
    }

    /// The account and generation an import runs under, once the page's
    /// authority is the current one and the conversation can take files now:
    /// it exists, is not in the trash and does not reply.
    async fn begin_import(&self, authority: &DeleteRequest) -> Result<(Identity, u64), String> {
        let sid = &authority.session_id;
        validate_id(sid)?;
        let identity = self.identity().await?;
        let generation = self.generation();
        if authority.expected_owner != identity.owner || authority.expected_generation != generation
        {
            return Err(super::host::IDENTITY_CHANGED.into());
        }
        self.ensure_idle(sid)?;
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
        Ok((identity, generation))
    }

    /// Copy `sources` into the conversation, unless the authority they were
    /// accepted under ended meanwhile (checked again before anything is saved).
    async fn import_checked(
        self: &Arc<Self>,
        identity: &Identity,
        generation: u64,
        sid: &str,
        sources: Vec<attachments::Source>,
    ) -> Result<Vec<Attachment>, String> {
        self.ensure_current(identity, generation).await?;
        let store = self.store(&identity.owner)?;
        let host = self.clone();
        let sid = sid.to_owned();
        blocking(move || {
            attachments::import_sources(&store, &sid, &sources, || {
                if host.generation() != generation {
                    return Err(super::host::IDENTITY_CHANGED.into());
                }
                Ok(())
            })
        })
        .await
    }

    /// Show another version of a turn: the user turn `message_id` and the
    /// latest of what followed it become the conversation's branch, first in
    /// its native history (backed up, and put back if the switch is not
    /// saved), then in its messages' `superseded`. Refused while the
    /// conversation replies, and no reply starts until it is done. The
    /// version shown already changes nothing; no switch names the
    /// conversation or moves it in the list.
    pub async fn switch_version(
        self: &Arc<Self>,
        request: SwitchVersionRequest,
    ) -> Result<Detail, String> {
        let SwitchVersionRequest {
            authority,
            message_id,
        } = request;
        let sid = authority.session_id;
        validate_id(&sid)?;
        let identity = self.identity().await?;
        let generation = self.generation();
        if authority.expected_owner != identity.owner || authority.expected_generation != generation
        {
            return Err(super::host::IDENTITY_CHANGED.into());
        }
        self.recover(&identity, &sid).await?;
        // Refused at once while it replies. The hold decides once the
        // history is ours: a title may be writing it meanwhile.
        self.ensure_idle(&sid)?;
        let _history = self
            .native_lock(&session_key(&identity.owner, &sid))?
            .lock_owned()
            .await;
        let _navigation = self.begin_navigation(&sid)?;
        self.ensure_current(&identity, generation).await?;
        let (record, native) = blocking({
            let store = self.store(&identity.owner)?;
            let sid = sid.clone();
            move || Ok((store.load_record(&sid)?, store.native_history_path(&sid)?))
        })
        .await?;
        if record.session.deleted_at.is_some() {
            return Err("assistant_session_deleted".into());
        }
        let (entry, shown) = record
            .messages
            .iter()
            .find(|m| m.id == message_id && m.role == "user")
            .and_then(|m| Some((m.native_entry_id.clone()?, !m.superseded)))
            .ok_or(VERSION_UNAVAILABLE)?;
        let mut detail = if shown {
            record.into_detail()
        } else {
            if !native.is_file() {
                return Err("assistant_native_history_missing".into());
            }
            let before = blocking({
                let native = native.clone();
                move || read_optional(&native)
            })
            .await?;
            let switched = self
                .navigate(&identity, generation, &sid, &native, &entry)
                .await;
            // Unless the conversation was deleted for good meanwhile: putting
            // its history back would bring its folder back.
            if switched.is_err() && native.is_file() {
                if let Err(error) = blocking(move || restore(&native, before)).await {
                    log::warn!("[Assistant] could not put back a native history: {error}");
                }
            }
            switched?
        };
        self.decorate(&identity.owner, &mut detail.session);
        Ok(detail)
    }

    /// The runtime's `navigate` to native `entry`, then its projection saved
    /// into the conversation, unless the authority changed meanwhile.
    async fn navigate(
        self: &Arc<Self>,
        identity: &Identity,
        generation: u64,
        sid: &str,
        native: &Path,
        entry: &str,
    ) -> Result<Detail, String> {
        let entries = navigate_native(&self.runtime()?, native, entry).await?;
        self.ensure_current(identity, generation).await?;
        let host = self.clone();
        let store = self.store(&identity.owner)?;
        let sid = sid.to_owned();
        let keys = self.known_keys();
        blocking(move || {
            store.mutate(&sid, |record| {
                if host.generation() != generation {
                    return Err(super::host::IDENTITY_CHANGED.into());
                }
                merge_native(record, &entries, &keys)?;
                Ok(record.clone().into_detail())
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
    let rewrites = [
        &request.edit_message_id,
        &request.regenerate_message_id,
        &request.retry_message_id,
    ]
    .into_iter()
    .filter(|mid| mid.is_some())
    .count();
    if (request.text.trim().is_empty()
        && request.attachment_ids.is_empty()
        && request.regenerate_message_id.is_none())
        || rewrites > 1
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
    /// The user message an edit, a regenerate or a retry replaces.
    revision_of: Option<String>,
    /// For a retry: the run of the turn it asks again. When native history
    /// holds its question, `edit_entry` names it and the retry is an edit;
    /// otherwise the send goes on from the end of the conversation and only
    /// that turn is superseded.
    retried_run: Option<String>,
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
        if let Some(mid) = &request.retry_message_id {
            let reply = find(mid, "assistant")
                .filter(|m| matches!(m.status.as_str(), "error" | "stopped") && !m.superseded)
                .ok_or(UNAVAILABLE)?;
            let run = reply.run_id.as_deref().ok_or(UNAVAILABLE)?;
            let of_run = |m: &&Message| m.run_id.as_deref() == Some(run);
            let user = record
                .messages
                .iter()
                .filter(of_run)
                .find(|m| m.role == "user")
                .ok_or(UNAVAILABLE)?;
            // Native history holds the question (the caller brought the
            // messages up to date with it): ask again from before it, like
            // an edit. Otherwise go on from the end, unless it holds some of
            // the turn without its question.
            if user.native_entry_id.is_none()
                && record
                    .messages
                    .iter()
                    .filter(of_run)
                    .any(|m| m.native_entry_id.is_some())
            {
                return Err(UNAVAILABLE.into());
            }
            return Ok(Self {
                revision_of: Some(user.id.clone()),
                retried_run: Some(run.into()),
                edit_entry: user.native_entry_id.clone(),
                ..Self::default()
            });
        }
        Ok(Self::default())
    }

    /// Whether the send branches native history before an earlier turn (an
    /// edit, a regenerate, or a retry of a question native history holds)
    /// rather than going on from its end.
    fn branches_native(&self) -> bool {
        self.edit_entry.is_some() || self.regenerate_entry.is_some()
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
        || (request.edit_message_id.is_some()
            && request.attachment_ids.is_empty()
            && !request.replace_attachments);
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
    // A send that branches native history starts again from the turn it
    // replaces; any other goes on from the end.
    let end = branch
        .revision_of
        .as_ref()
        .filter(|_| branch.branches_native())
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

/// The custom instructions part of a run's parameters: `instructions` only
/// when there are some, so that without them a run's parameters stay as they
/// were before the setting existed.
fn instruction_params(instructions: &str) -> Map<String, Value> {
    let mut params = Map::new();
    if !instructions.trim().is_empty() {
        params.insert("instructions".into(), json!(instructions));
    }
    params
}

/// The thinking level a run gets: the conversation's choice while its model
/// offers it (the catalog may have changed since), else none, and the
/// provider's default applies.
fn run_thinking_level<'a>(session: &'a Session, model: &catalog::Model) -> Option<&'a str> {
    session
        .thinking_level
        .as_deref()
        .filter(|level| model.offers_thinking_level(level))
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
/// it sends, the empty assistant message the stream fills with the model
/// the run asks (`model_id`), the title, and the running operation with the
/// search price the run was given. An edit or regenerate marks the replaced
/// turns superseded, a retry the turn it asks again.
#[allow(clippy::too_many_arguments)]
fn publish_turn(
    record: &mut SessionRecord,
    control: &RunControl,
    request: &SendRequest,
    branch: &Branch,
    text: &str,
    selected: &[Attachment],
    search_price: Option<catalog::SearchPrice>,
    model_id: &str,
) -> Result<(), String> {
    store::ensure_new_operation(record, &request.operation_id)?;
    // Admission may have waited on a Key while the user trashed the session
    // or removed an unsent file. Recheck under the same lock that saves it.
    if record.session.deleted_at.is_some() {
        return Err("assistant_session_deleted".into());
    }
    if selected
        .iter()
        .any(|chosen| !record.attachments.iter().any(|a| a.id == chosen.id))
    {
        return Err("assistant_attachment_not_found".into());
    }
    if let Some(run) = &branch.retried_run {
        let of_run = |m: &&Message| m.run_id.as_ref() == Some(run);
        // Asked again meanwhile (by a send that was resolved alongside).
        if !record.messages.iter().any(|m| of_run(&m))
            || record.messages.iter().filter(of_run).any(|m| m.superseded)
        {
            return Err("assistant_branch_unavailable".into());
        }
    }
    match (&branch.retried_run, &branch.revision_of) {
        (Some(run), _) if !branch.branches_native() => {
            // Hidden on every version: native history never got it.
            for message in record
                .messages
                .iter_mut()
                .filter(|m| m.run_id.as_ref() == Some(run))
            {
                message.superseded = true;
                message.anchor_entry_id = None;
            }
        }
        (_, Some(mid)) => {
            if let Some(index) = record.messages.iter().position(|m| &m.id == mid) {
                for message in &mut record.messages[index..] {
                    message.superseded = true;
                }
            }
        }
        _ => {}
    }
    // Should native history never get this turn, it shows with the version
    // of the last native message shown now.
    let anchor = record
        .messages
        .iter()
        .rev()
        .find(|m| !m.superseded && m.native_entry_id.is_some())
        .and_then(|m| m.native_entry_id.clone());
    let at = now();
    let user_text = branch.resent_text.as_deref().unwrap_or(text);
    record.messages.push(Message {
        id: id(),
        role: "user".into(),
        content: user_text.into(),
        created_at: at,
        reasoning: None,
        reasoning_ms: None,
        status: "completed".into(),
        error_code: None,
        attachments: selected.to_vec(),
        sources: vec![],
        superseded: false,
        revision_of: branch.revision_of.clone(),
        native_entry_id: None,
        run_id: Some(request.run_id.clone()),
        model_id: None,
        usage: None,
        completed_at: None,
        branch_from: None,
        anchor_entry_id: anchor.clone(),
    });
    record.messages.push(Message {
        id: control.message_id.clone(),
        role: "assistant".into(),
        content: String::new(),
        created_at: at,
        reasoning: None,
        reasoning_ms: None,
        status: "streaming".into(),
        error_code: None,
        attachments: vec![],
        sources: vec![],
        superseded: false,
        revision_of: request
            .regenerate_message_id
            .clone()
            .or_else(|| request.retry_message_id.clone()),
        native_entry_id: None,
        run_id: Some(request.run_id.clone()),
        model_id: Some(model_id.into()),
        usage: None,
        completed_at: None,
        branch_from: None,
        anchor_entry_id: anchor,
    });
    if record.session.title.is_empty() {
        // A file sent alone names the conversation.
        record.session.title = auto_title(user_text, selected);
        record.session.title_source = Some(TitleSource::Auto);
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
    fn an_edit_can_explicitly_remove_all_attachments_without_changing_the_original() {
        let (record, [a, _, _], user, reply) = fixture();
        let edit = send(
            &record,
            "Text only",
            json!({
                "editMessageId": user.id, "attachmentIds": [], "replaceAttachments": true
            }),
        );
        assert!(select(&record, &edit).unwrap().is_empty());
        assert_eq!(ids(&user.attachments), [a.id.as_str()]);
        let regenerate = send(
            &record,
            "",
            json!({
                "regenerateMessageId": reply.id, "attachmentIds": [], "replaceAttachments": true
            }),
        );
        assert_eq!(ids(&select(&record, &regenerate).unwrap()), [a.id.as_str()]);
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
            "m",
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

    #[test]
    fn the_title_a_send_fills_in_is_automatic_and_one_the_user_set_stays() {
        let (mut record, _, _, _) = fixture();
        let publish = |record: &mut SessionRecord, text: &str| {
            let request = send(record, text, json!({}));
            let branch = Branch::resolve(record, &request).unwrap();
            let control = RunControl::new(
                Identity {
                    owner: "ab".repeat(32),
                    apex: "ofox.ai".into(),
                },
                &request,
                "test-key-0123".into(),
            );
            publish_turn(record, &control, &request, &branch, text, &[], None, "m").unwrap();
            record.session.active_run = None;
        };
        publish(&mut record, "First question");
        assert_eq!(record.session.title, "First question");
        assert_eq!(record.session.title_source, Some(TitleSource::Auto));

        record.session.title = "Mine".into();
        record.session.title_source = Some(TitleSource::User);
        publish(&mut record, "Second question");
        assert_eq!(record.session.title, "Mine");
        assert_eq!(record.session.title_source, Some(TitleSource::User));

        // A title the user emptied is named by the next message again.
        record.session.title.clear();
        publish(&mut record, "Third question");
        assert_eq!(record.session.title, "Third question");
        assert_eq!(record.session.title_source, Some(TitleSource::Auto));
    }

    #[test]
    fn publishing_a_turn_rechecks_trash_and_removed_attachments_after_admission() {
        for trashed in [false, true] {
            let (mut record, [file, _, _], _, _) = fixture();
            let request = send(&record, "question", json!({"attachmentIds": [file.id]}));
            let branch = Branch::resolve(&record, &request).unwrap();
            let selected = selection(&record, &request, &branch).unwrap();
            let control = RunControl::new(
                Identity {
                    owner: "ab".repeat(32),
                    apex: "ofox.ai".into(),
                },
                &request,
                "test-key-0123".into(),
            );
            if trashed {
                record.session.deleted_at = Some(now());
            } else {
                record.attachments.retain(|item| item.id != file.id);
            }
            let before = serde_json::to_value(&record).unwrap();
            let result = publish_turn(
                &mut record,
                &control,
                &request,
                &branch,
                "question",
                &selected,
                None,
                "m",
            );
            assert_eq!(
                result.unwrap_err(),
                if trashed {
                    "assistant_session_deleted"
                } else {
                    "assistant_attachment_not_found"
                }
            );
            assert_eq!(serde_json::to_value(&record).unwrap(), before);
        }
    }

    fn sourced(message: &mut Message, urls: &[&str]) {
        message.sources = urls
            .iter()
            .map(|url| json!({"id": id(), "title": "t", "url": url, "excerpt": ""}))
            .collect();
    }

    fn price() -> catalog::SearchPrice {
        serde_json::from_value(json!({
            "model": catalog::SEARCH_MODEL, "perSearch": "0.014",
            "perInputToken": "0", "perOutputToken": "0",
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
            "m",
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
    fn a_run_gets_the_chosen_level_only_while_its_model_offers_it() {
        let (mut record, _, _, _) = fixture();
        let mut model: catalog::Model = serde_json::from_value(json!({
            "id": "m", "name": "M", "contextWindow": 128000, "maxOutputTokens": 8192,
            "supportsImages": false, "inputModalities": ["text"],
            "supportedEndpoints": ["/v1/chat/completions"], "inputPricePerMillion": null,
            "outputPricePerMillion": null, "pricing": {}, "cost": {}, "pricingKnown": false,
            "reasoning": true, "thinkingLevels": ["low", "high"],
        }))
        .unwrap();
        assert_eq!(run_thinking_level(&record.session, &model), None);
        record.session.thinking_level = Some("high".into());
        assert_eq!(run_thinking_level(&record.session, &model), Some("high"));
        // The catalog changed since: the run goes at the default.
        model.thinking_levels = vec!["low".into()];
        assert_eq!(run_thinking_level(&record.session, &model), None);
    }

    #[test]
    fn a_run_gets_the_custom_instructions_only_when_there_are_some() {
        // None: the run's parameters stay as they were before the setting.
        assert!(instruction_params("").is_empty());
        assert!(instruction_params(" \n\t").is_empty());
        assert_eq!(
            Value::Object(instruction_params("Be brief.\n")),
            json!({"instructions": "Be brief.\n"})
        );
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

    /// Publish `request`'s turn as the run `model` registers.
    fn publish(
        record: &mut SessionRecord,
        request: &SendRequest,
        model: &str,
    ) -> Result<RunControl, String> {
        let branch = Branch::resolve(record, request)?;
        let identity = Identity {
            owner: "ab".repeat(32),
            apex: "ofox.ai".into(),
        };
        let control = RunControl::new(identity, request, "test-key-0123".into());
        publish_turn(
            record,
            &control,
            request,
            &branch,
            &request.text,
            &[],
            None,
            model,
        )?;
        Ok(control)
    }

    fn turn_of<'a>(record: &'a SessionRecord, request: &SendRequest) -> &'a Message {
        record
            .messages
            .iter()
            .find(|m| m.role == "user" && m.run_id.as_deref() == Some(request.run_id.as_str()))
            .unwrap()
    }

    #[test]
    fn a_reply_names_the_model_its_run_asks() {
        let (mut record, _, _, _) = fixture();
        let request = send(&record, "Which model?", json!({}));
        let control = publish(&mut record, &request, "test/run-model").unwrap();
        let reply = record
            .messages
            .iter()
            .find(|m| m.id == control.message_id)
            .unwrap();
        assert_eq!(reply.model_id.as_deref(), Some("test/run-model"));
        assert!(reply.usage.is_none() && reply.completed_at.is_none());
        assert!(turn_of(&record, &request).model_id.is_none());
    }

    /// The fixture's turn, a turn native history never got (its reply
    /// `status`), then a later native turn that asked for a page.
    fn with_unkept_turn(status: &str) -> (SessionRecord, Message, Message) {
        let (mut record, _, _, _) = fixture();
        let run = id();
        let mut question = message("user", &run, &[]);
        let mut reply = message("assistant", &run, &[]);
        question.native_entry_id = None;
        reply.native_entry_id = None;
        reply.status = status.into();
        let later = id();
        let mut later_question = message("user", &later, &[]);
        later_question.content = "Read https://later.example.com/".into();
        record.messages.extend([
            question.clone(),
            reply.clone(),
            later_question,
            message("assistant", &later, &[]),
        ]);
        (record, question, reply)
    }

    #[test]
    fn a_retry_asks_again_and_supersedes_only_the_turn_it_retries() {
        let (mut record, question, failed) = with_unkept_turn("error");
        let request = send(&record, "Asked again", json!({"retryMessageId": failed.id}));
        let branch = Branch::resolve(&record, &request).unwrap();
        // It continues native history: nothing to branch from or resend.
        assert!(branch.edit_entry.is_none() && branch.regenerate_entry.is_none());
        assert!(branch.resent_text.is_none());
        // The turns after the retried one stay in its context, pages too.
        let params = web_params(&record, &branch, "Again", Some(&price()));
        assert_eq!(
            params["allowedWebUrls"],
            json!(["https://later.example.com/"])
        );

        let pristine = record.clone();
        let control = publish(&mut record, &request, "m").unwrap();
        let hidden: Vec<&str> = record
            .messages
            .iter()
            .filter(|m| m.superseded)
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(hidden, [question.id.as_str(), failed.id.as_str()]);
        let asked = turn_of(&record, &request);
        assert_eq!(asked.content, "Asked again");
        assert_eq!(asked.revision_of.as_deref(), Some(question.id.as_str()));
        let reply = record
            .messages
            .iter()
            .find(|m| m.id == control.message_id)
            .unwrap();
        assert_eq!(reply.revision_of.as_deref(), Some(failed.id.as_str()));
        // A second retry resolved alongside the first is refused as it saves.
        let mut twice = request.clone();
        twice.run_id = id();
        twice.operation_id = id();
        let branch = Branch::resolve(&pristine, &twice).unwrap();
        let identity = Identity {
            owner: "ab".repeat(32),
            apex: "ofox.ai".into(),
        };
        let control = RunControl::new(identity, &twice, "test-key-0123".into());
        let before = serde_json::to_value(&record).unwrap();
        assert_eq!(
            publish_turn(
                &mut record,
                &control,
                &twice,
                &branch,
                "Again",
                &[],
                None,
                "m"
            )
            .unwrap_err(),
            "assistant_branch_unavailable"
        );
        assert_eq!(serde_json::to_value(&record).unwrap(), before);
    }

    #[test]
    fn only_a_shown_failed_or_stopped_reply_can_be_retried() {
        let (record, question, stopped) = with_unkept_turn("stopped");
        let retry = |record: &SessionRecord, mid: &str| {
            let request = send(record, "x", json!({ "retryMessageId": mid }));
            Branch::resolve(record, &request).map(|_| ())
        };
        assert!(retry(&record, &stopped.id).is_ok());
        let unavailable = Err("assistant_branch_unavailable".to_string());
        // Not a reply, or no reply of this conversation.
        assert_eq!(retry(&record, &question.id), unavailable);
        assert_eq!(retry(&record, &id()), unavailable);
        // A completed reply has nothing to retry.
        let mut answered = record.clone();
        answered.messages[3].status = "completed".into();
        assert_eq!(retry(&answered, &stopped.id), unavailable);
        // A reply native history holds while its question it does not: the
        // history has nothing to ask again from.
        let mut odd = record.clone();
        odd.messages[3].native_entry_id = Some(id());
        assert_eq!(retry(&odd, &stopped.id), unavailable);
        // A hidden turn was already asked again.
        let mut hidden = record.clone();
        hidden.messages[3].superseded = true;
        assert_eq!(retry(&hidden, &stopped.id), unavailable);
        // One rewrite at a time.
        for other in [
            json!({"retryMessageId": stopped.id, "editMessageId": question.id}),
            json!({"retryMessageId": stopped.id, "regenerateMessageId": stopped.id}),
        ] {
            assert_eq!(
                check_message(&send(&record, "x", other)).unwrap_err(),
                "assistant_invalid_message"
            );
        }
    }

    #[test]
    fn a_retry_of_a_question_native_history_holds_asks_again_from_before_it() {
        // The runtime kept the question (and the failed reply) although the
        // host could not save that: the retry is an edit of that question.
        let (mut record, question, failed) = with_unkept_turn("error");
        let native = id();
        record.messages[2].native_entry_id = Some(native.clone());
        record.messages[3].native_entry_id = Some(id());
        let request = send(&record, "Asked again", json!({"retryMessageId": failed.id}));
        let branch = Branch::resolve(&record, &request).unwrap();
        assert_eq!(branch.edit_entry.as_deref(), Some(native.as_str()));
        assert!(branch.regenerate_entry.is_none() && branch.resent_text.is_none());
        // Its context ends before the question: the later turn's page too.
        let params = web_params(&record, &branch, "Again", Some(&price()));
        assert_eq!(params["allowedWebUrls"], json!([]));

        let control = publish(&mut record, &request, "m").unwrap();
        // From the question on, as an edit leaves them.
        let hidden: Vec<&str> = record
            .messages
            .iter()
            .filter(|m| m.superseded)
            .map(|m| m.role.as_str())
            .collect();
        assert_eq!(hidden, ["user", "assistant", "user", "assistant"]);
        let asked = turn_of(&record, &request);
        assert_eq!(asked.content, "Asked again");
        assert_eq!(asked.revision_of.as_deref(), Some(question.id.as_str()));
        let reply = record
            .messages
            .iter()
            .find(|m| m.id == control.message_id)
            .unwrap();
        assert_eq!(reply.revision_of.as_deref(), Some(failed.id.as_str()));
        // A turn already answered by native history's own copy, likewise.
        let (mut kept, _, _) = with_unkept_turn("stopped");
        kept.messages[1].status = "error".into();
        let kept_reply = kept.messages[1].id.clone();
        let request = send(&kept, "x", json!({ "retryMessageId": kept_reply }));
        let branch = Branch::resolve(&kept, &request).unwrap();
        assert_eq!(branch.edit_entry, kept.messages[0].native_entry_id);
    }

    #[test]
    fn a_new_turn_is_anchored_to_the_last_native_message_shown_and_a_retried_one_to_none() {
        let (mut record, _, failed) = with_unkept_turn("error");
        // The later native turn is another version: not shown.
        record.messages[4].superseded = true;
        record.messages[5].superseded = true;
        let shown = record.messages[1].native_entry_id.clone();
        let request = send(&record, "Asked again", json!({"retryMessageId": failed.id}));
        let control = publish(&mut record, &request, "m").unwrap();
        for message in [
            turn_of(&record, &request),
            record
                .messages
                .iter()
                .find(|m| m.id == control.message_id)
                .unwrap(),
        ] {
            assert_eq!(message.anchor_entry_id, shown, "{}", message.role);
        }
        // The retried turn stays hidden whatever version shows.
        let retried: Vec<&Message> = record
            .messages
            .iter()
            .filter(|m| m.run_id == failed.run_id)
            .collect();
        assert_eq!(retried.len(), 2);
        for message in retried {
            assert!(message.superseded && message.anchor_entry_id.is_none());
        }
        // An edit is anchored before the turn it edits; the first turn to none.
        let (mut edited, _, _) = with_unkept_turn("error");
        let first = edited.messages[0].id.clone();
        let request = send(&edited, "Edited", json!({ "editMessageId": first }));
        publish(&mut edited, &request, "m").unwrap();
        assert!(turn_of(&edited, &request).anchor_entry_id.is_none());
    }
}
