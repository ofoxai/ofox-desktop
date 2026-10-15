use super::{
    jobs::Jobs,
    process, provenance,
    receipt::{self, Proof, Receipt},
    release::{self, Release},
    runtime::{self, Runtime},
    state::{self, Frozen},
    Failure,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{process::Command, sync::watch};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Install,
    Update,
    Uninstall,
}
#[derive(Clone)]
struct Plan {
    root: PathBuf,
    root_id: Vec<(PathBuf, u64, u64)>,
    frozen: Frozen,
    runtime: Runtime,
    node_version: String,
    action: Action,
    release: Option<Release>,
    work: Arc<tempfile::TempDir>,
    work_hash: String,
}
fn jobs() -> &'static Arc<Jobs<Plan>> {
    static JOBS: OnceLock<Arc<Jobs<Plan>>> = OnceLock::new();
    JOBS.get_or_init(Jobs::new)
}
fn supported(tool: &str) -> bool {
    tool == "copilot" && cfg!(all(target_os = "macos", target_arch = "aarch64"))
}
fn root() -> PathBuf {
    crate::config::get_app_config_dir().join("catalog/copilot")
}
/// A read-only copy indicator, independent of shell discovery or account state.
pub(crate) fn inventory() -> Option<Value> {
    if !supported("copilot") {
        return None;
    }
    let root = root();
    match state::observe(&root) {
        Ok(Frozen {
            proof: Proof::Installed { version, .. },
            ..
        }) => Some(
            json!({"state":"managed","version":version,"path":root.join("prefix/bin/copilot")}),
        ),
        Ok(_) => None,
        Err(_) if fs::symlink_metadata(root.join("prefix")).is_ok() => {
            Some(json!({"state":"needsReview","path":root.join("prefix")}))
        }
        Err(_) => None,
    }
}
fn error(_failure: Failure) -> String {
    "catalogLifecycle.unavailable".into()
}
fn scratch() -> Result<Arc<tempfile::TempDir>, Failure> {
    let temp = tempfile::Builder::new()
        .prefix("ofox-catalog-")
        .tempdir()
        .map_err(|_| Failure::Unreadable)?;
    let path = fs::canonicalize(temp.path()).map_err(|_| Failure::Unreadable)?;
    for dir in ["tmp", "cache"] {
        fs::create_dir(path.join(dir)).map_err(|_| Failure::Unreadable)?;
    }
    for name in ["user.npmrc", "global.npmrc"] {
        crate::config::atomic_write(&path.join(name),b"registry=https://registry.npmjs.org/\n@github:registry=https://registry.npmjs.org/\nignore-scripts=true\n").map_err(|_|Failure::Unreadable)?;
    }
    Ok(Arc::new(temp))
}
fn work_path(work: &tempfile::TempDir) -> Result<PathBuf, Failure> {
    fs::canonicalize(work.path()).map_err(|_| Failure::Unreadable)
}
fn candidates(home: &Path) -> Vec<PathBuf> {
    let mut result = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|path| path.is_absolute())
                .take(128)
                .map(|path| path.join("node"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    result.extend([
        home.join(".local/share/fnm/aliases/default/bin/node"),
        home.join("Library/Application Support/fnm/aliases/default/bin/node"),
        PathBuf::from("/opt/homebrew/bin/node"),
        PathBuf::from("/usr/local/bin/node"),
    ]);
    for (base, suffix) in [
        (home.join(".nvm/versions/node"), "bin/node"),
        (
            home.join(".local/share/fnm/node-versions"),
            "installation/bin/node",
        ),
        (
            home.join("Library/Application Support/fnm/node-versions"),
            "installation/bin/node",
        ),
    ] {
        if let Ok(entries) = fs::read_dir(base) {
            let mut versions = entries
                .take(64)
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.path().join(suffix))
                .collect::<Vec<_>>();
            versions.sort();
            versions.reverse();
            result.extend(versions);
        }
    }
    result
}
fn command(runtime: &Runtime, work: &Path) -> Command {
    let mut command = Command::new(&runtime.node);
    command
        .env_clear()
        .envs(runtime::environment(work, &runtime.node))
        .current_dir(work);
    command
}
async fn probe(
    runtime: &Runtime,
    work: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<String, Failure> {
    let mut command = command(runtime, work);
    command.args(["-p","JSON.stringify({version:process.versions.node,platform:process.platform,arch:process.arch,execPath:process.execPath})"]);
    let output = process::run(command, cancel, Duration::from_secs(4)).await?;
    runtime::parse_probe(runtime, &output)
}
async fn discover(work: &Path) -> Result<(Runtime, String), Failure> {
    let (_sender, cancel) = watch::channel(false);
    let mut seen = std::collections::BTreeSet::new();
    let mut probes = 0;
    for node in candidates(&crate::config::get_home_dir()) {
        let Ok(real) = fs::canonicalize(&node) else {
            continue;
        };
        if !seen.insert(real) {
            continue;
        }
        let Ok(runtime) = Runtime::pair(&node) else {
            continue;
        };
        probes += 1;
        if probes > 8 {
            break;
        }
        if let Ok(version) = probe(&runtime, work, cancel.clone()).await {
            if Runtime::pair(&node)? == runtime {
                return Ok((runtime, version));
            }
        }
    }
    Err(Failure::InvalidPackage)
}
async fn fetch(url: &str, max: usize) -> Result<Vec<u8>, Failure> {
    use futures::StreamExt;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| Failure::Process)?;
    let response = client.get(url).send().await.map_err(|_| Failure::Process)?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|size| size > max as u64)
    {
        return Err(Failure::Limit);
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Failure::Process)?;
        if chunk.len() > max.saturating_sub(bytes.len()) {
            return Err(Failure::Limit);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
#[cfg(unix)]
fn root_id(root: &Path) -> Result<Vec<(PathBuf, u64, u64)>, Failure> {
    use std::os::unix::fs::MetadataExt;
    receipt::private_path(root, "prefix")?;
    let mut current = PathBuf::new();
    let mut identities = Vec::new();
    for component in root.components() {
        current.push(component);
        let meta = fs::symlink_metadata(&current).map_err(|_| Failure::Unreadable)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(Failure::Changed);
        }
        identities.push((current.clone(), meta.dev(), meta.ino()));
    }
    Ok(identities)
}
#[cfg(not(unix))]
fn root_id(_root: &Path) -> Result<Vec<(PathBuf, u64, u64)>, Failure> {
    Err(Failure::InvalidPackage)
}
fn actions(proof: &Proof) -> Vec<Action> {
    match proof {
        Proof::Installed { .. } => vec![Action::Update, Action::Uninstall],
        _ => vec![Action::Install],
    }
}
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
fn view(
    tool: &str,
    root: &Path,
    state: Result<Frozen, Failure>,
    runtime: Option<(&Runtime, &str)>,
) -> Value {
    let (status, version, available, reason) = match state {
        Ok(state) => match &state.proof {
            Proof::Installed { version, .. } => (
                "managed",
                Some(version.clone()),
                actions(&state.proof),
                None,
            ),
            _ => ("missing", None, actions(&state.proof), None),
        },
        Err(_) => ("needsReview", None, vec![], Some("ownershipUnknown")),
    };
    let launch = runtime.filter(|_| status == "managed").map(|(runtime, _)| {
        format!(
            "{} {}",
            quote(&runtime.node),
            quote(&root.join("prefix/lib/node_modules/@github/copilot/npm-loader.js"))
        )
    });
    json!({"toolId":tool,"supported":supported(tool),"prefix":root.join("prefix"),"state":status,"version":version,"nodeVersion":runtime.map(|(_,version)|version),"reason":reason.or(if runtime.is_none(){Some("runtimeUnavailable")}else{None}),"actions":if runtime.is_some(){available}else{vec![]},"launchCommand":launch})
}
pub(crate) async fn inspect(tool: &str) -> Result<Value, String> {
    if !supported(tool) {
        return Ok(
            json!({"toolId":tool,"supported":false,"prefix":null,"state":"unavailable","reason":"platform","actions":[]}),
        );
    }
    let root = root();
    let work = scratch().map_err(error)?;
    let work_path = work_path(&work).map_err(error)?;
    let runtime = discover(&work_path).await.ok();
    Ok(view(
        tool,
        &root,
        state::observe(&root),
        runtime
            .as_ref()
            .map(|(runtime, version)| (runtime, version.as_str())),
    ))
}
pub(crate) async fn prepare(tool: &str, action: Action) -> Result<Value, String> {
    if !supported(tool) {
        return Err(error(Failure::InvalidPackage));
    }
    let root = root();
    receipt::private_path(&root, "prefix").map_err(error)?;
    crate::config::create_private_dir_all(&root).map_err(|_| error(Failure::Unreadable))?;
    let root_id = root_id(&root).map_err(error)?;
    let frozen = state::observe(&root).map_err(error)?;
    if !actions(&frozen.proof).contains(&action) {
        return Err(error(Failure::Changed));
    }
    let work = scratch().map_err(error)?;
    let work_path = work_path(&work).map_err(error)?;
    let (runtime, node_version) = discover(&work_path).await.map_err(error)?;
    let release = if action == Action::Uninstall {
        None
    } else {
        let bytes = fetch("https://registry.npmjs.org/@github%2fcopilot/latest", 65536)
            .await
            .map_err(error)?;
        let manifest =
            serde_json::from_slice(&bytes).map_err(|_| error(Failure::InvalidPackage))?;
        let release = release::parse(&manifest, &node_version).map_err(error)?;
        let bytes = fetch(&release.url, 16 * 1024 * 1024).await.map_err(error)?;
        release::verify(&release, &bytes).map_err(error)?;
        crate::config::atomic_write(&work_path.join("copilot.tgz"), &bytes)
            .map_err(|_| error(Failure::Unreadable))?;
        Some(release)
    };
    let version = release
        .as_ref()
        .map(|release| release.version.clone())
        .or_else(|| match &frozen.proof {
            Proof::Installed { version, .. } => Some(version.clone()),
            _ => None,
        })
        .ok_or_else(|| error(Failure::Changed))?;
    let work_hash = provenance::tree_identity(&work_path).map_err(error)?.0;
    let plan = Plan {
        root: root.clone(),
        root_id,
        frozen,
        runtime,
        node_version: node_version.clone(),
        action,
        release,
        work,
        work_hash,
    };
    validate(&plan).map_err(error)?;
    let cmd = npm(&plan, &work_path);
    let display = std::iter::once(cmd.as_std().get_program())
        .chain(cmd.as_std().get_args())
        .map(|part| quote(Path::new(part)))
        .collect::<Vec<_>>()
        .join(" ");
    let id = jobs().reserve(plan).map_err(error)?;
    Ok(
        json!({"operationId":id,"toolId":tool,"action":action,"prefix":root.join("prefix"),"version":version,"nodeVersion":node_version,"command":display,"expiresAtMs":chrono::Utc::now().timestamp_millis()+60_000}),
    )
}
fn validate(plan: &Plan) -> Result<(), Failure> {
    if root_id(&plan.root)? != plan.root_id
        || state::observe(&plan.root)? != plan.frozen
        || Runtime::pair(&plan.runtime.node)? != plan.runtime
    {
        return Err(Failure::Changed);
    }
    if let Some(release) = &plan.release {
        release::verify(
            release,
            &provenance::read_regular(
                &work_path(&plan.work)?.join("copilot.tgz"),
                16 * 1024 * 1024,
            )?,
        )?;
    }
    if provenance::tree_identity(&work_path(&plan.work)?)?.0 != plan.work_hash {
        return Err(Failure::Changed);
    }
    Ok(())
}
fn validate_execution(plan: &Plan, id: &str) -> Result<(), Failure> {
    if root_id(&plan.root)? != plan.root_id
        || state::current(&plan.root)? != plan.frozen.proof
        || Runtime::pair(&plan.runtime.node)? != plan.runtime
        || provenance::tree_identity(&work_path(&plan.work)?)?.0 != plan.work_hash
        || receipt::read(&plan.root.join("receipt.json"))?
            != Some(Receipt {
                format: 1,
                operation: Some(id.into()),
                proof: plan.frozen.proof.clone(),
            })
    {
        return Err(Failure::Changed);
    }
    Ok(())
}
fn npm(plan: &Plan, work: &Path) -> Command {
    let mut cmd = command(&plan.runtime, work);
    cmd.arg(&plan.runtime.npm)
        .arg(if plan.action == Action::Uninstall {
            "uninstall"
        } else {
            "install"
        })
        .arg("--global")
        .arg("--prefix")
        .arg(plan.root.join("prefix"))
        .args([
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--no-progress",
            "--package-lock=false",
            "--registry=https://registry.npmjs.org/",
            "--@github:registry=https://registry.npmjs.org/",
        ])
        .arg(if plan.action == Action::Uninstall {
            PathBuf::from("@github/copilot")
        } else {
            work.join("copilot.tgz")
        });
    cmd
}
async fn execute(
    plan: Plan,
    id: &str,
    mut cancel: watch::Receiver<bool>,
    gate: Arc<Mutex<()>>,
    stage: impl Fn(&str),
) -> Value {
    let mut dirty = false;
    let attempt = async {
        stage("verify");
        validate(&plan)?;
        let work = work_path(&plan.work)?;
        if probe(&plan.runtime, &work, cancel.clone()).await? != plan.node_version {
            return Err(Failure::Changed);
        }
        validate(&plan)?;
        if *cancel.borrow() {
            return Err(Failure::Cancelled);
        }
        receipt::write(
            &plan.root.join("receipt.json"),
            &Receipt {
                format: 1,
                operation: Some(id.into()),
                proof: plan.frozen.proof.clone(),
            },
        )?;
        dirty = true;
        stage("execute");
        process::run_checked(
            npm(&plan, &work),
            cancel.clone(),
            Duration::from_secs(180),
            &gate,
            || validate_execution(&plan, id),
        )
        .await?;
        stage("cleanup");
        stage("inspect");
        if root_id(&plan.root)? != plan.root_id
            || Runtime::pair(&plan.runtime.node)? != plan.runtime
        {
            return Err(Failure::Changed);
        }
        let proof = state::current(&plan.root)?;
        match (&proof, &plan.release, plan.action) {
            (Proof::Installed { version, .. }, Some(release), Action::Install | Action::Update)
                if version == &release.version =>
            {
                let mut command = command(&plan.runtime, &work);
                command
                    .arg(
                        plan.root
                            .join("prefix/lib/node_modules/@github/copilot/npm-loader.js"),
                    )
                    .arg("--version");
                let bytes =
                    process::run_guarded(command, cancel.clone(), Duration::from_secs(8), &gate)
                        .await?;
                let text = std::str::from_utf8(&bytes).map_err(|_| Failure::InvalidPackage)?;
                if !text
                    .trim()
                    .starts_with(&format!("GitHub Copilot CLI {version}."))
                {
                    return Err(Failure::InvalidPackage);
                }
                if state::current(&plan.root)? != proof {
                    return Err(Failure::Changed);
                }
            }
            (Proof::Empty { .. } | Proof::Missing, None, Action::Uninstall) => {}
            _ => return Err(Failure::Changed),
        }
        if Runtime::pair(&plan.runtime.node)? != plan.runtime
            || root_id(&plan.root)? != plan.root_id
        {
            return Err(Failure::Changed);
        }
        if receipt::read(&plan.root.join("receipt.json"))?
            != Some(Receipt {
                format: 1,
                operation: Some(id.into()),
                proof: plan.frozen.proof.clone(),
            })
        {
            return Err(Failure::Changed);
        }
        receipt::write(
            &plan.root.join("receipt.json"),
            &Receipt {
                format: 1,
                operation: None,
                proof,
            },
        )?;
        dirty = false;
        Ok::<(), Failure>(())
    }
    .await;
    stage("cleanup");
    if dirty
        && !matches!(attempt, Err(Failure::Cleanup))
        && receipt::read(&plan.root.join("receipt.json"))
            .ok()
            .flatten()
            == Some(Receipt {
                format: 1,
                operation: Some(id.into()),
                proof: plan.frozen.proof.clone(),
            })
        && root_id(&plan.root).ok().as_ref() == Some(&plan.root_id)
        && Runtime::pair(&plan.runtime.node).ok().as_ref() == Some(&plan.runtime)
        && state::current(&plan.root).ok().as_ref() == Some(&plan.frozen.proof)
    {
        dirty = receipt::write(
            &plan.root.join("receipt.json"),
            &Receipt {
                format: 1,
                operation: None,
                proof: plan.frozen.proof.clone(),
            },
        )
        .is_err();
    }
    let outcome = match attempt {
        Ok(()) => "ok",
        Err(Failure::Cancelled) if !dirty => "cancelled",
        Err(_) if dirty => "partial",
        Err(_) => "failed",
    };
    let code = attempt.as_ref().err().map(Failure::code);
    // The actual postcondition wins a cancellation that arrives after verified success.
    let current_runtime = Runtime::pair(&plan.runtime.node)
        .ok()
        .filter(|runtime| runtime == &plan.runtime);
    let installation = view(
        "copilot",
        &plan.root,
        state::observe(&plan.root),
        current_runtime
            .as_ref()
            .map(|runtime| (runtime, plan.node_version.as_str())),
    );
    let _ = cancel.borrow_and_update();
    json!({"operationId":id,"state":outcome,"code":code,"installation":installation})
}
pub(crate) async fn run(
    id: &str,
    stage: impl Fn(&str) + Send + Sync + 'static,
) -> Result<Value, String> {
    let owned_id = id.to_string();
    let result = jobs()
        .start(id, move |plan, cancel, gate| async move {
            execute(plan, &owned_id, cancel, gate, stage).await
        })
        .map_err(error)?;
    result.await.map_err(|_| error(Failure::Changed))
}
pub(crate) fn cancel(id: &str) -> Result<bool, String> {
    jobs().cancel(id).map_err(error)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[tokio::test]
    #[ignore = "Opt-in official downloads in a temporary prefix; no global installation changes"]
    async fn native_copilot_private_lifecycle_acceptance() {
        async fn prepare_at(
            root: &Path,
            node: &Path,
            action: Action,
            version: Option<&str>,
        ) -> Plan {
            let frozen = state::observe(root).unwrap();
            let runtime = Runtime::pair(node).unwrap();
            let work = scratch().unwrap();
            let path = work_path(&work).unwrap();
            let (_sender, cancel) = watch::channel(false);
            let node_version = probe(&runtime, &path, cancel).await.unwrap();
            let release = if let Some(version) = version {
                let bytes = fetch(
                    &format!("https://registry.npmjs.org/@github%2fcopilot/{version}"),
                    65536,
                )
                .await
                .unwrap();
                let manifest = serde_json::from_slice(&bytes).unwrap();
                let release = release::parse(&manifest, &node_version).unwrap();
                let bytes = fetch(&release.url, 16 * 1024 * 1024).await.unwrap();
                release::verify(&release, &bytes).unwrap();
                crate::config::atomic_write(&path.join("copilot.tgz"), &bytes).unwrap();
                Some(release)
            } else {
                None
            };
            let work_hash = provenance::tree_identity(&path).unwrap().0;
            Plan {
                root: root.into(),
                root_id: root_id(root).unwrap(),
                frozen,
                runtime,
                node_version,
                action,
                release,
                work,
                work_hash,
            }
        }
        let node = PathBuf::from(
            std::env::var_os("OFOX_CATALOG_NATIVE_NODE")
                .expect("set an existing native arm64 Node >=22 binary"),
        );
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let sentinel = root.join("user-data");
        fs::create_dir(&sentinel).unwrap();
        fs::write(sentinel.join("settings.json"), "user settings").unwrap();
        fs::write(sentinel.join("auth.json"), "fixture authorization").unwrap();
        fs::write(sentinel.join("session.jsonl"), "fixture session").unwrap();
        for (action, version) in [(Action::Install, "1.0.97"), (Action::Update, "1.0.98")] {
            let plan = prepare_at(&root, &node, action, Some(version)).await;
            let (_sender, cancel) = watch::channel(false);
            let result = execute(
                plan,
                &uuid::Uuid::new_v4().to_string(),
                cancel,
                Arc::new(Mutex::new(())),
                |_| {},
            )
            .await;
            assert_eq!(result["state"], "ok", "{result}");
            assert_eq!(result["installation"]["version"], version);
            eprintln!("native Copilot {action:?}: {version}, state ok");
        }
        let plan = prepare_at(&root, &node, Action::Uninstall, None).await;
        let (sender, cancel) = watch::channel(false);
        let result = execute(
            plan,
            &uuid::Uuid::new_v4().to_string(),
            cancel,
            Arc::new(Mutex::new(())),
            |stage| {
                if stage == "execute" {
                    sender.send_replace(true);
                }
            },
        )
        .await;
        assert_eq!(result["state"], "cancelled", "{result}");
        assert_eq!(result["installation"]["state"], "managed");
        let plan = prepare_at(&root, &node, Action::Uninstall, None).await;
        let (_sender, cancel) = watch::channel(false);
        let result = execute(
            plan,
            &uuid::Uuid::new_v4().to_string(),
            cancel,
            Arc::new(Mutex::new(())),
            |_| {},
        )
        .await;
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["installation"]["state"], "missing");
        assert!(!root.join("prefix/bin/copilot").exists());
        for (file, bytes) in [
            ("settings.json", "user settings"),
            ("auth.json", "fixture authorization"),
            ("session.jsonl", "fixture session"),
        ] {
            assert_eq!(fs::read_to_string(sentinel.join(file)).unwrap(), bytes);
        }
        eprintln!(
            "native Copilot cancel/removal: retained settings, authorization and session fixtures"
        );
    }
    #[tokio::test]
    async fn execute_stage_cannot_redirect_the_manager_into_an_outside_directory() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let prefix = super::super::provenance::tests::fixture();
        fs::rename(prefix.path(), root.join("prefix")).unwrap();
        receipt::write(
            &root.join("receipt.json"),
            &Receipt {
                format: 1,
                operation: None,
                proof: state::current(&root).unwrap(),
            },
        )
        .unwrap();
        let runtime_dir = tempfile::tempdir().unwrap();
        let runtime_root = fs::canonicalize(runtime_dir.path()).unwrap();
        let node = super::super::runtime::tests::fixture(&runtime_root);
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        let spawned = outside.join("manager-ran");
        let probe = json!({"version":"22.12.0","platform":"darwin","arch":"arm64","execPath":node});
        fs::write(&node,format!("#!/bin/sh\nif [ \"$1\" = \"-p\" ]; then printf '%s' {}; exit 0; fi\nprintf ran > {}\n",quote(Path::new(&probe.to_string())),quote(&spawned))).unwrap();
        fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = Runtime::pair(&node).unwrap();
        let work = scratch().unwrap();
        let work_hash = provenance::tree_identity(&work_path(&work).unwrap())
            .unwrap()
            .0;
        let plan = Plan {
            root: root.clone(),
            root_id: root_id(&root).unwrap(),
            frozen: state::observe(&root).unwrap(),
            runtime,
            node_version: "22.12.0".into(),
            action: Action::Uninstall,
            release: None,
            work,
            work_hash,
        };
        let (_sender, cancel) = watch::channel(false);
        let result = execute(
            plan,
            &uuid::Uuid::new_v4().to_string(),
            cancel,
            Arc::new(Mutex::new(())),
            |stage| {
                if stage == "execute" {
                    fs::rename(root.join("prefix"), root.join("saved-prefix")).unwrap();
                    symlink(&outside, root.join("prefix")).unwrap();
                }
            },
        )
        .await;
        assert_eq!(result["state"], "partial");
        assert!(
            !spawned.exists(),
            "manager executed after its prefix was replaced"
        );
    }
    #[tokio::test]
    async fn invalidated_runtime_never_returns_old_launch_or_mutation_capabilities() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let prefix = super::super::provenance::tests::fixture();
        fs::rename(prefix.path(), root.join("prefix")).unwrap();
        receipt::write(
            &root.join("receipt.json"),
            &Receipt {
                format: 1,
                operation: None,
                proof: state::current(&root).unwrap(),
            },
        )
        .unwrap();
        let runtime_dir = tempfile::tempdir().unwrap();
        let runtime_root = fs::canonicalize(runtime_dir.path()).unwrap();
        let node = super::super::runtime::tests::fixture(&runtime_root);
        let runtime = Runtime::pair(&node).unwrap();
        let work = scratch().unwrap();
        let work_hash = provenance::tree_identity(&work_path(&work).unwrap())
            .unwrap()
            .0;
        let plan = Plan {
            root: root.clone(),
            root_id: root_id(&root).unwrap(),
            frozen: state::observe(&root).unwrap(),
            runtime,
            node_version: "22.12.0".into(),
            action: Action::Uninstall,
            release: None,
            work,
            work_hash,
        };
        fs::remove_file(node).unwrap();
        let (_sender, cancel) = watch::channel(false);
        let result = execute(
            plan,
            &uuid::Uuid::new_v4().to_string(),
            cancel,
            Arc::new(Mutex::new(())),
            |_| {},
        )
        .await;
        assert_eq!(result["state"], "failed");
        assert_eq!(result["installation"]["state"], "managed");
        assert_eq!(result["installation"]["actions"], json!([]));
        assert!(result["installation"]["launchCommand"].is_null());
        assert!(result["installation"]["nodeVersion"].is_null());
    }
}
