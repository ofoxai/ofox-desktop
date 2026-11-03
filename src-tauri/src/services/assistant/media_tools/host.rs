//! The generation tools' host side. A chat run already holds the
//! conversation's native lock, so these never take it: the runtime records
//! the tool call and its short result in native history, and the host keeps
//! each generation on the reply. Nothing here repeats a request whose outcome
//! is unknown; only a refused key is retried once with a new one.
use super::*;
use crate::services::assistant::{
    attachments::{self, Source},
    catalog::Model,
    host::{blocking, scrub_secrets, scrub_secrets_in, AssistantHost, RunControl, PROTOCOL, STATE},
    materials::{self, Material, Plan},
    media, media_host, media_http,
    speech::{self, SpeechRequest},
    store::SessionRecord,
    tools::registry,
    types::{now, validate_id, Attachment, MediaRecord},
    video, video_http,
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

/// A tool call's argument fingerprint and, once it ended, its result.
type CallRecord = (String, Option<Result<Value, String>>);
/// Files read for a video request, by id: MIME type and bytes.
type MaterialFiles = HashMap<String, (String, Vec<u8>)>;
/// A video request's inputs: frame or reference kind, and data URL.
type Inputs = Vec<(&'static str, String)>;

/// The generation tools of one chat run: the defaults it froze when it
/// started, and what it has made so far.
pub(in crate::services::assistant) struct MediaRun {
    pub defaults: MediaDefaults,
    images: AtomicUsize,
    calls: Mutex<HashMap<String, CallRecord>>,
    /// Submitted videos, polled once the run ends: (reply, tool call,
    /// authority generation).
    polls: Mutex<Vec<(String, String, u64)>>,
}

impl MediaRun {
    pub fn new(defaults: MediaDefaults) -> Self {
        Self {
            defaults,
            images: AtomicUsize::new(0),
            calls: Mutex::new(HashMap::new()),
            polls: Mutex::new(vec![]),
        }
    }

    pub fn take_polls(&self) -> Vec<(String, String, u64)> {
        std::mem::take(
            &mut *self
                .polls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Hold `count` of the run's images, or refuse when they do not fit.
    fn reserve_images(&self, count: usize) -> Result<(), String> {
        self.images
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |made| {
                (made + count <= MAX_IMAGES_PER_RUN).then_some(made + count)
            })
            .map(|_| ())
            .map_err(|made| {
                refuse(
                    TURN_LIMIT,
                    format!(
                        "at most {MAX_IMAGES_PER_RUN} images per reply; {} left",
                        MAX_IMAGES_PER_RUN.saturating_sub(made)
                    ),
                )
            })
    }

    fn release_images(&self, count: usize) {
        let _ = self
            .images
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |made| {
                Some(made.saturating_sub(count))
            });
    }
}

const AUDIO_REFERENCE_BYTES: u64 = 15 * 1024 * 1024;
const VIDEO_REFERENCE_BYTES: u64 = 25 * 1024 * 1024;

impl AssistantHost {
    /// A generation tool call of a chat run, answered with a short result
    /// (ids and status, never bytes).
    pub(in crate::services::assistant) async fn dispatch_media_tool(
        self: &Arc<Self>,
        run: &Arc<RunControl>,
        params: &Value,
    ) -> Result<Value, String> {
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err(crate::services::assistant::host::IDENTITY_CHANGED.into());
        }
        let call: registry::Call =
            serde_json::from_value(params.clone()).map_err(|_| "assistant_invalid_tool")?;
        if call.session_id != run.session_id
            || call.run_id != run.run_id
            || call.operation_id != run.operation_id
        {
            return Err("assistant_runtime_identity_mismatch".into());
        }
        if call.tool_call_id.is_empty() || call.tool_call_id.len() > 200 {
            return Err("assistant_invalid_tool".into());
        }
        let media = run
            .media
            .as_ref()
            .filter(|_| !run.compact_only)
            .ok_or("assistant_capability_disabled")?;
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&json!([call.name, call.args])).map_err(|_| PROTOCOL)?
            )
        );
        {
            let mut calls = media.calls.lock().map_err(|_| STATE)?;
            if let Some((previous, result)) = calls.get(&call.tool_call_id) {
                if previous != &fingerprint {
                    return Err("assistant_tool_call_conflict".into());
                }
                return result
                    .clone()
                    .unwrap_or_else(|| Err("unknown_outcome".into()));
            }
            calls.insert(call.tool_call_id.clone(), (fingerprint.clone(), None));
        }
        let mid = run.reply_id();
        let work = async {
            match call.name.as_str() {
                IMAGE => {
                    let args = serde_json::from_value(call.args.clone())
                        .map_err(|_| refuse(INVALID, "unknown or malformed arguments"))?;
                    self.tool_image(run, media, &mid, &call.tool_call_id, args)
                        .await
                }
                VIDEO => {
                    let args = serde_json::from_value(call.args.clone())
                        .map_err(|_| refuse(INVALID, "unknown or malformed arguments"))?;
                    self.tool_video(run, media, &mid, &call.tool_call_id, args)
                        .await
                }
                SPEECH => {
                    let args = serde_json::from_value(call.args.clone())
                        .map_err(|_| refuse(INVALID, "unknown or malformed arguments"))?;
                    self.tool_speech(run, media, &mid, &call.tool_call_id, args)
                        .await
                }
                _ => Err("assistant_invalid_tool".into()),
            }
        };
        let mut cancel = run.cancel.subscribe();
        let result = tokio::select! {
            biased;
            _ = cancel.wait_for(|cancelled| *cancelled) => Err(media::UNKNOWN.to_string()),
            result = work => result,
        };
        if result.is_err() {
            // A generation cut off before its answer keeps an unknown outcome.
            let _ = self
                .settle_generation(run, &mid, &call.tool_call_id, "unknown", None)
                .await;
        }
        let result = result.map_err(|error| scrub_secrets(&error, &run.secrets()));
        media
            .calls
            .lock()
            .map_err(|_| STATE)?
            .insert(call.tool_call_id, (fingerprint, Some(result.clone())));
        result
    }

    async fn tool_image(
        &self,
        run: &Arc<RunControl>,
        media: &MediaRun,
        mid: &str,
        call_id: &str,
        args: ImageArgs,
    ) -> Result<Value, String> {
        let defaults = media.defaults.image.as_ref();
        let catalog = self.models(&run.identity.apex).await?;
        let model = pick_model(
            &catalog.models,
            Kind::Image,
            args.model.as_deref(),
            defaults.and_then(|d| d.model.as_deref()),
        )?
        .clone();
        let (prompt, request) = image_request(&model, defaults, &args, 0)?;
        let prompt = scrub_secrets(&prompt, &run.secrets());
        let count = request.count();
        media.reserve_images(count)?;
        let outcome = self
            .make_image(run, mid, call_id, &model, &prompt, &request)
            .await;
        if outcome.as_ref().is_err_and(|(_, unbilled)| *unbilled) {
            media.release_images(count);
        }
        outcome.map_err(|(error, _)| error)
    }

    /// Generate and save the images of one call: `Err((code, unbilled))`.
    async fn make_image(
        &self,
        run: &Arc<RunControl>,
        mid: &str,
        call_id: &str,
        model: &Model,
        prompt: &str,
        request: &ImageRequest,
    ) -> Result<Value, (String, bool)> {
        let unsent = |error: String| (error, true);
        let record = self.load(run).await.map_err(unsent)?;
        attachments::tool_room(
            &record,
            request.count(),
            (request.count() * media::MAX_IMAGE_BYTES) as u64,
        )
        .map_err(unsent)?;
        let input = match &request.input_attachment_id {
            Some(id) => {
                let file = selectable(&record, id).map_err(unsent)?;
                if materials::Kind::of_mime(&file.mime_type) != Some(materials::Kind::Image) {
                    return Err(unsent(refuse(
                        INVALID,
                        format!("{} is not a PNG, JPEG or WebP image", file.name),
                    )));
                }
                let root = self.attachment_root(run).map_err(unsent)?;
                Some(
                    blocking(move || media_host::read_input(root, &file))
                        .await
                        .map_err(unsent)?,
                )
            }
            None => None,
        };
        let caps = model.image.as_ref().ok_or_else(|| unsent(INVALID.into()))?;
        let mut record_media = MediaRecord {
            tool_call_id: Some(call_id.into()),
            prompt: Some(prompt.into()),
            kind: "image".into(),
            apex: Some(run.identity.apex.clone()),
            submitted_at: Some(now()),
            price_snapshot: serde_json::to_value(caps).ok(),
            model: model.id.clone(),
            params: serde_json::to_value(request).map_err(|_| unsent(INVALID.into()))?,
            input_ids: request.input_attachment_id.iter().cloned().collect(),
            status: "submitting".into(),
            ..Default::default()
        };
        self.save_generation(run, mid, &record_media)
            .await
            .map_err(unsent)?;
        let gateway = self.env.gateway_base(&run.identity.apex).map_err(unsent)?;
        let client = crate::proxy::http_client::assistant_media_client().map_err(unsent)?;
        let mut key = run
            .secrets()
            .last()
            .cloned()
            .ok_or_else(|| unsent("assistant_key_rejected".into()))?;
        let mut result = media_http::request(
            &client,
            &gateway,
            &key,
            &model.id,
            prompt,
            request,
            input.as_ref(),
        )
        .await;
        for retry in [true, false] {
            match &result {
                Ok(response) => record_attempt(
                    &mut record_media,
                    Some(&response.receipt),
                    response.value.get("usage"),
                ),
                Err(error) => record_attempt(&mut record_media, error.receipt.as_ref(), None),
            }
            let refused = result
                .as_ref()
                .is_err_and(|e| e.code == "assistant_key_rejected");
            if !(retry && refused) {
                break;
            }
            key = self
                .key_for_retry(run, &json!({ "apiKey": key }))
                .await
                .map_err(unsent)?;
            run.add_secret(&key);
            self.remember_key(&key);
            self.validate_run(run).map_err(unsent)?;
            result = media_http::request(
                &client,
                &gateway,
                &key,
                &model.id,
                prompt,
                request,
                input.as_ref(),
            )
            .await;
        }
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                record_media.status = if error.unbilled { "error" } else { "unknown" }.into();
                record_media.cost = error.unbilled.then(|| "0".into());
                record_media.error_code = Some(error.code.clone());
                let _ = self.save_generation(run, mid, &record_media).await;
                let why = error
                    .detail
                    .unwrap_or_else(|| "the image request failed".into());
                return Err((format!("{}: {why}", error.code), error.unbilled));
            }
        };
        let billed = |error: String| (error, false);
        let images = media::decode(&response.value, request.count()).map_err(billed)?;
        record_media.cost = media::cost(
            caps,
            &response.value["usage"],
            request.count(),
            input.is_some(),
        )
        .map(|n| n.normalize().to_string());
        record_media.status = "completed".into();
        let files = blocking({
            let store = self.store(&run.identity.owner).map_err(billed)?;
            let (sid, mid, call) = (run.session_id.clone(), mid.to_owned(), call_id.to_owned());
            let run = run.clone();
            let record_media = record_media.clone();
            move || {
                let sources: Vec<_> = images
                    .into_iter()
                    .enumerate()
                    .map(|(i, (ext, bytes))| Source::Bytes {
                        name: format!("generated-{}.{ext}", i + 1),
                        bytes,
                    })
                    .collect();
                attachments::generated(
                    &store,
                    &sid,
                    &mid,
                    Some(&call),
                    &sources,
                    record_media,
                    |record| still_running(record, &run),
                )
            }
        })
        .await
        .map_err(billed)?;
        record_media.outputs = files.iter().map(|a| a.id.clone()).collect();
        self.publish_generation(run, mid, &record_media, &files);
        Ok(json!({
            "status": "completed",
            "model": model.id,
            "files": shown_files(&files),
            "estimatedCostUsd": record_media.cost,
        }))
    }

    async fn tool_video(
        self: &Arc<Self>,
        run: &Arc<RunControl>,
        media: &MediaRun,
        mid: &str,
        call_id: &str,
        args: VideoArgs,
    ) -> Result<Value, String> {
        let defaults = media.defaults.video.as_ref();
        let catalog = self.models(&run.identity.apex).await?;
        let model = pick_model(
            &catalog.models,
            Kind::Video,
            args.model.as_deref(),
            defaults.and_then(|d| d.model.as_deref()),
        )?
        .clone();
        let record = self.load(run).await?;
        let root = self.attachment_root(run)?;
        let (files, materials) = {
            let record = record.clone();
            let ids = args.material_ids.clone();
            blocking(move || read_materials(&record, root, &ids)).await?
        };
        let plan = materials::plan(&materials, args.material_use)
            .map_err(|code| refuse(&code, material_hint(&code)))?;
        let (prompt, request) = video_request(&model, defaults, &args, &plan)?;
        let prompt = scrub_secrets(&prompt, &run.secrets());
        let estimate = self
            .env
            .provider_prices(&run.identity.apex, &model.id)
            .await
            .ok()
            .and_then(|data| video::estimate(&data, &request))
            .map(|n| n.normalize().to_string());
        let details = json!({
            "model": model.id, "prompt": prompt, "duration": request.duration,
            "resolution": request.resolution, "aspectRatio": request.aspect_ratio,
            "generateAudio": request.generate_audio, "materials": plan,
            "estimatedCostUsd": estimate,
        });
        if !self
            .confirm_generation(run, call_id, VIDEO, details)
            .await?
        {
            return Err(refuse(
                DECLINED,
                "the user declined this video; ask what to change instead of retrying",
            ));
        }
        let admission = self.video_admission.lock().await;
        self.check_video_slots(run).await?;
        let mut record_media = MediaRecord {
            tool_call_id: Some(call_id.into()),
            prompt: Some(prompt.clone()),
            kind: "video".into(),
            price_snapshot: Some(json!({"quotedTotalUsd": estimate, "params": request})),
            model: model.id.clone(),
            params: video_params(&request, &plan),
            input_ids: plan.ids(),
            status: "submitting".into(),
            apex: Some(run.identity.apex.clone()),
            submitted_at: Some(now()),
            poll_started_at: Some(now()),
            ..Default::default()
        };
        self.save_generation(run, mid, &record_media).await?;
        drop(admission);
        let (frames, references) = encode_materials(&plan, &files);
        let gateway = self.env.gateway_base(&run.identity.apex)?;
        let client = crate::proxy::http_client::assistant_media_client()?;
        let mut key = run
            .secrets()
            .last()
            .cloned()
            .ok_or("assistant_key_rejected")?;
        let frames: Vec<(&str, String)> = frames.iter().map(|(k, v)| (*k, v.clone())).collect();
        let references: Vec<(&str, String)> =
            references.iter().map(|(k, v)| (*k, v.clone())).collect();
        let mut result = video_http::create(
            &client,
            &gateway,
            &key,
            &model.id,
            &prompt,
            &request,
            &frames,
            &references,
        )
        .await;
        for retry in [true, false] {
            match &result {
                Ok(response) => record_attempt(
                    &mut record_media,
                    Some(&response.receipt),
                    response.value.get("usage"),
                ),
                Err(error) => record_attempt(&mut record_media, error.receipt.as_ref(), None),
            }
            let refused = result
                .as_ref()
                .is_err_and(|e| e.code == "assistant_key_rejected");
            if !(retry && refused) {
                break;
            }
            key = self.key_for_retry(run, &json!({ "apiKey": key })).await?;
            run.add_secret(&key);
            self.remember_key(&key);
            self.validate_run(run)?;
            result = video_http::create(
                &client,
                &gateway,
                &key,
                &model.id,
                &prompt,
                &request,
                &frames,
                &references,
            )
            .await;
        }
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                record_media.status = if error.unbilled { "failed" } else { "unknown" }.into();
                record_media.cost = error.unbilled.then(|| "0".into());
                record_media.error_code = Some(error.code.clone());
                let _ = self.save_generation(run, mid, &record_media).await;
                let why = error
                    .detail
                    .unwrap_or_else(|| "the video request failed".into());
                return Err(format!("{}: {why}", error.code));
            }
        };
        let job = video::job_id(&response.value)?;
        record_media.job_id = Some(job);
        record_media.status = "queued".into();
        self.save_generation(run, mid, &record_media).await?;
        self.publish_generation(run, mid, &record_media, &[]);
        media.polls.lock().map_err(|_| STATE)?.push((
            mid.to_owned(),
            call_id.to_owned(),
            self.generation(),
        ));
        Ok(json!({
            "status": "queued",
            "model": model.id,
            "estimatedCostUsd": estimate,
            "note": "The video is being generated and will appear under this reply in a few minutes; do not submit it again.",
        }))
    }

    async fn tool_speech(
        &self,
        run: &Arc<RunControl>,
        media: &MediaRun,
        mid: &str,
        call_id: &str,
        args: SpeechArgs,
    ) -> Result<Value, String> {
        let defaults = media.defaults.speech.as_ref();
        let catalog = self.models(&run.identity.apex).await?;
        let model = pick_model(
            &catalog.models,
            Kind::Speech,
            args.model.as_deref(),
            defaults.and_then(|d| d.model.as_deref()),
        )?
        .clone();
        let choice = speech_choice(&model, defaults, &args)?;
        let text = scrub_secrets(&choice.text, &run.secrets());
        let request = SpeechRequest {
            session_id: run.session_id.clone(),
            expected_owner: run.identity.owner.clone(),
            expected_generation: self.generation(),
            run_id: run.run_id.clone(),
            operation_id: run.operation_id.clone(),
            model_id: model.id.clone(),
            input: text.clone(),
            voice: choice.voice.clone(),
            response_format: choice.format.clone(),
            language: choice.language.clone(),
            instructions: choice.style.clone(),
            source_message_id: Some(mid.into()),
        };
        request
            .body(&model)
            .map_err(|code| refuse(&code, "these voice settings do not fit the model"))?;
        let record = self.load(run).await?;
        attachments::tool_room(&record, 1, speech::MAX_OUTPUT_BYTES as u64)?;
        let caps = model.audio.as_ref().ok_or(INVALID)?;
        let mut record_media = MediaRecord {
            tool_call_id: Some(call_id.into()),
            prompt: Some(text.clone()),
            kind: "speech".into(),
            model: model.id.clone(),
            apex: Some(run.identity.apex.clone()),
            submitted_at: Some(now()),
            params: json!({"voice":choice.voice,"responseFormat":choice.format,"language":choice.language,"instructions":choice.style,"sourceMessageId":mid}),
            status: "submitting".into(),
            audio: Some(crate::services::assistant::audio::Trace {
                pricing: caps.pricing.clone(),
                estimate: speech::estimate(&model, &text),
                output: None,
                input: None,
                text: Some(text.clone()),
                usage: Value::Null,
                request_id: None,
                request_id_source: None,
                error_code: None,
                original_error: None,
            }),
            ..Default::default()
        };
        self.save_generation(run, mid, &record_media).await?;
        let client = blocking({
            let env = self.env.clone();
            move || env.audio_client()
        })
        .await?;
        let gateway = self.env.gateway_base(&run.identity.apex)?;
        let key = run
            .secrets()
            .last()
            .cloned()
            .ok_or("assistant_key_rejected")?;
        let mut result = speech::http::synthesize(&client, &gateway, &key, &model, &request).await;
        if result
            .as_ref()
            .is_err_and(|e| e.code == "assistant_key_rejected")
        {
            let key = self.key_for_retry(run, &json!({ "apiKey": key })).await?;
            run.add_secret(&key);
            self.remember_key(&key);
            self.validate_run(run)?;
            result = speech::http::synthesize(&client, &gateway, &key, &model, &request).await;
        }
        let trace = record_media.audio.as_mut().ok_or(INVALID)?;
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                trace.error_code = Some(error.code.into());
                trace.request_id = error.request_id;
                trace.request_id_source = error.request_id_source;
                record_media.status = if error.unbilled { "error" } else { "unknown" }.into();
                record_media.cost = error.unbilled.then(|| "0".into());
                record_media.error_code = Some(error.code.into());
                let _ = self.save_generation(run, mid, &record_media).await;
                return Err(format!("{}: the speech request failed", error.code));
            }
        };
        trace.output = Some(response.info.clone());
        trace.request_id = response.request_id;
        trace.request_id_source = response.request_id_source;
        record_media.cost = trace.estimate.clone();
        record_media.status = "completed".into();
        let duration_ms = response.info.duration_ms;
        let saved = blocking({
            let store = self.store(&run.identity.owner)?;
            let (sid, mid) = (run.session_id.clone(), mid.to_owned());
            let run = run.clone();
            let mut record_media = record_media.clone();
            move || {
                attachments::generated_audio(
                    &store,
                    &sid,
                    Some(&mid),
                    &mut record_media,
                    &response.bytes,
                    &response.info,
                    |record, saved, item| {
                        still_running(record, &run)?;
                        let message = record
                            .messages
                            .iter_mut()
                            .find(|m| m.id == mid)
                            .ok_or("assistant_message_not_found")?;
                        message.attachments.push(item.clone());
                        message.put_generation(saved.clone());
                        Ok((saved.clone(), item.clone()))
                    },
                )
            }
        })
        .await?;
        let (record_media, file) = saved;
        self.publish_generation(run, mid, &record_media, std::slice::from_ref(&file));
        Ok(json!({
            "status": "completed",
            "model": model.id,
            "files": shown_files(std::slice::from_ref(&file)),
            "durationMs": duration_ms,
            "estimatedCostUsd": record_media.cost,
        }))
    }

    async fn load(&self, run: &RunControl) -> Result<SessionRecord, String> {
        let store = self.store(&run.identity.owner)?;
        let sid = run.session_id.clone();
        let record = blocking(move || store.load_record(&sid)).await?;
        if record.session.deleted_at.is_some() {
            return Err("assistant_session_deleted".into());
        }
        Ok(record)
    }

    fn attachment_root(&self, run: &RunControl) -> Result<std::path::PathBuf, String> {
        self.store(&run.identity.owner)?
            .attachment_root(&run.session_id)
    }

    /// Save a generation on its reply, then show it.
    async fn save_generation(
        &self,
        run: &RunControl,
        mid: &str,
        media: &MediaRecord,
    ) -> Result<(), String> {
        let mut media = media.clone();
        let mut value = serde_json::to_value(&media).map_err(|_| INVALID)?;
        scrub_secrets_in(&mut value, &self.known_keys());
        media = serde_json::from_value(value).map_err(|_| INVALID)?;
        let store = self.store(&run.identity.owner)?;
        let (sid, mid_owned) = (run.session_id.clone(), mid.to_owned());
        let saved = media.clone();
        blocking(move || {
            store.mutate(&sid, |record| {
                let message = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == mid_owned)
                    .ok_or("assistant_message_not_found")?;
                message.put_generation(saved);
                record.session.updated_at = now();
                Ok(())
            })
        })
        .await?;
        self.publish_generation(run, mid, &media, &[]);
        Ok(())
    }

    /// Mark a generation still in flight as ended with `status`.
    async fn settle_generation(
        &self,
        run: &RunControl,
        mid: &str,
        call_id: &str,
        status: &str,
        cost: Option<String>,
    ) -> Result<(), String> {
        let store = self.store(&run.identity.owner)?;
        let (sid, mid_owned, call, status) = (
            run.session_id.clone(),
            mid.to_owned(),
            call_id.to_owned(),
            status.to_owned(),
        );
        let settled = blocking(move || {
            store.mutate(&sid, |record| {
                let Some(media) = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == mid_owned)
                    .and_then(|m| m.generation_mut(&call))
                    .filter(|g| g.status == "submitting")
                else {
                    return Ok(None);
                };
                media.status = status;
                media.cost = cost;
                Ok(Some(media.clone()))
            })
        })
        .await?;
        if let Some(media) = settled {
            self.publish_generation(run, mid, &media, &[]);
        }
        Ok(())
    }

    fn publish_generation(
        &self,
        run: &RunControl,
        mid: &str,
        media: &MediaRecord,
        files: &[Attachment],
    ) {
        let mut data = json!({"messageId": mid, "generation": media, "attachments": files});
        scrub_secrets_in(&mut data, &self.known_keys());
        self.emit(run, "generation", data);
    }

    /// Three videos at a time per account, as for direct video turns.
    async fn check_video_slots(&self, run: &RunControl) -> Result<(), String> {
        let store = self.store(&run.identity.owner)?;
        blocking(move || {
            let mut sessions = store.list(false, "")?.sessions;
            sessions.extend(store.list(true, "")?.sessions);
            let mut count = 0;
            for session in sessions {
                count += video::occupied_slots(&store.load_record(&session.id)?);
            }
            if count >= 3 {
                Err(refuse(
                    "assistant_media_video_limit",
                    "three videos are already being generated; wait for one to finish",
                ))
            } else {
                Ok(())
            }
        })
        .await
    }

    /// Poll the videos a run submitted, now that it has ended.
    pub(in crate::services::assistant) fn start_media_polls(self: &Arc<Self>, run: &RunControl) {
        for (mid, call, generation) in run.media.iter().flat_map(MediaRun::take_polls) {
            self.start_video_poll(
                run.identity.clone(),
                generation,
                run.session_id.clone(),
                mid,
                Some(call),
            );
        }
    }
}

/// A file the conversation holds and a run may use.
fn selectable(record: &SessionRecord, id: &str) -> Result<Attachment, String> {
    validate_id(id).map_err(|_| {
        refuse(
            "assistant_attachment_not_found",
            format!("{id} is not an attachment id"),
        )
    })?;
    record
        .attachments
        .iter()
        .find(|a| a.id == id && a.selectable())
        .cloned()
        .ok_or_else(|| {
            refuse(
                "assistant_attachment_not_found",
                format!("{id} is not an attachment of this conversation"),
            )
        })
}

/// The run is still the conversation's and it was not stopped.
fn still_running(record: &SessionRecord, run: &RunControl) -> Result<(), String> {
    if run.cancelled()
        || record.session.deleted_at.is_some()
        || record
            .session
            .active_run
            .as_ref()
            .is_none_or(|r| r.run_id != run.run_id)
    {
        return Err("assistant_identity_changed".into());
    }
    Ok(())
}

fn record_attempt(
    media: &mut MediaRecord,
    receipt: Option<&crate::services::assistant::provenance::Receipt>,
    usage: Option<&Value>,
) {
    if media.request_attempts.len() < 2 {
        media
            .request_attempts
            .push(receipt.cloned().unwrap_or_default());
    }
    if let Some(usage) = usage.filter(|u| u.is_object()) {
        media.response_usage = Some(usage.clone());
    }
}

fn shown_files(files: &[Attachment]) -> Vec<Value> {
    files
        .iter()
        .map(|f| json!({"id": f.id, "name": f.name, "mimeType": f.mime_type}))
        .collect()
}

fn material_hint(code: &str) -> String {
    match code {
        materials::FRAMES_IMAGES_ONLY => "only images can be first or last frames".into(),
        materials::TOO_MANY_FRAMES => "frames take one image to start and one to end".into(),
        materials::TOO_MANY_REFERENCES => format!(
            "references take at most {} images, {} audio clips and {} video",
            materials::MAX_REFERENCE_IMAGES,
            materials::MAX_REFERENCE_AUDIO,
            materials::MAX_REFERENCE_VIDEO
        ),
        materials::AUDIO_TOO_LONG => format!(
            "reference audio must be at most {} seconds",
            materials::MAX_REFERENCE_AUDIO_MS / 1000
        ),
        _ => "these files cannot be used".into(),
    }
}

/// The files `ids` names, read for a video request, and what they are.
fn read_materials(
    record: &SessionRecord,
    root: std::path::PathBuf,
    ids: &[String],
) -> Result<(MaterialFiles, Vec<Material>), String> {
    let mut files = HashMap::new();
    let mut materials = vec![];
    for id in ids {
        let file = selectable(record, id)?;
        let kind = materials::Kind::of_mime(&file.mime_type).ok_or_else(|| {
            refuse(
                materials::UNSUPPORTED_MATERIAL,
                format!("{} cannot be used in a video: use PNG, JPEG or WebP images, MP3 or WAV audio, or MP4 video", file.name),
            )
        })?;
        let limit = match kind {
            materials::Kind::Image => 15 * 1024 * 1024,
            materials::Kind::Audio => AUDIO_REFERENCE_BYTES,
            materials::Kind::Video => VIDEO_REFERENCE_BYTES,
        };
        let bytes = read_private(&root, &file, limit)?;
        let duration_ms = match kind {
            materials::Kind::Audio => {
                let format = if file.mime_type == "audio/wav" {
                    "wav"
                } else {
                    "mp3"
                };
                crate::services::assistant::audio_input::inspect_generated(format, &bytes)
                    .ok()
                    .map(|info| info.duration_ms)
            }
            _ => None,
        };
        if kind == materials::Kind::Image
            && media::image_type(&bytes).is_none_or(|ext| !matches!(ext, "png" | "jpg" | "webp"))
        {
            return Err(refuse(
                materials::UNSUPPORTED_MATERIAL,
                format!("{} is not a PNG, JPEG or WebP image", file.name),
            ));
        }
        files.insert(file.id.clone(), (file.mime_type.clone(), bytes));
        materials.push(Material {
            id: file.id,
            kind,
            duration_ms,
        });
    }
    Ok((files, materials))
}

/// A private attachment copy, never through a link, at most `limit` bytes.
fn read_private(root: &std::path::Path, file: &Attachment, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    if !std::fs::symlink_metadata(root)
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
    if meta.len() > limit {
        return Err(refuse(
            attachments::TOO_LARGE,
            format!("{} is larger than {} MB", file.name, limit / 1024 / 1024),
        ));
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
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| attachments::UNREADABLE)?;
    if bytes.len() as u64 > limit {
        return Err(attachments::TOO_LARGE.into());
    }
    Ok(bytes)
}

/// The plan's files as data URLs: frames, else references by kind.
fn encode_materials(plan: &Plan, files: &MaterialFiles) -> (Inputs, Inputs) {
    let url = |id: &str| {
        files.get(id).map(|(mime, bytes)| {
            format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        })
    };
    let frames = [
        ("first_frame", plan.first_frame.as_deref()),
        ("last_frame", plan.last_frame.as_deref()),
    ]
    .into_iter()
    .filter_map(|(kind, id)| Some((kind, url(id?)?)))
    .collect();
    let references = plan
        .references
        .iter()
        .filter_map(|r| {
            let kind = match r.kind {
                materials::Kind::Image => "image_url",
                materials::Kind::Audio => "audio_url",
                materials::Kind::Video => "video_url",
            };
            Some((kind, url(&r.id)?))
        })
        .collect();
    (frames, references)
}

/// What the result card shows and a regeneration starts from.
fn video_params(request: &VideoRequest, plan: &Plan) -> Value {
    let mut params = serde_json::to_value(request).unwrap_or_else(|_| json!({}));
    params["references"] = json!(plan.references);
    params
}
