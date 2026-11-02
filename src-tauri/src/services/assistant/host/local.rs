//! Local capability authority; runtime callbacks cannot change these grants.
use super::*;
use crate::services::assistant::{
    capabilities,
    tools::{
        approval::{Binding, Choice},
        commands,
        context::LocalPaths,
        diagnostics, editing, execute, files, journal,
        policy::{self, Decision, PermissionMode},
        registry,
        scope::{Access, Scope},
        transaction,
    },
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Default)]
pub(super) struct LocalState {
    mode: PermissionMode,
    // Exact tool + arguments, never a wildcard for arbitrary commands/paths.
    grants: HashSet<String>,
    folders: Vec<PathBuf>,
}

pub(super) static MUTATIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type CallResult = (String, Option<Result<Value, String>>);
#[derive(Default)]
pub(super) struct Calls(pub Mutex<HashMap<String, CallResult>>);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalRequest {
    pub session_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub approval_id: String,
    pub choice: Choice,
    pub expected_owner: String,
    pub expected_generation: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionRequest {
    pub session_id: String,
    pub mode: PermissionMode,
    #[serde(default)]
    pub confirmed_full_access: bool,
    pub expected_owner: String,
    pub expected_generation: u64,
}

impl AssistantHost {
    pub(crate) fn local_permission(&self, owner: &str, sid: &str) -> PermissionMode {
        self.local_states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&session_key(owner, sid))
            .map(|state| state.mode)
            .unwrap_or_default()
    }

    pub(crate) fn local_pending_approvals(&self, owner: &str, sid: &str) -> Vec<Value> {
        let runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(run) = runs
            .get(sid)
            .filter(|run| run.identity.owner == owner && !run.cancelled())
        else {
            return vec![];
        };
        self.local_approvals.outstanding(&self.local_binding(run)).into_iter().map(|data|json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"type":"approval","data":data})).collect()
    }

    pub(crate) fn clear_local_permission(&self, owner: &str, sid: &str) {
        self.local_states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&session_key(owner, sid));
    }

    fn local_binding(&self, run: &RunControl) -> Binding {
        Binding {
            owner: run.identity.owner.clone(),
            session_id: run.session_id.clone(),
            run_id: run.run_id.clone(),
            operation_id: run.operation_id.clone(),
            generation: self.generation(),
        }
    }

    pub async fn respond_approval(&self, request: ApprovalRequest) -> Result<(), String> {
        let identity = self.identity().await?;
        self.ensure_current(&identity, request.expected_generation)
            .await?;
        if identity.owner != request.expected_owner {
            return Err(IDENTITY_CHANGED.into());
        }
        let runs = self.runs.lock().map_err(|_| STATE)?;
        let run = runs
            .get(&request.session_id)
            .ok_or("assistant_approval_expired")?;
        self.validate_run(run)?;
        if run.run_id != request.run_id
            || run.operation_id != request.operation_id
            || run.identity != identity
        {
            return Err("assistant_approval_expired".into());
        }
        self.local_approvals.respond(
            &request.approval_id,
            &self.local_binding(run),
            request.choice,
        )
    }

    pub async fn set_permission(
        &self,
        request: PermissionRequest,
    ) -> Result<PermissionMode, String> {
        let identity = self.identity().await?;
        self.ensure_current(&identity, request.expected_generation)
            .await?;
        if identity.owner != request.expected_owner {
            return Err(IDENTITY_CHANGED.into());
        }
        if request.mode == PermissionMode::FullAccess && !request.confirmed_full_access {
            return Err("assistant_full_access_confirmation_required".into());
        }
        let record = blocking({
            let store = self.store(&identity.owner)?;
            let sid = request.session_id.clone();
            move || store.load_record(&sid)
        })
        .await?;
        if !capabilities::enabled(&record.session.capabilities) {
            return Err("assistant_capability_disabled".into());
        }
        self.ensure_current(&identity, request.expected_generation)
            .await?;
        // Serialize grants with registration and identity revocation.
        let runs = self.runs.lock().map_err(|_| STATE)?;
        if self.generation() != request.expected_generation {
            return Err(IDENTITY_CHANGED.into());
        }
        if let Some(run) = runs.get(&request.session_id) {
            run.cancel.send_replace(true);
        }
        let mut states = self.local_states.lock().map_err(|_| STATE)?;
        let state = states
            .entry(session_key(&identity.owner, &request.session_id))
            .or_default();
        state.mode = request.mode;
        state.grants.clear();
        Ok(request.mode)
    }

    pub(super) async fn dispatch_tool(
        &self,
        run: &Arc<RunControl>,
        params: &Value,
    ) -> Result<Value, String> {
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err(IDENTITY_CHANGED.into());
        }
        let call: registry::Call =
            serde_json::from_value(params.clone()).map_err(|_| "assistant_invalid_tool")?;
        if call.session_id != run.session_id
            || call.run_id != run.run_id
            || call.operation_id != run.operation_id
        {
            return Err("assistant_runtime_identity_mismatch".into());
        }
        if call.tool_call_id.is_empty() || call.tool_call_id.len() > 200 {
            return Err("assistant_invalid_tool".into());
        }
        let mut tool = registry::get(&call.name)?;
        if matches!(tool.name, "bash" | "powershell") {
            tool.irreversible = commands::irreversible(call.args["command"].as_str().unwrap_or(""));
        }
        registry::validate_args(tool, &call.args)?;
        let record = blocking({
            let store = self.store(&run.identity.owner)?;
            let sid = run.session_id.clone();
            move || store.load_record(&sid)
        })
        .await?;
        capabilities::validate(&record.session.assistant_id, &record.session.capabilities)?;
        if !run.local_enabled
            || !capabilities::enabled(&record.session.capabilities)
            || record.session.deleted_at.is_some()
            || run.compact_only
        {
            return Err("assistant_capability_disabled".into());
        }
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&json!([call.name, call.args])).map_err(|_| PROTOCOL)?
            )
        );
        {
            let mut calls = run.local_calls.0.lock().map_err(|_| STATE)?;
            if let Some((previous, result)) = calls.get(&call.tool_call_id) {
                if previous != &fingerprint {
                    return Err("assistant_tool_call_conflict".into());
                }
                return result
                    .clone()
                    .unwrap_or_else(|| Err("unknown_outcome".into()));
            }
            // Pi read/edit issue multiple callbacks for one model tool call.
            // The runtime counts model calls; this is the independent host
            // callback ceiling (stat/read/write), not that model budget.
            if calls.len() >= capabilities::local_config().limits.max_tool_calls * 4 {
                return Err("assistant_tool_limit".into());
            }
            calls.insert(call.tool_call_id.clone(), (fingerprint.clone(), None));
        }
        let result = self
            .execute_local(run, tool, &call.args, &fingerprint, &call.tool_call_id)
            .await;
        run.local_calls
            .0
            .lock()
            .map_err(|_| STATE)?
            .insert(call.tool_call_id, (fingerprint, Some(result.clone())));
        result
    }

    async fn execute_local(
        &self,
        run: &Arc<RunControl>,
        tool: registry::Tool,
        args: &Value,
        signature: &str,
        call_id: &str,
    ) -> Result<Value, String> {
        if tool.name == "probe_write" && cfg!(test) {
            self.authorize_local(
                run,
                tool,
                args,
                signature,
                call_id,
                tool.operation(true, false, false),
            )
            .await?;
            return Ok(json!({"approved":true}));
        }
        if tool.name == "request_folder" {
            self.emit(run, "folder_request", json!({"toolCallId":call_id}));
            return Ok(
                json!({"waitingForUser":true,"instruction":"Ask the user to choose the problem folder using the folder card, then continue."}),
            );
        }
        let (paths, scope) = self
            .local_scope(&run.identity.owner, &run.session_id)
            .await?;
        let requested = args["path"]
            .as_str()
            .or_else(|| args["folder"].as_str())
            .map(PathBuf::from);
        let checked = requested.as_ref().map(|p| scope.check(p)).transpose()?;
        let access = checked.as_ref().map_or(Access::Inside, |p| p.access);
        self.authorize_local(
            run,
            tool,
            args,
            signature,
            call_id,
            tool.operation(
                access == Access::Inside,
                access == Access::Forbidden
                    || (matches!(tool.name, "bash" | "powershell")
                        && commands::forbidden(args["command"].as_str().unwrap_or(""))),
                matches!(tool.name, "bash" | "powershell")
                    && commands::read_only(args["command"].as_str().unwrap_or(""), cfg!(windows)),
            ),
        )
        .await?;
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err(IDENTITY_CHANGED.into());
        }
        let path = checked
            .map(|p| p.path)
            .unwrap_or_else(|| paths.home.clone());
        let outside = access == Access::Outside;
        let operation = async {
            match tool.name {
                "repair_launcher" | "rebind_tool" | "set_tool_model" => {
                    let selected = registry::supported_tool(
                        args["tool"].as_str().ok_or("assistant_invalid_tool")?,
                    )?;
                    let _guard = MUTATIONS.lock().await;
                    let key = self.env.local_journal_key().await?;
                    self.validate_run(run)?;
                    if self.identity().await? != run.identity {
                        return Err(IDENTITY_CHANGED.into());
                    }
                    let journal = journal::Journal::open(
                        &self.local_changes_dir(&run.identity.owner, &run.session_id)?,
                        &key,
                        &session_key(&run.identity.owner, &run.session_id),
                    )?;
                    let previous_changes = journal.changes().len();
                    let context = Arc::new(transaction::Context {
                        journal: Mutex::new(journal),
                        tool: selected.as_str().into(),
                        apex: Some(run.identity.apex.clone()),
                        operation: tool.name.into(),
                        scope,
                        cancel: run.cancel.subscribe(),
                        deadline: run.deadline,
                        error: Mutex::new(None),
                        observed: AtomicBool::new(false),
                    });
                    let result = transaction::ACTIVE
                        .scope(
                            context.clone(),
                            self.env.local_mutate(
                                tool.name,
                                selected,
                                args["model"].as_str(),
                                call_id,
                            ),
                        )
                        .await;
                    let finish_failed = context.error.lock().map_err(|_| STATE)?.is_some();
                    let changes: Vec<_> = context
                        .journal
                        .lock()
                        .map_err(|_| STATE)?
                        .changes()
                        .into_iter()
                        .skip(previous_changes)
                        .collect();
                    if finish_failed {
                        return Err("assistant_mutation_outcome_unknown".into());
                    }
                    let mut result = match result {
                        Ok(value) => value,
                        Err(error) if !changes.iter().any(|change| change.changed) => {
                            return Err(error)
                        }
                        Err(_) => return Err("assistant_local_partial_change".into()),
                    };
                    if !context.observed.load(Ordering::SeqCst) {
                        return Err("assistant_journal_unavailable".into());
                    }
                    result["changed"] = json!(changes.iter().any(|change| change.changed));
                    result["changes"] = json!(changes);
                    Ok(result)
                }
                "install_tool" | "update_tool" | "uninstall_tool" => {
                    let selected = registry::supported_tool(
                        args["tool"].as_str().ok_or("assistant_invalid_tool")?,
                    )?;
                    self.env
                        .local_mutate(tool.name, selected, None, call_id)
                        .await
                }
                "bash" | "powershell" => {
                    let a: registry::ExecArgs = registry::decode(args)?;
                    execute::run(&a.command, &path, &paths.home, a.timeout).await
                }
                "write" | "edit" => {
                    let a: registry::WriteArgs = registry::decode(args)?;
                    let _guard = MUTATIONS.lock().await;
                    let key = self.env.local_journal_key().await?;
                    self.validate_run(run)?;
                    if self.identity().await? != run.identity {
                        return Err(IDENTITY_CHANGED.into());
                    }
                    let directory = self.local_changes_dir(&run.identity.owner, &run.session_id)?;
                    let context = session_key(&run.identity.owner, &run.session_id);
                    // This small transaction is synchronous: cancellation cannot detach
                    // a background writer after the host declares the run stopped.
                    let _bindings = crate::services::ofox_bind::BIND_LOCK.lock().await;
                    let _workbuddy = crate::workbuddy_config::CONFIG_LOCK.lock().await;
                    self.validate_run(run)?;
                    if self.identity().await? != run.identity {
                        return Err(IDENTITY_CHANGED.into());
                    }
                    let _io = crate::config_io::lock();
                    if scope.check(&path)?.path != path {
                        return Err("assistant_restore_conflict".into());
                    }
                    let original = if path.exists() {
                        files::read_original(&scope, &path, outside)?
                    } else {
                        String::new()
                    };
                    if a.expected
                        .as_ref()
                        .is_some_and(|old| *old != crate::redact::scrub(&original))
                    {
                        return Err("assistant_restore_conflict".into());
                    }
                    let content = editing::preserve_credentials(&original, &a.content)?;
                    let mut journal = journal::Journal::open(&directory, &key, &context)?;
                    let id =
                        journal.prepare(tool.name, std::slice::from_ref(&path), Value::Null)?;
                    let result = crate::config::atomic_write(&path, content.as_bytes())
                        .map_err(|_| "assistant_write_failed".to_string());
                    journal
                        .finish(&id, Value::Null)
                        .map_err(|_| "assistant_mutation_outcome_unknown")?;
                    if result.is_err()
                        && journal
                            .changes()
                            .last()
                            .is_some_and(|change| change.changed)
                    {
                        return Err("assistant_local_partial_change".into());
                    }
                    result?;
                    Ok(
                        json!({"changed":journal.changes().last().is_some_and(|change|change.changed),"changeId":id,"path":path}),
                    )
                }
                "read" | "stat" | "ls" | "find" | "grep" => {
                    let args = args.clone();
                    blocking(move || match tool.name {
                        "read" => Ok(json!(files::read(&scope, &path, outside)?)),
                        "stat" => files::stat(&scope, &path, outside),
                        "ls" => Ok(json!(files::list(
                            &scope,
                            &path,
                            registry::decode::<registry::ListArgs>(&args)?.limit,
                            outside
                        )?)),
                        "find" => {
                            let a: registry::SearchArgs = registry::decode(&args)?;
                            Ok(json!(files::find(
                                &scope, &path, &a.pattern, a.limit, outside
                            )?))
                        }
                        "grep" => {
                            let a: registry::SearchArgs = registry::decode(&args)?;
                            Ok(json!(files::grep(
                                &scope, &path, &a.pattern, a.limit, outside
                            )?))
                        }
                        _ => Err("assistant_unknown_tool".into()),
                    })
                    .await
                }
                "tool_status" | "diagnose_in_folder" | "verify_in_folder" => {
                    let tool = registry::supported_tool(
                        args["tool"].as_str().ok_or("assistant_invalid_tool")?,
                    )?;
                    let mut result = diagnostics::probe(tool, &path, &paths.home).await;
                    if args.get("folder").is_some() {
                        result["folderMetadata"] = blocking(move || {
                            Ok(diagnostics::folder_metadata(&scope, &path, outside))
                        })
                        .await?;
                    } else {
                        result["binding"] = self.env.local_binding_status(tool).await?;
                        result["configPath"] = json!(crate::commands::get_tool_config_file_path(
                            tool.as_str().into()
                        )
                        .map_err(|_| "assistant_local_unavailable")?);
                    }
                    Ok(result)
                }
                _ => Err("assistant_unknown_tool".into()),
            }
        };
        let mut cancel = run.cancel.subscribe();
        let mut result = tokio::select! {
            biased;
            _=cancel.changed()=>return Err(CANCELLED.into()),
            _=tokio::time::sleep_until(run.deadline)=>return Err("assistant_run_timeout".into()),
            result=operation=>result?,
        };
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err(IDENTITY_CHANGED.into());
        }
        fn scrub(value: &mut Value) {
            match value {
                Value::String(text) => *text = crate::redact::scrub(text).into_owned(),
                Value::Array(items) => items.iter_mut().for_each(scrub),
                Value::Object(items) => items.values_mut().for_each(scrub),
                _ => {}
            }
        }
        scrub(&mut result);
        scrub_secrets_in(&mut result, &self.known_keys());
        Ok(result)
    }

    pub async fn local_changes(
        &self,
        request: DeleteRequest,
        restore: bool,
    ) -> Result<Vec<journal::Change>, String> {
        let identity = self.identity().await?;
        self.ensure_current(&identity, request.expected_generation)
            .await?;
        if identity.owner != request.expected_owner {
            return Err(IDENTITY_CHANGED.into());
        }
        let _navigation = if restore {
            Some(self.begin_navigation(&request.session_id)?)
        } else {
            None
        };
        let directory = self.local_changes_dir(&identity.owner, &request.session_id)?;
        if !directory.join("journal.enc").exists() {
            return Ok(vec![]);
        }
        let _guard = MUTATIONS.lock().await;
        let key = self.env.local_journal_key().await?;
        self.ensure_current(&identity, request.expected_generation)
            .await?;
        let mut journal = journal::Journal::open(
            &directory,
            &key,
            &session_key(&identity.owner, &request.session_id),
        )?;
        if restore {
            let _bindings = crate::services::ofox_bind::BIND_LOCK.lock().await;
            let _workbuddy = crate::workbuddy_config::CONFIG_LOCK.lock().await;
            self.ensure_current(&identity, request.expected_generation)
                .await?;
            self.env.local_restore(&mut journal)?;
        }
        Ok(journal.changes())
    }

    pub(super) fn local_changes_dir(&self, owner: &str, sid: &str) -> Result<PathBuf, String> {
        if self
            .store(owner)?
            .load_record(sid)?
            .session
            .deleted_at
            .is_some()
        {
            return Err("assistant_session_deleted".into());
        }
        Ok(
            super::super::store::owner_dir(&self.env.config_dir(), owner)?
                .join(sid)
                .join("changes"),
        )
    }

    pub(super) async fn local_scope(
        &self,
        owner: &str,
        sid: &str,
    ) -> Result<(LocalPaths, Scope), String> {
        let paths = self.env.local_paths().await?;
        let folders = self
            .local_states
            .lock()
            .map_err(|_| STATE)?
            .get(&session_key(owner, sid))
            .map(|s| s.folders.clone())
            .unwrap_or_default();
        let scope = paths.scope(&folders)?;
        Ok((paths, scope))
    }

    pub async fn add_dropped_local_folder(
        &self,
        authority: DeleteRequest,
        token: String,
    ) -> Result<String, String> {
        let identity = self.identity().await?;
        self.ensure_current(&identity, authority.expected_generation)
            .await?;
        if identity.owner != authority.expected_owner {
            return Err(IDENTITY_CHANGED.into());
        }
        let paths = self.drops.lock().map_err(|_| STATE)?.take(
            &token,
            &identity.owner,
            authority.expected_generation,
            std::time::Instant::now(),
        )?;
        if paths.len() != 1 || !paths[0].is_dir() {
            return Err("assistant_invalid_folder".into());
        }
        self.add_local_folder(authority, paths[0].to_string_lossy().into_owned())
            .await
    }

    pub async fn add_local_folder(
        &self,
        authority: DeleteRequest,
        folder: String,
    ) -> Result<String, String> {
        let identity = self.identity().await?;
        self.ensure_current(&identity, authority.expected_generation)
            .await?;
        if identity.owner != authority.expected_owner {
            return Err(IDENTITY_CHANGED.into());
        }
        let record = blocking({
            let store = self.store(&identity.owner)?;
            let sid = authority.session_id.clone();
            move || store.load_record(&sid)
        })
        .await?;
        if !capabilities::enabled(&record.session.capabilities) {
            return Err("assistant_capability_disabled".into());
        }
        let (_, mut scope) = self
            .local_scope(&identity.owner, &authority.session_id)
            .await?;
        let folder = scope.add_folder(Path::new(&folder))?;
        let _runs = self.runs.lock().map_err(|_| STATE)?;
        if self.generation() != authority.expected_generation {
            return Err(IDENTITY_CHANGED.into());
        }
        let mut states = self.local_states.lock().map_err(|_| STATE)?;
        let state = states
            .entry(session_key(&identity.owner, &authority.session_id))
            .or_default();
        if !state.folders.contains(&folder) {
            state.folders.push(folder.clone());
        }
        Ok(folder.to_string_lossy().into_owned())
    }

    async fn authorize_local(
        &self,
        run: &Arc<RunControl>,
        tool: registry::Tool,
        args: &Value,
        signature: &str,
        call_id: &str,
        operation: policy::Operation,
    ) -> Result<(), String> {
        self.validate_run(run)?;
        let key = session_key(&run.identity.owner, &run.session_id);
        let decision = {
            let mut states = self.local_states.lock().map_err(|_| STATE)?;
            let state = states.entry(key.clone()).or_default();
            let decision = policy::decide(true, state.mode, operation);
            if decision == Decision::Confirm
                && !tool.irreversible
                && state.grants.contains(signature)
            {
                Decision::Allow
            } else {
                decision
            }
        };
        match decision {
            Decision::Allow => Ok(()),
            Decision::Deny => Err("assistant_operation_denied".into()),
            Decision::Confirm => {
                let binding = self.local_binding(run);
                let (id, pending) = self.local_approvals.request(binding.clone())?;
                let safe = crate::redact::scrub(&args.to_string()).into_owned();
                let mut display = json!({"approvalId":id,"toolCallId":call_id,"name":tool.name,"category":tool.category,"details":safe,"allowSession":!tool.irreversible,"expectedOwner":binding.owner,"expectedGeneration":binding.generation});
                scrub_secrets_in(&mut display, &self.known_keys());
                self.local_approvals.describe(&id, display.clone());
                self.emit(run, "approval", display);
                let timeout = run
                    .deadline
                    .saturating_duration_since(tokio::time::Instant::now())
                    .min(Duration::from_secs(120));
                let choice = self
                    .local_approvals
                    .wait(&id, pending, run.cancel.subscribe(), timeout)
                    .await;
                self.emit(run, "approval_end", json!({"approvalId":id}));
                self.validate_run(run)?;
                if self.identity().await? != run.identity || self.generation() != binding.generation
                {
                    return Err(IDENTITY_CHANGED.into());
                }
                match choice? {
                    Choice::Deny => Err("assistant_operation_denied".into()),
                    Choice::Session if !tool.irreversible => {
                        self.local_states
                            .lock()
                            .map_err(|_| STATE)?
                            .entry(key)
                            .or_default()
                            .grants
                            .insert(signature.into());
                        Ok(())
                    }
                    Choice::Session | Choice::Once => Ok(()),
                }
            }
        }
    }
}
