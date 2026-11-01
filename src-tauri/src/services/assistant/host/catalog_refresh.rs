//! Public catalog refresh is separate from paid run admission and rendering.
use super::*;
struct Pending {
    key: String,
    pending: Arc<Mutex<HashSet<String>>>,
    revision: Arc<AtomicU64>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            if pending.remove(&self.key) {
                self.revision.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
}
fn key(identity: &Identity, generation: u64) -> String {
    format!(
        "catalog:{}:{}:{}",
        identity.apex, identity.owner, generation
    )
}
impl AssistantHost {
    pub(in crate::services::assistant) fn cached_catalog(
        &self,
        identity: &Identity,
        generation: u64,
    ) -> Result<CatalogSnapshot, String> {
        // One coherent revision: writers update the counter under either lock.
        let cache = self.models.lock().map_err(|_| STATE)?;
        let pending = self.catalog_pending.lock().map_err(|_| STATE)?;
        let refreshing = pending.contains(&key(identity, generation));
        let mut failed = vec![];
        let mut source = |source| {
            let entry = cache.get(&(identity.apex.clone(), source));
            if entry.is_some_and(|e| e.failed.is_some()) {
                failed.push(source);
            }
            entry
                .and_then(|e| e.rows.as_ref())
                .map(|(_, rows)| rows.clone())
                .ok_or_else(|| {
                    entry
                        .and_then(|e| e.failed.as_ref())
                        .map(|(_, error)| error.clone())
                        .unwrap_or_else(|| "assistant_catalog_unavailable".into())
                })
        };
        let merged = catalog::merge(
            source(catalog::Source::V1),
            source(catalog::Source::Anthropic),
        );
        let (mut catalog, error) = match merged {
            Ok(c) => (c, None),
            Err(e) => (catalog::Catalog::default(), Some(e)),
        };
        catalog.partial = failed
            .iter()
            .copied()
            .filter(|source| {
                cache
                    .get(&(identity.apex.clone(), *source))
                    .is_none_or(|e| e.rows.is_none())
            })
            .collect();
        Ok(CatalogSnapshot {
            catalog,
            error: if refreshing { None } else { error },
            revision: self.catalog_revision.load(Ordering::SeqCst),
            refreshing,
            failed,
        })
    }
    pub(in crate::services::assistant) fn refresh_catalog(
        self: &Arc<Self>,
        identity: Identity,
        generation: u64,
        force: bool,
    ) -> Result<(), String> {
        // Catalogs are public, read-only metadata. A restored identity may
        // read them before paid admission reopens; shutdown still owns cleanup.
        if self.generation() != generation
            || self.admission.lock().map_err(|_| STATE)?.shutting_down
        {
            return Ok(());
        }
        let has_chat = self
            .models
            .lock()
            .map_err(|_| STATE)?
            .iter()
            .filter(|((apex, _), _)| apex == &identity.apex)
            .filter_map(|(_, entry)| entry.rows.as_ref())
            .any(|(_, rows)| rows.iter().any(|model| model.kind == catalog::Kind::Chat));
        let read = if force && !has_chat {
            CatalogRead::Fresh
        } else if force {
            CatalogRead::Page
        } else {
            CatalogRead::Send
        };
        let due = {
            let cache = self.models.lock().map_err(|_| STATE)?;
            [catalog::Source::V1, catalog::Source::Anthropic]
                .iter()
                .any(|source| {
                    cache
                        .get(&(identity.apex.clone(), *source))
                        .is_none_or(|entry| entry.reuse(read).is_none())
                })
        };
        if !due {
            return Ok(());
        }
        let key = key(&identity, generation);
        {
            let mut pending = self.catalog_pending.lock().map_err(|_| STATE)?;
            if !pending.insert(key.clone()) {
                return Ok(());
            }
            self.catalog_revision.fetch_add(1, Ordering::SeqCst);
        }
        let pending = Pending {
            key: key.clone(),
            pending: self.catalog_pending.clone(),
            revision: self.catalog_revision.clone(),
        };
        let host = self.clone();
        let mut revoked = self.video_revocation.subscribe();
        self.track(key, async move {
            let work = async {
                let v1 = async {
                    let _ = host
                        .catalog_source(&identity.apex, catalog::Source::V1, read)
                        .await;
                    let _ = host.publish_catalog(&identity, generation).await;
                };
                let anthropic = async {
                    let _ = host
                        .catalog_source(&identity.apex, catalog::Source::Anthropic, read)
                        .await;
                    let _ = host.publish_catalog(&identity, generation).await;
                };
                tokio::join!(v1, anthropic);
            };
            tokio::select! {biased;
                _=revoked.wait_for(|value|*value!=generation)=>{},
                _=work=>{},
            }
            drop(pending);
            let _ = host.publish_catalog(&identity, generation).await;
        });
        Ok(())
    }
}
