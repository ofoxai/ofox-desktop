// Paused-clock regressions: the real Host, canonical freeze and attachment
// publication path, with fixed runtime frames and no provider requests.
fn timeout_presentation_plan() -> Value {
    use sha2::{Digest, Sha256};
    let catalog = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assistant-runtime/skills/ofox-office-pptx/resources/catalog.json"
    ));
    let editing = json!({"text":true,"charts":false,"formulas":false,"diagrams":false});
    json!({
        "catalogRevision":format!("{:x}",Sha256::digest(catalog.as_bytes())),
        "candidateIds":std::iter::once("navy".to_owned()).chain(super::super::presentation::enabled_catalog_styles()).collect::<Vec<_>>(),
        "styleId":"navy","decisionSummary":"Trusted timeout regression, without images or paid calls",
        "requirements":{"audience":"Reviewers","purpose":"Exercise long document work","scenario":"business","language":"en","slideCount":1,"aspectRatio":"16:9","density":"low","nativeEditing":editing,"visualNeeds":["none"],"roundTrip":"unverifiedAllowed","durationMinutes":null,"contentMix":null,"brand":null},
        "slides":[{"id":"first","claim":"A fixed regression fixture","evidence":[],"relationship":"single-claim","composition":"Title and body","editing":editing,"visual":{"kind":"none","purpose":"No generated media","fallback":"omit"}}]
    })
}

fn timeout_presentation_run() -> (tempfile::TempDir, Arc<AssistantHost>, Arc<RunControl>) {
    use super::super::media_tools::host::MediaRun;
    let (tmp, host) = fixture();
    let store = host.store(&identity().owner).unwrap();
    let sid = store.create_with_capabilities("test/model", false, "ofox", &[
        super::super::capabilities::CapabilityRef { id: "local_config".into(), version: 1 },
    ]).unwrap().session.id;
    let mut run = control(&sid);
    let state = Arc::get_mut(&mut run).unwrap();
    state.presentation_enabled = true;
    state.local_enabled = true;
    state.local_home = Some(tmp.path().to_path_buf());
    state.media = Some(MediaRun::new(Default::default()));
    host.register_run(run.clone(), host.generation(), || {
        store.mutate(&sid, |record| {
            record.session.active_run = Some(RunRef {
                run_id: run.run_id.clone(), operation_id: run.operation_id.clone(),
            });
            record.messages.push(serde_json::from_value(json!({
                "id":run.message_id,"role":"assistant","content":"",
                "createdAt":now(),"status":"streaming","runId":run.run_id,
            })).unwrap());
            Ok(())
        })
    }).unwrap();
    (tmp, host, run)
}

async fn timeout_freeze(host: &Arc<AssistantHost>, run: &Arc<RunControl>) {
    let result = host.dispatch_presentation_tool(run, &json!({
        "sessionId":run.session_id,"runId":run.run_id,"operationId":run.operation_id,
        "toolCallId":"timeout-freeze","name":"freeze_presentation_plan",
        "args":timeout_presentation_plan(),
    })).await.unwrap();
    assert_eq!(result["status"], "frozen");
}

fn timeout_conversation(
    host: Arc<AssistantHost>, run: Arc<RunControl>, pipe: tokio::io::DuplexStream,
) -> tokio::task::JoinHandle<Result<Finish, Failure>> {
    tokio::spawn(async move {
        let (output, mut input) = tokio::io::split(pipe);
        let (_sender, mut commands) = mpsc::channel(1);
        host.converse(&run, &json!({}), false, "run",
            (&mut input, &mut tokio::io::BufReader::new(output), &mut Vec::new()),
            (&mut Attempt::default(), &mut commands),
        ).await
    })
}

#[tokio::test(start_paused = true)]
async fn presentation_timeout_allows_slow_document_to_finish_after_fifteen_minutes() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (_tmp, host, run) = timeout_presentation_run();
    timeout_freeze(&host, &run).await;
    let store = host.store(&identity().owner).unwrap();
    let (ready, locked) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let lock = tokio::task::spawn_blocking({
        let (store, sid) = (store.clone(), run.session_id.clone());
        move || store.with_attachment_lock(&sid, || {
            ready.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok(())
        })
    });
    locked.await.unwrap();
    let (host_pipe, runtime_pipe) = tokio::io::duplex(8192);
    let turn = timeout_conversation(host.clone(), run.clone(), host_pipe);
    let mut runtime = BufReader::new(runtime_pipe);
    let mut line = String::new();
    runtime.read_line(&mut line).await.unwrap();
    runtime.write_all(&frame(&json!({
        "id":"long-document","method":"tool","params":{
            "sessionId":run.session_id,"runId":run.run_id,"operationId":run.operation_id,
            "toolCallId":"long-document","name":"create_document",
            "args":{"name":"completed.txt","format":"txt","content":"Generated before final publication"},
        },
    })).unwrap()).await.unwrap();
    let outputs = store.native_history_path(&run.session_id).unwrap().with_file_name("outputs");
    let wall = std::time::Instant::now();
    loop {
        let exists = std::fs::read_dir(&outputs).ok().is_some_and(|entries|
            entries.flatten().any(|entry| entry.path().join("out.txt").is_file()));
        if exists { break; }
        assert!(wall.elapsed() < Duration::from_secs(5), "document did not reach publication");
        tokio::task::yield_now().await;
    }
    // Mirrors the real PPT run: document work spans the old 900-second cap.
    tokio::time::advance(Duration::from_secs(16 * 60)).await;
    release.send(()).unwrap();
    lock.await.unwrap().unwrap();
    line.clear();
    if runtime.read_line(&mut line).await.unwrap() > 0 {
        assert!(serde_json::from_str::<Value>(&line).unwrap().get("result").is_some());
        runtime.write_all(&frame(&json!({
            "id":"run","result":{"status":"completed","entries":[]},
        })).unwrap()).await.unwrap();
    }
    let result = turn.await.unwrap();
    assert!(matches!(&result, Ok(Finish::Completed)),
        "an active presentation was cut off at the chat deadline: {:?}", result.as_ref().err().map(|e| &e.code));
    host.finish(&run, result).await;
    assert_eq!(store.load_record(&run.session_id).unwrap().attachments.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn presentation_timeout_still_stops_a_silent_model() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let (_tmp, host, run) = timeout_presentation_run();
    timeout_freeze(&host, &run).await;
    let (host_pipe, runtime_pipe) = tokio::io::duplex(8192);
    let turn = timeout_conversation(host.clone(), run.clone(), host_pipe);
    let mut runtime = BufReader::new(runtime_pipe);
    runtime.read_line(&mut String::new()).await.unwrap();
    tokio::time::advance(Duration::from_secs(301)).await;
    for _ in 0..10 { tokio::task::yield_now().await; }
    let stopped = turn.is_finished();
    if !stopped { turn.abort(); } else {
        assert_eq!(turn.await.unwrap().unwrap_err().code, "assistant_run_timeout");
    }
    host.release_run(&run);
    assert!(stopped, "a frozen PPT plan must not let a silent model wait for the whole extended budget");
}

#[tokio::test(start_paused = true)]
async fn presentation_timeout_does_not_extend_an_ordinary_chat_with_workflow_support() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let (_tmp, host, run) = timeout_presentation_run();
    // Eligible to plan a deck, as ordinary renderer turns are; no plan frozen.
    let (host_pipe, runtime_pipe) = tokio::io::duplex(8192);
    let turn = timeout_conversation(host.clone(), run.clone(), host_pipe);
    let mut runtime = BufReader::new(runtime_pipe);
    runtime.read_line(&mut String::new()).await.unwrap();
    tokio::time::advance(Duration::from_secs(901)).await;
    let result = turn.await.unwrap();
    assert_eq!(result.unwrap_err().code, "assistant_run_timeout");
    host.release_run(&run);
}

#[tokio::test(start_paused = true)]
async fn presentation_timeout_late_freeze_cannot_revive_an_expired_turn() {
    let (_tmp, host, run) = timeout_presentation_run();
    tokio::time::advance(Duration::from_secs(901)).await;
    let result = host.dispatch_presentation_tool(&run, &json!({
        "sessionId":run.session_id,"runId":run.run_id,"operationId":run.operation_id,
        "toolCallId":"late-freeze","name":"freeze_presentation_plan","args":timeout_presentation_plan(),
    })).await;
    assert_eq!(result.unwrap_err(), "assistant_run_timeout");
    assert!(tokio::time::Instant::now() >= run.run_deadline());
    host.release_run(&run);
}

#[tokio::test(start_paused = true)]
async fn presentation_timeout_freeze_persistence_cannot_cross_the_old_deadline() {
    let (_tmp, host, run) = timeout_presentation_run();
    let store = host.store(&identity().owner).unwrap();
    let (ready, locked) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let lock = tokio::task::spawn_blocking({
        let (store, sid) = (store.clone(), run.session_id.clone());
        move || store.mutate(&sid, |_| {
            ready.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok(())
        })
    });
    locked.await.unwrap();
    tokio::time::advance(Duration::from_secs(899)).await;
    let freezing = tokio::spawn({
        let (host, run) = (host.clone(), run.clone());
        async move { host.dispatch_presentation_tool(&run, &json!({
            "sessionId":run.session_id,"runId":run.run_id,"operationId":run.operation_id,
            "toolCallId":"slow-freeze","name":"freeze_presentation_plan","args":timeout_presentation_plan(),
        })).await }
    });
    let wall = std::time::Instant::now();
    while run.presentation.lock().unwrap().current().is_none() {
        assert!(wall.elapsed() < Duration::from_secs(5));
        tokio::task::yield_now().await;
    }
    assert!(!freezing.is_finished());
    tokio::time::advance(Duration::from_secs(2)).await;
    release.send(()).unwrap();
    lock.await.unwrap().unwrap();
    assert_eq!(freezing.await.unwrap().unwrap_err(), "assistant_run_timeout");
    assert!(tokio::time::Instant::now() >= run.run_deadline());
    host.release_run(&run);
}

#[tokio::test(start_paused = true)]
async fn presentation_timeout_progress_cannot_renew_the_one_hour_ceiling() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (_tmp, host, run) = timeout_presentation_run();
    timeout_freeze(&host, &run).await;
    let (host_pipe, runtime_pipe) = tokio::io::duplex(8192);
    let turn = timeout_conversation(host.clone(), run.clone(), host_pipe);
    let mut runtime = BufReader::new(runtime_pipe);
    runtime.read_line(&mut String::new()).await.unwrap();
    for round in 1..15 {
        tokio::time::advance(Duration::from_secs(240)).await;
        runtime.write_all(&frame(&json!({"method":"event","params":{
            "sessionId":run.session_id,"runId":run.run_id,"operationId":run.operation_id,
            "type":"model_activity","data":{"bytes":round},
        }})).unwrap()).await.unwrap();
        for _ in 0..20 { tokio::task::yield_now().await; }
        assert!(!turn.is_finished(), "real model progress must outlive the chat cap");
    }
    tokio::time::advance(Duration::from_secs(241)).await;
    assert_eq!(turn.await.unwrap().unwrap_err().code, "assistant_run_timeout");
    host.release_run(&run);
}

#[tokio::test(start_paused = true)]
async fn presentation_timeout_empty_deltas_and_duplicate_activity_do_not_renew_idle_wait() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (_tmp, host, run) = timeout_presentation_run();
    timeout_freeze(&host, &run).await;
    let (host_pipe, runtime_pipe) = tokio::io::duplex(8192);
    let turn = timeout_conversation(host.clone(), run.clone(), host_pipe);
    let mut runtime = BufReader::new(runtime_pipe);
    runtime.read_line(&mut String::new()).await.unwrap();
    for round in 0..2 {
        tokio::time::advance(Duration::from_secs(240)).await;
        for (kind, data) in [("model_activity",json!({"bytes":1})),
            ("thinking_start",json!({})),("text_delta",json!({"delta":""}))] {
            runtime.write_all(&frame(&json!({"method":"event","params":{
                "sessionId":run.session_id,"runId":run.run_id,"operationId":run.operation_id,
                "type":kind,"data":data,
            }})).unwrap()).await.unwrap();
        }
        for _ in 0..20 { tokio::task::yield_now().await; }
        assert!(!turn.is_finished(), "fixture must still be inside its last real-progress window ({round})");
    }
    tokio::time::advance(Duration::from_secs(61)).await;
    for _ in 0..20 { tokio::task::yield_now().await; }
    let stopped = turn.is_finished();
    if stopped { assert_eq!(turn.await.unwrap().unwrap_err().code, "assistant_run_timeout"); }
    else { turn.abort(); }
    host.release_run(&run);
    assert!(stopped, "status/empty/duplicate updates must not hide a stalled model");
}

#[tokio::test(start_paused = true)]
async fn presentation_timeout_expired_local_tools_do_not_end_the_document_turn() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (tmp, host, run) = timeout_presentation_run();
    timeout_freeze(&host, &run).await;
    tokio::time::advance(Duration::from_secs(16 * 60)).await;
    assert!(tokio::time::Instant::now() > run.deadline);
    assert!(tokio::time::Instant::now() < run.run_deadline());
    let (host_pipe, runtime_pipe) = tokio::io::duplex(8192);
    let turn = timeout_conversation(host.clone(), run.clone(), host_pipe);
    let mut runtime = BufReader::new(runtime_pipe);
    runtime.read_line(&mut String::new()).await.unwrap();
    runtime.write_all(&frame(&json!({"id":"expired-local","method":"tool","params":{
        "sessionId":run.session_id,"runId":run.run_id,"operationId":run.operation_id,
        "toolCallId":"expired-local","name":"stat","args":{"path":tmp.path()},
    }})).unwrap()).await.unwrap();
    let mut line = String::new();
    assert!(runtime.read_line(&mut line).await.unwrap() > 0, "local timeout is a tool error, not a dropped presentation");
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert!(reply["error"].to_string().contains("assistant_run_timeout"), "{reply}");
    runtime.write_all(&frame(&json!({"id":"run","result":{"status":"completed","entries":[]}})).unwrap()).await.unwrap();
    assert_eq!(turn.await.unwrap().unwrap(), Finish::Completed);
    host.release_run(&run);
}
