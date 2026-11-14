//! Debug-only native isolation harness. It never initializes the real app,
//! authentication, database, keychain, provider or installed application state.
use crate::services::assistant::{
    catalog,
    host::{Admit, AssistantHost, BoxFuture, Environment, Identity},
    store::AssistantStore,
    types::{id, Attachment, Event},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tauri::{Manager, State, WebviewUrl, WebviewWindowBuilder};

const OWNER: &str = "abababababababababababababababababababababababababababababababab";
const MAIN_URL: &str = "http://127.0.0.1:3034/.real-eval/ppt-completion-2026-11-10/native-ui.html";
const POLICY: &str = "default-src 'self'; frame-src about: http://127.0.0.1:43763; img-src 'self' data: assistant-media:; media-src 'self' assistant-media:; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' ipc: http://ipc.localhost http://127.0.0.1:3034 ws://127.0.0.1:3034; object-src 'none'; base-uri 'none'; form-action 'none'";

struct EvalEnvironment {
    config: PathBuf,
    runtime: PathBuf,
    keys: AtomicUsize,
    models: AtomicUsize,
    providers: AtomicUsize,
}
impl Environment for EvalEnvironment {
    fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
        Box::pin(async {
            Ok(Identity {
                owner: OWNER.into(),
                apex: "ofox.ai".into(),
            })
        })
    }
    fn api_key<'a>(
        &'a self,
        _: &'a Identity,
        _: Option<&'a str>,
        _: &'a Admit<'a>,
    ) -> BoxFuture<'a, Result<String, String>> {
        self.keys.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err("preview_eval_no_keys".into()) })
    }
    fn key_ready<'a>(&'a self, _: &'a Identity) -> BoxFuture<'a, bool> {
        self.keys.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { false })
    }
    fn stored_key<'a>(&'a self, _: &'a Identity) -> BoxFuture<'a, Option<String>> {
        self.keys.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { None })
    }
    fn models<'a>(
        &'a self,
        _: &'a str,
        _: catalog::Source,
    ) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>> {
        self.models.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err("preview_eval_no_models".into()) })
    }
    fn provider_prices<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        self.providers.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err("preview_eval_no_providers".into()) })
    }
    fn gateway_base(&self, _: &str) -> Result<String, String> {
        self.providers.fetch_add(1, Ordering::SeqCst);
        Err("preview_eval_no_gateway".into())
    }
    fn config_dir(&self) -> PathBuf {
        self.config.clone()
    }
    fn runtime_dir(&self) -> Result<PathBuf, String> {
        Ok(self.runtime.clone())
    }
    fn emit(&self, _: Event) {}
}

struct EvalState {
    environment: Arc<EvalEnvironment>,
    fixture: Value,
    receipt: PathBuf,
    webview_data: PathBuf,
    probes: AtomicUsize,
}
impl EvalState {
    fn status(&self) -> Value {
        json!({"identifier":"ai.ofox.desktop.preview-eval","keyCalls":self.environment.keys.load(Ordering::SeqCst),"modelCalls":self.environment.models.load(Ordering::SeqCst),"providerCalls":self.environment.providers.load(Ordering::SeqCst),"probeCalls":self.probes.load(Ordering::SeqCst),"fixture":self.fixture,"purpose":"isolated native preview verification; no real account or provider"})
    }
    fn persist(&self) -> Result<Value, String> {
        let value = self.status();
        crate::config::atomic_write_with(
            &self.receipt,
            &serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?,
            crate::config::AtomicOpts { mode: Some(0o600) },
        )
        .map_err(|error| error.to_string())?;
        Ok(value)
    }
}

#[tauri::command]
fn preview_eval_fixture(state: State<'_, EvalState>) -> Result<Value, String> {
    state.persist()?;
    Ok(state.fixture.clone())
}
#[tauri::command]
fn preview_eval_status(state: State<'_, EvalState>) -> Result<Value, String> {
    state.persist()
}
#[tauri::command]
fn preview_eval_probe(state: State<'_, EvalState>) -> Result<Value, String> {
    state.probes.fetch_add(1, Ordering::SeqCst);
    state.persist()
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn attach(
    store: &AssistantStore,
    sid: &str,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<Attachment, String> {
    if bytes.is_empty() || bytes.len() > 25 * 1024 * 1024 {
        return Err("preview_eval_case_size".into());
    }
    let attachment = Attachment {
        origin: None,
        source_message_id: None,
        id: id(),
        name: name.into(),
        mime_type: mime.into(),
        size_bytes: bytes.len() as u64,
        status: "ready".into(),
        reason: None,
        preview_url: None,
        details: None,
    };
    let directory = store.attachment_dir(sid)?;
    crate::config::atomic_write_with(
        &directory.join(&attachment.id),
        bytes,
        crate::config::AtomicOpts { mode: Some(0o600) },
    )
    .map_err(|error| error.to_string())?;
    store.mutate(sid, |record| {
        record.attachments.push(attachment.clone());
        Ok(())
    })?;
    Ok(attachment)
}

const INTERACTIVE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Bounded interactive fixture</title><style>body{font:22px system-ui;padding:24px;background:#f8fafc;color:#18293b}button,input{font:inherit;margin:8px;padding:10px}#count{font-size:48px}</style></head><body><h1>Native isolated HTML</h1><p>Self-contained public test content. No model or account.</p><button id="increment">Increment counter</button><output id="count">0</output><label>Local input <input id="input" value="synthetic" /></label><p id="echo">synthetic</p><script>let n=0;document.querySelector('#increment').onclick=()=>document.querySelector('#count').textContent=String(++n);document.querySelector('#input').oninput=e=>document.querySelector('#echo').textContent=e.target.value;</script></body></html>"#;
const ATTACK: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Bounded native boundary checks</title><style>body{font:17px system-ui;padding:20px;background:#fff;color:#18293b}li{padding:5px}button{font:inherit;padding:8px}</style></head><body><h1>Bounded isolation checks</h1><p>Only synthetic parent data, a task canary file and the task loopback server are targeted.</p><button id="run">Run checks</button><ol id="results"></ol><script>
const token=location.pathname.split('/')[1];let running=false;const rows=[];const violations=[];
addEventListener('securitypolicyviolation',event=>violations.push(event.violatedDirective));
function row(check,blocked,detail=''){rows.push({check,blocked,detail:String(detail).slice(0,120)});const li=document.createElement('li');li.textContent=check+': '+(blocked===true?'blocked':blocked===false?'NOT BLOCKED':'check native counter')+(detail?' · '+detail:'');document.querySelector('#results').append(li);}
async function checks(){if(running)return;running=true;
try{row('parentDOM',!parent.document.querySelector('#preview-eval-canary'),'unexpected access');}catch(e){row('parentDOM',true,e.name)}
try{row('parentStorage',parent.localStorage.getItem('preview-eval-canary')===null,'unexpected access');}catch(e){row('parentStorage',true,e.name)}
try{localStorage.setItem('preview-eval-own','synthetic');row('opaqueStorage',false);}catch(e){row('opaqueStorage',true,e.name)}
for(const[check,url]of[['fileFetch',__FILE_CANARY__],['crossOriginFetch','http://127.0.0.1:3034/.real-eval/ppt-completion-2026-11-10/native-canary.txt']]){try{await fetch(url,{mode:'no-cors'});row(check,false);}catch(e){row(check,true,e.name)}}
try{const popup=window.open('about:blank');row('popup',popup===null);if(popup)popup.close();}catch(e){row('popup',true,e.name)}
const child=document.createElement('iframe');child.src='http://127.0.0.1:3034/.real-eval/ppt-completion-2026-11-10/native-canary.txt';document.body.append(child);await new Promise(resolve=>setTimeout(resolve,350));row('nestedFrame',violations.some(value=>value.startsWith('frame-src')));child.remove();
try{if(!window.__TAURI_INTERNALS__?.invoke)row('nativeInvoke',true,'not exposed');else{const result=await Promise.race([window.__TAURI_INTERNALS__.invoke('preview_eval_probe').then(()=> 'REACHED',()=> 'denied'),new Promise(resolve=>setTimeout(()=>resolve('timeout'),650))]);row('nativeInvoke',result==='timeout'?null:result!=='REACHED',result)}}catch(e){row('nativeInvoke',true,e.name)}
try{if(!window.__TAURI_INTERNALS__?.postMessage)row('rawBridge',true,'not exposed');else{window.__TAURI_INTERNALS__.postMessage({cmd:'preview_eval_probe',callback:42424241,error:42424242,payload:{},options:{}});row('rawBridge',null,'attempted; authoritative main probe counter must stay 1')}}catch(e){row('rawBridge',true,e.name)}
try{if(window.ipc?.postMessage)window.ipc.postMessage(JSON.stringify({cmd:'preview_eval_probe',callback:42424243,error:42424244,payload:{},options:{customProtocolIpcBlocked:true}}));}catch(e){}
await new Promise(resolve=>setTimeout(resolve,700));parent.postMessage({type:'ofox-preview-eval-report',token,rows},'*');}
document.querySelector('#run').onclick=checks;
const navigate=document.createElement('button');navigate.textContent='Last check: navigate to app origin';navigate.id='self-navigation';navigate.onclick=()=>location.replace('http://127.0.0.1:3034/.real-eval/ppt-completion-2026-11-10/navigation-probe.html');document.body.append(navigate);
</script></body></html>"#;

fn prepare() -> Result<(Arc<AssistantHost>, EvalState, PathBuf), Box<dyn std::error::Error>> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()?;
    let private = repository.join(".real-eval");
    let runtime = PathBuf::from(
        std::env::var_os("OFOX_PREVIEW_EVAL_RUNTIME")
            .ok_or("provide an explicit task-private OFOX_PREVIEW_EVAL_RUNTIME")?,
    )
    .canonicalize()?;
    if !runtime.starts_with(&private) {
        return Err("runtime must be a private copy in this checkout".into());
    }
    let directory = private
        .join("ppt-completion-2026-11-10")
        .join(format!("native-harness-{}", id()));
    crate::config::create_private_dir_all(&directory)?;
    let config = directory.join("config");
    crate::config::create_private_dir_all(&config)?;
    let environment = Arc::new(EvalEnvironment {
        config: config.clone(),
        runtime,
        keys: AtomicUsize::new(0),
        models: AtomicUsize::new(0),
        providers: AtomicUsize::new(0),
    });
    let host = AssistantHost::new(environment.clone());
    let store = AssistantStore::for_owner(&config, OWNER).map_err(std::io::Error::other)?;
    let sid = store
        .create("preview-eval/no-model", false)
        .map_err(std::io::Error::other)?
        .session
        .id;
    let mut attachments = vec![];
    let mut cases = vec![];
    let mut supplied: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if supplied.is_empty() {
        if let Some(case) = std::env::var_os("OFOX_PREVIEW_EVAL_CASE") {
            supplied.push(case.into());
        }
    }
    if supplied.len() > 12 {
        return Err("at most twelve explicit task cases".into());
    }
    for case in supplied {
        let case = case.canonicalize()?;
        if !case.starts_with(&private) {
            return Err("case must be task-owned, never a user document".into());
        }
        let mime = match case.extension().and_then(|value| value.to_str()) {
            Some("pptx") => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Some("html") => "text/html",
            _ => return Err("case must be PPTX or HTML".into()),
        };
        if std::fs::metadata(&case)?.len() > 25 * 1024 * 1024 {
            return Err("case exceeds preview limit".into());
        }
        let bytes = std::fs::read(&case)?;
        attachments.push(
            attach(
                &store,
                &sid,
                case.file_name()
                    .unwrap()
                    .to_str()
                    .ok_or("invalid case name")?,
                mime,
                &bytes,
            )
            .map_err(std::io::Error::other)?,
        );
        cases.push(
            json!({"name":case.file_name().unwrap().to_string_lossy(),"sha256":hash(&bytes)}),
        );
    }
    attachments.push(
        attach(
            &store,
            &sid,
            "self-contained-interactive.html",
            "text/html",
            INTERACTIVE.as_bytes(),
        )
        .map_err(std::io::Error::other)?,
    );
    let canary = directory.join("synthetic-file-canary.txt");
    std::fs::write(&canary, "synthetic native preview canary; no user data")?;
    let file_url = tauri::Url::from_file_path(&canary)
        .map_err(|_| "invalid canary URL")?
        .to_string();
    let attack = ATTACK.replace("__FILE_CANARY__", &serde_json::to_string(&file_url)?);
    attachments.push(
        attach(
            &store,
            &sid,
            "bounded-isolation-checks.html",
            "text/html",
            attack.as_bytes(),
        )
        .map_err(std::io::Error::other)?,
    );
    let state = EvalState {
        environment,
        fixture: json!({"sessionId":sid,"expectedOwner":OWNER,"expectedGeneration":host.generation(),"files":attachments,"cases":cases,"canary":"synthetic-parent-only"}),
        receipt: directory.join("native-receipt.json"),
        webview_data: directory.join("webview"),
        probes: AtomicUsize::new(0),
    };
    state.persist().map_err(std::io::Error::other)?;
    println!(
        "{}",
        json!({"previewEvalDirectory":directory,"nativeReceipt":state.receipt,"identifier":"ai.ofox.desktop.preview-eval"})
    );
    Ok((host, state, directory))
}

/// Run only when explicitly launched as the debug evaluation example.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let (host, state, directory) = prepare()?;
    let mut context = crate::app_context();
    context.config_mut().identifier = "ai.ofox.desktop.preview-eval".into();
    context.config_mut().product_name = Some("Ofox Preview Eval".into());
    context.config_mut().build.dev_url = Some("http://127.0.0.1:3034".parse()?);
    context.config_mut().app.windows.clear();
    context.config_mut().app.app_directories_override = Some(
        tauri::utils::config::AppDirectoriesOverride::Root(directory.join("app-data")),
    );
    context.config_mut().app.security.csp = Some(tauri::utils::config::Csp::Policy(POLICY.into()));
    context.config_mut().app.security.dev_csp =
        Some(tauri::utils::config::Csp::Policy(POLICY.into()));
    tauri::Builder::default()
        .manage(host)
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            crate::commands::assistant_file_preview,
            crate::commands::assistant_file_preview_page,
            crate::commands::assistant_file_preview_html,
            preview_eval_fixture,
            preview_eval_status,
            preview_eval_probe
        ])
        .setup(|app| {
            let data = app.state::<EvalState>().webview_data.clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(MAIN_URL.parse()?))
                .title("Ofox isolated preview evaluation")
                .inner_size(900.0, 640.0)
                .data_directory(data)
                .incognito(true)
                .build()?;
            Ok(())
        })
        .run(context)?;
    Ok(())
}
