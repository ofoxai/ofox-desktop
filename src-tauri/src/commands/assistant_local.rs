//! Application adapters for structured local operations. All URLs and credentials
//! are resolved by existing OFox services; no provider request is used as a test.
use crate::{
    app_config::{AppType, BindableTool},
    services::ofox_bind::{self, status::BindingStatus},
};
use serde_json::{json, Value};
use std::str::FromStr;
use tauri::Manager;

pub async fn mutate(
    app: &tauri::AppHandle,
    name: &str,
    tool: BindableTool,
    model: Option<&str>,
    operation: &str,
) -> Result<Value, String> {
    if name == "repair_launcher" {
        #[cfg(unix)]
        {
            let install = super::tool_update::probe_executable(tool.as_str())
                .await
                .map_err(|_| "assistant_local_unavailable")?;
            let home = crate::config::get_home_dir();
            super::node_launcher::ensure(
                tool.as_str(),
                &install,
                &home,
                &std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()),
            )?;
            return Ok(json!({"changed":true}));
        }
        #[cfg(not(unix))]
        return Err("assistant_platform_unsupported".into());
    }
    if matches!(name, "rebind_tool" | "set_tool_model") {
        let settings = crate::settings::get_settings();
        if !settings.ofox_apex_resolved.unwrap_or(false)
            && !settings.ofox_apex_pinned.unwrap_or(false)
        {
            let client = crate::proxy::http_client::get();
            crate::ofox_apex::probe_apex_on_startup(app, &client).await;
        }
        crate::services::assistant::tools::transaction::check_active()?;
    }
    let state = app.state::<crate::store::AppState>();
    let auth = app.state::<super::ofox_auth::OfoxAuthState>();
    match name {
        "install_tool" => {
            return super::install_tool::install_tool(app.clone(), tool.as_str().into(), None, None)
                .await
                .map(|code| json!({"started":true,"code":code,"reversible":false}))
                .map_err(|_| "unknown_outcome".into())
        }
        "update_tool" => {
            return super::tool_update::update_tool(
                app.clone(),
                tool.as_str().into(),
                operation.into(),
            )
            .await
            .and_then(|v| serde_json::to_value(v).map_err(|_| "unknown_outcome".into()))
            .map_err(|_| "unknown_outcome".into())
        }
        "uninstall_tool" => {
            return super::uninstall_tool::uninstall_tool(
                app.clone(),
                state,
                auth,
                tool.as_str().into(),
                operation.into(),
                None,
            )
            .await
            .and_then(|v| serde_json::to_value(v).map_err(|_| "unknown_outcome".into()))
            .map_err(|_| "unknown_outcome".into())
        }
        _ => {}
    }
    let native = BindableTool::native(tool.as_str()).is_some();
    let key = crate::ofox_secret::default_store()
        .load(crate::ofox_secret::Slot::ApiKey { tool })
        .map_err(|_| "assistant_key_storage_unavailable")?;
    let health = if native {
        crate::native_tools::binding_status(&state.db, tool).await
    } else if tool == BindableTool::WorkBuddy {
        crate::workbuddy_config::binding_status(&state.db).await
    } else {
        ofox_bind::status::binding_status(
            &state.db,
            ofox_bind::Tool::from_app(
                &AppType::from_str(tool.as_str()).map_err(|_| "assistant_invalid_tool")?,
            ),
            key.as_deref(),
        )
        .await
    };
    if name == "rebind_tool" && health.status == BindingStatus::Missing {
        super::ofox_auth::ofox_restore_tool_binding(
            app.clone(),
            state,
            auth,
            tool.as_str().into(),
            None,
        )
        .await?;
        return Ok(json!({"changed":true}));
    }
    if health.status != BindingStatus::Configured {
        return Err("assistant_restore_conflict".into());
    }
    if name == "rebind_tool" {
        if native {
            // Preserve the user's model and scope selections when refreshing a
            // native binding; the ordinary first-bind flow chooses a new model.
            let key = key.ok_or("assistant_key_unavailable")?;
            crate::native_tools::reconcile_tool_endpoint(&state.db, tool).await?;
            crate::native_tools::replace_key(&state.db, tool, &key, &key).await?;
        } else if tool == BindableTool::WorkBuddy {
            let models = crate::workbuddy_config::active_models(&state.db).await?;
            let catalog = super::model_selection::catalog(&auth, "openai").await?;
            let selections = models
                .iter()
                .map(|id| {
                    catalog
                        .iter()
                        .find(|m| m.id == *id)
                        .map(super::model_selection::workbuddy_selection)
                        .ok_or_else(|| "assistant_model_unavailable".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let key = super::ofox_auth::acquire_bind_token(tool, &auth.0).await?;
            crate::workbuddy_config::update_selected_models(&state.db, &key, &selections).await?;
        } else {
            super::ofox_auth::bind_existing_tool_to_ofox_internal(
                &state.db,
                &auth.0,
                tool.as_str(),
            )
            .await?;
        }
    } else if name == "set_tool_model" {
        let id = model.ok_or("assistant_invalid_tool")?;
        let catalog = super::model_selection::catalog(
            &auth,
            match tool {
                BindableTool::Claude => "anthropic",
                BindableTool::Gemini => "gemini",
                _ => "openai",
            },
        )
        .await?;
        let selected = catalog
            .iter()
            .find(|m| m.id == id)
            .ok_or("assistant_model_unavailable")?;
        let key = key.ok_or("assistant_key_unavailable")?;
        if native {
            crate::native_tools::set_model(&state.db, tool, &key, selected).await?;
        } else if tool == BindableTool::WorkBuddy {
            let selection = super::model_selection::workbuddy_selection(selected);
            crate::workbuddy_config::validate_selections(std::slice::from_ref(&selection))?;
            crate::workbuddy_config::update_selected_models(&state.db, &key, &[selection]).await?;
        } else {
            let app_type =
                AppType::from_str(tool.as_str()).map_err(|_| "assistant_invalid_tool")?;
            let provider_id = format!("ofox-{}", tool.as_str());
            let provider = state
                .db
                .get_provider_by_id(&provider_id, tool.as_str())
                .map_err(|_| "assistant_local_unavailable")?
                .ok_or("assistant_local_unavailable")?;
            let mut settings = provider.settings_config.clone();
            // Catalog validation only: compatibility probes would spend credits
            // outside the assistant's metered request and permission flow.
            let protocol = if tool == BindableTool::OpenCode {
                crate::model_compat::opencode_protocol_for_save(selected)
                    .map_err(|_| "assistant_model_unavailable")?
                    .to_string()
            } else {
                crate::model_compat::native_protocol_for_save(tool, selected)
                    .map_err(|_| "assistant_model_unavailable")?
                    .to_string()
            };
            super::manage_tool::write_model_into_settings(&app_type, &mut settings, id)?;
            if tool == BindableTool::OpenCode {
                settings["models"][id]["provider"] = json!({"npm":if protocol=="responses" {crate::opencode_config::OFOX_RESPONSES_NPM}else{crate::opencode_config::OFOX_CHAT_NPM}});
            }
            ofox_bind::persist_bound_settings(
                &state.db,
                ofox_bind::Tool::from_app(&app_type),
                &key,
                &provider.settings_config,
                &settings,
            )
            .await?;
        }
    } else {
        return Err("assistant_unknown_tool".into());
    }
    super::model_selection::notify(app, vec![tool.as_str().into()]);
    Ok(json!({"changed":true}))
}
