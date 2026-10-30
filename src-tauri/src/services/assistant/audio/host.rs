//! Authority-fenced explicit transcription runs.
use super::*;
use crate::services::assistant::{
    catalog::Kind,
    host::{
        blocking, load_native, session_key, AssistantHost, RunControl, IDENTITY_CHANGED, PROTOCOL,
    },
    projection::recount_usage,
    store,
};
use serde_json::json;
use std::{sync::Arc, time::Instant};

impl AssistantHost {
    pub async fn transcribe_audio(
        self: &Arc<Self>,
        request: TranscribeRequest,
    ) -> Result<ResultRecord, String> {
        request.validate()?;
        self.admit(request.expected_generation)?;
        let identity = self.identity().await?;
        if identity.owner != request.expected_owner {
            return Err(IDENTITY_CHANGED.into());
        }
        self.ensure_current(&identity, request.expected_generation)
            .await?;
        self.ensure_idle(&request.session_id)?;
        self.recover(&identity, &request.session_id).await?;
        let history = self
            .native_lock(&session_key(&identity.owner, &request.session_id))?
            .lock_owned()
            .await;
        let navigation = self.begin_navigation(&request.session_id)?;
        let store = self.store(&identity.owner)?;
        let record = blocking({
            let store = store.clone();
            let sid = request.session_id.clone();
            move || store.load_record(&sid)
        })
        .await?;
        if record.session.deleted_at.is_some() {
            return Err("assistant_session_deleted".into());
        }
        store::ensure_new_operation(&record, &request.operation_id)?;
        if record
            .operations
            .iter()
            .any(|op| op.run_id == request.run_id)
        {
            return Err("assistant_stale_operation".into());
        }
        let preview = self
            .audio_inputs
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .preview(&request.authority(), &request.input_id, Instant::now())?;
        let model = self
            .models(&identity.apex)
            .await?
            .models
            .into_iter()
            .find(|m| {
                m.id == request.model_id && m.kind == Kind::Transcription && m.audio.is_some()
            })
            .ok_or(INVALID)?;
        let pricing = model.audio.as_ref().ok_or(INVALID)?.pricing.clone();
        blocking({
            let store = store.clone();
            let model = model.id.clone();
            let host = self.clone();
            let generation = request.expected_generation;
            move || {
                host.admit(generation)?;
                store.remember_model_kind(&model, Kind::Transcription)
            }
        })
        .await?;
        let native = store.native_history_path(&request.session_id)?;
        let entries = if native.is_file() {
            load_native(&self.runtime()?, &native)
                .await?
                .as_array()
                .cloned()
                .ok_or(PROTOCOL)?
        } else {
            vec![]
        };
        let client = blocking({
            let env = self.env.clone();
            move || env.audio_client()
        })
        .await?;
        let gateway = self.env.gateway_base(&identity.apex)?;
        self.ensure_current(&identity, request.expected_generation)
            .await?;
        let key = self
            .env
            .api_key(&identity, None, &|| self.admit(request.expected_generation))
            .await?;
        self.ensure_current(&identity, request.expected_generation)
            .await?;
        self.remember_key(&key);
        let send: SendRequest = serde_json::from_value(json!({
            "sessionId": request.session_id,
            "runId": request.run_id,
            "operationId": request.operation_id,
            "text": "",
        }))
        .map_err(|_| INVALID)?;
        let mut control = RunControl::new(identity.clone(), &send, key.clone());
        control.audio_only = true;
        let control = Arc::new(control);
        let mut media = MediaRecord {
            kind: "transcription".into(),
            model: model.id.clone(),
            apex: Some(identity.apex.clone()),
            params: json!({"language":request.language,"inputName":preview.name}),
            status: "not_sent".into(),
            cost: Some("0".into()),
            audio: Some(Trace {
                pricing,
                input: Some(preview.info),
                text: None,
                usage: Value::Null,
                request_id: None,
                request_id_source: None,
                error_code: None,
            }),
            ..Default::default()
        };
        let chat_model = record.session.model_id;
        blocking({
            let store = store.clone();
            let sid = request.session_id.clone();
            let host = self.clone();
            let control = control.clone();
            let media = media.clone();
            let generation = request.expected_generation;
            move || {
                drop(navigation);
                host.register_run(control.clone(), generation, || {
                    store.mutate(&sid, |r| {
                        if r.session.deleted_at.is_some() {
                            return Err("assistant_session_deleted".into());
                        }
                        if r.session.model_id != chat_model || r.session.active_run.is_some() {
                            return Err("assistant_session_busy".into());
                        }
                        store::ensure_new_operation(r, &control.operation_id)?;
                        if r.operations.iter().any(|op| op.run_id == control.run_id) {
                            return Err("assistant_stale_operation".into());
                        }
                        r.operations.push(Operation {
                            id: control.operation_id.clone(),
                            run_id: control.run_id.clone(),
                            status: OperationStatus::Running,
                            media: Some(media),
                            search_price: None,
                        });
                        r.session.active_run = Some(RunRef {
                            run_id: control.run_id.clone(),
                            operation_id: control.operation_id.clone(),
                        });
                        r.session.updated_at = now();
                        Ok(())
                    })
                })
            }
        })
        .await?;
        let input = self
            .audio_inputs
            .lock()
            .map_err(|_| "assistant_state_unavailable".to_owned())
            .and_then(|mut slot| {
                slot.take(&request.authority(), &request.input_id, Instant::now())
            });
        let (tx, rx) = tokio::sync::oneshot::channel();
        let host = self.clone();
        let done = control.clone();
        let sid = request.session_id.clone();
        self.track(control.run_id.clone(), async move {
            let _history = history;
            let attempted = async {
                let input = input?;
                host.ensure_current(&done.identity, request.expected_generation).await?;
                host.validate_run(&done)?;
                // Durable submission intent precedes the network boundary. Recovery
                // must never replay a request whose response might have been lost.
                media.status = "submitting".into();
                media.cost = None;
                media.submitted_at = Some(now());
                let (usage, updated) = blocking({
                    let store = store.clone();
                    let sid = sid.clone();
                    let media = media.clone();
                    let done = done.clone();
                    let host = host.clone();
                    move || store.mutate(&sid, |r| {
                        host.validate_run(&done)?;
                        let op = r.operations.iter_mut()
                            .find(|op| op.id == done.operation_id)
                            .ok_or("assistant_stale_operation")?;
                        op.media = Some(media);
                        Ok((r.usage.clone(), r.session.updated_at))
                    })
                }).await?;
                host.audio_state(&done, &media, &usage, true, request.expected_generation, updated).await;
                if done.cancelled() {
                    return Err("assistant_cancelled".to_owned());
                }
                let mut cancelled = done.cancel.subscribe();
                let result = tokio::select! {
                    biased;
                    result = http::transcribe(&client, &gateway, &key, &model, &input, request.language.as_deref()) => result,
                    _ = cancelled.wait_for(|value| *value) => Err(http::Failure {
                        code: UNKNOWN,
                        unbilled: false,
                        request_id: None,
                        request_id_source: None,
                    }),
                };
                let trace = media.audio.as_mut().ok_or(INVALID)?;
                match result {
                    Ok(response) => {
                        media.status = "completed".into();
                        media.cost = cost(&trace.pricing, &response.usage)
                            .map(|amount| amount.normalize().to_string());
                        trace.text = Some(response.text);
                        trace.usage = response.usage;
                        trace.request_id = response.request_id;
                        trace.request_id_source = response.request_id_source;
                    }
                    Err(error) => {
                        media.status = if error.unbilled { "rejected" } else { "unknown" }.into();
                        media.cost = error.unbilled.then(|| "0".into());
                        trace.error_code = Some(error.code.into());
                        trace.request_id = error.request_id;
                        trace.request_id_source = error.request_id_source;
                    }
                }
                Ok::<(), String>(())
            }.await;
            // This branch contains only failures before http::transcribe was
            // polled. Submitted transport failures are represented above.
            if let Err(code) = attempted {
                media.status = "not_sent".into();
                media.cost = Some("0".into());
                if let Some(trace) = media.audio.as_mut() {
                    trace.error_code = Some(code);
                }
            }
            let mut encoded = serde_json::to_value(&media).map_err(|_| INVALID.to_owned());
            if let Ok(value) = &mut encoded {
                super::super::host::scrub_secrets_in(value, &host.known_keys());
            }
            let saved = match encoded.and_then(|value| {
                serde_json::from_value::<MediaRecord>(value).map_err(|_| INVALID.into())
            }) {
                Ok(media) => blocking({
                    let done = done.clone();
                    let store = store.clone();
                    let sid = sid.clone();
                    move || store.mutate(&sid, |r| {
                        let op = r.operations.iter_mut()
                            .find(|op| op.id == done.operation_id)
                            .ok_or("assistant_stale_operation")?;
                        op.status = if media.status == "completed" {
                            OperationStatus::Completed
                        } else if done.cancelled() {
                            OperationStatus::Cancelled
                        } else {
                            OperationStatus::Failed
                        };
                        op.media = Some(media.clone());
                        if r.session.active_run.as_ref().is_some_and(|active| active.run_id == done.run_id) {
                            r.session.active_run = None;
                        }
                        r.session.updated_at = now();
                        recount_usage(r, &entries);
                        Ok((ResultRecord {
                            operation_id: done.operation_id.clone(),
                            run_id: done.run_id.clone(),
                            media,
                        }, r.usage.clone(), r.session.updated_at))
                    })
                }).await,
                Err(error) => Err(error),
            };
            host.release_run(&done);
            if let Ok((result, usage, updated)) = &saved {
                host.audio_state(&done, &result.media, usage, false, request.expected_generation, *updated).await;
            }
            let _ = tx.send(saved.map(|(result, _, _)| result));
        });
        rx.await
            .map_err(|_| "assistant_audio_outcome_unknown".to_owned())?
    }

    async fn audio_state(
        &self,
        control: &RunControl,
        media: &MediaRecord,
        usage: &Value,
        running: bool,
        generation: u64,
        updated: i64,
    ) {
        if self
            .ensure_current(&control.identity, generation)
            .await
            .is_err()
        {
            return;
        }
        self.emit(
            control,
            "audio_state",
            json!({
                "owner": control.identity.owner,
                "generation": generation,
                "running": running,
                "updatedAt": updated,
                "result": ResultRecord {
                    operation_id: control.operation_id.clone(),
                    run_id: control.run_id.clone(),
                    media: media.clone(),
                },
                "usage": usage,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::assistant::{
        catalog,
        host::{Admit, BoxFuture, Environment, Identity, IDENTITY_CHANGED},
    };
    use serde_json::json;
    use std::{
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
    };
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };
    struct Env {
        root: PathBuf,
        identity: Mutex<Identity>,
        gateway: String,
        keys: AtomicUsize,
        events: Mutex<Vec<Event>>,
    }
    impl Environment for Env {
        fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
            Box::pin(async { Ok(self.identity.lock().unwrap().clone()) })
        }
        fn api_key<'a>(
            &'a self,
            _: &'a Identity,
            _: Option<&'a str>,
            admit: &'a Admit<'a>,
        ) -> BoxFuture<'a, Result<String, String>> {
            Box::pin(async move {
                admit()?;
                self.keys.fetch_add(1, Ordering::SeqCst);
                Ok("synthetic-audio-key-123456".into())
            })
        }
        fn key_ready<'a>(&'a self, _: &'a Identity) -> BoxFuture<'a, bool> {
            Box::pin(async { true })
        }
        fn models<'a>(
            &'a self,
            _: &'a str,
            source: catalog::Source,
        ) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>> {
            Box::pin(async move {
                Ok(if source == catalog::Source::V1 {
                    catalog::parse_models(
                        json!({"data":[{"id":"test/chat","context_length":32768,"supported_endpoints":["chat/completions"]},{"id":"openai/gpt-transcribe","supported_endpoints":["audio/transcriptions"],"pricing":{"input_audio_per_second":"0.000075"}}]}),
                        source,
                    )
                } else {
                    vec![]
                })
            })
        }
        fn gateway_base(&self, apex: &str) -> Result<String, String> {
            assert_eq!(apex, self.identity.lock().unwrap().apex);
            Ok(self.gateway.clone())
        }
        fn config_dir(&self) -> PathBuf {
            self.root.clone()
        }
        fn runtime_dir(&self) -> Result<PathBuf, String> {
            Err("assistant_runtime_unavailable".into())
        }
        fn emit(&self, event: Event) {
            self.events.lock().unwrap().push(event);
        }
        fn audio_client(&self) -> Result<reqwest::Client, String> {
            Ok(reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap())
        }
    }
    async fn fixture(
        apex: &str,
    ) -> (
        tempfile::TempDir,
        MockServer,
        Arc<Env>,
        Arc<AssistantHost>,
        TranscribeRequest,
    ) {
        let root = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let env = Arc::new(Env {
            root: root.path().into(),
            identity: Mutex::new(Identity {
                owner: "a".repeat(64),
                apex: apex.into(),
            }),
            gateway: server.uri(),
            keys: AtomicUsize::new(0),
            events: Mutex::default(),
        });
        let host = AssistantHost::new(env.clone());
        let store = host.store(&"a".repeat(64)).unwrap();
        let sid = store.create("test/chat", false).unwrap().session.id;
        store
            .mutate(&sid, |r| {
                r.session.draft = "保留的草稿".into();
                Ok(())
            })
            .unwrap();
        let authority = DeleteRequest {
            session_id: sid.clone(),
            expected_owner: "a".repeat(64),
            expected_generation: host.generation(),
        };
        let input = host
            .import_audio(
                authority,
                "speech.mp3".into(),
                include_bytes!("../../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3")
                    .to_vec(),
            )
            .await
            .unwrap();
        let request = TranscribeRequest {
            session_id: sid,
            expected_owner: "a".repeat(64),
            expected_generation: host.generation(),
            input_id: input.input_id,
            run_id: id(),
            operation_id: id(),
            model_id: "openai/gpt-transcribe".into(),
            language: Some("zh".into()),
        };
        (root, server, env, host, request)
    }
    #[tokio::test]
    async fn audio_host_transcribes_once_on_both_apexes_without_changing_chat_or_its_draft() {
        for apex in ["ofox.ai", "ofox.io"] {
            let (_root, server, env, host, request) = fixture(apex).await;
            Mock::given(method("POST")).and(path("/v1/audio/transcriptions")).respond_with(ResponseTemplate::new(200).insert_header("x-request-id","req_asr_1").set_body_json(json!({"text":"下午三点开会。 synthetic-audio-key-123456","usage":{"type":"duration","seconds":9}}))).expect(1).mount(&server).await;
            let result = host.transcribe_audio(request.clone()).await.unwrap();
            assert_eq!(result.media.apex.as_deref(), Some(apex));
            assert_eq!(result.media.cost.as_deref(), Some("0.000675"));
            assert_eq!(result.media.status, "completed");
            let store = host.store(&request.expected_owner).unwrap();
            let record = store.load_record(&request.session_id).unwrap();
            assert!(record.messages.is_empty());
            assert_eq!(record.session.draft, "保留的草稿");
            assert_eq!(record.session.model_id, "test/chat");
            assert!(record.session.active_run.is_none());
            assert_eq!(record.usage["modelRequests"], 1);
            assert_eq!(
                store.remembered_models().unwrap()["transcription"],
                request.model_id
            );
            let encoded = serde_json::to_string(&record).unwrap();
            assert!(!encoded.contains("synthetic-audio-key-123456"));
            assert!(encoded.contains("下午三点开会。"));
            assert!(host.transcribe_audio(request).await.is_err());
            assert_eq!(env.keys.load(Ordering::SeqCst), 1);
            let events = env.events.lock().unwrap();
            assert!(events.iter().all(|e| e.r#type == "audio_state"));
            assert_eq!(events.len(), 2);
        }
    }
    #[tokio::test]
    async fn audio_host_refuses_stale_owner_invalid_models_and_languages_before_key_acquisition() {
        let (_root, server, env, host, request) = fixture("ofox.ai").await;
        let mut stale = request.clone();
        stale.expected_generation += 1;
        assert_eq!(
            host.transcribe_audio(stale).await.unwrap_err(),
            IDENTITY_CHANGED
        );
        for request in [
            TranscribeRequest {
                model_id: "test/chat".into(),
                ..request.clone()
            },
            TranscribeRequest {
                language: Some("arbitrary\r\nheader".into()),
                ..request.clone()
            },
        ] {
            assert!(host.transcribe_audio(request).await.is_err());
        }
        assert_eq!(env.keys.load(Ordering::SeqCst), 0);
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn audio_host_cancellation_keeps_the_submitted_outcome_unknown_and_never_replays() {
        let (_root, server, _env, host, request) = fixture("ofox.io").await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_secs(2))
                    .set_body_json(json!({"text":"too late"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let running = tokio::spawn({
            let host = host.clone();
            let request = request.clone();
            async move { host.transcribe_audio(request).await }
        });
        for _ in 0..200 {
            if !server.received_requests().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut session = host
            .store(&request.expected_owner)
            .unwrap()
            .load_record(&request.session_id)
            .unwrap()
            .session;
        host.decorate(&request.expected_owner, &mut session);
        assert_eq!(session.audio_only, Some(true));
        host.cancel_session(&request.session_id);
        let result = running.await.unwrap().unwrap();
        assert_eq!(result.media.status, "unknown");
        assert!(result.media.cost.is_none());
        assert!(result.media.audio.unwrap().text.is_none());
        assert!(host.transcribe_audio(request).await.is_err());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
