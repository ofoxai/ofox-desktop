//! Host-kept task checklists. They describe a reply and grant no capabilities.
use super::{
    host::{blocking, scrub_secrets, scrub_secrets_in, AssistantHost, RunControl},
    tools::events::task_event,
    types::{scrub_task_step, TaskProgress, TaskStep},
};
use serde_json::{json, Value};
use std::sync::Arc;

/// Validate the full model labels first, remove all captured credentials from
/// those labels, then bound their safe presentation. No scrub sees statuses.
fn safe_steps(
    mut data: Value,
    snapshot: &Value,
    secrets: &[String],
) -> Result<Vec<TaskStep>, String> {
    task_event("task_plan", &data)?;
    super::mcp::scrub_task_steps(&mut data, snapshot)?;
    let mut steps: Vec<TaskStep> =
        serde_json::from_value(data["steps"].clone()).map_err(|_| "assistant_runtime_protocol")?;
    for step in &mut steps {
        step.step = scrub_task_step(&scrub_secrets(&step.step, secrets));
    }
    Ok(steps)
}

pub(super) fn safe_progress(
    value: Option<&Value>,
    snapshot: &Value,
    secrets: &[String],
) -> Result<Option<Value>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let mut progress: TaskProgress =
        serde_json::from_value(value.clone()).map_err(|_| "assistant_runtime_protocol")?;
    if !progress.valid() {
        return Err("assistant_runtime_protocol".into());
    }
    progress.steps = safe_steps(json!({"steps":progress.steps}), snapshot, secrets)?;
    serde_json::to_value(progress)
        .map(Some)
        .map_err(|_| "assistant_runtime_protocol".into())
}

fn canonical_message_status(message: &Value, steering: bool) -> Option<String> {
    let status = message.get("status")?.as_str()?;
    let allowed = match message.get("role").and_then(Value::as_str) {
        Some("assistant") => matches!(status, "streaming" | "completed" | "stopped" | "error"),
        Some("user") if steering => matches!(status, "queued" | "completed" | "error"),
        _ => false,
    };
    allowed.then(|| status.to_owned())
}

/// Only exact host-authored task metadata and canonical chat outcome paths
/// get typed treatment. Ordinary text and other event paths keep their scrub.
pub(super) fn scrub_event(
    kind: &str,
    data: &mut Value,
    snapshot: &Value,
    secrets: &[String],
) -> Result<(), String> {
    if kind == "task_plan" {
        // save_task_plan is the only writer: messageId/start/status are host
        // routing and enum values, while each complete label is scrubbed here.
        data["steps"] = serde_json::to_value(safe_steps(data.clone(), snapshot, secrets)?)
            .map_err(|_| "assistant_runtime_protocol")?;
        return Ok(());
    }
    let message = if kind == "message" {
        safe_progress(
            data.get("message")
                .and_then(|message| message.get("taskProgress")),
            snapshot,
            secrets,
        )?
    } else {
        None
    };
    let message_status = if kind == "message" {
        data.get("message")
            .and_then(|message| canonical_message_status(message, false))
    } else {
        None
    };
    let final_status = if kind == "run_end" {
        data.get("status")
            .and_then(Value::as_str)
            .filter(|status| matches!(*status, "completed" | "stopped" | "error"))
            .map(str::to_owned)
    } else {
        None
    };
    let mut steering = Vec::new();
    let mut steering_statuses = Vec::new();
    if kind == "steer_applied" {
        for (index, message) in data
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let Some(progress) = safe_progress(message.get("taskProgress"), snapshot, secrets)? {
                steering.push((index, progress));
            }
            if let Some(status) = canonical_message_status(message, true) {
                steering_statuses.push((index, status));
            }
        }
    }
    scrub_secrets_in(data, secrets);
    if let Some(progress) = message {
        data["message"]["taskProgress"] = progress;
    }
    if let Some(status) = message_status {
        data["message"]["status"] = json!(status);
    }
    if let Some(status) = final_status {
        data["status"] = json!(status);
    }
    for (index, progress) in steering {
        data["messages"][index]["taskProgress"] = progress;
    }
    for (index, status) in steering_statuses {
        data["messages"][index]["status"] = json!(status);
    }
    Ok(())
}

impl AssistantHost {
    pub(super) async fn save_task_plan(
        &self,
        control: &Arc<RunControl>,
        data: Value,
    ) -> Result<(), String> {
        self.validate_run(control)?;
        if self.identity().await? != control.identity {
            return Err("assistant_identity_changed".into());
        }
        let steps = safe_steps(data, &control.mcp, &control.secrets())?;
        let store = self.store(&control.identity.owner)?;
        let run = control.clone();
        let mid = run.reply_id();
        let target = mid.clone();
        let progress = blocking(move || {
            store.mutate(&run.session_id, |record| {
                if run.cancelled() {
                    return Err("assistant_cancelled".into());
                }
                let message = record
                    .messages
                    .iter_mut()
                    .find(|message| {
                        message.id == target
                            && message.role == "assistant"
                            && message.status == "streaming"
                            && message
                                .run_id
                                .as_deref()
                                .is_some_and(|id| run.owns_native_run(id))
                    })
                    .ok_or("assistant_state_unavailable")?;
                // Only older, already admitted replies lack a snapshot. New
                // replies capture their start before native history can retime them.
                let progress = message
                    .task_progress
                    .get_or_insert_with(|| TaskProgress::new(message.created_at));
                progress.steps = steps;
                Ok(progress.clone())
            })
        })
        .await?;
        self.validate_run(control)?;
        self.emit(
            control,
            "task_plan",
            json!({"messageId":mid,"startedAt":progress.started_at,"steps":progress.steps}),
        );
        Ok(())
    }
}
