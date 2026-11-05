//! Authority-fenced explicit transcription runs.
use super::*;
use crate::services::assistant::{
    host::{
        blocking, load_native, session_key, AssistantHost, RunControl, IDENTITY_CHANGED, PROTOCOL,
    },
    projection::recount_usage,
    speech::SpeechRequest,
    store,
};
use serde_json::json;
use std::sync::Arc;

impl AssistantHost {
    pub async fn transcribe_audio(
        self: &Arc<Self>,
        request: TranscribeRequest,
    ) -> Result<ResultRecord, String> {
        request.validate()?;
        self.audio_job(request.into()).await
    }

    pub async fn synthesize_audio(
        self: &Arc<Self>,
        request: SpeechRequest,
    ) -> Result<ResultRecord, String> {
        request.validate()?;
        self.audio_job(request.into()).await
    }

    async fn audio_job(
        self: &Arc<Self>,
        mut request: super::job::Job,
    ) -> Result<ResultRecord, String> {
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
        let preview = request.preview(self)?;
        let model = self
            .models(&identity.apex)
            .await?
            .models
            .into_iter()
            .find(|m| m.id == request.model_id && m.kind == request.kind() && m.audio.is_some())
            .ok_or(INVALID)?;
        request.check(&record, &model)?;
        let kind = request.kind();
        blocking({
            let store = store.clone();
            let model = model.id.clone();
            let host = self.clone();
            let generation = request.expected_generation;
            move || {
                host.admit(generation)?;
                store.remember_model_kind(&model, kind)
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
        request.scrub(&self.known_keys());
        request.check(&record, &model)?;
        if let Some((voice, format)) = request.speech_choice() {
            blocking({
                let store = store.clone();
                let host = self.clone();
                let model = model.id.clone();
                let generation = request.expected_generation;
                move || {
                    host.admit(generation)?;
                    store.remember_speech_choices(&model, &voice, &format)
                }
            })
            .await?;
        }
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
        let mut media = scrub_media(
            request.media(&model, preview, &identity.apex)?,
            &self.known_keys(),
        )?;
        let chat_model = record.session.model_id;
        blocking({
            let store = store.clone();
            let sid = request.session_id.clone();
            let host = self.clone();
            let control = control.clone();
            let media = media.clone();
            let generation = request.expected_generation;
            let check = request.clone();
            let model = model.clone();
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
                        check.check(r, &model)?;
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
        let input = request.take(self);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let host = self.clone();
        let done = control.clone();
        let sid = request.session_id.clone();
        self.track(control.run_id.clone(), async move {
            let _history = history;
            let mut output = None;
            let mut original = None;
            let attempted = async {
                let mut input = input?;
                if let Some(input)=&mut input {input.name=super::super::host::scrub_secrets(&input.name,&host.known_keys());}
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
                    result = request.perform(&client, &gateway, &key, &model, input.as_ref()) => result,
                    _ = cancelled.wait_for(|value| *value) => Err(http::Failure {
                        code: UNKNOWN,
                        unbilled: false,
                        request_id: None,
                        request_id_source: None,
                    }),
                };
                if request.keep_original() { original = input; }
                let trace = media.audio.as_mut().ok_or(INVALID)?;
                match result {
                    Ok(response) => {
                        media.status = "completed".into();
                        media.cost = if media.kind == "transcription" { cost(&trace.pricing, &response.usage).map(|amount| amount.normalize().to_string()) } else { trace.estimate.clone() };
                        trace.text = response.text;
                        if let Some((bytes, info)) = response.output { trace.output = Some(info.clone()); output = Some((bytes, info)); }
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
            let saved = match scrub_media(media,&host.known_keys()) {
                Ok(mut media) => blocking({
                    let done = done.clone();
                    let store = store.clone();
                    let sid = sid.clone();
                    let host = host.clone();
                    let generation = request.expected_generation;
                    move || {
                        let finish = |r: &mut store::SessionRecord, media: &MediaRecord| {
                            let op = r.operations.iter_mut().find(|op| op.id == done.operation_id).ok_or("assistant_stale_operation")?;
                            op.status = if media.status == "completed" { OperationStatus::Completed } else if done.cancelled() { OperationStatus::Cancelled } else { OperationStatus::Failed };
                            op.media = Some(media.clone());
                            if r.session.active_run.as_ref().is_some_and(|active| active.run_id == done.run_id) { r.session.active_run = None; }
                            r.session.updated_at = now();
                            recount_usage(r, &entries);
                            Ok((ResultRecord { operation_id: done.operation_id.clone(), run_id: done.run_id.clone(), media: media.clone() }, r.usage.clone(), r.session.updated_at))
                        };
                        if let Some((bytes, info)) = output {
                            let published = crate::services::assistant::attachments::generated_audio(&store, &sid, None, &mut media, &bytes, &info, |record, media, _| {
                                host.admit(generation)?;
                                host.validate_run(&done)?;
                                if record.session.deleted_at.is_some() {return Err("assistant_session_deleted".into());}
                                finish(record, media)
                            });
                            if published.is_ok() { return published; }
                            let code = published.err().unwrap_or_else(|| UNKNOWN.into());
                            media.status = "unknown".into(); media.cost = None; media.outputs.clear();
                            if let Some(trace) = &mut media.audio {trace.output = None; trace.error_code = Some(code);}
                        }
                        if let Some(input) = original {
                            let published = crate::services::assistant::attachments::retained_audio(&store, &sid, &mut media, &input, |record, media| {
                                host.admit(generation)?;
                                if !record.session.active_run.as_ref().is_some_and(|run|run.run_id==done.run_id && run.operation_id==done.operation_id) {return Err("assistant_stale_operation".into());}
                                if record.session.deleted_at.is_some() {return Err("assistant_session_deleted".into());}
                                finish(record, media)
                            });
                            if published.is_ok() { return published; }
                            media.input_ids.clear();
                            if let Some(trace) = &mut media.audio {trace.original_error = Some(published.err().unwrap_or_else(|| UNKNOWN.into()));}
                        }
                        store.mutate(&sid, |record| finish(record, &media))
                    }
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
            atomic::{AtomicBool, AtomicUsize, Ordering},
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
        key: Mutex<String>,
        fill_quota_on_key: AtomicBool,
        events: Mutex<Vec<Event>>,
    }
    impl Environment for Env {
        fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
            Box::pin(async { Ok(self.identity.lock().unwrap().clone()) })
        }
        fn api_key<'a>(
            &'a self,
            identity: &'a Identity,
            _: Option<&'a str>,
            admit: &'a Admit<'a>,
        ) -> BoxFuture<'a, Result<String, String>> {
            Box::pin(async move {
                admit()?;
                self.keys.fetch_add(1, Ordering::SeqCst);
                if self.fill_quota_on_key.swap(false, Ordering::SeqCst) {
                    let store = store::AssistantStore::for_owner(&self.root, &identity.owner)?;
                    let sid = store.list(false, "")?.sessions[0].id.clone();
                    store.mutate(&sid,|r|{r.attachments.push(serde_json::from_value(json!({"id":id(),"name":"late.bin","mimeType":"application/octet-stream","sizeBytes":500*1024*1024,"status":"ready"})).unwrap());Ok(())})?;
                }
                Ok(self.key.lock().unwrap().clone())
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
                        json!({"data":[{"id":"google/gemini-3.8-flash-tts","supported_endpoints":["audio/speech"],"pricing":{"prompt":"0.000001","output_audio":"0.00002"}},{"id":"elevenlabs/eleven_v4","supported_endpoints":["audio/speech"],"pricing":{"input_text_per_character":"0.0002"}},{"id":"test/chat","context_length":32768,"supported_endpoints":["chat/completions"]},{"id":"openai/gpt-transcribe","supported_endpoints":["audio/transcriptions"],"pricing":{"input_audio_per_second":"0.000075"}}]}),
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
            key: Mutex::new("synthetic-audio-key-123456".into()),
            fill_quota_on_key: AtomicBool::new(false),
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
                "synthetic-audio-key-123456.mp3".into(),
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
            keep_original: false,
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
            assert!(record.attachments.is_empty());
            assert!(result.media.input_ids.is_empty());
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
    async fn audio_host_gateway_timeout_keeps_unknown_cost_and_never_replays() {
        for apex in ["ofox.ai", "ofox.io"] {
            let (_root, server, _env, host, request) = fixture(apex).await;
            Mock::given(method("POST"))
                .and(path("/v1/audio/transcriptions"))
                .respond_with(
                    ResponseTemplate::new(408)
                        .insert_header("x-request-id", "req_timeout")
                        .set_body_json(json!({"error":{"message":"request timed out"}})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let result = host.transcribe_audio(request.clone()).await.unwrap();
            assert_eq!(result.media.status, "unknown");
            assert!(result.media.cost.is_none());
            let trace = result.media.audio.as_ref().unwrap();
            assert_eq!(trace.error_code.as_deref(), Some(UNKNOWN));
            assert_eq!(trace.request_id.as_deref(), Some("req_timeout"));
            let record = host
                .store(&request.expected_owner)
                .unwrap()
                .load_record(&request.session_id)
                .unwrap();
            assert_eq!(record.usage["modelRequests"], 1);
            assert!(record.session.active_run.is_none());
            assert!(host.transcribe_audio(request).await.is_err());
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }
    #[tokio::test]
    async fn audio_host_cancellation_keeps_the_submitted_outcome_unknown_and_never_replays() {
        for keep in [false, true] {
            let (_root, server, _env, host, mut request) = fixture("ofox.io").await;
            request.keep_original = keep;
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
            let trace = result.media.audio.as_ref().unwrap();
            assert!(trace.text.is_none());
            if keep {
                assert_eq!(result.media.input_ids.len(), 1);
                assert!(trace.original_error.is_none());
            } else {
                assert!(result.media.input_ids.is_empty());
            }
            assert!(host.transcribe_audio(request).await.is_err());
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }
    #[tokio::test]
    async fn speech_host_refuses_invalid_parameters_before_keys_and_never_changes_chat() {
        for apex in ["ofox.ai", "ofox.io"] {
            let (_root, server, env, host, asr) = fixture(apex).await;
            let request:crate::services::assistant::speech::SpeechRequest=serde_json::from_value(json!({
            "sessionId":asr.session_id,"expectedOwner":asr.expected_owner,"expectedGeneration":asr.expected_generation,
            "runId":id(),"operationId":id(),"modelId":"google/gemini-3.8-flash-tts","input":"请朗读。","voice":"Kore","responseFormat":"wav"
        })).unwrap();
            let mut invalid = request.clone();
            invalid.response_format = "mp3".into();
            assert!(host.synthesize_audio(invalid).await.is_err());
            assert_eq!(env.keys.load(Ordering::SeqCst), 0);
            let mut invalid_source = request.clone();
            invalid_source.source_message_id = Some(id());
            assert_eq!(
                host.synthesize_audio(invalid_source).await.unwrap_err(),
                "assistant_message_not_found"
            );
            let mut stale = request.clone();
            stale.expected_generation += 1;
            assert_eq!(
                host.synthesize_audio(stale).await.unwrap_err(),
                IDENTITY_CHANGED
            );
            assert_eq!(env.keys.load(Ordering::SeqCst), 0);
            let bytes = crate::services::assistant::speech::tests::wav(1);
            Mock::given(method("POST"))
                .and(path("/v1/audio/speech"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("Content-Type", "audio/wav")
                        .set_body_bytes(bytes),
                )
                .expect(1)
                .mount(&server)
                .await;
            let result = host.synthesize_audio(request.clone()).await.unwrap();
            assert_eq!(result.media.kind, "speech");
            assert_eq!(result.media.status, "completed");
            assert!(result.media.cost.is_none());
            let store = host.store(&request.expected_owner).unwrap();
            let record = store.load_record(&request.session_id).unwrap();
            assert_eq!(record.session.model_id, "test/chat");
            assert_eq!(record.session.draft, "保留的草稿");
            assert!(record.messages.is_empty());
            assert!(record.session.active_run.is_none());
            assert_eq!(record.attachments.len(), 1);
            assert_eq!(record.attachments[0].mime_type, "audio/wav");
            let path = store
                .attachment_dir(&request.session_id)
                .unwrap()
                .join(&result.media.outputs[0]);
            assert!(path.is_file());
            store
                .cleanup_orphan_attachments(&request.session_id)
                .unwrap();
            assert!(path.is_file());
            // Billed by tokens: the cost is unknown, here and in the summary.
            assert!(record.usage["estimatedCostUsd"].is_null());
            assert_eq!(
                store.usage_summary().unwrap()["usageCoverage"],
                "incomplete"
            );
            assert_eq!(
                store.remembered_models().unwrap()["speech"],
                request.model_id
            );
            assert!(host.synthesize_audio(request).await.is_err());
            assert_eq!(env.keys.load(Ordering::SeqCst), 1);
        }
    }
    #[tokio::test]
    async fn speech_host_cancellation_is_unknown_and_rejects_concurrent_and_stale_requests() {
        let (_root, server, env, host, asr) = fixture("ofox.io").await;
        let request:SpeechRequest=serde_json::from_value(json!({"sessionId":asr.session_id,"expectedOwner":asr.expected_owner,"expectedGeneration":asr.expected_generation,"runId":id(),"operationId":id(),"modelId":"elevenlabs/eleven_v4","input":"Hello","voice":"21m00Tcm4TlvDq8ikWAM","responseFormat":"mp3"})).unwrap();
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "audio/mpeg")
                    .set_body_bytes(
                        include_bytes!(
                            "../../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3"
                        )
                        .to_vec(),
                    )
                    .set_delay(std::time::Duration::from_secs(2)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let running = tokio::spawn({
            let host = host.clone();
            let request = request.clone();
            async move { host.synthesize_audio(request).await }
        });
        for _ in 0..200 {
            if !server.received_requests().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut concurrent = request.clone();
        concurrent.operation_id = id();
        concurrent.run_id = id();
        assert_eq!(
            host.synthesize_audio(concurrent).await.unwrap_err(),
            "assistant_session_busy"
        );
        host.cancel_all();
        let result = running.await.unwrap().unwrap();
        assert_eq!(result.media.status, "unknown");
        assert!(result.media.outputs.is_empty());
        assert!(result.media.cost.is_none());
        assert!(host.synthesize_audio(request.clone()).await.is_err());
        assert_eq!(env.keys.load(Ordering::SeqCst), 1);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        let record = host
            .store(&request.expected_owner)
            .unwrap()
            .load_record(&request.session_id)
            .unwrap();
        assert!(record.attachments.is_empty());
        assert!(env
            .events
            .lock()
            .unwrap()
            .iter()
            .all(|event| event.data["running"] == true));
    }
    #[tokio::test]
    async fn audio_retention_review_original_name_is_scrubbed_on_both_apexes() {
        for apex in ["ofox.ai", "ofox.io"] {
            let (_root, server, _env, host, request) = fixture(apex).await;
            let mut raw = serde_json::json!({"sessionId":request.session_id,"expectedOwner":request.expected_owner,"expectedGeneration":request.expected_generation,"inputId":request.input_id,"runId":request.run_id,"operationId":request.operation_id,"modelId":request.model_id,"keepOriginal":true});
            raw["language"] = json!("en");
            let request: TranscribeRequest = serde_json::from_value(raw).unwrap();
            Mock::given(method("POST"))
                .and(path("/v1/audio/transcriptions"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(
                        json!({"text":"Hello","usage":{"type":"duration","seconds":1}}),
                    ),
                )
                .expect(1)
                .mount(&server)
                .await;
            let result = host.transcribe_audio(request.clone()).await.unwrap();
            assert_eq!(result.media.status, "completed");
            assert_eq!(result.media.input_ids.len(), 1);
            assert!(result.media.outputs.is_empty());
            let store = host.store(&request.expected_owner).unwrap();
            let record = store.load_record(&request.session_id).unwrap();
            assert!(record.messages.is_empty());
            assert_eq!(record.attachments.len(), 1);
            assert_eq!(record.attachments[0].origin.as_deref(), Some("audio-input"));
            assert!(!serde_json::to_string(&record)
                .unwrap()
                .contains("synthetic-audio-key-123456"));
            let path = store
                .attachment_root(&request.session_id)
                .unwrap()
                .join(&result.media.input_ids[0]);
            assert_eq!(
                std::fs::read(&path).unwrap(),
                include_bytes!("../../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3")
            );
            store
                .cleanup_orphan_attachments(&request.session_id)
                .unwrap();
            assert!(path.exists());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }
    #[tokio::test]
    async fn original_audio_failure_keeps_transcript_and_usage_estimate_without_replay() {
        let (_root, server, env, host, mut request) = fixture("ofox.io").await;
        request.keep_original = true;
        let store = host.store(&request.expected_owner).unwrap();
        std::fs::write(
            store.attachment_root(&request.session_id).unwrap(),
            b"blocked directory",
        )
        .unwrap();
        Mock::given(method("POST")).and(path("/v1/audio/transcriptions")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"text":"Transcript remains available","usage":{"type":"duration","seconds":1}}))).expect(1).mount(&server).await;
        let result = host.transcribe_audio(request.clone()).await.unwrap();
        assert_eq!(result.media.status, "completed");
        assert_eq!(result.media.cost.as_deref(), Some("0.000075"));
        assert!(result.media.input_ids.is_empty());
        let trace = result.media.audio.as_ref().unwrap();
        assert_eq!(trace.text.as_deref(), Some("Transcript remains available"));
        assert!(trace.original_error.is_some());
        assert!(host.transcribe_audio(request).await.is_err());
        assert_eq!(env.keys.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn original_audio_quota_refuses_before_minting_a_key() {
        let (_root, _server, env, host, mut request) = fixture("ofox.ai").await;
        request.keep_original = true;
        let store = host.store(&request.expected_owner).unwrap();
        store.mutate(&request.session_id,|r|{r.attachments.push(serde_json::from_value(json!({"id":id(),"name":"existing.bin","mimeType":"application/octet-stream","sizeBytes":500*1024*1024,"status":"ready"})).unwrap());Ok(())}).unwrap();
        assert_eq!(
            host.transcribe_audio(request).await.unwrap_err(),
            "assistant_attachment_total_too_large"
        );
        assert_eq!(env.keys.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn audio_review_original_names_are_scrubbed_before_events_and_multipart_upload() {
        let (_root, server, env, host, request) = fixture("ofox.io").await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_secs(5))
                    .set_body_json(json!({"text":"Hello","usage":{"type":"duration","seconds":1}})),
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
        let record = host
            .store(&request.expected_owner)
            .unwrap()
            .load_record(&request.session_id)
            .unwrap();
        assert!(record.session.active_run.is_some());
        assert!(!serde_json::to_string(&record)
            .unwrap()
            .contains("synthetic-audio-key-123456"));
        host.cancel_session(&request.session_id);
        assert_eq!(running.await.unwrap().unwrap().media.status, "unknown");
        let shown: String = env
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.data.to_string())
            .collect();
        assert!(!shown.contains("synthetic-audio-key-123456"));
        let requests = server.received_requests().await.unwrap();
        assert!(!String::from_utf8_lossy(&requests[0].body).contains("synthetic-audio-key-123456"));
    }
    #[tokio::test]
    async fn audio_review_a_newly_minted_key_cannot_be_saved_as_a_voice_preference() {
        let (_root, server, env, host, asr) = fixture("ofox.ai").await;
        *env.key.lock().unwrap() = "SyntheticSecretAscii".into();
        let request:SpeechRequest=serde_json::from_value(json!({"sessionId":asr.session_id,"expectedOwner":asr.expected_owner,"expectedGeneration":asr.expected_generation,"runId":id(),"operationId":id(),"modelId":"google/gemini-3.8-flash-tts","input":"Hello","voice":"SyntheticSecretAscii","responseFormat":"wav"})).unwrap();
        let _ = host.synthesize_audio(request).await;
        let store = host.store(&asr.expected_owner).unwrap();
        assert!(!store
            .speech_preferences()
            .unwrap()
            .to_string()
            .contains("SyntheticSecretAscii"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn audio_review_quota_changed_during_key_fetch_is_checked_again_before_submission() {
        for speech in [false, true] {
            let (_root, server, env, host, mut asr) = fixture("ofox.io").await;
            env.fill_quota_on_key.store(true, Ordering::SeqCst);
            let result = if speech {
                let request:SpeechRequest=serde_json::from_value(json!({"sessionId":asr.session_id,"expectedOwner":asr.expected_owner,"expectedGeneration":asr.expected_generation,"runId":id(),"operationId":id(),"modelId":"google/gemini-3.8-flash-tts","input":"Hello","voice":"Kore","responseFormat":"wav"})).unwrap();
                host.synthesize_audio(request).await
            } else {
                asr.keep_original = true;
                host.transcribe_audio(asr).await
            };
            assert!(result.is_err());
            assert!(server.received_requests().await.unwrap().is_empty());
            assert_eq!(env.keys.load(Ordering::SeqCst), 1);
        }
    }
}
