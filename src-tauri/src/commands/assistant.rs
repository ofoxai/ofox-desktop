//! Built-in assistant commands (chat, S2 attachments). Each one finds the host and calls
//! its request of the same name (`services::assistant::requests`), so tests
//! drive the same code. [`AssistantEnvironment`] lends the host the app's
//! sign-in, the assistant key, the model catalog, files and event channel.
use crate::ofox_api_keys::{self, ApiKeyError, AssistantKeyStatus, FetchMode};
use crate::ofox_auth::{OfoxAuthManager, OfoxAuthStatus};
use crate::services::assistant::{
    attachments, audio, audio_input, catalog,
    host::{blocking, Admit, BoxFuture, Environment, Identity, KEY_REJECTED, SIGN_IN_REQUIRED},
    speech,
    store::{self, BackupReport, RestoreReport, SessionList},
    types::{
        Attachment, CompactRequest, CreateRequest, DeleteRequest, Detail, EmptyTrashReport,
        EmptyTrashRequest, Event, ExportMarkdownRequest, ForkRequest, ImportDropRequest,
        Preferences, RunProxy, RunRef, SendRequest, Session, SetPreferencesRequest, SteerRequest,
        SwitchVersionRequest, TransferRequest, UpdateRequest,
    },
    AssistantHost,
};
use crate::settings::KeyOwner;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::http::HeaderMap;
use tauri::ipc::{InvokeBody, Request};
use tauri::{DragDropEvent, Emitter, Manager, State};
use tokio::sync::RwLock;

/// The renderer's event channel for runs.
pub const ASSISTANT_EVENT: &str = "assistant:event";
/// A file drag entered or left the window: `{state: "enter" | "leave"}`.
pub const ASSISTANT_DRAG_EVENT: &str = "assistant:drag";
/// Files were dropped on the window: `{token, count}`, never their paths.
pub const ASSISTANT_DROP_EVENT: &str = "assistant:drop";

type Host<'a> = State<'a, Arc<AssistantHost>>;

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_bootstrap(
    host: Host<'_>,
    refresh_catalog: Option<bool>,
) -> Result<Value, String> {
    host.bootstrap_cached(refresh_catalog.unwrap_or(false))
        .await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_transcribe_audio(
    host: Host<'_>,
    request: audio::TranscribeRequest,
) -> Result<audio::ResultRecord, String> {
    host.transcribe_audio(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_synthesize_audio(
    host: Host<'_>,
    request: speech::SpeechRequest,
) -> Result<audio::ResultRecord, String> {
    host.synthesize_audio(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_list_sessions(
    host: Host<'_>,
    deleted: bool,
    search: String,
) -> Result<SessionList, String> {
    host.list_sessions(deleted, &search).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_usage_summary(host: Host<'_>) -> Result<Value, String> {
    host.usage_summary().await
}

/// The assistant settings: custom instructions and the two switches.
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_get_preferences(host: Host<'_>) -> Result<Preferences, String> {
    host.get_preferences().await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_set_preferences(
    host: Host<'_>,
    request: SetPreferencesRequest,
) -> Result<Preferences, String> {
    host.set_preferences(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_create_session(
    host: Host<'_>,
    request: CreateRequest,
) -> Result<Session, String> {
    host.create_session(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_get_session(host: Host<'_>, session_id: String) -> Result<Detail, String> {
    host.get_session(&session_id).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_update_session(
    host: Host<'_>,
    request: UpdateRequest,
) -> Result<Session, String> {
    host.update_session(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_delete_session(
    host: Host<'_>,
    request: DeleteRequest,
) -> Result<(), String> {
    host.delete_session(request).await
}

/// Permanently delete every conversation in the trash.
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_empty_trash(
    host: Host<'_>,
    request: EmptyTrashRequest,
) -> Result<EmptyTrashReport, String> {
    host.empty_trash(request).await
}

/// `None` when the save dialog was cancelled.
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_backup(
    app: tauri::AppHandle,
    host: Host<'_>,
    request: TransferRequest,
) -> Result<Option<BackupReport>, String> {
    use tauri_plugin_dialog::DialogExt;
    host.backup(request, move || {
        app.dialog()
            .file()
            .set_file_name("ofox-assistant-backup.zip")
            .add_filter("ZIP", &["zip"])
            .blocking_save_file()
            .map(|file| {
                file.into_path()
                    .map_err(|_| "assistant_backup_failed".to_string())
            })
            .transpose()
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_restore(
    app: tauri::AppHandle,
    host: Host<'_>,
    request: TransferRequest,
) -> Result<Option<RestoreReport>, String> {
    use tauri_plugin_dialog::DialogExt;
    host.restore(request, move || {
        app.dialog()
            .file()
            .add_filter("ZIP", &["zip"])
            .blocking_pick_file()
            .map(|file| {
                file.into_path()
                    .map_err(|_| "assistant_backup_failed".to_string())
            })
            .transpose()
    })
    .await
}

/// Export only after the native save dialog confirms a destination.
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_export_markdown(
    app: tauri::AppHandle,
    host: Host<'_>,
    request: ExportMarkdownRequest,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    host.export_markdown(request, move || {
        app.dialog()
            .file()
            .set_file_name("ofox-conversation.md")
            .add_filter("Markdown", &["md"])
            .blocking_save_file()
            .map(|file| {
                file.into_path()
                    .map_err(|_| "assistant_export_destination_invalid".to_string())
            })
            .transpose()
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_send(host: Host<'_>, request: SendRequest) -> Result<RunRef, String> {
    host.send(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_steer(host: Host<'_>, request: SteerRequest) -> Result<(), String> {
    host.steer(request).await
}

/// Show another version of a turn; returns the conversation as it shows now.
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_switch_version(
    host: Host<'_>,
    request: SwitchVersionRequest,
) -> Result<Detail, String> {
    host.switch_version(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_fork(host: Host<'_>, request: ForkRequest) -> Result<Detail, String> {
    host.fork_session(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_compact(host: Host<'_>, request: CompactRequest) -> Result<RunRef, String> {
    host.compact_session(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_stop(
    host: Host<'_>,
    session_id: String,
    run_id: String,
    operation_id: String,
) -> Result<(), String> {
    host.stop(&session_id, &run_id, &operation_id).await
}

/// Pick files with the native dialog and import them into the conversation.
/// A cancelled dialog returns no attachments.
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_pick_attachments(
    app: tauri::AppHandle,
    host: Host<'_>,
    session_id: String,
) -> Result<Vec<Attachment>, String> {
    host.pick_attachments(&session_id, move || pick_files(&app))
        .await
}

/// The native file dialog, offering only the formats the runtime reads.
/// Blocking: it waits for the user.
fn pick_files(app: &tauri::AppHandle) -> Result<Vec<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .add_filter(attachments::FILTER_NAME, attachments::EXTENSIONS)
        .blocking_pick_files()
        .unwrap_or_default()
        .into_iter()
        .map(|file| {
            file.into_path()
                .map_err(|_| attachments::UNREADABLE.to_string())
        })
        .collect()
}

/// Import the files the user last dropped on the window, named by the token
/// of the `assistant:drop` event. The host keeps the paths; the renderer
/// names none.
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_import_drop(
    host: Host<'_>,
    request: ImportDropRequest,
) -> Result<Vec<Attachment>, String> {
    host.import_dropped(request).await
}

/// Import one file pasted into the page. The body is the file's raw bytes;
/// the rest travels in headers, as a raw body leaves no room for arguments:
/// `x-session-id`, `x-expected-owner`, `x-expected-generation` and the
/// percent-encoded `x-file-name` (a hint only, see `attachments::classify`).
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_import_pasted(
    host: Host<'_>,
    request: Request<'_>,
) -> Result<Vec<Attachment>, String> {
    let (authority, name, bytes) = parse_paste(request.headers(), request.body())?;
    host.import_pasted(authority, name, bytes).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_pick_audio(
    app: tauri::AppHandle,
    host: Host<'_>,
    request: DeleteRequest,
) -> Result<Option<audio_input::Preview>, String> {
    host.pick_audio(request, move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog()
            .file()
            .add_filter("WAV, MP3", &["wav", "mp3"])
            .blocking_pick_file()
            .map(|file| {
                file.into_path()
                    .map_err(|_| attachments::UNREADABLE.to_string())
            })
            .transpose()
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_import_audio(
    host: Host<'_>,
    request: Request<'_>,
) -> Result<audio_input::Preview, String> {
    let (authority, name, bytes) = parse_paste(request.headers(), request.body())?;
    host.import_audio(authority, name, bytes).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_discard_audio(
    host: Host<'_>,
    request: audio_input::DiscardRequest,
) -> Result<bool, String> {
    host.discard_audio(request).await
}

const PASTE_SESSION_HEADER: &str = "x-session-id";
const PASTE_OWNER_HEADER: &str = "x-expected-owner";
const PASTE_GENERATION_HEADER: &str = "x-expected-generation";
const PASTE_NAME_HEADER: &str = "x-file-name";

/// The authority, name hint and bytes of a paste request. Anything but a raw
/// body with the headers is unreadable; a body over the file limit is refused
/// before it is copied (Tauri has already received it, so this is the
/// earliest point to stop it).
fn parse_paste(
    headers: &HeaderMap,
    body: &InvokeBody,
) -> Result<(DeleteRequest, String, Vec<u8>), String> {
    let InvokeBody::Raw(bytes) = body else {
        return Err(attachments::UNREADABLE.into());
    };
    if bytes.len() as u64 > attachments::MAX_FILE_BYTES {
        return Err(attachments::TOO_LARGE.into());
    }
    let header = |name: &str| -> Result<&str, String> {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| attachments::UNREADABLE.to_string())
    };
    let authority = DeleteRequest {
        session_id: header(PASTE_SESSION_HEADER)?.to_owned(),
        expected_owner: header(PASTE_OWNER_HEADER)?.to_owned(),
        expected_generation: header(PASTE_GENERATION_HEADER)?
            .parse()
            .map_err(|_| attachments::UNREADABLE.to_string())?,
    };
    // No name at all is fine: one is made from what the file is.
    let name = headers
        .get(PASTE_NAME_HEADER)
        .map(|value| {
            percent_encoding::percent_decode(value.as_bytes())
                .decode_utf8_lossy()
                .into_owned()
        })
        .unwrap_or_default();
    Ok((authority, name, bytes.clone()))
}

/// What the renderer is told about a file drag: `Enter` and `Leave` only.
/// `Over` repeats with every mouse move and `Drop` is announced by its token.
fn drag_state(event: &DragDropEvent) -> Option<&'static str> {
    match event {
        DragDropEvent::Enter { .. } => Some("enter"),
        DragDropEvent::Leave => Some("leave"),
        _ => None,
    }
}

/// The window's file drag and drop. The renderer is told that a drag entered
/// or left, and for a drop a single-use token and the number of files, never
/// a path (see [`AssistantHost::accept_drop`]). A drop ends the drag, so it
/// is announced as a leave first. Nothing is logged but a failure's code.
pub fn handle_assistant_drag_drop(app: &tauri::AppHandle, event: &DragDropEvent) {
    if let Some(state) = drag_state(event) {
        let _ = app.emit(ASSISTANT_DRAG_EVENT, serde_json::json!({ "state": state }));
    }
    let DragDropEvent::Drop { paths, .. } = event else {
        return;
    };
    let _ = app.emit(
        ASSISTANT_DRAG_EVENT,
        serde_json::json!({ "state": "leave" }),
    );
    let Some(host) = app.try_state::<Arc<AssistantHost>>() else {
        return;
    };
    let (host, app, paths) = (host.inner().clone(), app.clone(), paths.clone());
    tauri::async_runtime::spawn(async move {
        let folder = paths.len() == 1 && paths[0].is_dir();
        match host.accept_drop(paths).await {
            Ok(Some((token, count))) => {
                let _ = app.emit(
                    ASSISTANT_DROP_EVENT,
                    serde_json::json!({ "token": token, "count": count, "folder": folder }),
                );
            }
            Ok(None) => {}
            Err(code) => log::info!("[Assistant] a drop was not accepted: {code}"),
        }
    });
}

/// The host's view of the running app.
pub struct AssistantEnvironment {
    app: tauri::AppHandle,
}

type AuthManager = Arc<RwLock<OfoxAuthManager>>;

impl AssistantEnvironment {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    fn auth(&self) -> Result<AuthManager, String> {
        self.app
            .try_state::<crate::commands::ofox_auth::OfoxAuthState>()
            .map(|state| state.0.clone())
            .ok_or_else(|| SIGN_IN_REQUIRED.to_string())
    }
}

/// The signed-in account and region as the host's [`Identity`].
fn identity_from(status: OfoxAuthStatus, apex: &str) -> Result<Identity, String> {
    if status.state != crate::ofox_auth::OfoxAuthState::Active {
        return Err(SIGN_IN_REQUIRED.into());
    }
    let user = status.user.ok_or("assistant_identity_unavailable")?;
    Ok(Identity {
        owner: store::owner_key(&user.key_owner())?,
        apex: apex.into(),
    })
}

/// The signed-in account, and only if it is `identity`'s: a key is never
/// fetched for a signed-out state or for another account.
async fn key_owner(manager: &AuthManager, identity: &Identity) -> Result<KeyOwner, String> {
    let owner = manager
        .read()
        .await
        .current_key_owner()
        .await
        .ok_or(SIGN_IN_REQUIRED)?;
    if store::owner_key(&owner)? != identity.owner {
        return Err("assistant_identity_changed".into());
    }
    Ok(owner)
}

fn key_error(error: ApiKeyError) -> String {
    log::warn!(
        "[Assistant] the assistant key is unavailable: {}",
        crate::redact::scrub(&format!("{error:?}"))
    );
    match error {
        ApiKeyError::Unauthorized(_) => SIGN_IN_REQUIRED,
        ApiKeyError::RemoteRejected(_) => "assistant_key_unavailable",
        ApiKeyError::Storage(_) => "assistant_key_storage_unavailable",
    }
    .into()
}

/// The host's admission as the key module asks it (under the key lock),
/// keeping the host's own reason when it refuses.
fn admission_check<'a>(
    admit: &'a Admit<'a>,
    refusal: &'a Mutex<Option<String>>,
) -> impl Fn() -> bool + Send + Sync + 'a {
    move || match admit() {
        Ok(()) => true,
        Err(code) => {
            *refusal.lock().unwrap_or_else(|e| e.into_inner()) = Some(code);
            false
        }
    }
}

/// A key fetch's result as the host's code: the host's refusal wins, and a
/// key refused moments after minting is not replaced (another would be too).
fn key_outcome(
    result: Result<Option<String>, ApiKeyError>,
    refusal: Option<String>,
) -> Result<String, String> {
    match (result, refusal) {
        (Ok(Some(key)), _) => Ok(key),
        (Ok(None), _) => Err(KEY_REJECTED.into()),
        (Err(_), Some(code)) => Err(code),
        (Err(error), None) => Err(key_error(error)),
    }
}

/// Where the prepared runtime is: the checkout's in a debug build, the
/// bundled resource in a release build.
fn runtime_dir_for(
    debug: bool,
    resource_dir: impl FnOnce() -> Option<PathBuf>,
) -> Result<PathBuf, String> {
    if debug {
        return Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime/resources"));
    }
    resource_dir()
        .map(|dir| dir.join("assistant-runtime"))
        .ok_or_else(|| "assistant_runtime_unavailable".into())
}

impl Environment for AssistantEnvironment {
    fn local_mutate<'a>(
        &'a self,
        name: &'a str,
        tool: crate::app_config::BindableTool,
        model: Option<&'a str>,
        operation: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            super::assistant_local::mutate(&self.app, name, tool, model, operation)
                .await
                .map_err(|error| {
                    if error.starts_with("assistant_") || error == "unknown_outcome" {
                        error
                    } else {
                        "assistant_local_repair_failed".into()
                    }
                })
        })
    }
    fn local_restore(
        &self,
        journal: &mut crate::services::assistant::tools::journal::Journal,
    ) -> Result<(), String> {
        let _io = crate::config_io::lock();
        let state = self.app.state::<crate::store::AppState>();
        use crate::services::assistant::tools::db_state;
        journal.restore_with(
            |before| {
                db_state::capture(
                    &state.db,
                    before["scope"]
                        .as_str()
                        .ok_or("assistant_journal_unavailable")?,
                )
            },
            |before, after| db_state::restore(&state.db, before, after),
        )
    }
    fn local_journal_key(&self) -> BoxFuture<'_, Result<zeroize::Zeroizing<[u8; 32]>, String>> {
        Box::pin(async {
            crate::services::assistant::host::blocking(
                crate::services::assistant::tools::journal::system_key,
            )
            .await
        })
    }

    fn local_paths(
        &self,
    ) -> BoxFuture<'_, Result<crate::services::assistant::tools::context::LocalPaths, String>> {
        Box::pin(async move {
            crate::services::assistant::tools::context::application_paths(self.runtime_dir()?).await
        })
    }
    fn local_binding_status(
        &self,
        tool: crate::app_config::BindableTool,
    ) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(async move {
            let state = self.app.state::<crate::store::AppState>();
            let status = if crate::app_config::BindableTool::native(tool.as_str()).is_some() {
                crate::native_tools::binding_status(&state.db, tool).await
            } else if tool == crate::app_config::BindableTool::WorkBuddy {
                crate::workbuddy_config::binding_status(&state.db).await
            } else {
                let (binding, _) = crate::services::ofox_bind::tool_for(tool.as_str())
                    .ok_or("assistant_invalid_tool")?;
                let key = crate::ofox_secret::default_store()
                    .load(crate::ofox_secret::Slot::ApiKey { tool })
                    .map_err(|_| "assistant_key_storage_unavailable")?;
                crate::services::ofox_bind::status::binding_status(
                    &state.db,
                    binding,
                    key.as_deref(),
                )
                .await
            };
            serde_json::to_value(status).map_err(|_| "assistant_local_unavailable".into())
        })
    }
    fn local_key_cached(&self, tool: crate::app_config::BindableTool) -> bool {
        // A storage error counts as missing: the rebind then asks first.
        crate::ofox_secret::default_store()
            .load(crate::ofox_secret::Slot::ApiKey { tool })
            .is_ok_and(|key| key.is_some())
    }

    fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
        Box::pin(async move {
            let status = self.auth()?.read().await.get_auth_status().await;
            identity_from(status, crate::ofox_apex::current_apex())
        })
    }

    fn api_key<'a>(
        &'a self,
        identity: &'a Identity,
        rejected: Option<&'a str>,
        admit: &'a Admit<'a>,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let manager = self.auth()?;
            key_owner(&manager, identity).await?;
            let refusal = Mutex::new(None);
            let check = admission_check(admit, &refusal);
            let result = match rejected {
                None => ofox_api_keys::fetch_or_create_assistant_key(
                    FetchMode::CachedOk,
                    &identity.apex,
                    &check,
                    &manager,
                )
                .await
                .map(Some),
                Some(rejected) => {
                    ofox_api_keys::refresh_rejected_assistant_key(
                        rejected,
                        &identity.apex,
                        &check,
                        &manager,
                    )
                    .await
                }
            };
            drop(check);
            key_outcome(
                result,
                refusal.into_inner().unwrap_or_else(|e| e.into_inner()),
            )
        })
    }

    fn stored_key<'a>(&'a self, identity: &'a Identity) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let manager = self.auth().ok()?;
            let owner = key_owner(&manager, identity).await.ok()?;
            // The keychain read blocks.
            let read = blocking(move || {
                ofox_api_keys::stored_assistant_key(&owner)
                    .map_err(|error| crate::redact::scrub(&format!("{error:?}")).into_owned())
            })
            .await;
            match read {
                Ok(key) => key,
                Err(error) => {
                    log::warn!("[Assistant] the stored assistant key could not be read: {error}");
                    None
                }
            }
        })
    }

    fn key_ready<'a>(&'a self, identity: &'a Identity) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let Ok(manager) = self.auth() else {
                return false;
            };
            key_owner(&manager, identity).await.is_ok_and(|owner| {
                ofox_api_keys::assistant_key_status(Some(&owner)) == AssistantKeyStatus::Ready
            })
        })
    }

    fn models<'a>(
        &'a self,
        apex: &'a str,
        source: catalog::Source,
    ) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>> {
        Box::pin(catalog::fetch_models(apex, source))
    }

    fn gateway_base(&self, apex: &str) -> Result<String, String> {
        crate::ofox_apex::gateway_base_for(apex).ok_or_else(|| "assistant_unknown_apex".into())
    }

    fn config_dir(&self) -> PathBuf {
        crate::config::get_app_config_dir()
    }

    fn runtime_dir(&self) -> Result<PathBuf, String> {
        runtime_dir_for(cfg!(debug_assertions), || {
            self.app.path().resource_dir().ok()
        })
    }

    fn emit(&self, event: Event) {
        if let Err(error) = self.app.emit(ASSISTANT_EVENT, event) {
            log::warn!("[Assistant] could not publish an event: {error}");
        }
    }

    /// The app's proxy (the one the catalog and key requests use), else the
    /// system proxy as launched CLIs get it.
    fn proxy(&self) -> Option<RunProxy> {
        let configured = crate::proxy::http_client::get_current_proxy_url();
        crate::commands::assistant_proxy(configured.as_deref())
            .map(|(url, no_proxy)| RunProxy { url, no_proxy })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ofox_auth::OfoxUserInfo;
    use tauri::http::{HeaderName, HeaderValue};

    fn paste_headers(extra: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in [
            (PASTE_SESSION_HEADER, "session-1"),
            (PASTE_OWNER_HEADER, "owner-1"),
            (PASTE_GENERATION_HEADER, "7"),
        ]
        .iter()
        .chain(extra)
        {
            headers.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    }

    fn raw(bytes: &[u8]) -> InvokeBody {
        InvokeBody::Raw(bytes.to_vec())
    }

    #[test]
    fn a_paste_is_a_raw_body_with_its_authority_and_name_in_headers() {
        let headers = paste_headers(&[(PASTE_NAME_HEADER, "%E4%B8%AD%E6%96%87%20a%2Bb.png")]);
        let (authority, name, bytes) = parse_paste(&headers, &raw(b"\x89PNG")).unwrap();
        assert_eq!(authority.session_id, "session-1");
        assert_eq!(authority.expected_owner, "owner-1");
        assert_eq!(authority.expected_generation, 7);
        assert_eq!(name, "中文 a+b.png");
        assert_eq!(bytes, b"\x89PNG");
    }

    #[test]
    fn a_paste_without_a_name_or_with_a_broken_one_still_parses() {
        let (_, name, _) = parse_paste(&paste_headers(&[]), &raw(b"x")).unwrap();
        assert_eq!(name, "");
        // Invalid UTF-8 once decoded is replaced, not refused: it is a hint.
        let headers = paste_headers(&[(PASTE_NAME_HEADER, "a%FFb.txt")]);
        let (_, name, _) = parse_paste(&headers, &raw(b"x")).unwrap();
        assert_eq!(name, "a\u{fffd}b.txt");
    }

    #[test]
    fn a_paste_that_is_not_raw_bytes_with_the_headers_is_unreadable() {
        let json = InvokeBody::Json(serde_json::json!({"sessionId": "s"}));
        assert_eq!(
            parse_paste(&paste_headers(&[]), &json).unwrap_err(),
            attachments::UNREADABLE
        );
        for missing in [
            PASTE_SESSION_HEADER,
            PASTE_OWNER_HEADER,
            PASTE_GENERATION_HEADER,
        ] {
            let mut headers = paste_headers(&[]);
            headers.remove(missing);
            assert_eq!(
                parse_paste(&headers, &raw(b"x")).unwrap_err(),
                attachments::UNREADABLE,
                "{missing}"
            );
        }
        let headers = paste_headers(&[(PASTE_GENERATION_HEADER, "seven")]);
        assert_eq!(
            parse_paste(&headers, &raw(b"x")).unwrap_err(),
            attachments::UNREADABLE
        );
    }

    #[test]
    fn a_paste_over_the_file_limit_is_refused_before_anything_else() {
        let big = raw(&vec![0u8; attachments::MAX_FILE_BYTES as usize + 1]);
        assert_eq!(
            parse_paste(&HeaderMap::new(), &big).unwrap_err(),
            attachments::TOO_LARGE
        );
        let limit = raw(&vec![0u8; attachments::MAX_FILE_BYTES as usize]);
        assert!(parse_paste(&paste_headers(&[]), &limit).is_ok());
    }

    #[test]
    fn only_a_drag_entering_or_leaving_is_told_to_the_renderer() {
        let position = tauri::PhysicalPosition::new(1.0, 2.0);
        let paths = vec![PathBuf::from("/private/secret.txt")];
        assert_eq!(
            drag_state(&DragDropEvent::Enter {
                paths: paths.clone(),
                position
            }),
            Some("enter")
        );
        assert_eq!(drag_state(&DragDropEvent::Leave), Some("leave"));
        assert_eq!(drag_state(&DragDropEvent::Over { position }), None);
        assert_eq!(drag_state(&DragDropEvent::Drop { paths, position }), None);
    }

    fn user(id: &str, org: &str) -> OfoxUserInfo {
        serde_json::from_value(serde_json::json!({
            "id": id, "email": null, "name": null, "org_id": org, "avatar_url": null,
        }))
        .unwrap()
    }

    fn identity_of(id: &str, org: &str) -> Identity {
        Identity {
            owner: store::owner_key(&user(id, org).key_owner()).unwrap(),
            apex: "ofox.ai".into(),
        }
    }

    async fn manager(signed_in: Option<OfoxUserInfo>) -> (tempfile::TempDir, AuthManager) {
        let config = tempfile::tempdir().unwrap();
        let manager = OfoxAuthManager::new_with_secret_store(
            config.path().to_path_buf(),
            Arc::new(crate::ofox_secret::InMemoryStore::new()),
        );
        if let Some(user) = signed_in {
            manager.set_signed_in_for_test(user).await;
        }
        (config, Arc::new(RwLock::new(manager)))
    }

    #[tokio::test]
    async fn a_key_is_fetched_only_for_the_signed_in_account_of_the_run() {
        let run = identity_of("user_a", "org_a");
        let (_config, signed_out) = manager(None).await;
        assert_eq!(
            key_owner(&signed_out, &run).await.unwrap_err(),
            SIGN_IN_REQUIRED
        );
        let (_config, other) = manager(Some(user("user_b", "org_b"))).await;
        assert_eq!(
            key_owner(&other, &run).await.unwrap_err(),
            "assistant_identity_changed"
        );
        let (_config, same) = manager(Some(user("user_a", "org_a"))).await;
        assert_eq!(
            key_owner(&same, &run).await.unwrap().org_id.as_deref(),
            Some("org_a")
        );
    }

    #[tokio::test]
    async fn only_an_active_session_with_a_full_account_is_an_identity() {
        let (_config, signed_out) = manager(None).await;
        let status = signed_out.read().await.get_auth_status().await;
        assert_eq!(
            identity_from(status, "ofox.io").unwrap_err(),
            SIGN_IN_REQUIRED
        );
        let (_config, signed_in) = manager(Some(user("user_a", "org_a"))).await;
        let status = signed_in.read().await.get_auth_status().await;
        assert_eq!(
            identity_from(status, "ofox.io").unwrap(),
            Identity {
                apex: "ofox.io".into(),
                ..identity_of("user_a", "org_a")
            }
        );
        let mut no_org = user("user_a", "org_a");
        no_org.org_id = None;
        let (_config, partial) = manager(Some(no_org)).await;
        let status = partial.read().await.get_auth_status().await;
        assert_eq!(
            identity_from(status, "ofox.io").unwrap_err(),
            "assistant_identity_unavailable"
        );
    }

    #[test]
    fn key_failures_map_to_stable_codes_without_their_text() {
        for (error, code) in [
            (
                ApiKeyError::Unauthorized("token sk-of-abcdefghijklmnop".into()),
                SIGN_IN_REQUIRED,
            ),
            (
                ApiKeyError::RemoteRejected("500".into()),
                "assistant_key_unavailable",
            ),
            (
                ApiKeyError::Storage("keychain".into()),
                "assistant_key_storage_unavailable",
            ),
        ] {
            assert_eq!(key_error(error), code);
        }
    }

    #[test]
    fn a_refused_admission_is_reported_with_the_hosts_own_code() {
        let refusal = Mutex::new(None);
        let closed = || Err::<(), _>("assistant_signing_out".to_string());
        let check = admission_check(&closed, &refusal);
        assert!(!check());
        drop(check);
        let refused = refusal.into_inner().unwrap();
        assert_eq!(refused.as_deref(), Some("assistant_signing_out"));
        assert_eq!(
            key_outcome(
                Err(ApiKeyError::Unauthorized(
                    "assistant runs are closed".into()
                )),
                refused
            )
            .unwrap_err(),
            "assistant_signing_out"
        );

        let refusal = Mutex::new(None);
        let open = || Ok::<(), String>(());
        assert!(admission_check(&open, &refusal)());
        assert_eq!(refusal.into_inner().unwrap(), None);
    }

    #[test]
    fn key_results_map_to_the_hosts_codes() {
        assert_eq!(key_outcome(Ok(Some("k".into())), None).unwrap(), "k");
        assert_eq!(
            key_outcome(Ok(None), None).unwrap_err(),
            KEY_REJECTED,
            "a key refused moments after minting is not replaced"
        );
        assert_eq!(
            key_outcome(Err(ApiKeyError::Storage("keychain".into())), None).unwrap_err(),
            "assistant_key_storage_unavailable"
        );
    }

    #[test]
    fn the_runtime_comes_from_the_checkout_in_debug_and_the_bundle_in_release() {
        let checkout = runtime_dir_for(true, || panic!("debug builds read no resources")).unwrap();
        assert!(
            checkout.ends_with("assistant-runtime/resources"),
            "{checkout:?}"
        );
        assert_eq!(
            runtime_dir_for(false, || Some(PathBuf::from("/Resources"))).unwrap(),
            PathBuf::from("/Resources/assistant-runtime")
        );
        assert_eq!(
            runtime_dir_for(false, || None).unwrap_err(),
            "assistant_runtime_unavailable"
        );
    }
}

/// The destination comes only from the native save dialog, never IPC.
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_save_media(
    app: tauri::AppHandle,
    host: Host<'_>,
    request: crate::services::assistant::media_protocol::SaveMediaRequest,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    host.save_media(request, move |name| {
        app.dialog()
            .file()
            .set_file_name(name)
            .blocking_save_file()
            .map(|file| {
                file.into_path()
                    .map_err(|_| "assistant_export_destination_invalid".to_string())
            })
            .transpose()
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_quote_media(
    host: Host<'_>,
    request: crate::services::assistant::video::QuoteMediaRequest,
) -> Result<crate::services::assistant::video::MediaQuote, String> {
    host.quote_media(request).await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_regenerate_media(
    host: Host<'_>,
    request: crate::services::assistant::media_tools::RegenerateRequest,
) -> Result<serde_json::Value, String> {
    host.regenerate_media(request).await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_cancel_video(
    host: Host<'_>,
    request: crate::services::assistant::video::VideoActionRequest,
) -> Result<(), String> {
    host.cancel_video(request).await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_resume_video(
    host: Host<'_>,
    request: crate::services::assistant::video::VideoActionRequest,
) -> Result<(), String> {
    host.resume_video(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_respond_approval(
    host: Host<'_>,
    request: crate::services::assistant::host::local::ApprovalRequest,
) -> Result<(), String> {
    host.respond_approval(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_set_permission(
    host: Host<'_>,
    request: crate::services::assistant::host::local::PermissionRequest,
) -> Result<crate::services::assistant::tools::policy::PermissionMode, String> {
    host.set_permission(request).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_local_changes(
    host: Host<'_>,
    request: DeleteRequest,
) -> Result<Vec<crate::services::assistant::tools::journal::Change>, String> {
    host.local_changes(request, false).await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_restore_local_changes(
    host: Host<'_>,
    request: DeleteRequest,
) -> Result<Vec<crate::services::assistant::tools::journal::Change>, String> {
    host.local_changes(request, true).await
}
