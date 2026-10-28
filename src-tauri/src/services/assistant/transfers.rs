//! Native file transfers are bound to the account confirmed by the renderer.
use super::{
    host::{blocking, scrub_secrets, AssistantHost, Identity, IDENTITY_CHANGED},
    store::{BackupReport, RestoreReport, TRANSFER_BUSY},
    types::{validate_id, ExportMarkdownRequest, TransferRequest},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

const EXPORT_FAILED: &str = "assistant_export_failed";
const INVALID_DESTINATION: &str = "assistant_export_destination_invalid";
const MAX_MARKDOWN_BYTES: usize = 64 * 1024 * 1024;

/// Resolve the parent once, refuse links and the application's own data. The
/// native picker owns overwrite confirmation; writes are private and atomic.
pub(super) fn destination(path: &Path, config: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(INVALID_DESTINATION.into());
    }
    let parent = path
        .parent()
        .and_then(|dir| dir.canonicalize().ok())
        .ok_or(INVALID_DESTINATION)?;
    let path = parent.join(path.file_name().ok_or(INVALID_DESTINATION)?);
    let config = config.canonicalize().map_err(|_| INVALID_DESTINATION)?;
    if path.starts_with(config) {
        return Err(INVALID_DESTINATION.into());
    }
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if !meta.file_type().is_file() => return Err(INVALID_DESTINATION.into()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(INVALID_DESTINATION.into())
        }
        _ => {}
    }
    Ok(path)
}

impl AssistantHost {
    async fn transfer_identity(
        &self,
        request: &TransferRequest,
    ) -> Result<(Identity, u64), String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        if identity.owner != request.expected_owner || generation != request.expected_generation {
            return Err(IDENTITY_CHANGED.into());
        }
        Ok((identity, generation))
    }

    /// No transfer while one of this owner's replies runs: its conversation
    /// is being written. Each conversation is checked again under its lock.
    fn ensure_no_reply(&self, owner: &str) -> Result<(), String> {
        if self.has_live_run(owner) {
            return Err(TRANSFER_BUSY.into());
        }
        Ok(())
    }

    /// The keys a file leaving the app must not hold: every key this process
    /// handed to a run, and the account's stored assistant key.
    async fn transfer_secrets(&self, identity: &Identity) -> Vec<String> {
        let mut secrets = self.known_keys();
        if let Some(key) = self.env.stored_key(identity).await {
            if !secrets.contains(&key) {
                secrets.push(key);
            }
        }
        secrets
    }

    /// `None` when the dialog was cancelled.
    pub async fn backup(
        self: &Arc<Self>,
        request: TransferRequest,
        pick: impl FnOnce() -> Result<Option<PathBuf>, String> + Send + 'static,
    ) -> Result<Option<BackupReport>, String> {
        let (identity, generation) = self.transfer_identity(&request).await?;
        self.ensure_no_reply(&identity.owner)?;
        let Some(path) = blocking(pick).await? else {
            return Ok(None);
        };
        self.ensure_current(&identity, generation).await?;
        let secrets = self.transfer_secrets(&identity).await;
        let host = self.clone();
        blocking(move || {
            host.ensure_no_reply(&identity.owner)?;
            let store = host.store(&identity.owner)?;
            store
                .backup(&path, &secrets, || {
                    if host.generation() != generation {
                        Err(IDENTITY_CHANGED.into())
                    } else {
                        Ok(())
                    }
                })
                .map(Some)
        })
        .await
    }

    pub async fn restore(
        self: &Arc<Self>,
        request: TransferRequest,
        pick: impl FnOnce() -> Result<Option<PathBuf>, String> + Send + 'static,
    ) -> Result<Option<RestoreReport>, String> {
        let (identity, generation) = self.transfer_identity(&request).await?;
        self.ensure_no_reply(&identity.owner)?;
        let Some(path) = blocking(pick).await? else {
            return Ok(None);
        };
        self.ensure_current(&identity, generation).await?;
        let secrets = self.transfer_secrets(&identity).await;
        let host = self.clone();
        blocking(move || {
            host.ensure_no_reply(&identity.owner)?;
            let store = host.store(&identity.owner)?;
            store
                .restore(&path, &secrets, || {
                    if host.generation() != generation {
                        Err(IDENTITY_CHANGED.into())
                    } else {
                        Ok(())
                    }
                })
                .map(Some)
        })
        .await
    }

    pub async fn export_markdown(
        self: &Arc<Self>,
        request: ExportMarkdownRequest,
        pick: impl FnOnce() -> Result<Option<PathBuf>, String> + Send + 'static,
    ) -> Result<bool, String> {
        let authority = request.authority;
        validate_id(&authority.session_id)?;
        let identity = self.identity().await?;
        let generation = self.generation();
        if authority.expected_owner != identity.owner || authority.expected_generation != generation
        {
            return Err(IDENTITY_CHANGED.into());
        }
        if request.markdown.len() > MAX_MARKDOWN_BYTES {
            return Err("assistant_export_too_large".into());
        }
        if self
            .get_session(&authority.session_id)
            .await?
            .session
            .active_run
            .is_some()
        {
            return Err("assistant_session_busy".into());
        }
        self.ensure_current(&identity, generation).await?;
        let Some(path) = blocking(pick).await? else {
            return Ok(false);
        };
        self.ensure_current(&identity, generation).await?;
        let secrets = self.transfer_secrets(&identity).await;
        let host = self.clone();
        blocking(move || {
            if host.generation() != generation {
                return Err(IDENTITY_CHANGED.into());
            }
            // A reply may have started in another window while the dialog was open.
            let record = host
                .store(&identity.owner)?
                .load_record(&authority.session_id)?;
            if record.session.active_run.is_some() {
                return Err("assistant_session_busy".into());
            }
            let path = destination(&path, &host.env.config_dir())?;
            let text = scrub_secrets(&request.markdown, &secrets);
            if host.generation() != generation {
                return Err(IDENTITY_CHANGED.into());
            }
            crate::config::atomic_write_with(
                &path,
                text.as_bytes(),
                crate::config::AtomicOpts { mode: Some(0o600) },
            )
            .map_err(|_| EXPORT_FAILED.to_string())?;
            Ok(true)
        })
        .await
    }
}
