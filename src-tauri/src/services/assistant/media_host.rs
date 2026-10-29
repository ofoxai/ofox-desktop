//! Image runs use the same admission, authority, cancellation and operation
//! ledger as chat. Only the host ever owns their gateway credentials.
use super::{
    attachments::{self, Source},
    catalog::Model,
    host::{
        blocking, load_native, native_entries, session_key, AssistantHost, Failure, Finish,
        Identity, RunControl,
    },
    media::{self, ImageRequest},
    media_http::{self, InputImage},
    requests::{publish_turn, Branch},
    store::SessionRecord,
    types::*,
};
use serde_json::json;
use std::{io::Read, path::PathBuf, sync::Arc};

impl AssistantHost {
    /// Finish only the local history write after a crash; never re-submit.
    pub(super) async fn repair_media_history(
        &self,
        identity: &Identity,
        sid: &str,
    ) -> Result<(), String> {
        let pending = |record: &SessionRecord| {
            record.messages.iter().any(|m| {
                m.native_entry_id.is_none()
                    && m.media
                        .as_ref()
                        .is_some_and(|media| !media.outputs.is_empty())
            })
        };
        let record = blocking({
            let store = self.store(&identity.owner)?;
            let sid = sid.to_owned();
            move || store.load_record(&sid)
        })
        .await?;
        if !pending(&record) {
            return Ok(());
        }
        self.ensure_idle(sid)?;
        let generation = self.generation();
        let _history = self
            .native_lock(&session_key(&identity.owner, sid))?
            .lock_owned()
            .await;
        let _navigation = self.begin_navigation(sid)?;
        self.ensure_current(identity, generation).await?;
        let (record, file) = blocking({
            let store = self.store(&identity.owner)?;
            let sid = sid.to_owned();
            move || Ok((store.load_record(&sid)?, store.native_history_path(&sid)?))
        })
        .await?;
        if !file.is_file() && record.messages.iter().any(|m| m.native_entry_id.is_some()) {
            return Err("assistant_native_history_missing".into());
        }
        for message in record.messages.iter().filter(|m| {
            m.native_entry_id.is_none()
                && m.media
                    .as_ref()
                    .is_some_and(|media| !media.outputs.is_empty())
        }) {
            let media = message.media.as_ref().expect("media");
            let run = message
                .run_id
                .as_deref()
                .ok_or("assistant_native_run_missing")?;
            let operation = record
                .operations
                .iter()
                .find(|op| op.run_id == run)
                .ok_or("assistant_native_unknown_run")?;
            let prompt = record
                .messages
                .iter()
                .find(|m| m.role == "user" && m.run_id.as_deref() == Some(run))
                .map(|m| m.content.as_str())
                .ok_or("assistant_message_not_found")?;
            let entries=native_entries(&self.runtime()?,"append_media",json!({"sessionFile":file,"runId":run,"operationId":operation.id,"prompt":prompt,"text":history_text(media,prompt,&message.attachments),"details":media}), ("assistant_history_recovery_failed","assistant_history_recovery_timeout")).await?;
            self.ensure_current(identity, generation).await?;
            blocking({
                let store = self.store(&identity.owner)?;
                let sid = sid.to_owned();
                let keys = self.known_keys();
                let mid = message.id.clone();
                move || {
                    store.mutate(&sid, |record| {
                        super::projection::merge_native(record, &entries, &keys)?;
                        if let Some(message) = record.messages.iter_mut().find(|m| m.id == mid) {
                            if matches!(
                                message.error_code.as_deref(),
                                Some(
                                    "assistant_history_recovery_failed"
                                        | "assistant_history_recovery_timeout"
                                )
                            ) {
                                message.error_code = None;
                                message.status = "completed".into();
                            }
                        }
                        Ok(())
                    })
                }
            })
            .await?;
        }
        Ok(())
    }

    pub(super) async fn send_image(
        self: &Arc<Self>,
        mut request: SendRequest,
        identity: Identity,
        generation: u64,
        record: SessionRecord,
        native_file: PathBuf,
        model: Model,
    ) -> Result<RunRef, String> {
        if request.edit_message_id.is_some()
            || request.regenerate_message_id.is_some()
            || request.retry_message_id.is_some()
        {
            return Err(media::INVALID.into());
        }
        let params = request.media.clone().unwrap_or_default();
        params.validate(model.image.as_ref().ok_or(media::INVALID)?)?;
        let selected = match &params.input_attachment_id {
            None if request.attachment_ids.is_empty() => vec![],
            Some(id) => {
                validate_id(id)?;
                if request.attachment_ids.iter().any(|selected| selected != id) {
                    return Err(media::INVALID.into());
                }
                vec![record
                    .attachments
                    .iter()
                    .find(|a| &a.id == id && a.selectable())
                    .ok_or("assistant_attachment_not_found")?
                    .clone()]
            }
            _ => return Err(media::INVALID.into()),
        };
        attachments::image_room(&record, params.count())?;
        let input = if let Some(file) = selected.first() {
            let root = self
                .store(&identity.owner)?
                .attachment_root(&request.session_id)?;
            let file = file.clone();
            Some(blocking(move || read_input(root, &file)).await?)
        } else {
            None
        };
        self.ensure_current(&identity, generation).await?;
        let admit = || self.admit(generation);
        let key = self.env.api_key(&identity, None, &admit).await?;
        self.ensure_current(&identity, generation).await?;
        self.remember_key(&key);
        let control = Arc::new(RunControl::new(identity.clone(), &request, key));
        let prompt = super::host::scrub_secrets(&request.text, &control.secrets());
        request.attachment_ids = selected.iter().map(|a| a.id.clone()).collect();
        let media = MediaRecord {
            kind: "image".into(),
            model: model.id.clone(),
            params: serde_json::to_value(&params).map_err(|_| media::INVALID)?,
            input_ids: request.attachment_ids.clone(),
            status: "submitting".into(),
            outputs: vec![],
            cost: None,
        };
        let branch = Branch::resolve(&record, &request)?;
        blocking({
            let store = self.store(&identity.owner)?;
            let host = self.clone();
            let control = control.clone();
            let request = request.clone();
            let prompt = prompt.clone();
            let media = media.clone();
            move || {
                host.register_run(control.clone(), generation, || {
                    store.mutate(&request.session_id, |record| {
                        if record.session.model_id != media.model {
                            return Err(media::INVALID.into());
                        }
                        attachments::image_room(record, params.count())?;
                        publish_turn(
                            record,
                            &control,
                            &request,
                            &branch,
                            &prompt,
                            &selected,
                            None,
                            &media.model,
                        )?;
                        let message = record.messages.last_mut().expect("published reply");
                        message.role = "media".into();
                        message.media = Some(media.clone());
                        record
                            .operations
                            .last_mut()
                            .expect("published operation")
                            .media = Some(media);
                        Ok(())
                    })
                })
            }
        })
        .await?;
        self.emit_start(&control, false, None);
        let host = self.clone();
        let run = control.clone();
        self.track(control.run_id.clone(),async move {
            let mut cancel = run.cancel.subscribe();
            let outcome = tokio::select! {
                biased;
                _ = cancel.wait_for(|cancelled|*cancelled) => Err(Failure::from(media::UNKNOWN)),
                result = host.image_run(&run,&native_file,&model,&prompt,&media,input.as_ref()) => result,
            };
            if let Err(error) = &outcome {
                let _ = blocking({
                    let store = host.store(&run.identity.owner);
                    let sid = run.session_id.clone(); let mid = run.message_id.clone();
                    let unbilled = error.unbilled;
                    move || store?.mutate(&sid,|record| {
                        let mut saved = None;
                        if let Some(message) = record.messages.iter_mut().find(|m|m.id==mid) {
                            if let Some(media) = &mut message.media {
                                if media.outputs.is_empty() {
                                    media.status = if unbilled {"error"} else {"unknown"}.into();
                                    media.cost = unbilled.then(||"0".into());
                                }
                                saved = Some((message.run_id.clone(),media.clone()));
                            }
                        }
                        if let Some((rid,media)) = saved {
                            for op in record.operations.iter_mut().filter(|op|Some(&op.run_id)==rid.as_ref()) { op.media = Some(media.clone()); }
                        }
                        Ok(())
                    })
                }).await;
            }
            host.finish(&run,outcome).await;
        });
        Ok(RunRef {
            run_id: request.run_id,
            operation_id: request.operation_id,
        })
    }

    async fn image_run(
        &self,
        control: &Arc<RunControl>,
        native_file: &std::path::Path,
        model: &Model,
        prompt: &str,
        media: &MediaRecord,
        input: Option<&InputImage>,
    ) -> Result<Finish, Failure> {
        let lock = self.native_lock(&session_key(&control.identity.owner, &control.session_id))?;
        let _guard = lock.lock().await;
        self.validate_run(control).map_err(Failure::unsent)?;
        let before_entries = if native_file.is_file() {
            load_native(&self.runtime()?, native_file)
                .await
                .map_err(Failure::unsent)?
        } else {
            json!([])
        };
        let params: ImageRequest =
            serde_json::from_value(media.params.clone()).map_err(|_| media::INVALID)?;
        let gateway = self.env.gateway_base(&control.identity.apex)?;
        let client = crate::proxy::http_client::assistant_media_client()?;
        let key = control
            .secrets()
            .last()
            .cloned()
            .ok_or("assistant_key_rejected")?;
        let mut result =
            media_http::request(&client, &gateway, &key, &model.id, prompt, &params, input).await;
        if result
            .as_ref()
            .is_err_and(|e| e.code == "assistant_key_rejected")
        {
            let key = self
                .key_for_retry(control, &json!({"apiKey":key}))
                .await
                .map_err(Failure::unsent)?;
            control.add_secret(&key);
            self.remember_key(&key);
            self.validate_run(control).map_err(Failure::unsent)?;
            result =
                media_http::request(&client, &gateway, &key, &model.id, prompt, &params, input)
                    .await;
        }
        let response = result.map_err(|e| Failure {
            code: e.code,
            unbilled: e.unbilled,
            detail: e.detail,
            native: None,
        })?;
        self.validate_run(control)?;
        let images = media::decode(&response, params.count())?;
        let mut media = media.clone();
        media.cost = media::cost(
            model.image.as_ref().ok_or(media::INVALID)?,
            &response["usage"],
            params.count(),
            input.is_some(),
        )
        .map(|n| n.normalize().to_string());
        media.status = "completed".into();
        let files = blocking({
            let store = self.store(&control.identity.owner)?;
            let sid = control.session_id.clone();
            let mid = control.message_id.clone();
            let control = control.clone();
            let media = media.clone();
            move || {
                let sources: Vec<_> = images
                    .into_iter()
                    .enumerate()
                    .map(|(i, (ext, bytes))| Source::Bytes {
                        name: format!("generated-{}.{ext}", i + 1),
                        bytes,
                    })
                    .collect();
                attachments::generated(&store, &sid, &mid, &sources, media, |record| {
                    if control.cancelled()
                        || record.session.deleted_at.is_some()
                        || record
                            .session
                            .active_run
                            .as_ref()
                            .is_none_or(|r| r.run_id != control.run_id)
                    {
                        return Err("assistant_identity_changed".into());
                    }
                    Ok(())
                })
            }
        })
        .await?;
        media.outputs = files.iter().map(|a| a.id.clone()).collect();
        blocking({
            let store = self.store(&control.identity.owner)?;
            let sid = control.session_id.clone();
            move || {
                store.mutate(&sid, |record| {
                    super::projection::recount_usage(
                        record,
                        before_entries.as_array().ok_or(super::host::PROTOCOL)?,
                    );
                    Ok(())
                })
            }
        })
        .await?;
        let text = history_text(&media, prompt, &files);
        let entries = native_entries(&self.runtime()?,"append_media",json!({"sessionFile":native_file,"runId":control.run_id,"operationId":control.operation_id,"prompt":prompt,"text":text,"details":media}), ("assistant_history_recovery_failed","assistant_history_recovery_timeout")).await.map_err(Failure::unsent)?;
        self.apply_native(control, &entries).await?;
        let usage = blocking({
            let store = self.store(&control.identity.owner)?;
            let sid = control.session_id.clone();
            move || Ok(store.load_record(&sid)?.usage)
        })
        .await?;
        self.emit(control, "usage", json!({"usage":usage}));
        Ok(Finish::Completed)
    }
}

fn read_input(root: PathBuf, file: &Attachment) -> Result<InputImage, String> {
    const LIMIT: u64 = 15 * 1024 * 1024;
    if !std::fs::symlink_metadata(&root)
        .map_err(|_| attachments::UNREADABLE)?
        .file_type()
        .is_dir()
    {
        return Err(attachments::UNREADABLE.into());
    }
    let path = root.join(&file.id);
    let meta = std::fs::symlink_metadata(&path).map_err(|_| attachments::UNREADABLE)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(attachments::UNREADABLE.into());
    }
    if meta.len() > LIMIT {
        return Err(attachments::TOO_LARGE.into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut bytes = vec![];
    options
        .open(path)
        .map_err(|_| attachments::UNREADABLE)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| attachments::UNREADABLE)?;
    if bytes.len() as u64 > LIMIT {
        return Err(attachments::TOO_LARGE.into());
    }
    let ext = media::image_type(&bytes)
        .filter(|ext| matches!(*ext, "png" | "jpg"))
        .ok_or(media::INVALID)?;
    Ok(InputImage {
        name: format!("input.{ext}"),
        mime: attachments::mime_type(ext).ok_or(media::INVALID)?.into(),
        bytes,
    })
}

fn history_text(media: &MediaRecord, prompt: &str, files: &[Attachment]) -> String {
    format!(
        "User generated {} image(s) with {}. Parameters: {}. Prompt: {}. Files: {}",
        files.len(),
        media.model,
        media.params,
        prompt,
        files
            .iter()
            .map(|a| format!("{} (attachment id {})", a.name, a.id))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn image_input_refuses_a_linked_attachment_directory() {
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let aid = id();
        let bytes = b"\x89PNG\r\n\x1a\nfixture";
        std::fs::write(outside.join(&aid), bytes).unwrap();
        let file: Attachment = serde_json::from_value(json!({
            "id":aid,"name":"input.png","mimeType":"image/png",
            "sizeBytes":bytes.len(),"status":"ready"
        }))
        .unwrap();
        assert!(read_input(outside.clone(), &file).is_ok());
        let linked = root.path().join("attachments");
        std::os::unix::fs::symlink(outside, &linked).unwrap();
        assert!(read_input(linked, &file).is_err());
    }
}
