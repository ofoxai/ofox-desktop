use super::{
    host,
    policy::OperationClass,
    process, store,
    types::{id, now, AssistantKind},
};
use crate::{
    app_config::BindableTool,
    commands::{model_selection, ofox_auth::OfoxAuthState, tool_update},
    model_selection::{ExpectedWrite, EXPECTED_WRITE, SELECTION_LOCK},
    store::AppState,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tauri::Manager;
use tokio::process::Command;

tokio::task_local! {static WRITE_AUTHORITY: (tauri::AppHandle, Arc<host::RunControl>);}

/// Called inside an adapter's final write lock. Ordinary UI operations do not
/// have an assistant authority scope and retain their existing behavior.
pub(crate) fn check_write_authority() -> Result<(), String> {
    WRITE_AUTHORITY
        .try_with(|(app, run)| {
            app.state::<host::AssistantHost>().validate_run(run)?;
            let auth = app.state::<OfoxAuthState>();
            let manager = auth
                .0
                .try_read()
                .map_err(|_| "assistant_identity_unavailable")?;
            if !manager.is_authenticated() {
                return Err("assistant_identity_changed".into());
            }
            let user = manager
                .get_user_info()
                .ok_or("assistant_identity_unavailable")?
                .key_owner();
            let owner = store::namespace(
                user.user_id.as_deref().unwrap_or(""),
                user.org_id.as_deref().unwrap_or(""),
            )?;
            if owner != run.identity.owner {
                return Err("assistant_identity_changed".into());
            }
            if crate::ofox_apex::current_apex() != run.identity.apex {
                return Err("assistant_identity_changed".into());
            }
            Ok(())
        })
        .unwrap_or(Ok(()))
}

fn operation_class(name: &str) -> Option<OperationClass> {
    match name {
        "config_status" => Some(OperationClass::Read),
        "diagnose_tool" | "bind_tool" | "set_tool_model" | "install_tool" | "update_tool" => {
            Some(OperationClass::Configure)
        }
        "run_command" => Some(OperationClass::Command),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolArgs {
    tool: Option<String>,
    model_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandArgs {
    command: String,
    cwd: Option<PathBuf>,
}

fn tool_args(args: &Value) -> Result<ToolArgs, String> {
    serde_json::from_value(args.clone()).map_err(|_| "assistant_invalid_tool_arguments".into())
}
fn tool_name(args: &Value) -> Result<String, String> {
    let args = tool_args(args)?;
    let tool = args.tool.ok_or("assistant_tool_required")?;
    if !crate::model_selection::TOOLS.contains(&tool.as_str()) {
        return Err("assistant_unknown_tool".into());
    }
    Ok(tool)
}
fn validate_arguments(name: &str, args: &Value) -> Result<(), String> {
    if operation_class(name).is_none() {
        return Err("assistant_unknown_tool".into());
    }
    if name == "run_command" {
        let args: CommandArgs =
            serde_json::from_value(args.clone()).map_err(|_| "assistant_invalid_tool_arguments")?;
        if args.command.trim().is_empty()
            || args.command.len() > 16_384
            || args.command.contains('\0')
            || args.cwd.as_ref().is_some_and(|cwd| !cwd.is_absolute())
        {
            return Err("assistant_invalid_command".into());
        }
        return Ok(());
    }
    let typed = tool_args(args)?;
    if name == "config_status" && typed.tool.is_none() && typed.model_id.is_none() {
        return Ok(());
    }
    tool_name(args)?;
    if name == "set_tool_model"
        && typed
            .model_id
            .as_ref()
            .is_none_or(|model| model.trim().is_empty())
    {
        return Err("assistant_model_required".into());
    }
    if typed
        .model_id
        .as_ref()
        .is_some_and(|model| model.len() > 512 || model.chars().any(char::is_control))
        || (!matches!(name, "bind_tool" | "set_tool_model") && typed.model_id.is_some())
    {
        return Err("assistant_invalid_tool_arguments".into());
    }
    Ok(())
}

async fn recheck(app: &tauri::AppHandle, run: &Arc<host::RunControl>) -> Result<(), String> {
    app.state::<host::AssistantHost>().validate_run(run)?;
    if host::identity(app).await? != run.identity {
        return Err("assistant_identity_changed".into());
    }
    Ok(())
}

fn claim_operation(
    detail: &mut super::types::Detail,
    run: &host::RunControl,
    name: &str,
    call_id: &str,
    args: &Value,
) -> Result<Value, String> {
    if call_id.is_empty() || call_id.len() > 200 || call_id.chars().any(char::is_control) {
        return Err("assistant_invalid_tool_call_id".into());
    }
    if detail
        .operations
        .iter()
        .any(|operation| operation["runId"] == run.run_id && operation["callId"] == call_id)
    {
        return Err("assistant_tool_call_already_attempted".into());
    }
    let digest = crate::model_selection::digest(args)?;
    if detail.operations.iter().any(|operation| {
        operation["runId"] == run.run_id
            && operation["toolName"] == name
            && operation["argumentDigest"] == digest
            && operation["status"] == "unknown"
    }) {
        return Err("assistant_operation_outcome_unknown".into());
    }
    let title = match name {
        "config_status" => "Check configuration",
        "diagnose_tool" => "Diagnose tool",
        "bind_tool" => "Connect tool to Ofox",
        "set_tool_model" => "Change tool model",
        "install_tool" => "Install tool",
        "update_tool" => "Update tool",
        "run_command" => "Run local command",
        _ => "Tool operation",
    };
    let mut operation = json!({"id":id(),"runId":run.run_id,"operationId":run.operation_id,"callId":call_id,"title":title,"summary":serde_json::to_string(args).unwrap_or_default(),"status":"running","startedAt":now()});
    operation["toolName"] = json!(name);
    operation["argumentDigest"] = json!(digest);
    if name == "run_command" {
        operation["command"] = args["command"].clone();
    }
    store::scrub_value(&mut operation, &[&run.secret]);
    detail.operations.push(operation.clone());
    Ok(operation)
}

pub async fn execute(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    name: &str,
    args: Value,
    call_id: &str,
) -> Result<Value, String> {
    if run
        .outcome_unknown
        .load(std::sync::atomic::Ordering::Acquire)
        && name != "config_status"
    {
        return Err("assistant_operation_outcome_unknown".into());
    }
    if name == "web_search" {
        return super::web::search(app, run, args).await;
    }
    validate_arguments(name, &args)?;
    recheck(app, run).await?;
    let state = app.state::<AppState>();
    let configuration = store::get(&state.db, &run.identity.owner, &run.session_id)?
        .session
        .kind
        == AssistantKind::Configuration;
    if !configuration {
        return Err("assistant_operation_denied".into());
    }
    let operation = store::mutate(&state.db, &run.identity.owner, &run.session_id, |detail| {
        claim_operation(detail, run, name, call_id, &args)
    })?;
    host::emit(app, run, "operation", json!({"operation":operation}));
    let result = execute_inner(app, run, name, &args).await;
    let mut output = match &result {
        Ok(value) => value.clone(),
        Err(error) => json!({"error":error}),
    };
    store::scrub_value(&mut output, &[&run.secret]);
    let finished = store::mutate(&state.db, &run.identity.owner, &run.session_id, |detail| {
        let stored = detail
            .operations
            .iter_mut()
            .find(|stored| stored["id"] == operation["id"])
            .ok_or("assistant_operation_not_found")?;
        stored["status"] = json!(if result
            .as_ref()
            .is_err_and(|error| error.contains("outcome_unknown"))
        {
            "unknown"
        } else if *run.cancel.borrow() {
            "cancelled"
        } else if result.is_ok() {
            "completed"
        } else {
            "failed"
        });
        stored["output"] = json!(serde_json::to_string_pretty(&output).unwrap_or_default());
        stored["finishedAt"] = json!(now());
        if output["requiresUserAction"] == true {
            stored["action"] = json!("openTools");
        }
        Ok(stored.clone())
    })?;
    host::emit(app, run, "operation", json!({"operation":finished}));
    match result {
        Ok(_) => Ok(output),
        Err(error) => {
            let mut error = json!(error);
            store::scrub_value(&mut error, &[&run.secret]);
            Err(error.as_str().unwrap_or("assistant_tool_failed").into())
        }
    }
}

async fn execute_inner(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    let class = operation_class(name).ok_or("assistant_unknown_tool")?;
    if matches!(name, "install_tool" | "update_tool") {
        return lifecycle(app, run, name, &tool_name(args)?).await;
    }
    let revision = if matches!(name, "bind_tool" | "set_tool_model") {
        Some(crate::model_selection::revision(
            &app.state::<AppState>().db,
            &tool_name(args)?,
        )?)
    } else {
        None
    };
    let mut preview = args.clone();
    if let Some(revision) = &revision {
        preview["expectedRevision"] = json!(revision);
        preview["change"] = json!(if name == "bind_tool" {
            "Set Ofox-owned connection fields; preserve unrelated settings"
        } else {
            "Change the selected model in the current Ofox connection"
        });
    }
    app.state::<host::AssistantHost>()
        .authorize(app, run, true, name, &preview, class)
        .await?;
    recheck(app, run).await?;
    match name {
        "config_status" => configuration_status(app, args, false).await,
        "diagnose_tool" => diagnose(app, run, &tool_name(args)?).await,
        "run_command" => {
            let args: CommandArgs =
                serde_json::from_value(args.clone()).map_err(|_| "assistant_invalid_command")?;
            let mut command = shell_command(&args.command);
            command.current_dir(args.cwd.unwrap_or_else(crate::config::get_home_dir));
            let result = managed(app, run, command, Duration::from_secs(15 * 60)).await?;
            Ok(output_value(&result))
        }
        "bind_tool" | "set_tool_model" => {
            let tool = tool_name(args)?;
            let target = crate::model_selection::canonical_tool(&tool)?;
            let revision = revision.ok_or("assistant_revision_unavailable")?;
            let _selection = SELECTION_LOCK.lock().await;
            recheck(app, run).await?;
            if crate::model_selection::revision(&app.state::<AppState>().db, &target)? != revision {
                return Err("assistant_configuration_changed".into());
            }
            WRITE_AUTHORITY
                .scope(
                    (app.clone(), run.clone()),
                    EXPECTED_WRITE.scope(ExpectedWrite { target, revision }, async {
                        if name == "bind_tool" {
                            bind(app, run, &tool, tool_args(args)?.model_id.as_deref()).await
                        } else {
                            set_model(
                                app,
                                run,
                                &tool,
                                tool_args(args)?
                                    .model_id
                                    .as_deref()
                                    .ok_or("assistant_model_required")?,
                            )
                            .await
                        }
                    }),
                )
                .await
        }
        _ => Err("assistant_unknown_tool".into()),
    }
}

async fn configuration_status(
    app: &tauri::AppHandle,
    args: &Value,
    detailed: bool,
) -> Result<Value, String> {
    let typed = tool_args(args)?;
    let tools = typed.tool.map(|tool| vec![tool]).unwrap_or_else(|| {
        crate::model_selection::TOOLS
            .iter()
            .map(|tool| (*tool).into())
            .collect()
    });
    let mut statuses = Vec::new();
    for tool in tools {
        let state = app.state::<AppState>();
        let binding = if let Some(native) = BindableTool::native(&tool) {
            crate::native_tools::binding_status(&state.db, native).await
        } else if tool == "workbuddy" {
            crate::workbuddy_config::binding_status(&state.db).await
        } else {
            // The legacy inspector may initialize a login-shell directory cache
            // or run Codex's strict-config probe. The assistant does neither.
            crate::services::ofox_bind::status::ToolBindingStatus::unknown()
        };
        let revision = crate::model_selection::revision(&state.db, &tool).ok();
        statuses.push(json!({"tool":tool,"binding":binding,"revision":revision,"diagnosticsRequired":BindableTool::native(&tool).is_none() && tool!="workbuddy","envOverrides":crate::services::env_override::find(&tool)}));
    }
    let partial = statuses
        .iter()
        .any(|status| status["diagnosticsRequired"] == true);
    Ok(
        json!({"tools":statuses,"partialCoverage":partial,"requiresUserAction":partial,"page":"tools","coverage":if detailed {"Managed CLI version checks plus static configuration indicators"} else {"Static configuration indicators only"},"note":if partial {Some("Legacy managed-field configuration inspection is incomplete; use the Tools page for full checks")} else {None}}),
    )
}

async fn set_model(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    tool: &str,
    model: &str,
) -> Result<Value, String> {
    let canonical = crate::model_selection::canonical_tool(tool)?;
    let tool = canonical.as_str();
    let state = app.state::<AppState>();
    let auth = app.state::<OfoxAuthState>();
    let catalog = model_selection::catalog(&auth, "openai").await?;
    let selected = catalog
        .iter()
        .find(|item| item.id == model)
        .ok_or("assistant_model_not_in_catalog")?;
    recheck(app, run).await?;
    if tool == "workbuddy" {
        crate::commands::manage_tool::set_workbuddy_managed_models(
            state,
            auth,
            vec![model_selection::workbuddy_selection(selected)],
        )
        .await?;
    } else if tool == "opencode" {
        crate::commands::manage_tool::set_active_ofox_models(
            state,
            auth,
            tool.into(),
            vec![model.into()],
        )
        .await?;
    } else {
        crate::commands::manage_tool::set_active_ofox_model(
            state,
            auth,
            tool.into(),
            model.into(),
            None,
            None,
        )
        .await?;
    }
    model_selection::notify(app, vec![crate::model_selection::canonical_tool(tool)?]);
    Ok(json!({"status":"configured","tool":tool,"modelId":model}))
}

async fn bind(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    tool: &str,
    model: Option<&str>,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let auth = app.state::<OfoxAuthState>();
    let settings = crate::settings::get_settings();
    if !settings.ofox_apex_resolved.unwrap_or(false) && !settings.ofox_apex_pinned.unwrap_or(false)
    {
        return Ok(handoff(
            "Resolve the Ofox region in Settings before writing tool configuration",
        ));
    }
    let selections = if tool == "workbuddy" {
        let catalog = model_selection::catalog(&auth, "openai").await?;
        let selected = if let Some(model) = model {
            catalog.iter().find(|item| item.id == model)
        } else {
            catalog
                .iter()
                .find(|item| item.id == "openai/gpt-6.1-sol")
                .or_else(|| catalog.first())
        }
        .ok_or("assistant_no_compatible_models")?;
        Some(vec![model_selection::workbuddy_selection(selected)])
    } else {
        None
    };
    recheck(app, run).await?;
    crate::commands::ofox_auth::bind_tool_to_ofox_internal(&state.db, &auth.0, tool, selections)
        .await?;
    model_selection::notify(app, vec![crate::model_selection::canonical_tool(tool)?]);
    if let Some(model) = model.filter(|_| tool != "workbuddy") {
        // Binding was committed. A later model change has its own fresh revision;
        // preserve that completed result if compatibility validation rejects it.
        let target = crate::model_selection::canonical_tool(tool)?;
        let revision = crate::model_selection::revision(&state.db, &target)?;
        let result = EXPECTED_WRITE
            .scope(
                ExpectedWrite { target, revision },
                set_model(app, run, tool, model),
            )
            .await;
        if let Err(error) = result {
            return Ok(
                json!({"status":"partiallyCompleted","tool":tool,"binding":"configured","modelError":error}),
            );
        }
    }
    Ok(json!({"status":"configured","tool":tool,"modelId":model}))
}

fn output_value(output: &std::process::Output) -> Value {
    json!({"exitCode":output.status.code(),"success":output.status.success(),"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr),"outputLimitBytes":65536})
}

async fn managed(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    command: Command,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    recheck(app, run).await?;
    let future = process::run(command, run.cancel.subscribe(), timeout);
    tokio::pin!(future);
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {result=&mut future=>return result,_=ticker.tick()=>recheck(app,run).await?}
    }
}

fn shell_command(text: &str) -> Command {
    #[cfg(windows)]
    let mut command = {
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let mut command = Command::new(root.join("System32/WindowsPowerShell/v1.0/powershell.exe"));
        command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", text]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", text]);
        command
    };
    process::clean_environment(&mut command);
    command
}

fn program_command(program: &Path, args: &[String], path: &str) -> Result<Command, String> {
    #[cfg(windows)]
    let mut command = {
        if program.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        }) {
            if args.iter().any(|arg| arg.contains(['"', '\0', '\r', '\n'])) {
                return Err("assistant_invalid_process_arguments".into());
            }
            let root = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
            let mut command = Command::new(root.join("System32/cmd.exe"));
            process::clean_environment(&mut command);
            let mut line = "\"\"%OFOX_ASSISTANT_PROGRAM%\"".to_string();
            command.env("OFOX_ASSISTANT_PROGRAM", program);
            for (index, arg) in args.iter().enumerate() {
                let name = format!("OFOX_ASSISTANT_ARG_{index}");
                line.push_str(&format!(" \"%{name}%\""));
                command.env(name, arg);
            }
            line.push('"');
            command.args(["/D", "/V:OFF", "/S", "/C"]).raw_arg(line);
            command
        } else {
            let mut command = Command::new(program);
            process::clean_environment(&mut command);
            command.args(args);
            command
        }
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new(program);
        process::clean_environment(&mut command);
        command.args(args);
        command
    };
    command
        .env("PATH", path)
        .current_dir(crate::config::get_home_dir());
    Ok(command)
}

async fn probe(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    tool: &str,
) -> Result<tool_update::Installation, String> {
    if !tool
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err("assistant_invalid_executable".into());
    }
    #[cfg(windows)]
    let (path, search_path) = {
        let path =
            crate::commands::windows_tools::find_tool(tool).ok_or("assistant_program_missing")?;
        (
            path,
            crate::commands::windows_tools::effective_path()
                .to_string_lossy()
                .into_owned(),
        )
    };
    #[cfg(not(windows))]
    let (path, search_path) = {
        let (fallback, raw_path, _error) = crate::commands::cli_search_paths(tool);
        let mut directories = std::env::split_paths(&raw_path)
            .filter(|path| path.is_absolute())
            .collect::<Vec<_>>();
        for directory in fallback {
            if directory.is_absolute() && !directories.contains(&directory) {
                directories.push(directory);
            }
        }
        let path = directories
            .iter()
            .map(|directory| directory.join(tool))
            .find(|path| {
                use std::os::unix::fs::PermissionsExt;
                path.metadata().is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
            .ok_or("assistant_program_missing")?;
        let search_path = std::env::join_paths(&directories)
            .map_err(|_| "assistant_launch_path_unavailable")?
            .to_string_lossy()
            .into_owned();
        (path, search_path)
    };
    let real = std::fs::canonicalize(&path).map_err(|_| "assistant_program_unreadable")?;
    let command = program_command(&path, &["--version".into()], &search_path)?;
    let output = managed(app, run, command, Duration::from_secs(15)).await;
    let (version, error) = tool_update::executable_version(output);
    Ok(tool_update::Installation {
        path,
        real,
        version,
        error,
        search_path,
    })
}

async fn diagnose(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    tool: &str,
) -> Result<Value, String> {
    let configuration = configuration_status(app, &json!({"tool":tool}), true).await?;
    let executable = crate::model_selection::canonical_tool(tool)?;
    if executable == "workbuddy" {
        return Ok(json!({"configuration":configuration,"cli":"notApplicable"}));
    }
    let installation = probe(app, run, &executable).await?;
    let dependency_check = if executable == "hermes" {
        Some(output_value(
            &managed(
                app,
                run,
                program_command(
                    &installation.path,
                    &["--help".into()],
                    &installation.search_path,
                )?,
                Duration::from_secs(15),
            )
            .await?,
        ))
    } else {
        None
    };
    Ok(
        json!({"requiresUserAction":configuration["requiresUserAction"],"page":"tools","configuration":configuration,"version":installation.version,"versionError":installation.error,"dependencyCheck":dependency_check}),
    )
}

fn handoff(reason: &str) -> Value {
    json!({"status":"requiresUserAction","requiresUserAction":true,"page":"tools","reason":reason})
}

fn unknown(error: impl std::fmt::Display) -> String {
    format!("assistant_operation_outcome_unknown: {error}")
}

async fn mutate_process(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    command: Command,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    managed(app, run, command, timeout).await.map_err(unknown)
}

async fn verify_install(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    tool: &str,
    output: std::process::Output,
) -> Result<Value, String> {
    if !output.status.success() {
        return Err(unknown(
            serde_json::to_string(&output_value(&output)).unwrap_or_default(),
        ));
    }
    let installed = probe(app, run, tool).await.map_err(unknown)?;
    if installed.error.is_some() {
        return Err(unknown("assistant_install_verification_failed"));
    }
    if tool == "hermes" {
        let help = managed(
            app,
            run,
            program_command(&installed.path, &["--help".into()], &installed.search_path)
                .map_err(unknown)?,
            Duration::from_secs(15),
        )
        .await
        .map_err(unknown)?;
        if !help.status.success() {
            return Err(unknown("assistant_install_verification_failed"));
        }
    }
    Ok(
        json!({"status":"installed","tool":tool,"version":installed.version,"process":output_value(&output)}),
    )
}

async fn lifecycle(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    name: &str,
    tool: &str,
) -> Result<Value, String> {
    if matches!(tool, "chatgpt" | "workbuddy") {
        return Ok(handoff(
            "Complete this desktop application's native installer or update from the Tools page",
        ));
    }
    if !cfg!(any(target_os = "macos", target_os = "windows"))
        || (name == "install_tool" && cfg!(all(target_os = "macos", not(target_arch = "aarch64"))))
    {
        return Ok(handoff(
            "Automatic installation is unavailable on this platform",
        ));
    }
    let initial_preview = json!({"tool":tool,"action":name,"checks":["Find the installed executable without starting a login shell","Run the fixed --version check and inspect the original installation source"],"changes":"If needed, install or update this CLI and its dependencies; the exact command is confirmed separately"});
    app.state::<host::AssistantHost>()
        .authorize(
            app,
            run,
            true,
            name,
            &initial_preview,
            OperationClass::Configure,
        )
        .await?;
    let _operation = tool_update::ToolOperationGuard::acquire(tool)?;
    if name == "update_tool" {
        return update(app, run, tool).await;
    }
    match probe(app,run,tool).await {
        Ok(installed) if installed.error.is_none()=>return Ok(json!({"status":"alreadyInstalled","tool":tool,"version":installed.version})),
        Ok(_)=>return Ok(handoff("The existing installation needs repair. Use its original installer or the update action; another installation will not be added")),
        Err(error) if error=="assistant_program_missing"=>{},
        Err(error)=>return Err(error),
    }
    let mut preview = json!({"tool":tool,"source":"official","action":"install","changes":"Install the CLI and its package dependencies"});
    if tool == "hermes" {
        preview["installer"] = json!(if cfg!(windows) {
            "https://hermes-agent.nousresearch.com/install.ps1"
        } else {
            "https://hermes-agent.nousresearch.com/install.sh"
        });
        app.state::<host::AssistantHost>()
            .authorize(app, run, true, name, &preview, OperationClass::Configure)
            .await?;
        return install_hermes(app, run).await;
    }
    #[cfg(not(windows))]
    let package = tool_update::npm_package(tool).ok_or("assistant_install_unsupported")?;
    #[cfg(windows)]
    {
        let script = crate::commands::windows_install::install_script_with(
            tool,
            crate::commands::windows_install::InstallHost {
                mainland: crate::ofox_apex::current_apex() == "ofox.io",
                node_here: crate::commands::windows_tools::find_tool("npm").is_some(),
                winget_here: crate::commands::windows_tools::find_tool("winget").is_some(),
            },
            None,
        )?;
        preview["command"] = json!(script);
        app.state::<host::AssistantHost>()
            .authorize(app, run, true, name, &preview, OperationClass::Configure)
            .await?;
        let output = mutate_process(
            app,
            run,
            shell_command(&script),
            Duration::from_secs(15 * 60),
        )
        .await?;
        return verify_install(app, run, tool, output).await;
    }
    #[cfg(not(windows))]
    {
        let npm=match probe(app,run,"npm").await {Ok(npm) if npm.error.is_none()=>npm,_=>return Ok(handoff("Install or repair Node.js and npm from the Tools page, then retry this CLI installation"))};
        let args = vec![
            "install".into(),
            "--global".into(),
            format!("{package}@latest"),
        ];
        preview["program"] = json!(npm.path);
        preview["args"] = json!(args);
        app.state::<host::AssistantHost>()
            .authorize(app, run, true, name, &preview, OperationClass::Configure)
            .await?;
        let output = mutate_process(
            app,
            run,
            program_command(&npm.path, &args, &npm.search_path)?,
            Duration::from_secs(15 * 60),
        )
        .await?;
        verify_install(app, run, tool, output).await
    }
}

async fn install_hermes(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
) -> Result<Value, String> {
    use futures::StreamExt;
    let url = if cfg!(windows) {
        "https://hermes-agent.nousresearch.com/install.ps1"
    } else {
        "https://hermes-agent.nousresearch.com/install.sh"
    };
    let mut cancel = run.cancel.subscribe();
    let response = tokio::select! {
        _=cancel.changed()=>return Err("assistant_cancelled".into()),
        response=crate::proxy::http_client::get().get(url).timeout(Duration::from_secs(60)).send()=>response.map_err(|_|"assistant_installer_download_failed")?,
    };
    if !response.status().is_success() {
        return Err("assistant_installer_download_failed".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {_=cancel.changed()=>return Err("assistant_cancelled".into()),chunk=stream.next()=>chunk};
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(|_| "assistant_installer_download_failed")?;
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("assistant_installer_too_large".into());
        }
        bytes.extend_from_slice(&chunk);
        recheck(app, run).await?;
    }
    if bytes.is_empty() || std::str::from_utf8(&bytes).is_err() {
        return Err("assistant_installer_download_failed".into());
    }
    let work = tempfile::Builder::new()
        .prefix("ofox-assistant-installer-")
        .tempdir()
        .map_err(|_| "assistant_installer_storage_failed")?;
    let file = work.path().join(if cfg!(windows) {
        "install.ps1"
    } else {
        "install.sh"
    });
    crate::config::atomic_write(&file, &bytes).map_err(|_| "assistant_installer_storage_failed")?;
    #[cfg(windows)]
    let mut command = {
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let mut command = Command::new(root.join("System32/WindowsPowerShell/v1.0/powershell.exe"));
        process::clean_environment(&mut command);
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&file)
            .args(["-NonInteractive", "-SkipBrowser"]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("/bin/bash");
        process::clean_environment(&mut command);
        command
            .arg(&file)
            .args(["--non-interactive", "--skip-setup", "--skip-browser"]);
        command
    };
    command.current_dir(crate::config::get_home_dir());
    let output = mutate_process(app, run, command, Duration::from_secs(15 * 60)).await?;
    verify_install(app, run, "hermes", output).await
}

async fn update(
    app: &tauri::AppHandle,
    run: &Arc<host::RunControl>,
    tool: &str,
) -> Result<Value, String> {
    let before = probe(app, run, tool).await?;
    let plan = tool_update::resolve_plan_cancellable(tool, &before, run.cancel.subscribe()).await?;
    let latest = crate::commands::latest_tool_version_for_package(
        tool,
        Some(&before.version),
        tool_update::installed_npm_package(tool, &before.path, &before.real),
    )
    .await
    .ok_or("assistant_latest_version_unavailable")?;
    if before.error.is_none()
        && tool_update::version_status(Some(&before.version), Some(&latest)) != "available"
    {
        return Ok(json!({"status":"current","tool":tool,"version":before.version}));
    }
    if before.error.is_some() && plan.source != "pnpm" {
        return Err("assistant_repair_requires_original_installer".into());
    }
    let args = tool_update::execution_args(&plan, &before, &latest);
    let preview = json!({"tool":tool,"source":plan.source,"program":plan.program,"args":args,"currentVersion":before.version,"latestVersion":latest});
    app.state::<host::AssistantHost>()
        .authorize(
            app,
            run,
            true,
            "update_tool",
            &preview,
            OperationClass::Configure,
        )
        .await?;
    let fresh = probe(app, run, tool).await?;
    if fresh.real != before.real || fresh.version != before.version {
        return Err("assistant_installation_changed".into());
    }
    let fresh_plan =
        tool_update::resolve_plan_cancellable(tool, &fresh, run.cancel.subscribe()).await?;
    if fresh_plan.source != plan.source
        || fresh_plan.program != plan.program
        || fresh_plan.args != plan.args
    {
        return Err("assistant_installation_changed".into());
    }
    let mut command = program_command(&plan.program, &args, &plan.path)?;
    command.env("CI", "1");
    let output = mutate_process(app, run, command, Duration::from_secs(15 * 60)).await?;
    if !output.status.success() {
        return Err(format!(
            "assistant_operation_outcome_unknown: update exited unsuccessfully: {}",
            serde_json::to_string(&output_value(&output)).unwrap_or_default()
        ));
    }
    let after = probe(app, run, tool).await.map_err(unknown)?;
    if after.error.is_some() {
        return Err(unknown("assistant_update_verification_failed"));
    }
    let after_plan = tool_update::resolve_plan_cancellable(tool, &after, run.cancel.subscribe())
        .await
        .map_err(unknown)?;
    if after_plan.source != plan.source
        || after_plan.program != plan.program
        || after_plan.args != plan.args
    {
        return Err(unknown("assistant_installation_changed"));
    }
    let verification = tool_update::verify_result(&before.version, &after.version, &latest);
    Ok(
        json!({"status":if before.error.is_some() {"repaired"} else {verification.status.as_str()},"tool":tool,"before":before.version,"after":after.version,"process":output_value(&output)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(windows)]
    #[tokio::test]
    #[serial_test::serial]
    async fn system_powershell_is_absolute_noninteractive_and_handles_unicode_with_no_tool_path() {
        let sandbox = crate::test_sandbox::sandbox();
        let directory = sandbox.home().join("配置助手 空 格");
        let fake_bin = directory.join("fake bin");
        std::fs::create_dir_all(&fake_bin).unwrap();
        let marker = directory.join("path-command-was-used");
        std::fs::write(
            fake_bin.join("powershell.cmd"),
            "@echo off\r\necho wrong-path>\"%OFOX_FAKE_PATH_MARKER%\"\r\nexit /b 91\r\n",
        )
        .unwrap();
        let mut command = shell_command(
            "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); [ordered]@{message='助手 Unicode ✓';cwd=(Get-Location).ProviderPath;edition=$PSVersionTable.PSEdition;major=$PSVersionTable.PSVersion.Major;path=$env:PATH} | ConvertTo-Json -Compress",
        );
        let system_root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        assert_eq!(
            command.as_std().get_program(),
            system_root
                .join("System32/WindowsPowerShell/v1.0/powershell.exe")
                .as_os_str()
        );
        let flags = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            &flags[..4],
            &["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]
        );
        // Only a deliberately fake same-name command is on PATH. The script
        // uses PowerShell/.NET builtins and explicitly requests UTF-8 output.
        command
            .env("PATH", &fake_bin)
            .env("OFOX_FAKE_PATH_MARKER", &marker)
            .current_dir(&directory);
        let (_sender, cancel) = tokio::sync::watch::channel(false);
        let output = process::run(command, cancel, Duration::from_secs(20))
            .await
            .unwrap();
        assert!(output.status.success(), "system PowerShell fixture failed");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["message"], "助手 Unicode ✓");
        assert_eq!(value["edition"], "Desktop");
        assert_eq!(value["major"], 5);
        assert_eq!(value["path"], fake_bin.to_string_lossy().as_ref());
        assert_eq!(
            std::fs::canonicalize(value["cwd"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(&directory).unwrap()
        );
        assert!(!marker.exists());
    }

    #[test]
    fn repeated_tool_calls_are_not_replayed_after_an_uncertain_attempt() {
        let db = crate::database::Database::memory().unwrap();
        let owner = store::namespace("fixture-user", "fixture-org").unwrap();
        let mut detail = store::create(
            &db,
            &owner,
            AssistantKind::Configuration,
            "fixture/model",
            false,
        )
        .unwrap();
        let (cancel, _) = tokio::sync::watch::channel(false);
        let run = host::RunControl {
            identity: host::Identity {
                owner,
                apex: "ofox.ai".into(),
            },
            session_id: detail.session.id.clone(),
            run_id: id(),
            operation_id: id(),
            message_id: id(),
            cancel,
            epoch: 0,
            approvals: std::sync::Mutex::new(std::collections::HashMap::new()),
            secret: "opaque-fixture-token".into(),
            search_capabilities: json!({}),
            deadline: tokio::time::Instant::now() + Duration::from_secs(900),
            outcome_unknown: std::sync::atomic::AtomicBool::new(false),
        };
        let args = json!({"tool":"codex","note":"opaque-fixture-token"});
        let first =
            claim_operation(&mut detail, &run, "install_tool", "call-fixture", &args).unwrap();
        assert!(!first.to_string().contains("opaque-fixture-token"));
        assert_eq!(
            claim_operation(&mut detail, &run, "install_tool", "call-fixture", &args).unwrap_err(),
            "assistant_tool_call_already_attempted"
        );
        assert_eq!(detail.operations.len(), 1);
        assert_eq!(detail.operations[0]["status"], "running");
        detail.operations[0]["status"] = json!("unknown");
        assert_eq!(
            claim_operation(
                &mut detail,
                &run,
                "install_tool",
                "different-call-id",
                &args
            )
            .unwrap_err(),
            "assistant_operation_outcome_unknown"
        );
        assert_eq!(detail.operations.len(), 1);
    }

    #[test]
    fn only_registered_host_tools_receive_a_permission_class() {
        assert_eq!(operation_class("config_status"), Some(OperationClass::Read));
        assert_eq!(
            operation_class("diagnose_tool"),
            Some(OperationClass::Configure)
        );
        for tool in ["bind_tool", "set_tool_model", "install_tool", "update_tool"] {
            assert_eq!(operation_class(tool), Some(OperationClass::Configure));
        }
        assert_eq!(
            operation_class("run_command"),
            Some(OperationClass::Command)
        );
        assert_eq!(operation_class("read_secret"), None);
        assert_eq!(operation_class("launch_tool"), None);
    }

    #[test]
    fn structured_tools_reject_shell_injection_and_unknown_arguments() {
        assert!(validate_arguments(
            "set_tool_model",
            &json!({"tool":"codex", "modelId":"test/model"})
        )
        .is_ok());
        assert!(validate_arguments(
            "set_tool_model",
            &json!({"tool":"codex; touch marker", "modelId":"test/model"})
        )
        .is_err());
        assert!(validate_arguments(
            "set_tool_model",
            &json!({"tool":"codex", "modelId":"test/model", "command":"arbitrary"})
        )
        .is_err());
        assert!(validate_arguments("set_tool_model", &json!({"tool":"codex"})).is_err());
        assert!(validate_arguments("config_status", &json!({})).is_ok());
        assert!(validate_arguments(
            "run_command",
            &json!({"command":"echo hello", "cwd":"relative"})
        )
        .is_err());
        assert!(validate_arguments("run_command", &json!({"command":""})).is_err());
    }
}
