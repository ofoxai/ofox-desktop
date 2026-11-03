//! Quoted submission and recoverable background video jobs.
use super::{
    host::{blocking, AssistantHost, Identity, IDENTITY_CHANGED, STATE},
    media,
    types::{id, now, validate_id},
    video::{self, MediaQuote, QuoteMediaRequest, VideoRequest},
};
use rust_decimal::Decimal;

#[derive(Clone)]
pub(super) struct StoredQuote {
    pub identity: Identity,
    pub generation: u64,
    pub session_id: String,
    pub model: String,
    pub params: VideoRequest,
    pub price: Option<Decimal>,
    pub issued_at: i64,
}
impl AssistantHost {
    pub async fn quote_media(&self, request: QuoteMediaRequest) -> Result<MediaQuote, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let authority = request.authority;
        if identity.owner != authority.expected_owner || generation != authority.expected_generation
        {
            return Err(IDENTITY_CHANGED.into());
        }
        validate_id(&authority.session_id)?;
        self.ensure_accepting_runs()?;
        let record = blocking({
            let store = self.store(&identity.owner)?;
            let sid = authority.session_id.clone();
            move || store.load_record(&sid)
        })
        .await?;
        if record.session.deleted_at.is_some() {
            return Err("assistant_session_deleted".into());
        }
        let model = self.model(&identity.apex, &record.session.model_id).await?;
        request
            .params
            .validate(model.video.as_ref().ok_or(media::INVALID)?)?;
        for aid in request
            .params
            .first_frame_id
            .iter()
            .chain(request.params.last_frame_id.iter())
        {
            if !record.attachments.iter().any(|file| {
                &file.id == aid
                    && file.selectable()
                    && matches!(
                        file.mime_type.as_str(),
                        "image/png" | "image/jpeg" | "image/webp"
                    )
            }) {
                return Err(media::INVALID.into());
            }
        }
        let price = self
            .env
            .provider_prices(&identity.apex, &model.id)
            .await
            .ok()
            .and_then(|data| video::estimate(&data, &request.params));
        self.ensure_current(&identity, generation).await?;
        let issued_at = now();
        let quote_id = id();
        let quote = MediaQuote {
            id: quote_id.clone(),
            estimated_cost_usd: price.map(|n| n.normalize().to_string()),
            expires_at: issued_at + video::QUOTE_TTL_MS,
        };
        let mut quotes = self.video_quotes.lock().map_err(|_| STATE)?;
        quotes.retain(|_, quote| {
            quote.generation == generation
                && issued_at.saturating_sub(quote.issued_at) < video::QUOTE_TTL_MS
        });
        if quotes.len() >= 1024 {
            return Err(STATE.into());
        }
        quotes.insert(
            quote_id,
            StoredQuote {
                identity,
                generation,
                session_id: authority.session_id,
                model: model.id,
                params: request.params,
                price,
                issued_at,
            },
        );
        Ok(quote)
    }
}

use super::{
    catalog::Model,
    host::{native_entries, session_key, Failure, RunControl},
    requests::{publish_turn, Branch},
    store::SessionRecord,
    types::{MediaRecord, OperationStatus, RunRef, SendRequest},
    video_http,
};
use base64::Engine;
use serde_json::json;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

struct PollRegistration {
    set: Arc<Mutex<HashSet<String>>>,
    key: String,
}
impl Drop for PollRegistration {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

impl AssistantHost {
    pub(super) async fn send_video(
        self: &Arc<Self>,
        mut request: SendRequest,
        identity: Identity,
        generation: u64,
        record: SessionRecord,
        native_file: PathBuf,
        model: Model,
    ) -> Result<RunRef, String> {
        if request.media.is_some()
            || request.edit_message_id.is_some()
            || request.regenerate_message_id.is_some()
            || request.retry_message_id.is_some()
        {
            return Err(media::INVALID.into());
        }
        let params = request.video.as_ref().ok_or(media::INVALID)?.clone();
        params.validate(model.video.as_ref().ok_or(media::INVALID)?)?;
        let quote_id = request
            .quote_id
            .as_deref()
            .ok_or(video::QUOTE_EXPIRED)?
            .to_owned();
        let quote = self
            .video_quotes
            .lock()
            .map_err(|_| STATE)?
            .get(&quote_id)
            .cloned()
            .ok_or(video::QUOTE_EXPIRED)?;
        if quote.identity != identity
            || quote.generation != generation
            || quote.session_id != request.session_id
            || quote.model != model.id
        {
            return Err(video::QUOTE_EXPIRED.into());
        }
        let price = self
            .env
            .provider_prices(&identity.apex, &model.id)
            .await
            .ok()
            .and_then(|data| video::estimate(&data, &params));
        video::validate_quote(
            &quote.params,
            &params,
            quote.price,
            price,
            quote.issued_at,
            now(),
        )?;
        let mut selected = vec![];
        let mut frames = vec![];
        for (kind, aid) in [
            ("first_frame", params.first_frame_id.as_ref()),
            ("last_frame", params.last_frame_id.as_ref()),
        ] {
            if let Some(aid) = aid {
                let file = record
                    .attachments
                    .iter()
                    .find(|file| &file.id == aid && file.selectable())
                    .ok_or("assistant_attachment_not_found")?
                    .clone();
                let input = blocking({
                    let root = self
                        .store(&identity.owner)?
                        .attachment_root(&request.session_id)?;
                    let file = file.clone();
                    move || super::media_host::read_frame(root, &file)
                })
                .await?;
                frames.push((
                    kind,
                    format!(
                        "data:{};base64,{}",
                        input.mime,
                        base64::engine::general_purpose::STANDARD.encode(input.bytes)
                    ),
                ));
                selected.push(file);
            }
        }
        request.attachment_ids = selected.iter().map(|file| file.id.clone()).collect();
        self.ensure_current(&identity, generation).await?;
        // Serialize the account-wide scan and publication across conversations.
        let admission = self.video_admission.lock().await;
        blocking({
            let store = self.store(&identity.owner)?;
            let sid = request.session_id.clone();
            move || {
                let mut sessions = store.list(false, "")?.sessions;
                sessions.extend(store.list(true, "")?.sessions);
                let mut count = 0;
                for session in sessions {
                    let record = store.load_record(&session.id)?;
                    let jobs = video::occupied_slots(&record);
                    if session.id == sid && jobs > 0 {
                        return Err("assistant_media_video_limit".into());
                    }
                    count += jobs;
                }
                if count >= 3 {
                    Err("assistant_media_video_limit".into())
                } else {
                    Ok(())
                }
            }
        })
        .await?;
        let key = self
            .env
            .api_key(&identity, None, &|| self.admit(generation))
            .await?;
        self.ensure_current(&identity, generation).await?;
        self.remember_key(&key);
        let run = Arc::new(RunControl::new(identity.clone(), &request, key));
        let prompt = super::host::scrub_secrets(&request.text, &self.known_keys());
        let media = MediaRecord {
            kind: "video".into(),
            price_snapshot: Some(
                json!({"quotedTotalUsd":quote.price.map(|v|v.normalize().to_string()),"params":params}),
            ),
            model: model.id.clone(),
            params: serde_json::to_value(&params).map_err(|_| media::INVALID)?,
            input_ids: request.attachment_ids.clone(),
            status: "submitting".into(),
            apex: Some(identity.apex.clone()),
            submitted_at: Some(now()),
            poll_started_at: Some(now()),
            ..Default::default()
        };
        let branch = Branch::resolve(&record, &request)?;
        blocking({
            let host = self.clone();
            let store = self.store(&identity.owner)?;
            let request = request.clone();
            let run = run.clone();
            let media = media.clone();
            let prompt = prompt.clone();
            move || {
                host.register_run(run.clone(), generation, || {
                    let mut quotes = host.video_quotes.lock().map_err(|_| STATE)?;
                    let quote = quotes.remove(&quote_id).ok_or(video::QUOTE_EXPIRED)?;
                    video::validate_quote(
                        &quote.params,
                        &params,
                        quote.price,
                        price,
                        quote.issued_at,
                        now(),
                    )?;
                    store.mutate(&request.session_id, |record| {
                        if record.session.model_id != media.model {
                            return Err(video::QUOTE_EXPIRED.into());
                        }
                        publish_turn(
                            record,
                            &run,
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
                        record.operations.last_mut().expect("operation").media = Some(media);
                        Ok(())
                    })
                })
            }
        })
        .await?;
        drop(admission);
        self.emit_start(&run, false, None);
        let host = self.clone();
        let control = run.clone();
        self.track(run.run_id.clone(),async move {
            let mut cancel=control.cancel.subscribe();
            let result=tokio::select! {biased;
                _=cancel.wait_for(|cancelled|*cancelled)=>Err(Failure::from(media::UNKNOWN)),
                result=host.submit_video(&control,generation,native_file,&prompt,media,frames)=>result,
            };
            if let Err(error)=result {
                let status=if error.unbilled {"failed"} else {"unknown"};
                let _=host.write_video_state(&control,None,status,error.unbilled.then(||"0".into()),Some(&error.code)).await;
                host.finish(&control,Err(error)).await;
            }
        });
        Ok(RunRef {
            run_id: request.run_id,
            operation_id: request.operation_id,
        })
    }

    async fn submit_video(
        self: &Arc<Self>,
        run: &Arc<RunControl>,
        generation: u64,
        native_file: PathBuf,
        prompt: &str,
        media: MediaRecord,
        frames: Vec<(&str, String)>,
    ) -> Result<(), Failure> {
        let _history = self
            .native_lock(&session_key(&run.identity.owner, &run.session_id))?
            .lock_owned()
            .await;
        self.validate_run(run).map_err(Failure::unsent)?;
        let entries=native_entries(&self.runtime()?,"append_media",json!({"sessionFile":native_file,"runId":run.run_id,"operationId":run.operation_id,"prompt":prompt,"text":format!("User requested a video with {}. Parameters: {}. Prompt: {}",media.model,media.params,prompt),"details":media}),("assistant_history_recovery_failed","assistant_history_recovery_timeout")).await.map_err(Failure::unsent)?;
        self.ensure_current(&run.identity, generation)
            .await
            .map_err(Failure::unsent)?;
        blocking({
            let store = self.store(&run.identity.owner)?;
            let sid = run.session_id.clone();
            let keys = self.known_keys();
            let entries = entries.clone();
            move || {
                store.mutate(&sid, |record| {
                    super::projection::merge_native(record, &entries, &keys)
                })
            }
        })
        .await
        .map_err(Failure::unsent)?;
        let client = crate::proxy::http_client::assistant_media_client()?;
        let gateway = self.env.gateway_base(&run.identity.apex)?;
        let params: VideoRequest =
            serde_json::from_value(media.params.clone()).map_err(|_| media::INVALID)?;
        let key = run
            .secrets()
            .last()
            .cloned()
            .ok_or("assistant_key_rejected")?;
        self.validate_run(run).map_err(Failure::unsent)?;
        let mut result = video_http::create(
            &client,
            &gateway,
            &key,
            &media.model,
            prompt,
            &params,
            &frames,
            &[],
        )
        .await;
        let (receipt, usage) = match &result {
            Ok(r) => (Some(&r.receipt), r.value.get("usage")),
            Err(e) => (e.receipt.as_ref(), None),
        };
        self.record_media_attempt(run, receipt, usage).await?;
        if result
            .as_ref()
            .is_err_and(|error| error.code == "assistant_key_rejected")
        {
            let key = self
                .key_for_retry(run, &json!({"apiKey":key}))
                .await
                .map_err(Failure::unsent)?;
            run.add_secret(&key);
            self.remember_key(&key);
            self.validate_run(run).map_err(Failure::unsent)?;
            result = video_http::create(
                &client,
                &gateway,
                &key,
                &media.model,
                prompt,
                &params,
                &frames,
                &[],
            )
            .await;
            let (receipt, usage) = match &result {
                Ok(r) => (Some(&r.receipt), r.value.get("usage")),
                Err(e) => (e.receipt.as_ref(), None),
            };
            self.record_media_attempt(run, receipt, usage).await?;
        }
        let response = result.map_err(|error| Failure {
            code: error.code,
            unbilled: error.unbilled,
            detail: error.detail,
            native: None,
        })?;
        let job = video::job_id(&response)?;
        // Preserve the known id even if the account was revoked while awaiting
        // the response. This writes only to the originating account's record.
        let detail = blocking({
            let store = self.store(&run.identity.owner)?;
            let sid = run.session_id.clone();
            let mid = run.message_id.clone();
            let job = job.clone();
            move || {
                store.mutate(&sid, |record| {
                    let message = record
                        .messages
                        .iter_mut()
                        .find(|m| m.id == mid)
                        .ok_or("assistant_message_not_found")?;
                    let media = message.media.as_mut().ok_or(media::INVALID)?;
                    media.job_id = Some(job);
                    media.status = "queued".into();
                    message.status = "queued".into();
                    let media = media.clone();
                    let run = message.run_id.clone();
                    for op in record
                        .operations
                        .iter_mut()
                        .filter(|op| Some(&op.run_id) == run.as_ref())
                    {
                        op.status = OperationStatus::PendingMedia;
                        op.media = Some(media.clone());
                    }
                    record.session.active_run = None;
                    super::projection::recount_usage(
                        record,
                        entries.as_array().ok_or(super::host::PROTOCOL)?,
                    );
                    Ok(())
                })?;
                store.load_detail(&sid)
            }
        })
        .await?;
        self.release_run(run);
        if self.ensure_current(&run.identity, generation).await.is_ok() {
            if let Some(message) = detail.messages.iter().find(|m| m.id == run.message_id) {
                self.emit(run, "message", json!({"message":message}));
            }
            self.emit(run, "session", json!({"session":detail.session}));
            self.emit(run, "run_end", json!({"status":"queued","stopped":false}));
            self.start_video_poll(
                run.identity.clone(),
                generation,
                run.session_id.clone(),
                run.message_id.clone(),
                None,
            );
        }
        Ok(())
    }

    /// Update only this job; a terminal state written by cancellation wins
    /// over a slower query. Known job metadata is never removed.
    async fn write_video_state(
        &self,
        run: &RunControl,
        job: Option<&str>,
        status: &str,
        cost: Option<String>,
        error: Option<&str>,
    ) -> Result<(), String> {
        let history = if cost.is_some() {
            Some(
                self.native_lock(&session_key(&run.identity.owner, &run.session_id))?
                    .lock_owned()
                    .await,
            )
        } else {
            None
        };
        let entries = if history.is_some() {
            let file = blocking({
                let store = self.store(&run.identity.owner)?;
                let sid = run.session_id.clone();
                move || store.native_history_path(&sid)
            })
            .await?;
            Some(super::host::load_native(&self.runtime()?, &file).await?)
        } else {
            None
        };
        let store = self.store(&run.identity.owner)?;
        let sid = run.session_id.clone();
        let mid = run.message_id.clone();
        let artifact = run.artifact.clone();
        let job = job.map(str::to_owned);
        let status = status.to_owned();
        let error = error.map(str::to_owned);
        blocking(move || {
            store.mutate(&sid, |record| {
                let message = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == mid)
                    .ok_or("assistant_message_not_found")?;
                let media = message
                    .media_for_mut(artifact.as_deref())
                    .ok_or(media::INVALID)?;
                if job
                    .as_ref()
                    .is_some_and(|job| media.job_id.as_ref() != Some(job))
                    || !video::active_status(&media.status)
                {
                    return Ok(());
                }
                media.status = status.clone();
                if cost.is_some() {
                    media.cost = cost;
                }
                if artifact.is_some() {
                    // The reply's own status is its chat run's; only the
                    // generation changes.
                    media.error_code = error;
                    if let Some(entries) = &entries {
                        super::projection::recount_usage(
                            record,
                            entries.as_array().ok_or(super::host::PROTOCOL)?,
                        );
                    }
                    return Ok(());
                }
                message.status = if matches!(
                    status.as_str(),
                    "failed" | "unknown" | "download_failed" | "query_expired"
                ) {
                    "error"
                } else {
                    &status
                }
                .into();
                message.error_code = error;
                let media = message.media.clone().ok_or(media::INVALID)?;
                let rid = message.run_id.clone();
                for op in record
                    .operations
                    .iter_mut()
                    .filter(|op| Some(&op.run_id) == rid.as_ref())
                {
                    op.media = Some(media.clone());
                    op.status = match status.as_str() {
                        "completed" => OperationStatus::Completed,
                        "failed" | "unknown" | "expired" => OperationStatus::Failed,
                        "cancelled" => OperationStatus::Cancelled,
                        _ => OperationStatus::PendingMedia,
                    };
                }
                if let Some(entries) = &entries {
                    super::projection::recount_usage(
                        record,
                        entries.as_array().ok_or(super::host::PROTOCOL)?,
                    );
                }
                Ok(())
            })
        })
        .await
    }

    pub(super) fn schedule_video_resume(self: &Arc<Self>) {
        let host = self.clone();
        // Auth events may arrive on a native event thread with no Tokio
        // context. Enter the app runtime before registering tracked work.
        tauri::async_runtime::spawn(async move {
            let worker = host.clone();
            host.track(
                format!("video-resume:{}:{}", host.generation(), id()),
                async move {
                    worker.resume_video_jobs().await;
                },
            );
        });
    }
    pub(super) async fn resume_video_jobs(self: &Arc<Self>) {
        if self.ensure_accepting_runs().is_err() {
            return;
        }
        let Ok(identity) = self.identity().await else {
            return;
        };
        let generation = self.generation();
        let Ok(store) = self.store(&identity.owner) else {
            return;
        };
        let apex = identity.apex.clone();
        let jobs = blocking(move || {
            let mut sessions = store.list(false, "")?.sessions;
            sessions.extend(store.list(true, "")?.sessions);
            let mut jobs = vec![];
            for session in sessions {
                let record = store.load_record(&session.id)?;
                for message in record.messages {
                    for media in message.all_media() {
                        if media.kind == "video"
                            && media.apex.as_deref() == Some(&apex)
                            && media.job_id.is_some()
                            && matches!(
                                media.status.as_str(),
                                "queued" | "in_progress" | "downloading" | "paused"
                            )
                        {
                            jobs.push((
                                session.id.clone(),
                                message.id.clone(),
                                media.tool_call_id.clone(),
                            ));
                        }
                    }
                }
            }
            Ok(jobs)
        })
        .await;
        if self.ensure_current(&identity, generation).await.is_err() {
            return;
        }
        if let Ok(jobs) = jobs {
            for (sid, mid, artifact) in jobs {
                self.start_video_poll(identity.clone(), generation, sid, mid, artifact);
            }
        }
    }
    pub(super) fn start_video_poll(
        self: &Arc<Self>,
        identity: Identity,
        generation: u64,
        sid: String,
        mid: String,
        artifact: Option<String>,
    ) {
        let key = poll_key(&identity, &sid, &mid, artifact.as_deref(), generation);
        let mut set = self
            .video_polling
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !set.insert(key.clone()) {
            return;
        }
        drop(set);
        let registration = PollRegistration {
            set: self.video_polling.clone(),
            key: key.clone(),
        };
        let host = self.clone();
        self.track(format!("video-poll:{key}"), async move {
            let _registration = registration;
            let _ = host
                .poll_video(identity, generation, sid, mid, artifact)
                .await;
        });
    }
    async fn poll_video(
        self: &Arc<Self>,
        identity: Identity,
        generation: u64,
        sid: String,
        mid: String,
        artifact: Option<String>,
    ) -> Result<(), String> {
        let client = crate::proxy::http_client::assistant_media_client()?;
        let gateway = self.env.gateway_base(&identity.apex)?;
        let mut revoked = self.video_revocation.subscribe();
        let mut delay = Duration::ZERO;
        loop {
            self.ensure_current(&identity, generation).await?;
            self.admit(generation)?;
            if !delay.is_zero() {
                tokio::select! {biased;_=revoked.changed()=>return Ok(()),_=tokio::time::sleep(delay)=>{}}
            }
            self.ensure_current(&identity, generation).await?;
            let record = blocking({
                let store = self.store(&identity.owner)?;
                let sid = sid.clone();
                move || store.load_record(&sid)
            })
            .await?;
            let message = record
                .messages
                .iter()
                .find(|m| m.id == mid)
                .ok_or("assistant_message_not_found")?;
            let media = message
                .media_for(artifact.as_deref())
                .ok_or(media::INVALID)?;
            if !matches!(
                media.status.as_str(),
                "queued" | "in_progress" | "downloading" | "paused"
            ) || media.apex.as_deref() != Some(&identity.apex)
            {
                return Ok(());
            }
            let job = media.job_id.as_deref().ok_or(media::INVALID)?;
            let operation = record
                .operations
                .iter()
                .find(|op| Some(&op.run_id) == message.run_id.as_ref())
                .ok_or(STATE)?;
            let request:SendRequest=serde_json::from_value(json!({"sessionId":sid,"runId":operation.run_id,"operationId":operation.id,"text":""})).map_err(|_|STATE)?;
            let key = self
                .env
                .api_key(&identity, None, &|| self.admit(generation))
                .await?;
            self.remember_key(&key);
            let mut control = RunControl::new(identity.clone(), &request, key.clone());
            control.message_id = mid.clone();
            control.artifact = artifact.clone();
            let control = Arc::new(control);
            if now().saturating_sub(media.poll_started_at.unwrap_or_else(now))
                >= 24 * 60 * 60 * 1000
            {
                self.write_video_state(
                    &control,
                    Some(job),
                    "query_expired",
                    None,
                    Some("assistant_media_query_expired"),
                )
                .await?;
                return self.emit_video_state(&control, generation).await;
            }
            let mut result = video_http::query(&client, &gateway, &key, job).await;
            if result
                .as_ref()
                .is_err_and(|error| error.code == "assistant_key_rejected")
            {
                let key = self.key_for_retry(&control, &json!({"apiKey":key})).await?;
                control.add_secret(&key);
                self.remember_key(&key);
                self.ensure_current(&identity, generation).await?;
                result = video_http::query(&client, &gateway, &key, job).await;
            }
            self.ensure_current(&identity, generation).await?;
            match result {
                Ok(value) if value["id"].as_str() == Some(job) => {
                    let status = match value["status"].as_str() {
                        Some("pending" | "queued") => "queued",
                        Some("in_progress") => "in_progress",
                        Some("failed") => "failed",
                        Some("cancelled") => "cancelled",
                        Some("expired") => "expired",
                        Some("completed") => "downloading",
                        _ => {
                            delay = Duration::from_secs(15);
                            continue;
                        }
                    };
                    self.record_video_usage(&control, job, &value["usage"])
                        .await?;
                    let cost = value["usage"]["video_cost"]
                        .as_str()
                        .and_then(|n| n.parse::<Decimal>().ok())
                        .filter(|n| *n >= Decimal::ZERO)
                        .map(|n| n.normalize().to_string());
                    self.write_video_state(&control, Some(job), status, cost.clone(), None)
                        .await?;
                    self.emit_video_state(&control, generation).await?;
                    if status == "downloading" {
                        let proxy = blocking({
                            let env = self.env.clone();
                            move || Ok(env.proxy())
                        })
                        .await?;
                        self.ensure_current(&identity, generation).await?;
                        let downloaded = video_http::download(
                            self.store(&identity.owner)?,
                            &client,
                            &value,
                            proxy,
                            &control.secrets(),
                        )
                        .await;
                        self.ensure_current(&identity, generation).await?;
                        let result = match downloaded {
                            Ok(mut file) => {
                                let mut media = media.clone();
                                super::provenance::video_facts(&mut media, &value["usage"], cost);
                                if let Some(usage) = &mut media.response_usage {
                                    super::host::scrub_secrets_in(usage, &self.known_keys());
                                }
                                blocking({
                                    let store = self.store(&identity.owner)?;
                                    let sid = sid.clone();
                                    let mid = mid.clone();
                                    let host = self.clone();
                                    let job = job.to_owned();
                                    let artifact = artifact.clone();
                                    move || {
                                        super::attachments::generated_video(
                                            &store,
                                            &sid,
                                            &mid,
                                            artifact.as_deref(),
                                            file.as_file_mut(),
                                            media,
                                            |record| {
                                                host.admit(generation)?;
                                                let current = record
                                                    .messages
                                                    .iter()
                                                    .find(|m| m.id == mid)
                                                    .and_then(|m| m.media_for(artifact.as_deref()))
                                                    .ok_or(media::INVALID)?;
                                                if current.job_id.as_ref() != Some(&job)
                                                    || current.status != "downloading"
                                                {
                                                    return Err(media::INVALID.into());
                                                }
                                                Ok(())
                                            },
                                        )
                                        .map(|_| ())
                                    }
                                })
                                .await
                            }
                            Err(error) => Err(error),
                        };
                        if let Err(error) = result {
                            self.write_video_state(
                                &control,
                                Some(job),
                                "download_failed",
                                None,
                                Some(&error),
                            )
                            .await?;
                        }
                        self.emit_video_state(&control, generation).await?;
                        return Ok(());
                    }
                    if !matches!(status, "queued" | "in_progress") {
                        return Ok(());
                    }
                }
                Err(error) if error.code == "assistant_media_job_not_found" => {
                    self.write_video_state(
                        &control,
                        Some(job),
                        "query_expired",
                        None,
                        Some(&error.code),
                    )
                    .await?;
                    return self.emit_video_state(&control, generation).await;
                }
                Err(error) if error.code == "assistant_rate_limited" => {
                    delay = Duration::from_secs(60);
                    continue;
                }
                _ => {
                    delay = Duration::from_secs(15);
                    continue;
                }
            }
            delay = Duration::from_secs(
                if now().saturating_sub(media.submitted_at.unwrap_or_else(now)) < 120_000 {
                    6
                } else {
                    15
                },
            );
        }
    }
}

impl AssistantHost {
    async fn video_action_control(
        &self,
        request: &video::VideoActionRequest,
    ) -> Result<(Arc<RunControl>, MediaRecord, u64), String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        if request.authority.expected_owner != identity.owner
            || request.authority.expected_generation != generation
        {
            return Err(IDENTITY_CHANGED.into());
        }
        validate_id(&request.authority.session_id)?;
        validate_id(&request.message_id)?;
        self.ensure_accepting_runs()?;
        let record = blocking({
            let store = self.store(&identity.owner)?;
            let sid = request.authority.session_id.clone();
            move || store.load_record(&sid)
        })
        .await?;
        let message = record
            .messages
            .iter()
            .find(|m| m.id == request.message_id)
            .ok_or("assistant_message_not_found")?;
        let media = message
            .media_for(request.tool_call_id.as_deref())
            .cloned()
            .filter(|m| m.kind == "video")
            .ok_or(media::INVALID)?;
        if media.apex.as_deref() != Some(&identity.apex) {
            return Err(IDENTITY_CHANGED.into());
        }
        let op = record
            .operations
            .iter()
            .find(|op| Some(&op.run_id) == message.run_id.as_ref())
            .ok_or(STATE)?;
        let send:SendRequest=serde_json::from_value(json!({"sessionId":request.authority.session_id,"runId":op.run_id,"operationId":op.id,"text":""})).map_err(|_|STATE)?;
        let mut control = RunControl::new(identity.clone(), &send, String::new());
        control.message_id = message.id.clone();
        control.artifact = request.tool_call_id.clone();
        self.ensure_current(&identity, generation).await?;
        Ok((Arc::new(control), media, generation))
    }
    pub async fn cancel_video(
        self: &Arc<Self>,
        request: video::VideoActionRequest,
    ) -> Result<(), String> {
        let (control, media, generation) = self.video_action_control(&request).await?;
        if media.status == "submitting" {
            return self
                .stop(&control.session_id, &control.run_id, &control.operation_id)
                .await;
        }
        if !matches!(
            media.status.as_str(),
            "queued" | "in_progress" | "paused" | "query_expired"
        ) {
            return Err(media::INVALID.into());
        }
        let job = media.job_id.as_deref().ok_or(media::INVALID)?;
        let client = crate::proxy::http_client::assistant_media_client()?;
        let gateway = self.env.gateway_base(&control.identity.apex)?;
        let key = self
            .env
            .api_key(&control.identity, None, &|| self.admit(generation))
            .await?;
        self.remember_key(&key);
        control.add_secret(&key);
        self.ensure_current(&control.identity, generation).await?;
        let mut result = video_http::cancel(&client, &gateway, &key, job).await;
        if result
            .as_ref()
            .is_err_and(|e| e.code == "assistant_key_rejected")
        {
            let key = self
                .env
                .api_key(&control.identity, Some(&key), &|| self.admit(generation))
                .await?;
            self.remember_key(&key);
            control.add_secret(&key);
            self.ensure_current(&control.identity, generation).await?;
            result = video_http::cancel(&client, &gateway, &key, job).await;
        }
        self.ensure_current(&control.identity, generation).await?;
        result.map_err(|e| e.code)?;
        self.write_video_state(&control, Some(job), "cancelled", None, None)
            .await?;
        self.emit_video_state(&control, generation).await
    }
    pub async fn resume_video(
        self: &Arc<Self>,
        request: video::VideoActionRequest,
    ) -> Result<(), String> {
        let (control, media, generation) = self.video_action_control(&request).await?;
        let job = media.job_id.as_ref().ok_or(media::INVALID)?.clone();
        if !matches!(
            media.status.as_str(),
            "download_failed" | "query_expired" | "paused"
        ) {
            return Err(media::INVALID.into());
        }
        let sid = control.session_id.clone();
        let mid = control.message_id.clone();
        blocking({
            let store = self.store(&control.identity.owner)?;
            let host = self.clone();
            let control = control.clone();
            let sid = sid.clone();
            let mid = mid.clone();
            move || {
                store.mutate(&sid, |record| {
                    host.admit(generation)?;
                    let message = record
                        .messages
                        .iter_mut()
                        .find(|m| m.id == mid)
                        .ok_or("assistant_message_not_found")?;
                    let media = message
                        .media_for_mut(control.artifact.as_deref())
                        .ok_or(media::INVALID)?;
                    if media.job_id.as_ref() != Some(&job)
                        || !matches!(
                            media.status.as_str(),
                            "download_failed" | "query_expired" | "paused"
                        )
                    {
                        return Err(media::INVALID.into());
                    }
                    media.status = "queued".into();
                    media.poll_started_at = Some(now());
                    media.error_code = None;
                    if control.artifact.is_some() {
                        return Ok(());
                    }
                    message.status = "queued".into();
                    message.error_code = None;
                    let media = message.media.clone().ok_or(media::INVALID)?;
                    for op in record
                        .operations
                        .iter_mut()
                        .filter(|op| op.run_id == control.run_id)
                    {
                        op.media = Some(media.clone());
                        op.status = OperationStatus::PendingMedia;
                    }
                    Ok(())
                })
            }
        })
        .await?;
        self.ensure_current(&control.identity, generation).await?;
        // A prior terminal poll may still be emitting its last event. Wait
        // for that registration to leave before asking for the new GET.
        let host = self.clone();
        let identity = control.identity.clone();
        let artifact = control.artifact.clone();
        self.track(format!("video-retry:{}", id()), async move {
            let key = poll_key(&identity, &sid, &mid, artifact.as_deref(), generation);
            loop {
                if host.ensure_current(&identity, generation).await.is_err() {
                    return;
                }
                let active = host
                    .video_polling
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .contains(&key);
                if !active {
                    host.start_video_poll(identity, generation, sid, mid, artifact);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
        self.emit_video_state(&control, generation).await
    }
    async fn emit_video_state(&self, control: &RunControl, generation: u64) -> Result<(), String> {
        let detail = blocking({
            let store = self.store(&control.identity.owner)?;
            let sid = control.session_id.clone();
            move || store.load_detail(&sid)
        })
        .await?;
        self.ensure_current(&control.identity, generation).await?;
        if let Some(message) = detail.messages.iter().find(|m| m.id == control.message_id) {
            let media = message.media_for(control.artifact.as_deref());
            let elapsed = media
                .and_then(|m| m.submitted_at)
                .map(|start| now().saturating_sub(start));
            self.emit(control,"media_progress",json!({"owner":control.identity.owner,"generation":generation,"apex":control.identity.apex,"messageId":message.id,"toolCallId":control.artifact,"status":media.map(|m|&m.status),"message":message,"usage":detail.usage,"elapsedMs":elapsed,"updatedAt":detail.session.updated_at}));
        }
        Ok(())
    }
}

/// One poller per job: a direct media turn's (`artifact` `None`) or a
/// generation tool call's.
fn poll_key(
    identity: &Identity,
    sid: &str,
    mid: &str,
    artifact: Option<&str>,
    generation: u64,
) -> String {
    format!(
        "{}:{}:{sid}:{mid}:{}:{generation}",
        identity.owner,
        identity.apex,
        artifact.unwrap_or("")
    )
}
