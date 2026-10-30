//! Private media reads: scoped to the account/generation that rendered the URI.
use super::{
    host::{blocking, AssistantHost, IDENTITY_CHANGED},
    store::AssistantStore,
    types::{validate_id, Attachment, DeleteRequest},
};
use std::{
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::Arc,
};
use tauri::http::{Method, Request, Response};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveMediaRequest {
    #[serde(flatten)]
    pub authority: DeleteRequest,
    pub attachment_id: String,
}
fn empty(status: u16) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Cache-Control", "no-store")
        .body(vec![])
        .unwrap()
}
fn open_file(
    store: &AssistantStore,
    sid: &str,
    aid: &str,
) -> Result<(Attachment, std::fs::File, u64), String> {
    validate_id(sid)?;
    validate_id(aid)?;
    let record = store.load_record(sid)?;
    let file = record
        .attachments
        .into_iter()
        .find(|f| {
            f.id == aid
                && (f.origin.as_deref() == Some("generated")
                    || (f.origin.as_deref() == Some("audio-input")
                        && matches!(f.mime_type.as_str(), "audio/wav" | "audio/mpeg")
                        && record.operations.iter().any(|op| {
                            op.media.as_ref().is_some_and(|media| {
                                media.kind == "transcription"
                                    && media.audio.is_some()
                                    && media.input_ids.contains(&f.id)
                            })
                        })))
                && matches!(
                    f.mime_type.as_str(),
                    "image/png"
                        | "image/jpeg"
                        | "image/webp"
                        | "image/gif"
                        | "video/mp4"
                        | "audio/wav"
                        | "audio/mpeg"
                        | "audio/ogg"
                )
        })
        .ok_or("assistant_attachment_not_found")?;
    let fail = || "assistant_attachment_read_failed".to_string();
    let directory = store.attachment_root(sid)?;
    if !std::fs::symlink_metadata(&directory)
        .map_err(|_| fail())?
        .file_type()
        .is_dir()
    {
        return Err(fail());
    }
    let path = directory.join(aid);
    if !std::fs::symlink_metadata(&path)
        .map_err(|_| fail())?
        .file_type()
        .is_file()
    {
        return Err(fail());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let handle = options.open(&path).map_err(|_| fail())?;
    let meta = handle.metadata().map_err(|_| fail())?;
    let cap = if file.mime_type.starts_with("video/") {
        200 * 1024 * 1024
    } else if file.mime_type.starts_with("audio/") {
        super::speech::MAX_OUTPUT_BYTES as u64
    } else {
        30 * 1024 * 1024
    };
    if !meta.is_file() || meta.len() == 0 || meta.len() > cap || meta.len() != file.size_bytes {
        return Err(fail());
    }
    Ok((file, handle, meta.len()))
}
fn byte_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let (start, end) = header.strip_prefix("bytes=")?.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?;
        return (suffix > 0).then(|| (len.saturating_sub(suffix), len - 1));
    }
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse::<u64>().ok()?.min(len - 1)
    };
    (start < len && start <= end).then_some((start, end))
}
fn serve(
    store: &AssistantStore,
    owner: &str,
    generation: u64,
    request: &Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return empty(405);
    }
    let Ok(url) = url::Url::parse(&request.uri().to_string()) else {
        return empty(400);
    };
    if !matches!(
        (url.scheme(), url.host_str()),
        ("assistant-media", Some("localhost"))
            | ("http", Some("assistant-media.localhost"))
            | ("https", Some("assistant-media.localhost"))
    ) {
        return empty(400);
    }
    let query: Vec<_> = url.query_pairs().collect();
    if query.len() != 2
        || query
            .iter()
            .filter(|(k, v)| k == "owner" && v == owner)
            .count()
            != 1
        || query
            .iter()
            .filter(|(k, v)| k == "generation" && v.parse::<u64>().ok() == Some(generation))
            .count()
            != 1
    {
        return empty(403);
    }
    let parts: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
    if parts.len() != 2 {
        return empty(400);
    }
    let Ok((file, mut handle, len)) = open_file(store, parts[0], parts[1]) else {
        return empty(404);
    };
    let range = request.headers().get("range");
    let (start, end, status) = if let Some(header) = range {
        let Some((start, end)) = header.to_str().ok().and_then(|h| byte_range(h, len)) else {
            return Response::builder()
                .status(416)
                .header("Content-Range", format!("bytes */{len}"))
                .header("Cache-Control", "no-store")
                .body(vec![])
                .unwrap();
        };
        (start, end, 206)
    } else {
        (0, len - 1, 200)
    };
    let count = end - start + 1;
    let mut bytes = vec![];
    if request.method() != Method::HEAD
        && (handle.seek(SeekFrom::Start(start)).is_err()
            || handle.take(count).read_to_end(&mut bytes).is_err()
            || bytes.len() as u64 != count)
    {
        return empty(404);
    }
    let mut response = Response::builder()
        .status(status)
        .header("Content-Type", file.mime_type)
        .header("Content-Length", count)
        .header("Accept-Ranges", "bytes")
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff");
    if status == 206 {
        response = response.header("Content-Range", format!("bytes {start}-{end}/{len}"));
    }
    response.body(bytes).unwrap()
}
impl AssistantHost {
    pub async fn media_response(self: &Arc<Self>, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
        let Ok(identity) = self.identity().await else {
            return empty(403);
        };
        let generation = self.generation();
        let Ok(store) = self.store(&identity.owner) else {
            return empty(404);
        };
        let owner = identity.owner.clone();
        let result = blocking(move || Ok(serve(&store, &owner, generation, &request))).await;
        if self.ensure_current(&identity, generation).await.is_err() {
            return empty(403);
        }
        result.unwrap_or_else(|_| empty(404))
    }
    pub async fn save_media(
        self: &Arc<Self>,
        request: SaveMediaRequest,
        pick: impl FnOnce(String) -> Result<Option<PathBuf>, String> + Send + 'static,
    ) -> Result<bool, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let authority = request.authority;
        if identity.owner != authority.expected_owner || generation != authority.expected_generation
        {
            return Err(IDENTITY_CHANGED.into());
        }
        validate_id(&authority.session_id)?;
        validate_id(&request.attachment_id)?;
        let name = blocking({
            let store = self.store(&identity.owner)?;
            let sid = authority.session_id.clone();
            let aid = request.attachment_id.clone();
            move || open_file(&store, &sid, &aid).map(|(file, _, _)| file.name)
        })
        .await?;
        self.ensure_current(&identity, generation).await?;
        let Some(path) = blocking(move || pick(name)).await? else {
            return Ok(false);
        };
        self.ensure_current(&identity, generation).await?;
        let host = self.clone();
        blocking(move || {
            let store = host.store(&identity.owner)?;
            let (_, file, len) = open_file(&store, &authority.session_id, &request.attachment_id)?;
            let mut bytes = vec![];
            file.take(len + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "assistant_attachment_read_failed")?;
            if bytes.len() as u64 != len {
                return Err("assistant_attachment_read_failed".into());
            }
            let path = super::transfers::destination(&path, &host.env.config_dir())?;
            if host.generation() != generation {
                return Err(IDENTITY_CHANGED.into());
            }
            crate::config::atomic_write_with(
                &path,
                &bytes,
                crate::config::AtomicOpts { mode: Some(0o600) },
            )
            .map_err(|_| "assistant_export_failed".to_string())?;
            Ok(true)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{id, Attachment};
    use super::*;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, AssistantStore, String, String, String) {
        let root = tempfile::tempdir().unwrap();
        let owner = "ab".repeat(32);
        let store = AssistantStore::for_owner(root.path(), &owner).unwrap();
        let sid = store.create("test/image", false).unwrap().session.id;
        let aid = id();
        let file:Attachment=serde_json::from_value(json!({"id":aid,"name":"result.png","mimeType":"image/png","sizeBytes":12,"status":"ready","origin":"generated"})).unwrap();
        store
            .mutate(&sid, |record| {
                record.attachments.push(file);
                Ok(())
            })
            .unwrap();
        std::fs::write(
            store.attachment_dir(&sid).unwrap().join(&aid),
            b"\x89PNG\r\n\x1a\n1234",
        )
        .unwrap();
        (root, store, owner, sid, aid)
    }
    fn request(owner: &str, sid: &str, aid: &str) -> Request<Vec<u8>> {
        Request::builder()
            .uri(format!(
                "assistant-media://localhost/{sid}/{aid}?owner={owner}&generation=7"
            ))
            .body(vec![])
            .unwrap()
    }
    #[test]
    fn protocol_serves_only_current_owner_generation_and_valid_media_ids() {
        let (_root, store, owner, sid, aid) = fixture();
        let req = request(&owner, &sid, &aid);
        let response = serve(&store, &owner, 7, &req);
        assert_eq!(response.status(), 200);
        assert_eq!(response.body(), b"\x89PNG\r\n\x1a\n1234");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["content-type"], "image/png");
        assert_eq!(serve(&store, &"cd".repeat(32), 7, &req).status(), 403);
        assert_eq!(serve(&store, &owner, 8, &req).status(), 403);
        for bad in ["..", "%2e%2e", "../secret", "missing"] {
            assert!(!serve(&store, &owner, 7, &request(&owner, &sid, bad))
                .status()
                .is_success());
        }
        let windows = Request::builder()
            .uri(format!(
                "http://assistant-media.localhost/{sid}/{aid}?owner={owner}&generation=7"
            ))
            .body(vec![])
            .unwrap();
        assert_eq!(serve(&store, &owner, 7, &windows).status(), 200);
        store
            .mutate(&sid, |record| {
                record.attachments[0].origin = None;
                Ok(())
            })
            .unwrap();
        assert_eq!(serve(&store, &owner, 7, &req).status(), 404);
    }
    #[test]
    fn protocol_handles_single_byte_ranges_head_and_unsatisfiable_ranges() {
        let (_root, store, owner, sid, aid) = fixture();
        for (range, expected) in [
            ("bytes=8-10", b"123".as_slice()),
            ("bytes=-2", b"34"),
            ("bytes=10-", b"34"),
        ] {
            let mut req = request(&owner, &sid, &aid);
            req.headers_mut().insert("range", range.parse().unwrap());
            let response = serve(&store, &owner, 7, &req);
            assert_eq!(response.status(), 206);
            assert_eq!(response.body(), expected);
            assert!(response.headers()["content-range"]
                .to_str()
                .unwrap()
                .ends_with("/12"));
        }
        for range in ["bytes=99-", "bytes=3-2", "bytes=0-1,4-5", "bytes=-0"] {
            let mut req = request(&owner, &sid, &aid);
            req.headers_mut().insert("range", range.parse().unwrap());
            let response = serve(&store, &owner, 7, &req);
            assert_eq!(response.status(), 416);
            assert_eq!(response.headers()["content-range"], "bytes */12");
        }
        let mut req = request(&owner, &sid, &aid);
        *req.method_mut() = tauri::http::Method::HEAD;
        let response = serve(&store, &owner, 7, &req);
        assert!(response.body().is_empty());
        assert_eq!(response.headers()["content-length"], "12");
    }
    #[cfg(unix)]
    #[test]
    fn protocol_refuses_symlinks() {
        let (root, store, owner, sid, aid) = fixture();
        let path = store.attachment_root(&sid).unwrap().join(&aid);
        let outside = root.path().join("outside.png");
        std::fs::write(&outside, b"private").unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(outside, path).unwrap();
        assert_eq!(
            serve(&store, &owner, 7, &request(&owner, &sid, &aid)).status(),
            404
        );
    }
    #[cfg(unix)]
    #[test]
    fn protocol_refuses_a_linked_attachment_directory() {
        let (root, store, owner, sid, aid) = fixture();
        let directory = store.attachment_root(&sid).unwrap();
        let outside = root.path().join("outside");
        std::fs::rename(&directory, &outside).unwrap();
        std::os::unix::fs::symlink(outside, directory).unwrap();
        assert_eq!(
            serve(&store, &owner, 7, &request(&owner, &sid, &aid)).status(),
            404
        );
    }
    #[test]
    fn audio_protocol_supports_all_output_mimes_ranges_and_its_own_fifty_mib_limit() {
        for mime in ["audio/wav", "audio/mpeg", "audio/ogg"] {
            let (_root, store, owner, sid, aid) = fixture();
            let size = 31 * 1024 * 1024u64;
            let path = store.attachment_root(&sid).unwrap().join(&aid);
            std::fs::File::create(&path).unwrap().set_len(size).unwrap();
            store
                .mutate(&sid, |r| {
                    r.attachments[0].mime_type = mime.into();
                    r.attachments[0].size_bytes = size;
                    Ok(())
                })
                .unwrap();
            let mut req = request(&owner, &sid, &aid);
            req.headers_mut()
                .insert("range", "bytes=1048576-1048591".parse().unwrap());
            let response = serve(&store, &owner, 7, &req);
            assert_eq!(response.status(), 206);
            assert_eq!(response.body(), &vec![0u8; 16]);
            assert_eq!(response.headers()["content-type"], mime);
            assert_eq!(response.headers()["content-length"], "16");
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(serve(&store, &owner, 8, &req).status(), 403);
            let mut head = request(&owner, &sid, &aid);
            *head.method_mut() = Method::HEAD;
            let response = serve(&store, &owner, 7, &head);
            assert_eq!(response.status(), 200);
            assert!(response.body().is_empty());
            assert_eq!(response.headers()["content-length"], size.to_string());
            let too_big = super::super::speech::MAX_OUTPUT_BYTES as u64 + 1;
            std::fs::File::create(path)
                .unwrap()
                .set_len(too_big)
                .unwrap();
            store
                .mutate(&sid, |r| {
                    r.attachments[0].size_bytes = too_big;
                    Ok(())
                })
                .unwrap();
            assert_eq!(serve(&store, &owner, 7, &head).status(), 404);
        }
    }

    struct ReadEnv {
        root: PathBuf,
        identity: std::sync::Mutex<super::super::host::Identity>,
    }
    impl super::super::host::Environment for ReadEnv {
        fn identity(
            &self,
        ) -> super::super::host::BoxFuture<'_, Result<super::super::host::Identity, String>>
        {
            Box::pin(async { Ok(self.identity.lock().unwrap().clone()) })
        }
        fn api_key<'a>(
            &'a self,
            _: &'a super::super::host::Identity,
            _: Option<&'a str>,
            _: &'a super::super::host::Admit<'a>,
        ) -> super::super::host::BoxFuture<'a, Result<String, String>> {
            panic!("local audio playback/save must not acquire a key")
        }
        fn key_ready<'a>(
            &'a self,
            _: &'a super::super::host::Identity,
        ) -> super::super::host::BoxFuture<'a, bool> {
            Box::pin(async { false })
        }
        fn models<'a>(
            &'a self,
            _: &'a str,
            _: super::super::catalog::Source,
        ) -> super::super::host::BoxFuture<'a, Result<Vec<super::super::catalog::Model>, String>>
        {
            panic!("local audio must not query model catalogs")
        }
        fn gateway_base(&self, _: &str) -> Result<String, String> {
            panic!("local audio must not use a gateway")
        }
        fn config_dir(&self) -> PathBuf {
            self.root.clone()
        }
        fn runtime_dir(&self) -> Result<PathBuf, String> {
            Err("assistant_runtime_unavailable".into())
        }
        fn emit(&self, _: super::super::types::Event) {}
    }
    #[tokio::test]
    async fn audio_save_reuses_bytes_with_no_paid_call_and_rechecks_owner_after_picker() {
        for apex in ["ofox.ai", "ofox.io"] {
            let (root, store, owner, sid, aid) = fixture();
            let bytes = super::super::speech::tests::wav(1);
            std::fs::write(store.attachment_root(&sid).unwrap().join(&aid), &bytes).unwrap();
            store
                .mutate(&sid, |r| {
                    let f = &mut r.attachments[0];
                    f.mime_type = "audio/wav".into();
                    f.size_bytes = bytes.len() as u64;
                    f.name = "speech.wav".into();
                    Ok(())
                })
                .unwrap();
            let env = Arc::new(ReadEnv {
                root: root.path().into(),
                identity: std::sync::Mutex::new(super::super::host::Identity {
                    owner: owner.clone(),
                    apex: apex.into(),
                }),
            });
            let host = AssistantHost::new(env.clone());
            let make = || SaveMediaRequest {
                authority: DeleteRequest {
                    session_id: sid.clone(),
                    expected_owner: owner.clone(),
                    expected_generation: host.generation(),
                },
                attachment_id: aid.clone(),
            };
            let output = tempfile::tempdir().unwrap();
            let destination = output.path().join("speech.wav");
            let target = destination.clone();
            assert!(host
                .save_media(make(), move |name| {
                    assert_eq!(name, "speech.wav");
                    Ok(Some(target))
                })
                .await
                .unwrap());
            assert_eq!(std::fs::read(&destination).unwrap(), bytes);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&destination)
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
            assert!(!host.save_media(make(), |_| Ok(None)).await.unwrap());
            let mut stale = make();
            stale.authority.expected_generation += 1;
            assert_eq!(
                host.save_media(stale, |_| panic!("no stale picker"))
                    .await
                    .unwrap_err(),
                IDENTITY_CHANGED
            );
            let target = output.path().join("refused.wav");
            let rejected = target.clone();
            let changed = env.clone();
            assert_eq!(
                host.save_media(make(), move |_| {
                    changed.identity.lock().unwrap().owner = "cd".repeat(32);
                    Ok(Some(target))
                })
                .await
                .unwrap_err(),
                IDENTITY_CHANGED
            );
            assert!(!rejected.exists());
        }
    }
    #[test]
    fn audio_retention_review_protocol_requires_a_retained_asr_ledger_reference() {
        let (_root, store, owner, sid, aid) = fixture();
        store
            .mutate(&sid, |r| {
                r.attachments[0].mime_type = "audio/wav".into();
                r.attachments[0].origin = Some("audio-input".into());
                Ok(())
            })
            .unwrap();
        let req = request(&owner, &sid, &aid);
        assert_eq!(serve(&store, &owner, 7, &req).status(), 404);
        store.mutate(&sid,|r| {r.operations.push(serde_json::from_value(json!({"id":id(),"runId":id(),"status":"completed","media":{"kind":"transcription","model":"test/asr","params":{},"status":"completed","inputIds":[aid],"audio":{"pricing":{"unit":"seconds"}}}})).unwrap());Ok(())}).unwrap();
        assert_eq!(serve(&store, &owner, 7, &req).status(), 200);
        store
            .mutate(&sid, |r| {
                r.operations[0].media.as_mut().unwrap().kind = "speech".into();
                Ok(())
            })
            .unwrap();
        assert_eq!(serve(&store, &owner, 7, &req).status(), 404);
    }
}
