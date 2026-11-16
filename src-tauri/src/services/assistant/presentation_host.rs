//! Trusted planning and material resolution. Model-visible results contain IDs,
//! never paths or image bytes. Only the runtime's private hydration call reads bytes.
use super::{
    attachments,
    host::{blocking, AssistantHost, RunControl, STATE},
    media_tools::host::still_running,
    presentation::AssetBinding,
    types::{validate_id, Attachment},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::Read, sync::Arc};

const INVALID: &str = "assistant_presentation_asset_invalid";

fn read_asset(
    root: &std::path::Path,
    file: &Attachment,
) -> Result<(Vec<u8>, AssetBinding), String> {
    if !matches!(file.mime_type.as_str(), "image/png" | "image/jpeg") || file.status == "missing" {
        return Err(INVALID.into());
    }
    if !std::fs::symlink_metadata(root).is_ok_and(|m| m.is_dir()) {
        return Err(INVALID.into());
    }
    let (handle, size) = attachments::open_written(&root.join(&file.id), 10 * 1024 * 1024)?;
    if size == 0 || size != file.size_bytes {
        return Err(INVALID.into());
    }
    let mut bytes = vec![];
    handle
        .take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| INVALID)?;
    if bytes.len() as u64 != size {
        return Err(INVALID.into());
    }
    let (width, height) = image_dimensions(&bytes, &file.mime_type).ok_or(INVALID)?;
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 40_000_000
    {
        return Err(INVALID.into());
    }
    let binding = AssetBinding {
        asset_id: file.id.clone(),
        attachment_id: file.id.clone(),
        mime_type: file.mime_type.clone(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        size_bytes: size,
        width: Some(width),
        height: Some(height),
    };
    Ok((bytes, binding))
}
fn image_dimensions(bytes: &[u8], mime: &str) -> Option<(u32, u32)> {
    if mime == "image/png"
        && bytes.len() >= 33
        && bytes.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR")
    {
        return Some((
            u32::from_be_bytes(bytes[16..20].try_into().ok()?),
            u32::from_be_bytes(bytes[20..24].try_into().ok()?),
        ));
    }
    if mime != "image/jpeg" || !bytes.starts_with(&[255, 216, 255]) || !bytes.ends_with(&[255, 217])
    {
        return None;
    }
    let mut at = 2usize;
    while at + 4 <= bytes.len() {
        if bytes[at] != 255 {
            return None;
        }
        let marker = bytes[at + 1];
        at += 2;
        if marker == 255 {
            at -= 1;
            continue;
        }
        if marker == 218 || marker == 217 {
            return None;
        }
        let size = usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]));
        if size < 2 || at.checked_add(size)? > bytes.len() {
            return None;
        }
        if matches!(marker,192..=195|197..=199|201..=203|205..=207) {
            if size < 8 {
                return None;
            }
            return Some((
                u32::from(u16::from_be_bytes([bytes[at + 5], bytes[at + 6]])),
                u32::from(u16::from_be_bytes([bytes[at + 3], bytes[at + 4]])),
            ));
        }
        at += size;
    }
    None
}
impl AssistantHost {
    pub(super) async fn dispatch_presentation_tool(
        self: &Arc<Self>,
        run: &Arc<RunControl>,
        params: &Value,
    ) -> Result<Value, String> {
        let call = self.tool_call(run, params).await?;
        if !run.presentation_enabled {
            return Err("assistant_capability_disabled".into());
        }
        match call.name.as_str() {
            "freeze_presentation_plan" => {
                let source = self
                    .prepare_presentation_source(run, call.args.get("source"))
                    .await?;
                let inherited = source
                    .as_ref()
                    .and_then(|source| source.style_id.as_deref());
                let manual = run
                    .presentation_style_id
                    .as_deref()
                    .filter(|style| *style != "auto")
                    .or(inherited);
                let (plan, visual_budget) = {
                    let mut state = run.presentation.lock().map_err(|_| STATE)?;
                    state.check_unfunded_images(&call.args)?;
                    let plan = state.freeze(&call.args, manual, self.python_offered())?;
                    (plan, state.visual_budget_snapshot())
                };
                *run.presentation_source.lock().map_err(|_| STATE)? = source;
                let mut shown = serde_json::to_value(&plan).map_err(|_| STATE)?;
                shown["styleId"] = json!(plan.effective_style);
                shown["status"] = json!("frozen");
                shown["visualBudget"] = visual_budget;
                // Persist the explicit design summary, not internal reasoning.
                let store = self.store(&run.identity.owner)?;
                let (r, value) = (run.clone(), shown.clone());
                let mid = run.reply_id();
                let target = mid.clone();
                blocking(move || {
                    store.mutate(&r.session_id, |record| {
                        still_running(record, &r)?;
                        let message = record
                            .messages
                            .iter_mut()
                            .find(|m| m.id == target)
                            .ok_or(STATE)?;
                        message.presentation_plan = Some(value);
                        Ok(())
                    })
                })
                .await?;
                self.validate_run(run)?;
                self.emit(
                    run,
                    "presentation_plan",
                    json!({"messageId":mid,"plan":shown}),
                );
                Ok(shown)
            }
            "resolve_presentation_assets" => {
                let plan = call.args["planId"].as_str().ok_or(INVALID)?;
                let revision = call.args["revision"].as_u64().ok_or(INVALID)?;
                let ids: Vec<String> =
                    serde_json::from_value(call.args["assetIds"].clone()).map_err(|_| INVALID)?;
                let result = self
                    .presentation_assets(run, plan, revision, &ids, 1024 * 1024)
                    .await?;
                Ok(result)
            }
            _ => Err("assistant_invalid_tool".into()),
        }
    }
    /// Files are resolved anew, checked against the immutable plan and hash,
    /// and re-fenced after IO. The cap differs for IPC and a private Python file.
    pub(super) async fn presentation_assets(
        &self,
        run: &Arc<RunControl>,
        plan: &str,
        revision: u64,
        ids: &[String],
        max_bytes: u64,
    ) -> Result<Value, String> {
        if ids.len() > super::presentation::MAX_ASSETS
            || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
        {
            return Err(INVALID.into());
        }
        let (required, mut bindings) = {
            let state = run.presentation.lock().map_err(|_| STATE)?;
            state.validate_document_plan(plan, revision)?;
            (state.required_assets(), state.assets())
        };
        let record = self.load(run).await?;
        let root = self
            .store(&run.identity.owner)?
            .attachment_root(&run.session_id)?;
        let mut result = vec![];
        let mut total = 0u64;
        for id in ids {
            let required = required
                .iter()
                .find(|item| &item.asset_id == id)
                .ok_or(INVALID)?;
            let existing = bindings.iter().find(|item| &item.asset_id == id).cloned();
            let aid = match &existing {
                Some(binding) => binding.attachment_id.clone(),
                None if required.kind == "user-image" => {
                    validate_id(id)?;
                    id.clone()
                }
                None => return Err("assistant_presentation_asset_missing".into()),
            };
            let file = record
                .attachments
                .iter()
                .find(|file| file.id == aid)
                .ok_or(INVALID)?
                .clone();
            let root = root.clone();
            let (bytes, mut found) = blocking(move || read_asset(&root, &file)).await?;
            found.asset_id = id.clone();
            if existing.as_ref().is_some_and(|binding| binding != &found) {
                return Err(INVALID.into());
            }
            total = total.checked_add(bytes.len() as u64).ok_or(INVALID)?;
            if total > max_bytes {
                return Err("assistant_presentation_asset_limit".into());
            }
            if existing.is_none() {
                let mut state = run.presentation.lock().map_err(|_| STATE)?;
                state.validate_document_plan(plan, revision)?;
                state.bind_user_asset(found.clone())?;
                bindings.push(found.clone());
            }
            result.push(json!({"assetId":id,"mimeType":found.mime_type,"sha256":found.sha256,"dataBase64":STANDARD.encode(bytes)}));
        }
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err(super::host::IDENTITY_CHANGED.into());
        }
        Ok(json!({"planId":plan,"revision":revision,"assets":result}))
    }
    pub(super) async fn bind_presentation_image(
        &self,
        run: &Arc<RunControl>,
        token: &str,
        asset_id: &str,
        attachment_id: &str,
    ) -> Result<(), String> {
        let record = self.load(run).await?;
        let file = record
            .attachments
            .into_iter()
            .find(|file| file.id == attachment_id)
            .ok_or(INVALID)?;
        let root = self
            .store(&run.identity.owner)?
            .attachment_root(&run.session_id)?;
        let (_, mut binding) = blocking(move || read_asset(&root, &file)).await?;
        binding.asset_id = asset_id.into();
        self.validate_run(run)?;
        run.presentation
            .lock()
            .map_err(|_| STATE)?
            .bind_asset(token, binding)?;
        Ok(())
    }
}
