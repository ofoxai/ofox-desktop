//! Built-in assistant commands (S1: chat). Each one finds the host and calls
//! its request of the same name (`services::assistant::requests`), so tests
//! drive the same code. [`AssistantEnvironment`] lends the host the app's
//! sign-in, the assistant key, the model catalog, files and event channel.
use crate::ofox_api_keys::{self, ApiKeyError, AssistantKeyStatus, FetchMode};
use crate::services::assistant::{
    catalog,
    host::{BoxFuture, Environment, Identity, SIGN_IN_REQUIRED},
    store::{self, SessionList},
    types::{CreateRequest, Detail, Event, RunRef, SendRequest, Session, UpdateRequest},
    AssistantHost,
};
use crate::settings::KeyOwner;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tokio::sync::RwLock;

/// The renderer's event channel for runs.
pub const ASSISTANT_EVENT: &str = "assistant:event";

type Host<'a> = State<'a, Arc<AssistantHost>>;

#[tauri::command(rename_all = "camelCase")]
pub async fn assistant_bootstrap(host: Host<'_>) -> Result<Value, String> {
    host.bootstrap().await
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
pub async fn assistant_send(host: Host<'_>, request: SendRequest) -> Result<RunRef, String> {
    host.send(request).await
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

/// The host's view of the running app.
pub struct AssistantEnvironment {
    app: tauri::AppHandle,
}

impl AssistantEnvironment {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    fn auth(&self) -> Result<Arc<RwLock<crate::ofox_auth::OfoxAuthManager>>, String> {
        self.app
            .try_state::<crate::commands::ofox_auth::OfoxAuthState>()
            .map(|state| state.0.clone())
            .ok_or_else(|| SIGN_IN_REQUIRED.to_string())
    }

    /// The signed-in account, and only if it is `identity`'s: a key is never
    /// fetched for a signed-out state or for another account.
    async fn key_owner(&self, identity: &Identity) -> Result<KeyOwner, String> {
        let owner = self
            .auth()?
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

impl Environment for AssistantEnvironment {
    fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
        Box::pin(async move {
            let auth = self.auth()?;
            let status = auth.read().await.get_auth_status().await;
            if status.state != crate::ofox_auth::OfoxAuthState::Active {
                return Err(SIGN_IN_REQUIRED.into());
            }
            let user = status.user.ok_or("assistant_identity_unavailable")?;
            Ok(Identity {
                owner: store::owner_key(&user.key_owner())?,
                apex: crate::ofox_apex::current_apex().into(),
            })
        })
    }

    fn api_key<'a>(
        &'a self,
        identity: &'a Identity,
        rejected: Option<&'a str>,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            self.key_owner(identity).await?;
            let manager = self.auth()?;
            match rejected {
                None => {
                    ofox_api_keys::fetch_or_create_assistant_key(
                        FetchMode::CachedOk,
                        &identity.apex,
                        &manager,
                    )
                    .await
                }
                Some(rejected) => {
                    ofox_api_keys::refresh_rejected_assistant_key(
                        rejected,
                        &identity.apex,
                        &manager,
                    )
                    .await
                }
            }
            .map_err(key_error)
        })
    }

    fn key_ready<'a>(&'a self, identity: &'a Identity) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            self.key_owner(identity).await.is_ok_and(|owner| {
                ofox_api_keys::assistant_key_status(Some(&owner)) == AssistantKeyStatus::Ready
            })
        })
    }

    fn models<'a>(&'a self, apex: &'a str) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>> {
        Box::pin(catalog::fetch_models(apex))
    }

    fn gateway_base(&self, apex: &str) -> Result<String, String> {
        crate::ofox_apex::gateway_base_for(apex).ok_or_else(|| "assistant_unknown_apex".into())
    }

    fn config_dir(&self) -> PathBuf {
        crate::config::get_app_config_dir()
    }

    fn runtime_dir(&self) -> Result<PathBuf, String> {
        if cfg!(debug_assertions) {
            return Ok(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime/resources")
            );
        }
        self.app
            .path()
            .resource_dir()
            .map(|dir| dir.join("assistant-runtime"))
            .map_err(|_| "assistant_runtime_unavailable".into())
    }

    fn emit(&self, event: Event) {
        if let Err(error) = self.app.emit(ASSISTANT_EVENT, event) {
            log::warn!("[Assistant] could not publish an event: {error}");
        }
    }
}
