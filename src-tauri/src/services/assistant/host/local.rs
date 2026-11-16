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
    // "Always allow in this conversation": the whole configuration-change
    // category, one program for commands, or one exact composed command.
    grants: HashSet<String>,
    // A repair still waiting for verification, carried into the next run of
    // the conversation. Memory only: a restart does not resume tasks.
    task: Option<Value>,
    verification: VerificationTargets,
    // Host evidence can outlive a cancelled callback. Only an actual matching
    // Host verification may clear it; a runtime empty snapshot cannot.
    verification_debt: HashSet<(String, String)>,
}

const MAX_VERIFICATION_TARGETS: usize = 32;
const TARGET_LIMIT: &str = "assistant_verification_target_limit";
const VERIFICATION_MUTATIONS: &[&str] = &[
    "repair_launcher",
    "rebind_tool",
    "install_tool",
    "update_tool",
    "uninstall_tool",
    "set_tool_model",
];

#[derive(Default)]
struct ReservedTarget {
    retained: bool,
    pending: usize,
}

/// Capacity is checked against Host-resolved paths, before approval or effects.
/// Pending slots count too; a refused/failed no-effect call gives its slot back.
#[derive(Default)]
struct VerificationTargets {
    run_id: String,
    targets: HashMap<(String, String), ReservedTarget>,
}

impl VerificationTargets {
    fn reserve(
        &mut self,
        run_id: &str,
        task: Option<&Value>,
        tool: &str,
        folder: &str,
        mutation: bool,
    ) -> Result<Option<(String, String)>, String> {
        if folder.trim().is_empty() || folder.len() > 4096 {
            return Err("assistant_invalid_path".into());
        }
        if self.run_id != run_id {
            self.run_id = run_id.into();
            self.targets.clear();
            if let Some(targets) = task.and_then(|task| task["targets"].as_array()) {
                for target in targets.iter().take(MAX_VERIFICATION_TARGETS) {
                    if let (Some(tool), Some(folder)) =
                        (target["tool"].as_str(), target["folder"].as_str())
                    {
                        self.targets.insert(
                            (tool.into(), folder.into()),
                            ReservedTarget {
                                retained: true,
                                pending: 0,
                            },
                        );
                    }
                }
            }
        }
        // A mutation keeps that tool's original problem folders. Home is the
        // fallback only when no successfully observed scope exists for it.
        if mutation
            && self
                .targets
                .iter()
                .any(|((known, _), target)| known == tool && target.retained)
        {
            return Ok(None);
        }
        let key = (tool.into(), folder.into());
        if !self.targets.contains_key(&key) && self.targets.len() >= MAX_VERIFICATION_TARGETS {
            return Err(TARGET_LIMIT.into());
        }
        let target = self.targets.entry(key.clone()).or_default();
        target.pending = target.pending.checked_add(1).ok_or(STATE)?;
        Ok(Some(key))
    }

    fn finish(&mut self, run_id: &str, key: &(String, String), retained: bool) {
        if self.run_id != run_id {
            return;
        }
        if let Some(target) = self.targets.get_mut(key) {
            target.pending = target.pending.saturating_sub(1);
            target.retained |= retained;
            if target.pending == 0 && !target.retained {
                self.targets.remove(key);
            }
        }
    }
}

struct TargetReservation<'a> {
    host: &'a AssistantHost,
    session: String,
    run_id: String,
    key: (String, String),
    retained: bool,
}

impl Drop for TargetReservation<'_> {
    fn drop(&mut self) {
        if let Ok(mut states) = self.host.local_states.lock() {
            if let Some(state) = states.get_mut(&self.session) {
                state
                    .verification
                    .finish(&self.run_id, &self.key, self.retained);
            }
        }
    }
}

/// Armed only immediately before entering a known mutator. Dropping an
/// interrupted future cannot erase effects it may already have committed.
struct MutationDebt<'a> {
    host: &'a AssistantHost,
    session: String,
    run_id: &'a str,
    tool: &'static str,
    folder: &'a Path,
    started: AtomicBool,
}

impl Drop for MutationDebt<'_> {
    fn drop(&mut self) {
        if !self.started.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut states) = self.host.local_states.lock() else {
            return;
        };
        let Some(state) = states.get_mut(&self.session) else {
            return;
        };
        let mut scopes = state.verification_debt.clone();
        if state.verification.run_id == self.run_id {
            for (key, target) in &mut state.verification.targets {
                if target.retained || (key.0 == self.tool && target.pending > 0) {
                    scopes.insert(key.clone());
                    target.retained = true;
                }
            }
        }
        if !scopes.iter().any(|(tool, _)| tool == self.tool) {
            if let Some(folder) = self.folder.to_str() {
                scopes.insert((self.tool.into(), folder.into()));
            }
        }
        // Like the runtime's changed result, a new effect invalidates the
        // earlier checks for all retained problem folders in this task.
        if let Some(targets) = state
            .task
            .as_ref()
            .and_then(|task| task["targets"].as_array())
        {
            for target in targets {
                if let (Some(tool), Some(folder)) =
                    (target["tool"].as_str(), target["folder"].as_str())
                {
                    scopes.insert((tool.into(), folder.into()));
                }
            }
        }
        state.verification_debt = scopes;
        state.task = merge_verification_debt(state.task.take(), &state.verification_debt);
    }
}

impl LocalState {
    fn verified(&mut self, tool: &str, folder: &str) {
        self.verification_debt.remove(&(tool.into(), folder.into()));
        if let Some(targets) = self
            .task
            .as_mut()
            .and_then(|task| task["targets"].as_array_mut())
        {
            for target in targets
                .iter_mut()
                .filter(|target| target["tool"] == tool && target["folder"] == folder)
            {
                target["verified"] = json!(true);
            }
        }
    }
}

fn merge_verification_debt(
    reported: Option<Value>,
    debt: &HashSet<(String, String)>,
) -> Option<Value> {
    if debt.is_empty() {
        return reported;
    }
    let mut state = reported.unwrap_or_else(|| json!({"changed":true,"targets":[]}));
    state["changed"] = json!(true);
    let targets = state["targets"]
        .as_array_mut()
        .expect("sanitized pending targets");
    let mut missing: Vec<_> = debt.iter().collect();
    missing.sort();
    let mut overflow = false;
    for (tool, folder) in missing {
        if let Some(target) = targets
            .iter_mut()
            .find(|target| target["tool"] == *tool && target["folder"] == *folder)
        {
            target["verified"] = json!(false);
        } else if targets.len() < MAX_VERIFICATION_TARGETS {
            targets.push(json!({"tool":tool,"folder":folder,"verified":false}));
        } else {
            overflow = true;
        }
    }
    if overflow {
        state["scopeIncomplete"] = json!(true);
    }
    Some(state)
}

/// Keeps only the pending-verification shape the runtime's task gate reads.
fn task_state(value: &Value) -> Option<Value> {
    if value.get("changed") != Some(&Value::Bool(true)) {
        return None;
    }
    let raw = value["targets"].as_array();
    let mut incomplete = value
        .get("scopeIncomplete")
        .is_some_and(|scope| scope != &Value::Bool(false))
        || raw.is_none_or(|targets| targets.is_empty() || targets.len() > MAX_VERIFICATION_TARGETS);
    let mut seen = HashSet::new();
    let mut targets = vec![];
    for target in raw.into_iter().flatten().take(MAX_VERIFICATION_TARGETS) {
        let tool = target["tool"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.len() <= 64);
        let folder = target["folder"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.len() <= 4096);
        let valid_verified = target.get("verified").is_none_or(Value::is_boolean);
        if let (Some(tool), Some(folder), true) = (tool, folder, valid_verified) {
            let tool = registry::supported_tool(tool)
                .map(|tool| tool.as_str())
                .unwrap_or(tool);
            if !seen.insert((tool, folder)) {
                incomplete = true;
                continue;
            }
            targets
                .push(json!({"tool":tool,"folder":folder,"verified":target["verified"] == true}));
        } else {
            incomplete = true;
        }
    }
    let mut state = json!({"changed":true,"targets":targets});
    if incomplete {
        state["scopeIncomplete"] = Value::Bool(true);
    }
    Some(state)
}

/// What an approval card shows, and what "always allow" would remember.
struct Ask<'a> {
    args: &'a Value,
    grant: &'a str,
    overwrites: Option<Vec<String>>,
}

/// The user-modified fields a rebind resets, shown on its approval card.
fn rebind_overwrites(status: &Value) -> Option<Vec<String>> {
    let fields: Vec<String> = status["modifiedFields"]
        .as_array()?
        .iter()
        .filter_map(|f| f.as_str().map(str::to_owned))
        .collect();
    (status["status"] == "modified" && !fields.is_empty()).then_some(fields)
}

fn check_local_home(run: &RunControl, paths: &LocalPaths) -> Result<(), String> {
    if let Some(admitted) = &run.local_home {
        if crate::services::assistant::tools::scope::resolve(&paths.home)? != *admitted {
            return Err("assistant_local_scope_changed".into());
        }
    }
    Ok(())
}

pub(super) static MUTATIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type CallResult = (String, Option<Result<Value, String>>);
#[derive(Default)]
pub(super) struct Calls(pub Mutex<HashMap<String, CallResult>>);

pub(super) fn tool_failure(message: &str, known_keys: &[String]) -> String {
    if message.contains("-----BEGIN ") && message.contains("PRIVATE KEY-----") {
        return "assistant_local_unavailable".into();
    }
    scrub_secrets(&crate::redact::scrub(message), known_keys)
}

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
    fn reserve_verification_target(
        &self,
        run: &RunControl,
        name: &str,
        args: &Value,
        folder: &Path,
    ) -> Result<Option<TargetReservation<'_>>, String> {
        let mutation = VERIFICATION_MUTATIONS.contains(&name);
        if name != "diagnose_in_folder" && !mutation {
            return Ok(None);
        }
        let tool =
            registry::supported_tool(args["tool"].as_str().ok_or("assistant_invalid_tool")?)?;
        let folder = folder.to_str().ok_or("assistant_invalid_path")?;
        let session = session_key(&run.identity.owner, &run.session_id);
        let key = {
            let mut states = self.local_states.lock().map_err(|_| STATE)?;
            let state = states.entry(session.clone()).or_default();
            state.verification.reserve(
                &run.run_id,
                state.task.as_ref(),
                tool.as_str(),
                folder,
                mutation,
            )?
        };
        Ok(key.map(|key| TargetReservation {
            host: self,
            session,
            run_id: run.run_id.clone(),
            key,
            retained: false,
        }))
    }

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

    /// A finished run reports the verification it still owes; a run that
    /// failed before reporting leaves the earlier state in place. Host-observed
    /// effects survive an empty report until a matching Host check succeeds.
    pub(super) fn keep_local_task(&self, run: &RunControl, result: &Value) {
        let Some(reported) = result.get("localTaskState") else {
            return;
        };
        if !run.local_enabled {
            return;
        }
        let mut states = self
            .local_states
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let state = states
            .entry(session_key(&run.identity.owner, &run.session_id))
            .or_default();
        state.task = merge_verification_debt(task_state(reported), &state.verification_debt);
    }

    pub(crate) fn local_task_state(&self, owner: &str, sid: &str) -> Value {
        self.local_states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&session_key(owner, sid))
            .and_then(|state| state.task.clone())
            .unwrap_or(Value::Null)
    }

    pub(crate) fn clear_local_permission(&self, owner: &str, sid: &str) {
        self.local_states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&session_key(owner, sid));
    }

    pub(in crate::services::assistant) fn local_binding(&self, run: &RunControl) -> Binding {
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
        if request.mode == PermissionMode::FullAuto && !request.confirmed_full_access {
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

    /// Internal sidecar admission: annotations come from Pi's registered tool,
    /// never from model arguments. Bind grants to config + registered tool schema.
    pub(in crate::services::assistant) async fn authorize_mcp(
        &self,
        run: &Arc<RunControl>,
        params: &Value,
    ) -> Result<Value, String> {
        self.validate_run(run)?;
        self.ensure_current(&run.identity, run.mcp_generation)
            .await?;
        let call: registry::Call = serde_json::from_value(params.clone()).map_err(|_| PROTOCOL)?;
        if call.session_id != run.session_id
            || call.run_id != run.run_id
            || call.operation_id != run.operation_id
            || call.tool_call_id.is_empty()
            || call.tool_call_id.len() > 200
            || run.compact_only
        {
            return Err("assistant_runtime_identity_mismatch".into());
        }
        if run.mcp["snapshot"].is_null() || call.args["snapshot"] != run.mcp["snapshot"] {
            return Err("assistant_mcp_snapshot_changed".into());
        }
        let current = super::super::mcp::current(self.env.config_dir()).await?;
        if current["snapshot"] != run.mcp["snapshot"] {
            return Err("assistant_mcp_snapshot_changed".into());
        }
        let name = call.args["toolName"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .ok_or(PROTOCOL)?;
        let schema = call.args["identity"]
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or(PROTOCOL)?;
        let read_only = call.args["readOnly"].as_bool().ok_or(PROTOCOL)?;
        let server = call.args["server"].as_str();
        if server.is_some_and(|s| {
            run.mcp["servers"]
                .get(s)
                .is_none_or(|config| config["unavailable"] == true)
        }) || (server.is_none()
            && !matches!(
                name,
                "tool_search"
                    | "list_mcp_resources"
                    | "list_mcp_resource_templates"
                    | "read_mcp_resource"
            ))
        {
            return Err("assistant_mcp_unknown_tool".into());
        }
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&call.args).map_err(|_| PROTOCOL)?)
        );
        {
            let mut calls = run.local_calls.0.lock().map_err(|_| STATE)?;
            if let Some((previous, result)) = calls.get(&call.tool_call_id) {
                if previous != &fingerprint {
                    return Err("assistant_tool_call_conflict".into());
                }
                return result
                    .clone()
                    .unwrap_or_else(|| Err("assistant_tool_pending".into()));
            }
            if calls.len() >= capabilities::local_config().limits.max_tool_calls * 4 {
                return Err("assistant_tool_limit".into());
            }
            calls.insert(call.tool_call_id.clone(), (fingerprint.clone(), None));
        }
        let result = async {
            let key = session_key(&run.identity.owner,&run.session_id);
            let grant = format!("mcp:{}:{schema}:{name}",run.mcp["snapshot"].as_str().ok_or(PROTOCOL)?);
            let free = {
                let mut states=self.local_states.lock().map_err(|_|STATE)?;
                let state=states.entry(key.clone()).or_default();
                read_only || state.mode == PermissionMode::FullAuto || state.grants.contains(&grant)
            };
            if !free {
                let (preview,truncated)=super::super::mcp::arguments_preview(&call.args["args"],&run.mcp)?;
                let binding=self.local_binding(run);
                let (id,pending)=self.local_approvals.request(binding.clone())?;
                let display=json!({"approvalId":id,"toolCallId":call.tool_call_id,"name":name,"category":"mcp","details":{"server":server,"tool":name},"argumentsPreview":preview,"argumentsTruncated":truncated,"allowSession":true,"expectedOwner":binding.owner,"expectedGeneration":binding.generation});
                self.local_approvals.describe(&id,display.clone());
                self.emit(run,"approval",display);
                let timeout=run.deadline.saturating_duration_since(tokio::time::Instant::now()).min(Duration::from_secs(120));
                let choice=self.local_approvals.wait(&id,pending,run.cancel.subscribe(),timeout).await;
                self.emit(run,"approval_end",json!({"approvalId":id}));
                self.validate_run(run)?;
                self.ensure_current(&run.identity,run.mcp_generation).await?;
                if super::super::mcp::current(self.env.config_dir()).await?["snapshot"] != run.mcp["snapshot"] { return Err("assistant_mcp_snapshot_changed".into()); }
                match choice? {
                    Choice::Deny => return Ok(json!({"approved":false})),
                    Choice::Session => { self.local_states.lock().map_err(|_|STATE)?.entry(key).or_default().grants.insert(grant); }
                    Choice::Once => {}
                }
            }
            self.validate_run(run)?;
            Ok(json!({"approved":true}))
        }.await;
        run.local_calls
            .0
            .lock()
            .map_err(|_| STATE)?
            .insert(call.tool_call_id, (fingerprint, Some(result.clone())));
        result
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
        let tool = registry::get(&call.name)?;
        if super::super::skill_tools::is_tool(tool.name) {
            if self
                .runs
                .lock()
                .map_err(|_| STATE)?
                .get(&run.session_id)
                .is_none_or(|active| active.run_id != run.run_id)
            {
                return Err("not_in_results".into());
            }
            let reply = run.reply_id();
            let mut state = run.skill_tools.lock().map_err(|_| STATE)?;
            if state.closed {
                return Err("not_in_results".into());
            }
            if state.reply_id != reply {
                let epoch = state.epoch + 1;
                *state = super::super::skill_tools::State {
                    reply_id: reply,
                    epoch,
                    ..Default::default()
                };
            }
        }
        registry::validate_args(tool, &call.args)?;
        // A new search replaces the earlier results; an invalid one does not.
        if call.name == "find_skills"
            && !run
                .local_calls
                .0
                .lock()
                .map_err(|_| STATE)?
                .contains_key(&call.tool_call_id)
        {
            let mut skills = run.skill_tools.lock().map_err(|_| STATE)?;
            skills.epoch += 1;
            skills.results.clear();
            skills.installed.clear();
        }
        if call.name == "install_skill" {
            self.check_skill_result(run, &call.args)?;
        }
        let record = blocking({
            let store = self.store(&run.identity.owner)?;
            let sid = run.session_id.clone();
            move || store.load_record(&sid)
        })
        .await?;
        capabilities::validate(&record.session.assistant_id, &record.session.capabilities)?;
        if (tool.name != "use_skill"
            && !super::super::skill_tools::is_tool(tool.name)
            && (!run.local_enabled || !capabilities::enabled(&record.session.capabilities)))
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
        let result = if super::super::skill_tools::is_tool(tool.name) {
            self.dispatch_skill_tool(run, &call).await
        } else if tool.name == "use_skill" {
            self.load_run_skill(
                run,
                call.args["name"].as_str().ok_or("assistant_invalid_tool")?,
                false,
            )
            .await
        } else {
            self.execute_local(run, tool, &call.args, &fingerprint, &call.tool_call_id)
                .await
        };
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
                call_id,
                tool.operation(false, false),
                Ask {
                    args,
                    grant: signature,
                    overwrites: None,
                },
            )
            .await?;
            return Ok(json!({"approved":true}));
        }
        let (paths, scope) = self.local_scope().await?;
        check_local_home(run, &paths)?;
        let scope = self.skill_scope(run, scope).await?;
        let requested = args["path"]
            .as_str()
            .or_else(|| args["folder"].as_str())
            .map(PathBuf::from);
        let checked = requested.as_ref().map(|p| scope.check(p)).transpose()?;
        let access = checked.as_ref().map_or(Access::Inside, |p| p.access);
        let verification_folder = if VERIFICATION_MUTATIONS.contains(&tool.name) {
            scope.check(&paths.home)?.path
        } else {
            match checked.as_ref() {
                Some(path) => path.path.clone(),
                None => scope.check(&paths.home)?.path,
            }
        };
        let mut reservation =
            self.reserve_verification_target(run, tool.name, args, &verification_folder)?;
        let shell = matches!(tool.name, "bash" | "powershell");
        let command = args["command"].as_str().unwrap_or("");
        let read_only = shell && commands::read_only(command, cfg!(windows));
        // The bundled skill is trusted instructions: never rewritten by a run.
        let bundled = matches!(tool.name, "write" | "edit")
            && checked.as_ref().is_some_and(|p| {
                paths
                    .skill
                    .parent()
                    .is_some_and(|dir| p.path.starts_with(dir))
            });
        let mut tool = tool;
        let mut overwrites = None;
        if tool.name == "rebind_tool" {
            let selected = registry::supported_tool(args["tool"].as_str().unwrap_or(""))?;
            // Issuing a new account key cannot be undone by a restore: it asks
            // like a command unless the conversation is in full auto.
            if !self.env.local_key_cached(selected) {
                tool.category = policy::Category::Exec;
            }
            overwrites = rebind_overwrites(&self.env.local_binding_status(selected).await?);
        }
        // A generic write outside the tools' configuration folders (a shell
        // profile, a LaunchAgent, any other file) can make code run later: it
        // asks like a command unless the conversation is in full auto.
        let generic_write = matches!(tool.name, "write" | "edit");
        if generic_write && !checked.as_ref().is_some_and(|p| scope.in_root(&p.path)) {
            tool.category = policy::Category::Exec;
        }
        let forbidden = access == Access::Forbidden
            || (shell
                && (commands::forbidden(command)
                    || commands::protected_operation(
                        command,
                        requested.as_deref().unwrap_or(&paths.home),
                        &paths.home,
                        &scope,
                    )))
            || bundled
            || (generic_write && requested.as_ref().is_some_and(|p| scope.write_forbidden(p)));
        let grant = match tool.category {
            policy::Category::WriteConfig => "write_config".to_string(),
            policy::Category::Exec if shell => commands::program(command)
                .map(|program| format!("exec:{program}"))
                .unwrap_or_else(|| signature.to_string()),
            policy::Category::Exec if generic_write => checked
                .as_ref()
                .map(|p| format!("write:{}", p.path.display()))
                .unwrap_or_else(|| signature.to_string()),
            policy::Category::Exec => format!("exec:{}", tool.name),
            policy::Category::Read => signature.to_string(),
        };
        self.authorize_local(
            run,
            tool,
            call_id,
            tool.operation(forbidden, read_only),
            Ask {
                args,
                grant: &grant,
                overwrites,
            },
        )
        .await?;
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err(IDENTITY_CHANGED.into());
        }
        check_local_home(run, &self.env.local_paths().await?)?;
        let path = checked
            .map(|p| p.path)
            .unwrap_or_else(|| verification_folder.clone());
        let outside = access == Access::Outside;
        let mutation = if VERIFICATION_MUTATIONS.contains(&tool.name) {
            Some(MutationDebt {
                host: self,
                session: session_key(&run.identity.owner, &run.session_id),
                run_id: &run.run_id,
                tool: registry::supported_tool(
                    args["tool"].as_str().ok_or("assistant_invalid_tool")?,
                )?
                .as_str(),
                folder: &verification_folder,
                started: AtomicBool::new(false),
            })
        } else {
            None
        };
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
                    check_local_home(run, &self.env.local_paths().await?)?;
                    self.validate_run(run)?;
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
                    if let Some(debt) = &mutation {
                        debt.started.store(true, Ordering::Release);
                    }
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
                    if let Some(debt) = &mutation {
                        debt.started.store(true, Ordering::Release);
                    }
                    self.env
                        .local_mutate(tool.name, selected, None, call_id)
                        .await
                }
                "bash" | "powershell" => {
                    let scope = self.skill_scope(run, scope.clone()).await?;
                    if scope.check(&path)?.access == Access::Forbidden
                        || commands::protected_operation(command, &path, &paths.home, &scope)
                    {
                        return Err("assistant_operation_denied".into());
                    }
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
                    check_local_home(run, &self.env.local_paths().await?)?;
                    self.validate_run(run)?;
                    let _io = crate::config_io::lock();
                    if scope.write_forbidden(&path) || scope.check(&path)?.path != path {
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
                    let env = self.env.clone();
                    let grants = run.skills.lock().map_err(|_| STATE)?.grants();
                    let target = path.clone();
                    blocking(move || {
                        super::super::skills::read_in_skill(
                            env.as_ref(),
                            grants,
                            &target,
                            Box::new(move || match tool.name {
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
                            }),
                        )
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
        let result = tokio::select! {
            biased;
            _=cancel.changed()=>return Err(CANCELLED.into()),
            _=tokio::time::sleep_until(run.deadline)=>return Err("assistant_run_timeout".into()),
            result=operation=>result,
        };
        // Lifecycle adapters use this older bare spelling. Normalize at the
        // Host boundary so both retained debt and the runtime's current run
        // classify it as an unknown mutation, rather than a retryable refusal.
        let result = result.map_err(|error| {
            if mutation.is_some() && error == "unknown_outcome" {
                "assistant_mutation_outcome_unknown".into()
            } else {
                error
            }
        });
        if let Some(debt) = &mutation {
            let effect = match &result {
                Ok(value) => value["changed"] == true,
                Err(error) => {
                    error.contains("assistant_local_partial_change")
                        || error.contains("outcome_unknown")
                        || error.contains("assistant_journal_unavailable")
                }
            };
            if !effect {
                debt.started.store(false, Ordering::Release);
            }
        }
        if let Some(reservation) = reservation.as_mut() {
            reservation.retained = match &result {
                Ok(value) => tool.name == "diagnose_in_folder" || value["changed"] == true,
                Err(error) => {
                    error.contains("assistant_local_partial_change")
                        || error.contains("outcome_unknown")
                }
            };
        }
        let mut result = result?;
        if VERIFICATION_MUTATIONS.contains(&tool.name) {
            result["tool"] = json!(registry::supported_tool(
                args["tool"].as_str().ok_or("assistant_invalid_tool")?
            )?
            .as_str());
            result["verificationFolder"] = json!(verification_folder);
        }
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err(IDENTITY_CHANGED.into());
        }
        if tool.name == "verify_in_folder" && result["verified"] == true {
            let selected =
                registry::supported_tool(args["tool"].as_str().ok_or("assistant_invalid_tool")?)?;
            if let Some(folder) = verification_folder.to_str() {
                if let Some(state) = self
                    .local_states
                    .lock()
                    .map_err(|_| STATE)?
                    .get_mut(&session_key(&run.identity.owner, &run.session_id))
                {
                    state.verified(selected.as_str(), folder);
                }
            }
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

    pub(super) async fn local_scope(&self) -> Result<(LocalPaths, Scope), String> {
        let paths = self.env.local_paths().await?;
        let scope = paths.scope(&[])?;
        Ok((paths, scope))
    }

    /// Ask before a paid generation, with what it will cost; nothing is
    /// asked in full auto or after "allow in this conversation". `Ok(false)`:
    /// the user declined.
    pub(in crate::services::assistant) async fn confirm_generation(
        &self,
        run: &Arc<RunControl>,
        call_id: &str,
        name: &str,
        details: Value,
    ) -> Result<bool, String> {
        self.validate_run(run)?;
        let key = session_key(&run.identity.owner, &run.session_id);
        let grant = format!("generate:{name}");
        {
            let mut states = self.local_states.lock().map_err(|_| STATE)?;
            let state = states.entry(key.clone()).or_default();
            if state.mode == PermissionMode::FullAuto || state.grants.contains(&grant) {
                return Ok(true);
            }
        }
        let binding = self.local_binding(run);
        let (id, pending) = self.local_approvals.request(binding.clone())?;
        let mut display = json!({"approvalId":id,"toolCallId":call_id,"name":name,"category":"generation","details":details,"allowSession":true,"expectedOwner":binding.owner,"expectedGeneration":binding.generation});
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
        if self.identity().await? != run.identity || self.generation() != binding.generation {
            return Err(IDENTITY_CHANGED.into());
        }
        match choice {
            Ok(Choice::Deny) | Err(_) if !run.cancelled() => Ok(false),
            Err(error) => Err(error),
            Ok(Choice::Deny) => Ok(false),
            Ok(Choice::Session) => {
                self.local_states
                    .lock()
                    .map_err(|_| STATE)?
                    .entry(key)
                    .or_default()
                    .grants
                    .insert(grant);
                Ok(true)
            }
            Ok(Choice::Once) => Ok(true),
        }
    }

    async fn authorize_local(
        &self,
        run: &Arc<RunControl>,
        tool: registry::Tool,
        call_id: &str,
        operation: policy::Operation,
        ask: Ask<'_>,
    ) -> Result<(), String> {
        let Ask {
            args,
            grant,
            overwrites,
        } = ask;
        self.validate_run(run)?;
        let key = session_key(&run.identity.owner, &run.session_id);
        let decision = {
            let mut states = self.local_states.lock().map_err(|_| STATE)?;
            let state = states.entry(key.clone()).or_default();
            let decision = policy::decide(true, state.mode, operation);
            if decision == Decision::Confirm && state.grants.contains(grant) {
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
                let mut display = json!({"approvalId":id,"toolCallId":call_id,"name":tool.name,"category":tool.category,"details":safe,"overwrites":overwrites,"allowSession":true,"expectedOwner":binding.owner,"expectedGeneration":binding.generation});
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
                    Choice::Session => {
                        self.local_states
                            .lock()
                            .map_err(|_| STATE)?
                            .entry(key)
                            .or_default()
                            .grants
                            .insert(grant.into());
                        Ok(())
                    }
                    Choice::Once => Ok(()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn local_task_state_keeps_only_a_pending_verification() {
        assert_eq!(super::task_state(&json!(null)), None);
        assert_eq!(super::task_state(&json!({"changed":"yes"})), None);
        assert_eq!(
            super::task_state(&json!({"changed":true})),
            Some(json!({"changed":true,"targets":[],"scopeIncomplete":true}))
        );
        let kept = super::task_state(&json!({"changed":true,"extra":1,"targets":[
            {"tool":"codex","folder":"/p","verified":true,"x":1},
            {"tool":7,"folder":"/q"},
        ]}))
        .unwrap();
        assert_eq!(
            kept,
            json!({"changed":true,"targets":[{"tool":"codex","folder":"/p","verified":true}],"scopeIncomplete":true})
        );
    }

    #[test]
    fn local_task_state_keeps_complete_scopes_and_never_hides_lost_evidence() {
        let complete = json!({"changed":true,"targets":[
            {"tool":"codex","folder":"/p","verified":false},
            {"tool":"claude","folder":"/home","verified":true}
        ]});
        assert_eq!(super::task_state(&complete), Some(complete.clone()));
        for malformed in [
            json!({"changed":true,"targets":[]}),
            json!({"changed":true,"targets":"broken"}),
            json!({"changed":true,"targets":[{"tool":"","folder":"/p"}]}),
            json!({"changed":true,"targets":[{"tool":"codex","folder":" "}]}),
            json!({"changed":true,"targets":[{"tool":"codex","folder":"/p","verified":"yes"}]}),
            json!({"changed":true,"targets":[{"tool":"codex","folder":"/p"},{"tool":"codex","folder":"/p"}]}),
        ] {
            assert_eq!(
                super::task_state(&malformed).unwrap()["scopeIncomplete"],
                true
            );
        }
        for flag in [json!(true), json!(null), json!("false")] {
            let mut value = complete.clone();
            value["scopeIncomplete"] = flag;
            assert_eq!(super::task_state(&value).unwrap()["scopeIncomplete"], true);
        }
        let targets: Vec<_> = (0..33)
            .map(|i| json!({"tool":"codex","folder":format!("/p{i}"),"verified":true}))
            .collect();
        let kept = super::task_state(&json!({"changed":true,"targets":targets})).unwrap();
        assert_eq!(kept["targets"].as_array().unwrap().len(), 32);
        assert_eq!(kept["scopeIncomplete"], true);
    }

    #[test]
    fn canonical_target_capacity_blocks_new_work_but_allows_existing_scope() {
        let mut scopes = super::VerificationTargets::default();
        for i in 0..32 {
            let key = scopes
                .reserve("run", None, "codex", &format!("/p{i}"), false)
                .unwrap()
                .unwrap();
            scopes.finish("run", &key, true);
        }
        for mutation in [false, true] {
            let tool = if mutation { "claude" } else { "codex" };
            let mut effects = 0;
            let result = scopes
                .reserve("run", None, tool, "/new", mutation)
                .map(|_| effects += 1);
            assert_eq!(result.unwrap_err(), super::TARGET_LIMIT);
            assert_eq!(effects, 0);
        }
        let duplicate = scopes
            .reserve("run", None, "codex", "/p0", false)
            .unwrap()
            .unwrap();
        scopes.finish("run", &duplicate, false);
        assert_eq!(scopes.targets.len(), 32);
        assert!(scopes
            .reserve("run", None, "codex", "/home", true)
            .unwrap()
            .is_none());
    }

    #[test]
    fn target_reservations_release_no_effect_failures_and_keep_concurrent_success() {
        let mut scopes = super::VerificationTargets::default();
        for i in 0..31 {
            let key = scopes
                .reserve("run", None, "codex", &format!("/p{i}"), false)
                .unwrap()
                .unwrap();
            scopes.finish("run", &key, true);
        }
        let key = scopes
            .reserve("run", None, "claude", "/home", true)
            .unwrap()
            .unwrap();
        assert!(scopes.reserve("run", None, "codex", "/new", false).is_err());
        scopes.finish("run", &key, false);
        assert_eq!(scopes.targets.len(), 31);
        let first = scopes
            .reserve("run", None, "claude", "/home", true)
            .unwrap()
            .unwrap();
        let second = scopes
            .reserve("run", None, "claude", "/home", true)
            .unwrap()
            .unwrap();
        scopes.finish("run", &first, false);
        assert_eq!(scopes.targets.len(), 32);
        scopes.finish("run", &second, true);
        assert_eq!(scopes.targets.len(), 32);
    }

    #[test]
    fn host_debt_survives_empty_reports_until_the_matching_scope_verifies() {
        let debt = std::collections::HashSet::from([("codex".into(), "/original".into())]);
        let mut state = super::LocalState {
            task: super::merge_verification_debt(None, &debt),
            verification_debt: debt,
            ..Default::default()
        };
        state.verified("claude", "/original");
        state.verified("codex", "/other");
        assert_eq!(state.verification_debt.len(), 1);
        assert_eq!(
            super::merge_verification_debt(None, &state.verification_debt).unwrap()["targets"][0]
                ["verified"],
            false
        );
        state.verified("codex", "/original");
        assert!(state.verification_debt.is_empty());
        assert!(super::merge_verification_debt(None, &state.verification_debt).is_none());
    }

    #[test]
    fn new_run_seeds_pending_targets_and_ignores_prior_reservation_cleanup() {
        let mut scopes = super::VerificationTargets::default();
        let old = scopes
            .reserve("old", None, "codex", "/old", false)
            .unwrap()
            .unwrap();
        let pending = json!({"changed":true,"targets":[
            {"tool":"claude","folder":"/pending","verified":false}
        ]});
        let current = scopes
            .reserve("new", Some(&pending), "codex", "/new", false)
            .unwrap()
            .unwrap();
        scopes.finish("old", &old, false);
        assert_eq!(scopes.targets.len(), 2);
        scopes.finish("new", &current, false);
        assert_eq!(scopes.targets.len(), 1);
        assert!(scopes
            .targets
            .contains_key(&("claude".into(), "/pending".into())));
    }

    #[test]
    fn unrepresentable_target_scope_is_refused_before_reservation() {
        let mut scopes = super::VerificationTargets::default();
        for folder in [String::new(), " ".into(), "中".repeat(1366)] {
            assert_eq!(
                scopes
                    .reserve("run", None, "codex", &folder, false)
                    .unwrap_err(),
                "assistant_invalid_path"
            );
            assert!(scopes.targets.is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn canonical_alias_at_capacity_reuses_its_existing_diagnosis() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let canonical = super::super::super::tools::scope::resolve(&real).unwrap();
        let aliased = super::super::super::tools::scope::resolve(&alias).unwrap();
        assert_eq!(canonical, aliased);
        let mut scopes = super::VerificationTargets::default();
        let key = scopes
            .reserve("run", None, "codex", &canonical.to_string_lossy(), false)
            .unwrap()
            .unwrap();
        scopes.finish("run", &key, true);
        for i in 0..31 {
            let key = scopes
                .reserve("run", None, "codex", &format!("/p{i}"), false)
                .unwrap()
                .unwrap();
            scopes.finish("run", &key, true);
        }
        assert!(scopes
            .reserve("run", None, "codex", &aliased.to_string_lossy(), false)
            .is_ok());
    }

    #[test]
    fn tool_failure_redacts_credentials_before_crossing_the_runtime_boundary() {
        let known = "known-tool-credential-do-not-forward".to_string();
        let message = format!("writer failed: {known}; api_key=third-party-canary");
        let safe = super::tool_failure(&message, std::slice::from_ref(&known));
        assert!(!safe.contains(&known));
        assert!(!safe.contains("third-party-canary"));
        assert_eq!(
            super::tool_failure("assistant_restore_conflict", &[]),
            "assistant_restore_conflict"
        );
        assert_eq!(
            super::tool_failure("-----BEGIN RSA PRIVATE KEY----- canary", &[]),
            "assistant_local_unavailable"
        );
    }
}
