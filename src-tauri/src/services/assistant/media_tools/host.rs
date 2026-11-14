//! The generation tools' host side. A chat run already holds the
//! conversation's native lock, so these never take it: the runtime records
//! the tool call and its short result in native history, and the host keeps
//! each generation on the reply. Nothing here repeats a request whose outcome
//! is unknown; only a refused key is retried once with a new one.
use super::*;
use crate::services::assistant::{
    attachments::{self, Source},
    catalog::Model,
    documents::create::{
        DocumentFormat, PreviewRecord, LIMIT as DOCUMENT_LIMIT, MAX_PREVIEWS_PER_NAME,
        MAX_PREVIEWS_PER_RUN,
    },
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
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard,
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
    /// A regeneration the user started from a result card: its parameters
    /// and cost were confirmed there, without the chat model.
    direct: bool,
    images: AtomicUsize,
    speech: AtomicUsize,
    /// Documents made so far (see `documents::create`).
    documents: AtomicUsize,
    /// The ids of the `outputs/<id>` folders documents were made in; they
    /// are removed when the run ends.
    document_folders: Mutex<Vec<String>>,
    /// The documents made as previews, and the files delivered.
    previews: Mutex<Previews>,
    calls: Mutex<HashMap<String, CallRecord>>,
    /// Submitted videos, polled once the run ends: (reply, tool call,
    /// authority generation).
    polls: Mutex<Vec<(String, String, u64)>>,
}

/// A run's previews (see `documents::create`): each one asked for, in
/// order, the file names it delivered documents as, and when it last
/// delivered a document of each format.
#[derive(Default)]
struct Previews {
    asked: Vec<AskedPreview>,
    delivered: HashSet<String>,
    /// By format extension, the [`Self::clock`] of the run's latest
    /// delivery of that format.
    last_delivery: HashMap<&'static str, u64>,
    /// Counts the previews made and the documents delivered, in order.
    clock: u64,
}

/// A preview of the file `name` in the folder `doc_id`; `made` while it is
/// made and not delivered, `made_at` the [`Previews::clock`] when it was
/// first made.
struct AskedPreview {
    name: String,
    doc_id: String,
    made: Option<PreviewRecord>,
    made_at: Option<u64>,
}

/// Ordinary images one presentation reply may make while the deck's image
/// setting is off.
const UNPLANNED_DECK_IMAGES: usize = 2;

/// Whether an image call may go on in a presentation turn. With the deck's
/// images on, every picture goes through the plan's budget (`complete`: it
/// names the plan, revision and asset). With them off, a picture the user's
/// message asks for is still made, as an ordinary image the plan then places
/// (`user-image`): a small fixed number, since no budget covers it. `total`
/// is the reply's images so far plus this call's.
fn deck_image_gate(
    requires_plan: bool,
    complete: bool,
    planned: bool,
    images_on: bool,
    total: usize,
) -> Result<(), String> {
    if requires_plan && !planned && !images_on {
        return if total > UNPLANNED_DECK_IMAGES {
            Err(format!("assistant_presentation_image_budget: slide images are off; at most {UNPLANNED_DECK_IMAGES} pictures the user's message asks for can be made in one reply"))
        } else {
            Ok(())
        };
    }
    if (planned || requires_plan) && !complete {
        return Err("assistant_presentation_image_budget: freeze a page plan and explicitly authorize an image budget before image generation".into());
    }
    Ok(())
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[test]
    fn with_deck_images_off_an_asked_for_picture_is_made_but_only_a_couple() {
        // Outside a presentation turn nothing changes.
        assert!(deck_image_gate(false, false, false, false, 8).is_ok());
        // Images off: the picture the message asks for, without a plan.
        assert!(deck_image_gate(true, false, false, false, 1).is_ok());
        assert!(deck_image_gate(true, false, false, false, 2).is_ok());
        let refused = deck_image_gate(true, false, false, false, 3).unwrap_err();
        assert!(refused.contains("slide images are off"), "{refused}");
        // Images on: only through the plan and its budget.
        assert!(deck_image_gate(true, false, false, true, 1)
            .unwrap_err()
            .contains("freeze a page plan"));
        assert!(deck_image_gate(true, true, true, true, 1).is_ok());
        // A half-named plan is refused whatever the setting.
        assert!(deck_image_gate(true, false, true, false, 1).is_err());
        assert!(deck_image_gate(false, false, true, false, 1).is_err());
    }
}

impl MediaRun {
    pub fn new(defaults: MediaDefaults) -> Self {
        Self {
            defaults,
            direct: false,
            images: AtomicUsize::new(0),
            speech: AtomicUsize::new(0),
            documents: AtomicUsize::new(0),
            document_folders: Mutex::new(vec![]),
            previews: Mutex::default(),
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

    /// Hold one of the run's `limit` documents; `false` when none is left.
    pub fn reserve_document(&self, limit: usize) -> bool {
        self.documents
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |made| {
                (made < limit).then_some(made + 1)
            })
            .is_ok()
    }

    /// Remember the folder `outputs/<doc_id>` before it is made, so that the
    /// end of the run removes it however the document went.
    pub fn track_document_folder(&self, doc_id: &str) {
        self.document_folders
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(doc_id.to_owned());
    }

    pub fn take_document_folders(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .document_folders
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Give back a document that was not made.
    pub fn release_document(&self) {
        let _ = self
            .documents
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |made| {
                Some(made.saturating_sub(1))
            });
    }

    fn previews(&self) -> MutexGuard<'_, Previews> {
        self.previews
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Hold one of the [`MAX_PREVIEWS_PER_NAME`] previews of the file
    /// `name`, and one of the reply's [`MAX_PREVIEWS_PER_RUN`], for the
    /// folder `doc_id`; refused when none is left.
    pub fn record_preview(&self, name: &str, doc_id: &str) -> Result<(), String> {
        let mut previews = self.previews();
        if previews.asked.len() >= MAX_PREVIEWS_PER_RUN {
            return Err(refuse(
                DOCUMENT_LIMIT,
                format!(
                    "at most {MAX_PREVIEWS_PER_RUN} previews per reply; deliver a file with fromPreview, or send the content without preview"
                ),
            ));
        }
        let asked = previews.asked.iter().filter(|p| p.name == name).count();
        if asked >= MAX_PREVIEWS_PER_NAME {
            return Err(refuse(
                DOCUMENT_LIMIT,
                format!(
                    "at most {MAX_PREVIEWS_PER_NAME} previews of {name} per reply; deliver one with fromPreview, or send the content without preview"
                ),
            ));
        }
        previews.asked.push(AskedPreview {
            name: name.into(),
            doc_id: doc_id.into(),
            made: None,
            made_at: None,
        });
        Ok(())
    }

    /// Give back the preview held for the folder `doc_id`: it was not made.
    pub fn release_preview(&self, doc_id: &str) {
        self.previews().asked.retain(|p| p.doc_id != doc_id);
    }

    /// Keep `preview`, made in the folder held for it, until it is
    /// delivered; again when its delivery failed.
    pub fn keep_preview(&self, preview: PreviewRecord) {
        let mut previews = self.previews();
        let Previews { asked, clock, .. } = &mut *previews;
        if let Some(asked) = asked.iter_mut().find(|p| p.doc_id == preview.doc_id) {
            if asked.made_at.is_none() {
                *clock += 1;
                asked.made_at = Some(*clock);
            }
            asked.made = Some(preview);
        }
    }

    /// The preview in the folder `doc_id` this run made and has not
    /// delivered, taken for its delivery.
    pub fn take_preview(&self, doc_id: &str) -> Option<PreviewRecord> {
        self.previews()
            .asked
            .iter_mut()
            .find(|p| p.doc_id == doc_id)
            .and_then(|p| p.made.take())
    }

    /// Remember that the run delivered a document of `format` as the file
    /// `name`.
    pub fn record_delivery(&self, name: &str, format: DocumentFormat) {
        let mut previews = self.previews();
        previews.clock += 1;
        let now = previews.clock;
        previews.delivered.insert(name.into());
        previews.last_delivery.insert(format.extension(), now);
    }

    /// For each file the run previewed and delivered no document as, its
    /// latest preview, taken for its delivery; in the order they were asked
    /// for. A preview made before the run delivered a document of its
    /// format is left out: that document stands for it, whatever its name
    /// (the model renamed the file when it delivered it).
    pub fn undelivered_previews(&self) -> Vec<PreviewRecord> {
        let mut previews = self.previews();
        let Previews {
            asked,
            delivered,
            last_delivery,
            ..
        } = &mut *previews;
        let superseded = |preview: &AskedPreview| match (&preview.made, preview.made_at) {
            (Some(made), Some(at)) => last_delivery
                .get(made.format.extension())
                .is_some_and(|&delivered| delivered > at),
            _ => false,
        };
        let mut latest: Vec<PreviewRecord> = vec![];
        for preview in asked.iter_mut().rev() {
            if delivered.contains(&preview.name)
                || latest.iter().any(|p| p.name == preview.name)
                || superseded(preview)
            {
                continue;
            }
            latest.extend(preview.made.take());
        }
        latest.reverse();
        latest
    }

    /// What an earlier call with the same id answered (an unknown outcome
    /// while it still runs, a conflict when its arguments differ); `None`
    /// for a new call, which is then remembered as running.
    pub fn begin_call(&self, id: &str, fingerprint: &str) -> Option<Result<Value, String>> {
        let Ok(mut calls) = self.calls.lock() else {
            return Some(Err(STATE.into()));
        };
        if let Some((previous, result)) = calls.get(id) {
            if previous != fingerprint {
                return Some(Err("assistant_tool_call_conflict".into()));
            }
            return Some(
                result
                    .clone()
                    .unwrap_or_else(|| Err("unknown_outcome".into())),
            );
        }
        calls.insert(id.into(), (fingerprint.into(), None));
        None
    }

    /// Remember how the call `id` ended.
    pub fn end_call(
        &self,
        id: String,
        fingerprint: String,
        result: &Result<Value, String>,
    ) -> Result<(), String> {
        self.calls
            .lock()
            .map_err(|_| STATE)?
            .insert(id, (fingerprint, Some(result.clone())));
        Ok(())
    }
}

/// The fingerprint a repeated tool call must match: its name and arguments.
pub(in crate::services::assistant) fn fingerprint(call: &registry::Call) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&json!([call.name, call.args])).map_err(|_| PROTOCOL)?)
    ))
}

const AUDIO_REFERENCE_BYTES: u64 = 15 * 1024 * 1024;
const VIDEO_REFERENCE_BYTES: u64 = 25 * 1024 * 1024;

impl AssistantHost {
    /// The runtime's tool call of `run`, once its run, identity and call id
    /// are checked.
    pub(in crate::services::assistant) async fn tool_call(
        &self,
        run: &RunControl,
        params: &Value,
    ) -> Result<registry::Call, String> {
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
        Ok(call)
    }

    /// A generation tool call of a chat run, answered with a short result
    /// (ids and status, never bytes).
    pub(in crate::services::assistant) async fn dispatch_media_tool(
        self: &Arc<Self>,
        run: &Arc<RunControl>,
        params: &Value,
    ) -> Result<Value, String> {
        let call = self.tool_call(run, params).await?;
        let media = run
            .media
            .as_ref()
            .filter(|_| !run.compact_only)
            .ok_or("assistant_capability_disabled")?;
        let fingerprint = fingerprint(&call)?;
        if let Some(earlier) = media.begin_call(&call.tool_call_id, &fingerprint) {
            return earlier;
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
        media.end_call(call.tool_call_id, fingerprint, &result)?;
        result
    }

    async fn tool_image(
        &self,
        run: &Arc<RunControl>,
        media: &MediaRun,
        mid: &str,
        call_id: &str,
        mut args: ImageArgs,
    ) -> Result<Value, String> {
        let planned = args.presentation_asset_id.is_some()
            || args.plan_id.is_some()
            || args.revision.is_some();
        if planned && args.n.is_none() {
            // A planned asset is one image; generic composer batch defaults
            // must not silently multiply its budget or outputs.
            args.n = Some(1);
        }
        let requires_plan = run.presentation_enabled
            && (run.presentation_context
                || run.presentation_style_id.is_some()
                || run
                    .presentation
                    .lock()
                    .map_err(|_| STATE)?
                    .current()
                    .is_some());
        let images_on = run.presentation.lock().map_err(|_| STATE)?.images_on();
        deck_image_gate(
            requires_plan,
            args.presentation_asset_id.is_some()
                && args.plan_id.is_some()
                && args.revision.is_some(),
            planned,
            images_on,
            media.images.load(Ordering::SeqCst) + args.n.map_or(1, usize::from),
        )?;
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
        let prompt = if let Some(asset) = args.presentation_asset_id.as_deref() {
            format!(
                "{}\n{}",
                run.presentation
                    .lock()
                    .map_err(|_| STATE)?
                    .image_guidance(asset)
                    .chars()
                    .take(1000)
                    .collect::<String>(),
                prompt
            )
        } else {
            prompt
        };
        let prompt = scrub_secrets(&prompt, &run.secrets());
        if prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(refuse(INVALID,"Image prompt plus presentation style guidance exceeds the 4000-character limit; shorten the prompt"));
        }
        let count = request.count();
        let reservation = if planned {
            // A per-image price is held as it is; a token-priced model holds
            // the most this generation can cost.
            let quote = model.image.as_ref().and_then(|caps| {
                caps.per_image_price.clone().or_else(|| {
                    media::token_upper_bound(caps, &prompt, &request).map(|bound| {
                        bound
                            .round_dp_with_strategy(6, rust_decimal::RoundingStrategy::AwayFromZero)
                            .normalize()
                            .to_string()
                    })
                })
            });
            let token = run.presentation.lock().map_err(|_| STATE)?.reserve_image(
                args.plan_id.as_deref().ok_or(INVALID)?,
                args.revision.ok_or(INVALID)?,
                args.presentation_asset_id.as_deref().ok_or(INVALID)?,
                count,
                quote.as_deref(),
            )?;
            Some(token)
        } else {
            None
        };
        if let Err(error) = media.reserve_images(count) {
            if let Some(token) = &reservation {
                run.presentation
                    .lock()
                    .map_err(|_| STATE)?
                    .release_image(token, true);
            }
            return Err(error);
        }
        let outcome = self
            .make_image(
                run,
                (mid, call_id),
                &model,
                &prompt,
                &request,
                planned.then_some(&args),
            )
            .await;
        if outcome.as_ref().is_err_and(|(_, unbilled)| *unbilled) {
            media.release_images(count);
            if let Some(token) = &reservation {
                run.presentation
                    .lock()
                    .map_err(|_| STATE)?
                    .release_image(token, true);
            }
        }
        let mut outcome = outcome.map_err(|(error, _)| error)?;
        if let Some(token) = &reservation {
            let aid = outcome["files"]
                .as_array()
                .filter(|files| files.len() == 1)
                .and_then(|files| files[0]["id"].as_str())
                .ok_or("assistant_presentation_asset_invalid")?;
            let asset = args.presentation_asset_id.as_deref().ok_or(INVALID)?;
            self.bind_presentation_image(run, token, asset, aid).await?;
            outcome["presentationAssetId"] = json!(asset);
            outcome["planId"] = json!(args.plan_id);
            outcome["revision"] = json!(args.revision);
        }
        Ok(outcome)
    }

    /// Generate and save the images of one call: `Err((code, unbilled))`.
    async fn make_image(
        &self,
        run: &Arc<RunControl>,
        origin: (&str, &str),
        model: &Model,
        prompt: &str,
        request: &ImageRequest,
        presentation: Option<&ImageArgs>,
    ) -> Result<Value, (String, bool)> {
        let (mid, call_id) = origin;
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
        if let Some(plan) = presentation {
            record_media.params["presentationPlanId"] = json!(plan.plan_id);
            record_media.params["presentationRevision"] = json!(plan.revision);
            record_media.params["presentationAssetId"] = json!(plan.presentation_asset_id);
        }
        self.save_generation(run, mid, &record_media)
            .await
            .map_err(unsent)?;
        let gateway = self.env.gateway_base(&run.identity.apex).map_err(unsent)?;
        let client = crate::proxy::http_client::assistant_media_client().map_err(unsent)?;
        let mut key = run.ofox_key().map_err(unsent)?;
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
        if !media.direct
            && !self
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
        let mut key = run.ofox_key()?;
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
        media
            .speech
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |made| {
                (made < MAX_SPEECH_PER_RUN).then_some(made + 1)
            })
            .map_err(|_| {
                refuse(
                    SPEECH_LIMIT,
                    format!("at most {MAX_SPEECH_PER_RUN} speech files per reply"),
                )
            })?;
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
        let key = run.ofox_key()?;
        let mut result = speech::http::synthesize(&client, &gateway, &key, &model, &request).await;
        if result
            .as_ref()
            .is_err_and(|e| e.code == "assistant_key_rejected")
        {
            let key = self.key_for_retry(run, &json!({ "apiKey": key })).await?;
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

    /// Make a result card's generation again with changed parameters,
    /// without the chat model. A video first answers with its estimate; it is
    /// submitted once the request carries that estimate as confirmed. The new
    /// result is added to the same reply.
    pub async fn regenerate_media(
        self: &Arc<Self>,
        request: RegenerateRequest,
    ) -> Result<Value, String> {
        use crate::services::assistant::{
            host::IDENTITY_CHANGED,
            store,
            types::{id, Operation, OperationStatus, RunRef, SendRequest},
        };
        for value in [
            &request.session_id,
            &request.operation_id,
            &request.message_id,
        ] {
            validate_id(value)?;
        }
        self.ensure_accepting_runs()?;
        let identity = self.identity().await?;
        let generation = request.expected_generation;
        if identity.owner != request.expected_owner || self.generation() != generation {
            return Err(IDENTITY_CHANGED.into());
        }
        self.ensure_idle(&request.session_id)?;
        let record = blocking({
            let store = self.store(&identity.owner)?;
            let sid = request.session_id.clone();
            move || store.load_record(&sid)
        })
        .await?;
        if record.session.deleted_at.is_some() {
            return Err("assistant_session_deleted".into());
        }
        store::ensure_new_operation(&record, &request.operation_id)?;
        let source = record
            .messages
            .iter()
            .find(|m| m.id == request.message_id)
            .and_then(|m| m.media_for(request.tool_call_id.as_deref()))
            .cloned()
            .ok_or(INVALID)?;
        let args = regeneration_args(&source, &request)?;
        if let Regeneration::Video(args) = &args {
            let catalog = self.models(&identity.apex).await?;
            let model = pick_model(&catalog.models, Kind::Video, args.model.as_deref(), None)?;
            let materials = {
                let record = record.clone();
                let root = self
                    .store(&identity.owner)?
                    .attachment_root(&request.session_id)?;
                let ids = args.material_ids.clone();
                blocking(move || read_materials(&record, root, &ids))
                    .await?
                    .1
            };
            let plan = materials::plan(&materials, args.material_use)
                .map_err(|code| refuse(&code, material_hint(&code)))?;
            let (_, video) = video_request(model, None, args, &plan)?;
            let estimate = self
                .env
                .provider_prices(&identity.apex, &model.id)
                .await
                .ok()
                .and_then(|data| video::estimate(&data, &video))
                .map(|n| n.normalize().to_string());
            if request.confirmed_cost_usd != estimate
                || estimate.is_none() && !request.confirm_unknown_cost
            {
                return Ok(json!({"needsConfirmation": true, "estimatedCostUsd": estimate}));
            }
        }
        let key = self
            .env
            .api_key(&identity, None, &|| self.admit(generation))
            .await?;
        self.ensure_current(&identity, generation).await?;
        self.remember_key(&key);
        let run_id = id();
        let send: SendRequest = serde_json::from_value(json!({
            "sessionId": request.session_id, "runId": run_id,
            "operationId": request.operation_id, "text": "",
        }))
        .map_err(|_| INVALID)?;
        let mut control = RunControl::new(identity.clone(), &send, key);
        control.message_id = request.message_id.clone();
        control.media = Some(MediaRun {
            direct: true,
            ..MediaRun::new(record.session.media.clone())
        });
        let control = Arc::new(control);
        blocking({
            let host = self.clone();
            let store = self.store(&identity.owner)?;
            let control = control.clone();
            move || {
                host.register_run(control.clone(), generation, || {
                    store.mutate(&control.session_id, |record| {
                        if record.session.deleted_at.is_some() {
                            return Err("assistant_session_deleted".into());
                        }
                        if record.session.active_run.is_some() {
                            return Err("assistant_session_busy".into());
                        }
                        store::ensure_new_operation(record, &control.operation_id)?;
                        record.operations.push(Operation {
                            id: control.operation_id.clone(),
                            run_id: control.run_id.clone(),
                            status: OperationStatus::Running,
                            media: None,
                            search_price: None,
                        });
                        record.session.active_run = Some(RunRef {
                            run_id: control.run_id.clone(),
                            operation_id: control.operation_id.clone(),
                        });
                        Ok(())
                    })
                })
            }
        })
        .await?;
        let call_id = format!("regen-{}", id());
        let host = self.clone();
        let done = control.clone();
        let call = call_id.clone();
        self.track(format!("regenerate:{}", control.run_id), async move {
            let media = done.media.as_ref().expect("direct media run");
            let mid = done.message_id.clone();
            let outcome = match args {
                Regeneration::Image(args) => host.tool_image(&done, media, &mid, &call, args).await,
                Regeneration::Video(args) => host.tool_video(&done, media, &mid, &call, args).await,
                Regeneration::Speech(args) => {
                    host.tool_speech(&done, media, &mid, &call, args).await
                }
            };
            if outcome.is_err() {
                let _ = host
                    .settle_generation(&done, &mid, &call, "unknown", None)
                    .await;
            }
            host.end_regeneration(&done, outcome.is_ok()).await;
        });
        Ok(json!({"status": "started", "toolCallId": call_id}))
    }

    /// Settle a regeneration's operation, recount the conversation and
    /// publish the reply; then poll a video it submitted.
    async fn end_regeneration(self: &Arc<Self>, run: &Arc<RunControl>, completed: bool) {
        use crate::services::assistant::{
            host::{load_native, session_key},
            projection::recount_usage,
            types::OperationStatus,
        };
        let saved = async {
            let _history = self
                .native_lock(&session_key(&run.identity.owner, &run.session_id))?
                .lock_owned()
                .await;
            let store = self.store(&run.identity.owner)?;
            let file = {
                let store = store.clone();
                let sid = run.session_id.clone();
                blocking(move || store.native_history_path(&sid)).await?
            };
            let entries = if file.is_file() {
                load_native(&self.runtime()?, &file).await?
            } else {
                json!([])
            };
            let run = run.clone();
            blocking(move || {
                store.mutate(&run.session_id, |record| {
                    if let Some(op) = record
                        .operations
                        .iter_mut()
                        .find(|op| op.id == run.operation_id)
                    {
                        op.status = if completed {
                            OperationStatus::Completed
                        } else {
                            OperationStatus::Failed
                        };
                    }
                    if record
                        .session
                        .active_run
                        .as_ref()
                        .is_some_and(|active| active.run_id == run.run_id)
                    {
                        record.session.active_run = None;
                    }
                    recount_usage(record, entries.as_array().ok_or(PROTOCOL)?);
                    record.session.updated_at = now();
                    Ok(())
                })?;
                store.load_detail(&run.session_id)
            })
            .await
        }
        .await;
        self.release_run(run);
        match saved {
            Ok(detail) => {
                if let Some(message) = detail.messages.iter().find(|m| m.id == run.message_id) {
                    self.emit(run, "message", json!({ "message": message }));
                }
                self.emit(
                    run,
                    "session",
                    json!({ "session": detail.session, "usage": detail.usage }),
                );
            }
            Err(error) => log::warn!("[Assistant] could not save a regeneration: {error}"),
        }
        self.start_media_polls(run);
    }

    pub(in crate::services::assistant) async fn load(
        &self,
        run: &RunControl,
    ) -> Result<SessionRecord, String> {
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

    pub(in crate::services::assistant) fn publish_generation(
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

    /// One video at a time per conversation and three per account, as for
    /// direct video turns.
    async fn check_video_slots(&self, run: &RunControl) -> Result<(), String> {
        let store = self.store(&run.identity.owner)?;
        let sid = run.session_id.clone();
        blocking(move || {
            let mut sessions = store.list(false, "")?.sessions;
            sessions.extend(store.list(true, "")?.sessions);
            let mut count = 0;
            for session in sessions {
                let jobs = video::occupied_slots(&store.load_record(&session.id)?);
                if session.id == sid && jobs > 0 {
                    return Err(refuse(
                        "assistant_media_video_limit",
                        "a video of this conversation is still being generated; wait for it to finish",
                    ));
                }
                count += jobs;
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
pub(in crate::services::assistant) fn still_running(
    record: &SessionRecord,
    run: &RunControl,
) -> Result<(), String> {
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

/// A request to make a result card's generation again.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegenerateRequest {
    pub session_id: String,
    pub expected_owner: String,
    pub expected_generation: u64,
    pub operation_id: String,
    pub message_id: String,
    /// The generation to start from; `None` for a direct media turn's own.
    #[serde(default)]
    pub tool_call_id: Option<String>,
    pub prompt: String,
    pub model: String,
    /// What changes: image `size`, `quality`, `n`; video `duration`,
    /// `resolution`, `aspectRatio`, `generateAudio`, `materialUse`; speech
    /// `voice`, `format`, `language`, `style`.
    #[serde(default)]
    pub params: Value,
    /// For a video: the estimate the user confirmed.
    #[serde(default)]
    pub confirmed_cost_usd: Option<String>,
    /// For a video without an estimate: the user accepted an unknown cost.
    #[serde(default)]
    pub confirm_unknown_cost: bool,
}

enum Regeneration {
    Image(ImageArgs),
    Video(VideoArgs),
    Speech(SpeechArgs),
}

/// The tool arguments that make `source` again with the request's changes,
/// from the same files.
fn regeneration_args(
    source: &MediaRecord,
    request: &RegenerateRequest,
) -> Result<Regeneration, String> {
    let text = |key: &str| request.params[key].as_str().map(str::to_owned);
    let model = Some(request.model.clone());
    Ok(match source.kind.as_str() {
        "image" => Regeneration::Image(ImageArgs {
            presentation_asset_id: None,
            plan_id: None,
            revision: None,
            prompt: request.prompt.clone(),
            model,
            size: text("size"),
            quality: text("quality"),
            n: request.params["n"]
                .as_u64()
                .and_then(|n| u8::try_from(n).ok()),
            image_ids: source.input_ids.iter().take(1).cloned().collect(),
        }),
        "video" => {
            let material_use = match request.params.get("materialUse") {
                Some(value) => serde_json::from_value(value.clone()).map_err(|_| INVALID)?,
                None if source.params["firstFrameId"].is_string() => materials::Use::Frames,
                None if source.params["references"]
                    .as_array()
                    .is_some_and(|r| !r.is_empty()) =>
                {
                    materials::Use::Reference
                }
                None => materials::Use::Auto,
            };
            Regeneration::Video(VideoArgs {
                prompt: request.prompt.clone(),
                model,
                duration: request.params["duration"].as_u64(),
                resolution: text("resolution"),
                aspect_ratio: text("aspectRatio"),
                generate_audio: request.params["generateAudio"].as_bool(),
                material_ids: source.input_ids.clone(),
                material_use,
            })
        }
        "speech" => Regeneration::Speech(SpeechArgs {
            text: request.prompt.clone(),
            model,
            voice: text("voice"),
            format: text("format"),
            language: text("language"),
            style: text("style"),
        }),
        _ => return Err(INVALID.into()),
    })
}
