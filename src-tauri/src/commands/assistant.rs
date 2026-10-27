//! Built-in assistant commands (S1: chat). Each one finds the host and calls
//! its request of the same name (`services::assistant::requests`), so tests
//! drive the same code. [`AssistantEnvironment`] lends the host the app's
//! sign-in, the assistant key, the model catalog, files and event channel.
use crate::ofox_api_keys::{self, ApiKeyError, AssistantKeyStatus, FetchMode};
use crate::ofox_auth::{OfoxAuthManager, OfoxAuthStatus};
use crate::services::assistant::{
    catalog,
    host::{Admit, BoxFuture, Environment, Identity, KEY_REJECTED, SIGN_IN_REQUIRED},
    store::{self, SessionList},
    types::{CreateRequest, Detail, Event, RunProxy, RunRef, SendRequest, Session, UpdateRequest},
    AssistantHost,
};
use crate::settings::KeyOwner;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
