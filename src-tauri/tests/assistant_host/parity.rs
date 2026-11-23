//! Opt-in strict parity driver. The parent controller owns all paid credentials
//! and accounting; this process receives only a private loopback token.
use super::*;
#[cfg(windows)]
use cc_switch_lib::assistant::documents::runner;
use cc_switch_lib::assistant::{
    catalog::{Kind, Protocol},
    documents::engines::engines_at,
};
use sha2::{Digest, Sha256};

const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
const MODELS: [&str; 2] = ["deepseek/deepseek-v4.1-flash-1017", "openai/gpt-5-mini"];
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn setting(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("explicit {name} required"))
}
fn json_file(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn write_json(path: &Path, value: &Value, token: &str) {
    let text = serde_json::to_string_pretty(value)
        .unwrap()
        .replace(token, "[proxy-token-redacted]");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.write_all(text.as_bytes()).unwrap();
    file.sync_all().unwrap();
}

/// Run by the release packager before publishing any paid-task driver. The
/// fixed chat fixture uses the real runtime without document startup probes.
#[test]
#[ignore = "keyless release-driver packaging gate; fixed localhost replies only"]
#[serial_test::serial]
fn release_gateway_preflight() {
    assert!(std::env::var_os("OFOX_API_KEY").is_none());
    assert!(std::env::var_os("OFOX_REAL_EVAL_KEY").is_none());
    assert!(std::env::var_os("OFOX_PARITY_PROXY_TOKEN").is_none());
    let bundle = runtime_dir().expect("prepared runtime is mandatory");
    let fixture = title_runtime(&bundle);
    for apex in ["ofox.ai", "ofox.io"] {
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let unregistered = Gateway::start();
        let env = TestEnv::new(config.path(), fixture.path(), &[(apex, &gateway)]);
        assert!(env.allows_local_gateway(&gateway.base_url()));
        assert!(!env.allows_local_gateway(&unregistered.base_url()));
        // Even explicitly registered malformed/nonlocal bases cannot grant it.
        for denied in [
            "https://api.ofox.ai",
            "http://localhost",
            "http://localhost:0",
            "http://localhost:08088",
            "http://localhost:65536",
            "http://127.0.0.1:8088",
            "http://localhost.evil.com:8088",
            "http://localhost:8088/v1",
            "http://localhost:8088?query=1",
            "http://user@localhost:8088",
        ] {
            env.route(apex, denied.into());
            assert!(!env.allows_local_gateway(denied), "{denied}");
        }
        env.route(apex, gateway.base_url());
        let host = AssistantHost::new(env.clone());
        runtime().block_on(async {
            host.set_preferences(
                serde_json::from_value(json!({
                    "instructions":"","autoTitle":false,"notifyOnFinish":false,
                    "expectedOwner":owner(),"expectedGeneration":host.generation()
                }))
                .unwrap(),
            )
            .await
            .unwrap();
            let sid = create(&host).await;
            gateway.reply(Reply::Stream("Fixed release gateway response.".into()));
            let (_, end) = turn(
                &host,
                &env,
                request(&sid, "Fixed gateway preflight", json!({})),
            )
            .await;
            assert_eq!(end.data["status"], "completed");
            let requests = gateway.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].path, "/v1/chat/completions");
            assert_eq!(requests[0].authorization, format!("Bearer {KEY}"));
            assert_eq!(requests[0].body["model"], MODEL);
            let detail = host.get_session(&sid).await.unwrap();
            assert!(active(&detail)
                .contains(&("assistant".into(), "Fixed release gateway response.".into())));
            host.shutdown().await;
        });
    }
}
fn captured_calls(history: &str) -> Vec<Value> {
    history
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["type"] == "message" && entry["message"]["role"] == "assistant")
        .flat_map(|entry| {
            entry["message"]["content"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter(|part| part["type"] == "toolCall")
        .collect()
}

// Pair raw model arguments with an accepted tool result for the current Host
// plan. SDK-refused or truncated calls remain in the untouched native history.
fn captured_freeze(history: &str, plan: &Value) -> Value {
    let entries: Vec<Value> = history
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    entries
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, entry)| {
            let message = &entry["message"];
            (entry["type"] == "message"
                && message["role"] == "toolResult"
                && message["toolName"] == "freeze_presentation_plan"
                && message["isError"] == false
                && message["details"]["status"] == "frozen"
                && plan["planId"].is_string()
                && message["details"]["planId"] == plan["planId"]
                && message["details"]["revision"] == plan["revision"])
                .then(|| {
                    entries[..index]
                        .iter()
                        .rev()
                        .filter(|entry| {
                            entry["type"] == "message" && entry["message"]["role"] == "assistant"
                        })
                        .filter_map(|entry| entry["message"]["content"].as_array())
                        .flat_map(|parts| parts.iter().rev())
                        .find(|call| {
                            call["type"] == "toolCall"
                                && call["name"] == "freeze_presentation_plan"
                                && call["id"].is_string()
                                && call["id"] == message["toolCallId"]
                        })
                        .map(|call| call["arguments"].clone())
                })
                .flatten()
        })
        .unwrap_or(Value::Null)
}

#[test]
fn capture_uses_accepted_current_freeze_and_keeps_raw_alias() {
    let call = |id: &str, relationship: &str| json!({"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":id,"name":"freeze_presentation_plan","arguments":{"slides":[{"relationship":relationship}]}}]}});
    let result = |id: &str, plan: &str, error: bool| json!({"type":"message","message":{"role":"toolResult","toolName":"freeze_presentation_plan","toolCallId":id,"isError":error,"details":{"status":"frozen","planId":plan,"revision":1}}});
    let entries = [
        call("truncated", "invalid"),
        result("truncated", "current", true),
        call("old", "spatial"),
        result("old", "old-plan", false),
        call("accepted", "process"),
        result("accepted", "current", false),
        // Reusing an ID later is refused by the runtime; it must not replace
        // the raw arguments paired with the earlier successful result.
        call("accepted", "comparison"),
        result("accepted", "current", true),
    ];
    let history = entries
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let current = json!({"planId":"current","revision":1});
    assert_eq!(
        captured_freeze(&history, &current)["slides"][0]["relationship"],
        "process"
    );
    assert_eq!(captured_calls(&history).len(), 4);
    assert!(captured_freeze(&history, &json!({"planId":"missing","revision":1})).is_null());
    assert!(captured_freeze(&history, &json!({"planId":"current","revision":2})).is_null());
    assert!(captured_freeze(&history, &Value::Null).is_null());
}

fn verify_compiled_source(case: &Value) {
    assert_eq!(
        option_env!("OFOX_BUILD_SOURCE_CLEAN"),
        Some("true"),
        "paid/replay acceptance requires a compiled clean checkout"
    );
    assert_eq!(
        option_env!("OFOX_BUILD_SOURCE_SHA"),
        case["gitSha"].as_str(),
        "compiled source SHA differs from the campaign"
    );
    let sources = [
        (
            "host.rs",
            include_str!("../../src/services/assistant/host.rs"),
        ),
        (
            "host/local.rs",
            include_str!("../../src/services/assistant/host/local.rs"),
        ),
        (
            "documents/create.rs",
            include_str!("../../src/services/assistant/documents/create.rs"),
        ),
        (
            "documents/runner.rs",
            include_str!("../../src/services/assistant/documents/runner.rs"),
        ),
        (
            "documents/engines.rs",
            include_str!("../../src/services/assistant/documents/engines.rs"),
        ),
        (
            "documents/windows_sandbox.rs",
            include_str!("../../src/services/assistant/documents/windows_sandbox.rs"),
        ),
    ];
    for (name, contents) in sources {
        let key = format!("src-tauri/src/services/assistant/{name}");
        assert_eq!(
            sha(contents.replace("\r\n", "\n").as_bytes()),
            case["compiledSources"][key].as_str().unwrap(),
            "compiled driver source differs from the frozen campaign"
        );
    }
    assert_eq!(
        sha(include_str!("parity.rs").replace("\r\n", "\n").as_bytes()),
        case["compiledSources"]["src-tauri/tests/assistant_host/parity.rs"]
            .as_str()
            .unwrap(),
        "compiled parity protocol differs from the frozen campaign"
    );
    assert_eq!(
        sha(&std::fs::read(std::env::current_exe().unwrap()).unwrap()),
        case["driverSha256"].as_str().unwrap()
    );
}

fn actual_native_host() -> Value {
    #[cfg(windows)]
    {
        use windows::{
            Wdk::System::SystemServices::RtlGetVersion,
            Win32::System::SystemInformation::{OSVERSIONINFOEXW, OSVERSIONINFOW},
        };
        let mut version = OSVERSIONINFOEXW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOEXW>() as u32,
            ..Default::default()
        };
        unsafe {
            RtlGetVersion((&mut version as *mut OSVERSIONINFOEXW).cast::<OSVERSIONINFOW>())
                .ok()
                .expect("actual kernel version required");
        }
        assert_eq!(
            version.wProductType, 1,
            "client acceptance cannot run on Windows Server"
        );
        assert!(version.dwMajorVersion == 10 && version.dwBuildNumber >= 17763);
        json!({"platform":if version.dwBuildNumber>=22000 {"win11"}else{"win10"},
            "osRelease":format!("{}.{}.{}",version.dwMajorVersion,version.dwMinorVersion,version.dwBuildNumber),
            "productType":"WinNT","kernelProductType":version.wProductType,"architecture":std::env::consts::ARCH})
    }
    #[cfg(target_os = "macos")]
    {
        let release = std::process::Command::new("/usr/bin/uname")
            .arg("-r")
            .output()
            .expect("actual kernel version required");
        assert!(release.status.success());
        json!({"platform":"macos","osRelease":String::from_utf8_lossy(&release.stdout).trim(),"architecture":std::env::consts::ARCH})
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        panic!("native parity acceptance requires macOS or Windows clients")
    }
}

#[test]
fn capture_keeps_the_exact_python_input_and_rejects_non_tool_prose() {
    let history = [
        json!({"type":"message","message":{"role":"assistant","content":[
            {"type":"text","text":"not a command"},
            {"type":"toolCall","name":"create_document","arguments":{"format":"pptx","content":{"kind":"python","code":"exact code\n"}}}
        ]}}).to_string(),
        "malformed".into(),
    ].join("\n");
    let calls = captured_calls(&history);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["arguments"]["content"]["code"], "exact code\n");
}

#[cfg(windows)]
fn synthetic_plan(count: usize, source: Option<Value>) -> Value {
    let editing = json!({"text":true,"charts":false,"formulas":false,"diagrams":false});
    let slides: Vec<_> = (0..count).map(|index| json!({
        "id":format!("fixture-page-{}",index+1),"claim":"A synthetic native Host fixture",
        "evidence":[],"relationship":"single-claim","composition":"Native title and body",
        "editing":editing,"visual":{"kind":"none","purpose":"Synthetic native text","fallback":"omit"}
    })).collect();
    let mut plan = json!({
        "styleId":"navy","decisionSummary":"A trusted synthetic native pipeline fixture; no paid requests.",
        "requirements":{"audience":"Test reviewers","purpose":"Verify isolated Host document execution",
            "scenario":"business","language":"en","slideCount":count,"aspectRatio":"16:9","density":"low",
            "nativeEditing":editing,"visualNeeds":["none"],"roundTrip":"unverifiedAllowed",
            "durationMinutes":null,"contentMix":null,"brand":null},"slides":slides
    });
    if let Some(source) = source {
        plan["source"] = source;
    }
    plan
}

#[cfg(windows)]
async fn native_finished(env: &TestEnv, run_id: &str) -> Event {
    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    loop {
        if let Some(event) = env
            .events(run_id)
            .into_iter()
            .find(|event| event.r#type == "run_end")
        {
            return event;
        }
        assert!(Instant::now() < deadline, "native Host run did not finish");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Required native Windows coverage of the production Host/Work path, rather
/// than another direct LPAC-core probe. Missing engines or proof fail this test.
#[cfg(windows)]
#[test]
#[serial_test::serial]
fn windows_host_python_preview_and_page_three_source_edit_use_real_lpac() {
    let bundle = runtime_dir().expect("prepared Windows runtime is mandatory");
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    let before = runner::windows_document_sandbox_receipts(0)
        .last()
        .and_then(|receipt| receipt["sequence"].as_u64())
        .unwrap_or(0);
    runtime().block_on(async {
        host.set_preferences(serde_json::from_value(json!({
            "instructions":"","autoTitle":false,"notifyOnFinish":false,
            "expectedOwner":owner(),"expectedGeneration":host.generation()
        })).unwrap()).await.unwrap();
        let sid = create(&host).await;
        gateway.reply(Reply::ToolCalls(vec![("freeze_presentation_plan", synthetic_plan(8,None))]));
        gateway.reply(Reply::ToolCalls(vec![("create_document",json!({
            "name":"native-host-seed.pptx","format":"pptx","preview":true,
            "content":{"kind":"python","code":"prs=new_deck(title='Native Host fixture')\nfor page in range(1,9):\n    slide=content_slide(prs,f'HOST_PAGE_{page:03}',notes=f'Preserve note {page}')\n    text(slide,f'Synthetic fixture page {page}',1,2,10,1,size=24)\n"}
        }))]));
        gateway.reply(Reply::Stream("The isolated preview is ready for delivery.".into()));
        let extra = json!({"presentationWorkflowVersion":1,"presentationStyleId":"auto",
            "presentationVisualPolicy":{"mode":"off","maxImages":0,"maxCostUsd":"0"},
            "expectedOwner":owner(),"expectedGeneration":host.generation()});
        let first = host.send(request(&sid,"Create the trusted synthetic eight-page fixture.",extra.clone())).await.unwrap().run_id;
        let first_end = native_finished(&env,&first).await;
        assert_eq!(first_end.data["status"],"completed","{:?}",env.events(&first));
        let first_detail = host.get_session(&sid).await.unwrap();
        assert_eq!(first_detail.attachments.len(),1);
        let parent = &first_detail.attachments[0];
        let parent_path = session_dir(config.path(),&sid).join("attachments").join(&parent.id);
        let original = std::fs::read(&parent_path).unwrap();
        let version = sha(&original);
        gateway.reply(Reply::ToolCalls(vec![("freeze_presentation_plan",synthetic_plan(1,Some(json!({
            "attachmentId":parent.id,"version":version,"pageNumbers":[3]
        }))))]));
        gateway.reply(Reply::ToolCalls(vec![("create_document",json!({
            "name":"native-host-revision.pptx","format":"pptx","preview":true,
            "content":{"kind":"python","code":"prs=source_deck()\ntarget_slides(prs)[0].shapes.title.text='EDITED_HOST_PAGE_003'\n"}
        }))]));
        gateway.reply(Reply::Stream("Only source page three was revised.".into()));
        let mut edit = extra;
        edit["fileReferences"] = json!([{"attachmentId":parent.id,"version":version,"page":3}]);
        let second = host.send(request(&sid,"Revise only page 3 of the referenced source.",edit)).await.unwrap().run_id;
        let second_end = native_finished(&env,&second).await;
        assert_eq!(second_end.data["status"],"completed","{:?}",env.events(&second));
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.attachments.len(),2);
        let revision = detail.attachments.iter().find(|file|file.id != parent.id).unwrap();
        let revised = std::fs::read(session_dir(config.path(),&sid).join("attachments").join(&revision.id)).unwrap();
        assert_eq!(std::fs::read(&parent_path).unwrap(),original);
        fn members(bytes:&[u8])->std::collections::BTreeMap<String,Vec<u8>> {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            (0..zip.len()).filter_map(|index| {
                let mut part=zip.by_index(index).unwrap();
                if part.is_dir(){return None;}
                let name=part.name().unwrap().into_owned();
                let mut bytes=vec![];part.read_to_end(&mut bytes).unwrap();Some((name,bytes))
            }).collect()
        }
        let old = members(&original);let new = members(&revised);
        assert_eq!(old.keys().collect::<Vec<_>>(),new.keys().collect::<Vec<_>>());
        let changed:Vec<_>=old.iter().filter(|(name,bytes)|new.get(*name)!=Some(*bytes)).map(|(name,_)|name.as_str()).collect();
        assert_eq!(changed,["ppt/slides/slide3.xml"]);
        assert!(String::from_utf8_lossy(&new["ppt/slides/slide3.xml"]).contains("EDITED_HOST_PAGE_003"));
        let manifest = host.file_preview(serde_json::from_value(json!({
            "sessionId":sid,"attachmentId":revision.id,"expectedOwner":owner(),"expectedGeneration":host.generation()
        })).unwrap()).await.unwrap();
        assert_eq!(manifest["totalPages"],8);
        let page = host.file_preview_page(serde_json::from_value(json!({
            "sessionId":sid,"attachmentId":revision.id,"version":manifest["version"],"page":3,"scale":1,
            "expectedOwner":owner(),"expectedGeneration":host.generation()
        })).unwrap()).await.expect("the revised page must render through the production isolated preview path");
        assert_eq!(page["mimeType"],"image/jpeg");
        assert!(page["dataBase64"].as_str().is_some_and(|data|!data.is_empty()));
        host.shutdown().await;
    });
    let receipts = runner::windows_document_sandbox_receipts(before);
    let actual: Vec<_> = receipts
        .iter()
        .filter(|receipt| {
            receipt["trustedProbe"] == false
                && receipt["policy"] == "lpac-v1"
                && receipt["tokenVerified"] == true
                && receipt["childProcessDenied"] == true
                && receipt["lowImageDenied"] == true
                && receipt["jobStopped"] == true
                && receipt["cleanupComplete"] == true
                && receipt["taskRemoved"] == true
                && receipt["profileRemoved"] == true
                && receipt["cancelled"] == false
                && receipt["status"] == 0
        })
        .collect();
    assert!(
        actual
            .iter()
            .filter(|receipt| receipt["executable"]
                .as_str()
                .is_some_and(|exe| exe.replace('\\', "/").starts_with("node/")))
            .count()
            >= 2,
        "both Host Python executions need fresh nonprobe LPAC receipts: {receipts:?}"
    );
    assert!(
        actual.iter().any(|receipt| receipt["executable"]
            .as_str()
            .is_some_and(|exe| exe.contains("officecli"))),
        "Host OfficeCLI execution proof missing"
    );
}

/// Windows counterpart of the Python Node fixture tests, which intentionally
/// skip Windows. This executes trusted inputs through the real Host/LPAC path.
#[cfg(windows)]
#[test]
#[serial_test::serial]
fn windows_host_clip_contract_generates_native_math_and_renders_eight_pages() {
    let bundle = runtime_dir().expect("prepared Windows runtime is mandatory");
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    let before = runner::windows_document_sandbox_receipts(0)
        .last()
        .and_then(|receipt| receipt["sequence"].as_u64())
        .unwrap_or(0);
    let evidence = runtime().block_on(async {
        host.set_preferences(serde_json::from_value(json!({
            "instructions":"","autoTitle":false,"notifyOnFinish":false,
            "expectedOwner":owner(),"expectedGeneration":host.generation()
        })).unwrap()).await.unwrap();
        let sid = create(&host).await;
        let mut plan = synthetic_plan(8, None);
        plan["requirements"]["nativeEditing"]["formulas"] = json!(true);
        plan["requirements"]["nativeEditing"]["diagrams"] = json!(true);
        for page in plan["slides"].as_array_mut().unwrap() {
            page["editing"]["formulas"] = json!(true);
            page["editing"]["diagrams"] = json!(true);
        }
        // The model-facing alias must never cross the canonical Rust boundary.
        plan["slides"][3]["relationship"] = json!("process");
        gateway.reply(Reply::ToolCalls(vec![("freeze_presentation_plan", plan)]));
        gateway.reply(Reply::ToolCalls(vec![("create_document", json!({
            "name":"clip-contract.pptx","format":"pptx","preview":true,
            "content":{"kind":"python","code":include_str!("../../../assistant-runtime/test/fixtures/clip-loss-contract.py")}
        }))]));
        gateway.reply(Reply::Stream("The trusted CLIP contract fixture is ready.".into()));
        let run = host.send(request(&sid, "Generate the trusted eight-page CLIP contract fixture.", json!({
            "presentationWorkflowVersion":1,"presentationStyleId":"auto",
            "presentationVisualPolicy":{"mode":"off","maxImages":0,"maxCostUsd":"0"},
            "expectedOwner":owner(),"expectedGeneration":host.generation()
        }))).await.unwrap().run_id;
        let end = native_finished(&env, &run).await;
        assert_eq!(end.data["status"], "completed", "{:?}", env.events(&run));
        assert!(of_type(&env, &run, "tool_end").iter().all(|event| event["success"] == true));
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.attachments.len(), 1);
        let frozen = detail.messages.iter().rev().find_map(|m| m.presentation_plan.as_ref()).unwrap();
        assert_eq!(frozen["slides"][3]["relationship"], "sequence");
        let attachment = &detail.attachments[0];
        let bytes = std::fs::read(session_dir(config.path(), &sid).join("attachments").join(&attachment.id)).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut formula = String::new();
        archive.by_name("ppt/slides/slide5.xml").unwrap().read_to_string(&mut formula).unwrap();
        assert_eq!(formula.matches("<m:nary>").count(), 3);
        assert!(formula.contains("<m:scr m:val=\"script\""));
        assert!(formula.contains(">log</m:t>") && formula.contains(">exp</m:t>"));
        assert!(formula.contains("Equation fallback (rendered image, not editable math)"));
        let authority = json!({"sessionId":sid,"attachmentId":attachment.id,"expectedOwner":owner(),"expectedGeneration":host.generation()});
        let manifest = host.file_preview(serde_json::from_value(authority.clone()).unwrap()).await.unwrap();
        assert_eq!(manifest["totalPages"], 8);
        let evidence = std::env::var_os("OFOX_CLIP_CONTRACT_OUT").map(PathBuf::from);
        if let Some(out) = &evidence {
            assert!(out.is_absolute());
            std::fs::create_dir(out).expect("use a fresh evidence directory");
            std::fs::write(out.join("clip-contract.pptx"), &bytes).unwrap();
        }
        for number in 1..=8 {
            let mut page = authority.clone();
            page["version"] = manifest["version"].clone();
            page["page"] = json!(number);
            page["scale"] = json!(1);
            let image = host.file_preview_page(serde_json::from_value(page).unwrap()).await.unwrap();
            assert_eq!(image["mimeType"], "image/jpeg");
            use base64::Engine;
            let jpeg = base64::engine::general_purpose::STANDARD.decode(image["dataBase64"].as_str().unwrap()).unwrap();
            assert!(jpeg.starts_with(&[0xff, 0xd8]) && jpeg.len() > 100);
            if let Some(out) = &evidence { std::fs::write(out.join(format!("page-{number}.jpg")), jpeg).unwrap(); }
        }
        host.shutdown().await;
        evidence.map(|out| (out, json!({
                "scope":"trusted-offline-clip-contract-through-native-windows-host",
                "sourceSha":option_env!("OFOX_BUILD_SOURCE_SHA").unwrap_or("uncommitted"),
                "sourceClean":option_env!("OFOX_BUILD_SOURCE_CLEAN") == Some("true"),
                "artifactSha256":sha(&bytes),"generatedAttachments":1,"renderedPages":8,
                "canonicalFourthPageRelationship":"sequence","nativeSums":3,"paidModelCalls":0,
                "liveModelAcceptance":false,"powerPointRoundTrip":"pending"
            })))
    });
    let receipts = runner::windows_document_sandbox_receipts(before);
    let successful: Vec<_> = receipts
        .iter()
        .filter(|receipt| {
            receipt["trustedProbe"] == false
                && receipt["policy"] == "lpac-v1"
                && receipt["tokenVerified"] == true
                && receipt["childProcessDenied"] == true
                && receipt["lowImageDenied"] == true
                && receipt["jobStopped"] == true
                && receipt["cleanupComplete"] == true
                && receipt["taskRemoved"] == true
                && receipt["profileRemoved"] == true
                && receipt["cancelled"] == false
                && receipt["status"] == 0
        })
        .collect();
    assert!(
        successful.iter().any(|r| r["executable"]
            .as_str()
            .is_some_and(|p| p.replace('\\', "/").starts_with("node/"))),
        "fresh Python LPAC execution proof missing: {receipts:?}"
    );
    assert!(
        successful.iter().any(|r| r["executable"]
            .as_str()
            .is_some_and(|p| p.contains("officecli"))),
        "fresh OfficeCLI proof missing: {receipts:?}"
    );
    if let Some((out, mut receipt)) = evidence {
        receipt["isolationReceipts"] = json!(successful);
        receipt["status"] = json!("passed");
        write_json(&out.join("receipt.json"), &receipt, KEY);
    }
}

#[test]
#[ignore = "explicit #1701 root controller only; loopback proxy, bounded shared ledger, no real key in this process"]
#[serial_test::serial]
fn one_guarded_task() {
    assert_eq!(
        setting("OFOX_PARITY_ACK"),
        "USD10_TOTAL_USD1_TASK_NO_IMAGES"
    );
    assert!(std::env::var_os("OFOX_API_KEY").is_none());
    assert!(std::env::var_os("OFOX_REAL_EVAL_KEY").is_none());
    let case_path = PathBuf::from(setting("OFOX_PARITY_CASE"));
    let out = PathBuf::from(setting("OFOX_PARITY_OUT"));
    let bundle = PathBuf::from(setting("OFOX_PARITY_RUNTIME"));
    assert!(case_path.is_absolute() && out.is_absolute() && bundle.is_absolute());
    assert!(
        !out.join("outcome.json").exists(),
        "never rerun an earlier attempt"
    );
    let case = json_file(&case_path);
    verify_compiled_source(&case);
    let native_host = actual_native_host();
    let model_id = case["modelId"].as_str().unwrap();
    assert!(MODELS.contains(&model_id));
    let apex = case["selectedApex"].as_str().unwrap();
    assert!(["ofox.ai", "ofox.io"].contains(&apex));
    assert_eq!(case["expectedSlides"], 8);
    let mode = setting("OFOX_PARITY_MODE");
    assert!(mode == "live" || mode == "replay");
    if mode == "replay" {
        assert_eq!(
            native_host["platform"], "win10",
            "Win10 replay requires an actual Win10 client kernel"
        );
        assert_eq!(case["platform"], "win11");
    } else {
        assert!(
            matches!(native_host["platform"].as_str(), Some("macos" | "win11")),
            "unsupported live platform"
        );
        assert_eq!(native_host["platform"], case["platform"]);
    }
    let token = setting("OFOX_PARITY_PROXY_TOKEN");
    assert!(token.starts_with("parity-proxy-") && token.len() >= 48);
    let proxy = setting("OFOX_PARITY_PROXY_BASE");
    let url = reqwest::Url::parse(&proxy).unwrap();
    assert!(url.scheme() == "http" && url.host_str() == Some("localhost"));
    assert!(url.port().is_some() && url.path() == "/" && url.query().is_none());
    let catalog = json_file(&PathBuf::from(setting("OFOX_PARITY_CATALOG")));
    let mut model = parse_models(catalog, Source::V1)
        .into_iter()
        .find(|model| model.id == model_id && model.kind == Kind::Chat)
        .expect("exact catalog model required");
    assert!(
        model.tools
            && model
                .supported_endpoints
                .iter()
                .any(|endpoint| endpoint == "/v1/chat/completions")
    );
    model.max_output_tokens = 16000;
    model.supports_images = false;
    model.input_modalities = vec!["text".into()];
    model.protocol = Some(Protocol::OpenAiCompletions);
    model.supported_endpoints = vec!["/v1/chat/completions".into()];
    let engines = engines_at(&bundle);
    assert!(
        engines.officecli.is_some() && engines.python.is_some(),
        "isolated Python capability must be available"
    );
    #[cfg(windows)]
    let before_sequence = runner::windows_document_sandbox_receipts(0)
        .last()
        .and_then(|receipt| receipt["sequence"].as_u64())
        .unwrap_or(0);
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &bundle, &[(apex, &gateway)]);
    env.set_key(token.clone());
    *env.catalog.lock().unwrap() = vec![model];
    if mode == "live" {
        env.route(apex, proxy);
    }
    let host = AssistantHost::new(env.clone());
    let started = Instant::now();
    let replayed_capture_sha = (mode == "replay")
        .then(|| sha(&std::fs::read(PathBuf::from(setting("OFOX_PARITY_CAPTURE"))).unwrap()));
    let (sid, run, end, detail, actual, rendered, source_reference) = runtime().block_on(async {
        host.set_preferences(serde_json::from_value(json!({
            "instructions":"","autoTitle":false,"notifyOnFinish":false,
            "expectedOwner":owner(),"expectedGeneration":host.generation()
        })).unwrap()).await.unwrap();
        let sid = host.create_session(serde_json::from_value(json!({
            "modelId":model_id,"modelPicked":true,"webSearch":false
        })).unwrap()).await.unwrap().id;
        let mut extra = json!({
            "presentationWorkflowVersion":1,"presentationStyleId":"auto",
            "presentationVisualPolicy":{"mode":"off","maxImages":0,"maxCostUsd":"0"},
            "expectedOwner":owner(),"expectedGeneration":host.generation(),"uiLanguage":"zh"
        });
        let mut source_reference = Value::Null;
        if case["sourcePage"] == 3 {
            let source = PathBuf::from(setting("OFOX_PARITY_SOURCE"));
            assert_eq!(sha(&std::fs::read(&source).unwrap()), case["sourceSha"]);
            let token = host.accept_drop(vec![source]).await.unwrap().unwrap().0;
            let files = host.import_dropped(drop_request(&host, &sid, &token)).await.unwrap();
            assert_eq!(files.len(), 1);
            source_reference = json!({"attachmentId":files[0].id,"version":case["sourceSha"],"page":3});
            extra["fileReferences"] = json!([source_reference.clone()]);
        }
        if mode == "replay" {
            let capture = json_file(&PathBuf::from(setting("OFOX_PARITY_CAPTURE")));
            assert_eq!(capture["inputHash"], case["inputHash"]);
            assert_eq!(capture["gitSha"], case["gitSha"]);
            let mut freeze = capture["freeze"].clone();
            assert!(freeze.is_object());
            if !source_reference.is_null() {
                freeze["source"]["attachmentId"] = source_reference["attachmentId"].clone();
                freeze["source"]["version"] = source_reference["version"].clone();
            }
            gateway.reply(Reply::ToolCalls(vec![("freeze_presentation_plan", freeze)]));
            for create in capture["creates"].as_array().expect("captured Python inputs") {
                assert_eq!(create["content"]["kind"], "python");
                let mut create = create.clone();
                let args = create.as_object_mut().unwrap();
                args.remove("planId"); args.remove("revision");
                gateway.reply(Reply::ToolCalls(vec![("create_document", create)]));
            }
            gateway.reply(Reply::Stream("Captured inputs replayed by the application engine.".into()));
        }
        let run = host.send(request(&sid, case["prompt"].as_str().unwrap(), extra)).await.unwrap().run_id;
        let deadline = Instant::now() + Duration::from_secs(15 * 60);
        let end = loop {
            if let Some(end) = env.events(&run).into_iter().find(|event| event.r#type == "run_end") { break Some(end); }
            if Instant::now() >= deadline { host.cancel_all(); break None; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let detail = host.get_session(&sid).await.unwrap();
        let mut actual = None;
        let mut rendered = 0;
        for file in detail.attachments.iter().filter(|file| file.origin.as_deref() == Some("generated") && file.mime_type == PPTX) {
            let authority = json!({"sessionId":sid,"attachmentId":file.id,"expectedOwner":owner(),"expectedGeneration":host.generation()});
            if let Ok(manifest) = host.file_preview(serde_json::from_value(authority.clone()).unwrap()).await {
                actual = manifest["totalPages"].as_u64();
                for page in 1..=8 {
                    let mut page_request = authority.clone();
                    page_request["version"] = manifest["version"].clone();
                    page_request["page"] = json!(page); page_request["scale"] = json!(1);
                    if host.file_preview_page(serde_json::from_value(page_request).unwrap()).await.is_ok() { rendered += 1; }
                }
            }
        }
        host.shutdown().await;
        (sid, run, end, detail, actual, rendered, source_reference)
    });
    let history = std::fs::read_to_string(session_dir(config.path(), &sid).join("session.jsonl"))
        .unwrap_or_default();
    let calls = captured_calls(&history);
    let plan = detail
        .messages
        .iter()
        .rev()
        .find_map(|message| message.presentation_plan.clone())
        .unwrap_or(Value::Null);
    let freeze = captured_freeze(&history, &plan);
    let creates: Vec<_> = calls
        .iter()
        .filter(|call| {
            call["name"] == "create_document" && call["arguments"]["content"].is_object()
        })
        .map(|call| call["arguments"].clone())
        .collect();
    let python = !creates.is_empty()
        && creates
            .iter()
            .all(|call| call["content"]["kind"] == "python");
    let capture = json!({"version":1,"taskId":case["id"],"modelId":model_id,"inputHash":case["inputHash"],"gitSha":case["gitSha"],"generationHash":case["generationHash"],"sourceSha":case["sourceSha"],"freeze":freeze,"creates":creates});
    write_json(&out.join("capture.json"), &capture, &token);
    let files: Vec<_> = detail
        .attachments
        .iter()
        .filter(|file| file.origin.as_deref() == Some("generated") && file.mime_type == PPTX)
        .collect();
    let artifact = if files.len() == 1 {
        let bytes = std::fs::read(
            session_dir(config.path(), &sid)
                .join("attachments")
                .join(&files[0].id),
        )
        .unwrap();
        std::fs::write(out.join("artifact.pptx"), &bytes).unwrap();
        json!({"path":"artifact.pptx","sha256":sha(&bytes),"bytes":bytes.len()})
    } else {
        Value::Null
    };
    let engine_executed = python && files.len() == 1;
    #[cfg(windows)]
    let isolation_receipts = runner::windows_document_sandbox_receipts(before_sequence);
    #[cfg(not(windows))]
    let isolation_receipts: Vec<Value> = vec![];
    let isolation_verified = if cfg!(windows) {
        isolation_receipts.iter().any(|receipt| {
            receipt["policy"] == "lpac-v1"
                && receipt["trustedProbe"] == false
                && receipt["tokenVerified"] == true
                && receipt["childProcessDenied"] == true
                && receipt["lowImageDenied"] == true
                && receipt["jobStopped"] == true
                && receipt["cleanupComplete"] == true
                && receipt["taskRemoved"] == true
                && receipt["profileRemoved"] == true
                && receipt["cancelled"] == false
                && receipt["status"] == 0
                && receipt["executable"]
                    .as_str()
                    .is_some_and(|exe| exe.replace('\\', "/").starts_with("node/"))
        })
    } else {
        cfg!(target_os = "macos") && engine_executed
    };
    let status = end
        .as_ref()
        .and_then(|event| event.data["status"].as_str())
        .unwrap_or("timeout");
    let all_tools_succeeded = of_type(&env, &run, "tool_end")
        .iter()
        .all(|event| event["success"] == true);
    let outcome = json!({"taskId":case["id"],"modelId":model_id,"inputHash":case["inputHash"],
        "gitSha":option_env!("OFOX_BUILD_SOURCE_SHA").unwrap_or("unknown"),"compiledSourceSha":option_env!("OFOX_BUILD_SOURCE_SHA").unwrap_or("unknown"),
        "compiledSourceClean":option_env!("OFOX_BUILD_SOURCE_CLEAN")==Some("true"),"generationHash":case["generationHash"],
        "status":if all_tools_succeeded { status } else { "error" },"generatedAttachments":files.len(),"actualSlides":actual,
        "artifact":artifact,"route":if python {"python"}else{"unknown"},"engineExecuted":engine_executed,
        "isolationVerified":isolation_verified,"isolationReceipts":isolation_receipts,"sourceSha":case["sourceSha"],
        "sourceReference":source_reference,"frozenPlan":plan,"captureSha256":sha(&std::fs::read(out.join("capture.json")).unwrap()),
        "parentRevision":files.first().and_then(|file|file.details.as_ref()).and_then(|details|details.get("presentationRevision")),
        "checks":{"validation":engine_executed && all_tools_succeeded,"renderedPages":rendered},
        "usage":detail.usage,"seconds":started.elapsed().as_secs_f64(),"mode":mode,
        "replayedCaptureSha256":replayed_capture_sha,"nativeHost":native_host,
        "visualReview":{"status":"pending"},"targetApplicationReview":{"status":"pending"}});
    write_json(
        &out.join("detail.json"),
        &serde_json::to_value(&detail).unwrap(),
        &token,
    );
    write_json(
        &out.join("events.json"),
        &serde_json::to_value(env.events(&run)).unwrap(),
        &token,
    );
    write_json(&out.join("outcome.json"), &outcome, &token);
    std::fs::write(
        out.join("native-history.jsonl"),
        history.replace(&token, "[proxy-token-redacted]"),
    )
    .unwrap();
    assert!(
        status == "completed"
            && all_tools_succeeded
            && actual == Some(8)
            && files.len() == 1
            && rendered == 8
            && isolation_verified,
        "strict parity task failed; inspect outcome, never rerun automatically"
    );
}
