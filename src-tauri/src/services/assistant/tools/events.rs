//! Engine-independent events. Never publish arbitrary tool arguments or output.
use serde_json::{json, Map, Value};
const PROTOCOL: &str = "assistant_runtime_protocol";

/// A tool event's tool and call; anything else is a protocol error.
fn tool_call(data: &Value) -> Result<(&str, &str), String> {
    match (data["name"].as_str(), data["toolCallId"].as_str()) {
        (Some(name), Some(call)) if !name.is_empty() && !call.is_empty() => Ok((name, call)),
        _ => Err(PROTOCOL.into()),
    }
}

/// The `tool_start` the renderer is shown: the tool, its call and the one
/// argument worth showing (the query, the page, the attachment).
pub(crate) fn tool_start_event(data: &Value) -> Result<Value, String> {
    let (name, call) = tool_call(data)?;
    let shown = match name {
        "web_search" => Some("query"),
        "web_fetch" => Some("url"),
        "read_attachment" => Some("id"),
        "read" | "write" | "edit" | "ls" | "find" | "grep" => Some("path"),
        "tool_status" | "repair_launcher" | "rebind_tool" | "set_tool_model" => Some("tool"),
        "diagnose_in_folder" | "verify_in_folder" => Some("folder"),
        _ => None,
    };
    let mut args = Map::new();
    if let Some((key, value)) =
        shown.and_then(|key| Some((key, data["args"][key].as_str()?.to_owned())))
    {
        args.insert(key.into(), json!(value));
    }
    let mut shown = json!({"name": name, "toolCallId": call, "args": args});
    if let Some(intent) = data["args"]["intent"].as_str() {
        shown["intent"] = json!(crate::redact::scrub(intent)
            .chars()
            .take(200)
            .collect::<String>());
    }
    Ok(shown)
}

/// A tool failure's code: the runtime's `details.code`, else the head of its
/// `<code>: <message>`; `None` for free text.
fn failure_code(data: &Value) -> Option<String> {
    let is_code = |code: &&str| {
        (1..=64).contains(&code.len())
            && code.starts_with(|c: char| c.is_ascii_lowercase())
            && code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    data["details"]["code"]
        .as_str()
        .filter(is_code)
        .or_else(|| {
            data["message"]
                .as_str()
                .and_then(|message| message.split_once(':'))
                .map(|(code, _)| code)
                .filter(is_code)
        })
        .map(str::to_owned)
}

/// The `tool_end` the renderer is shown: whether the call succeeded and, if
/// not, its code. The model's error text is not shown.
pub(crate) fn tool_end_event(data: &Value) -> Result<Value, String> {
    let (name, call) = tool_call(data)?;
    let success = data["success"].as_bool().ok_or(PROTOCOL)?;
    let code = if success { None } else { failure_code(data) };
    Ok(json!({"name": name, "toolCallId": call, "success": success, "code": code}))
}

/// Only bounded task presentation crosses the engine-neutral event boundary.
pub(crate) fn task_event(kind: &str, data: &Value) -> Result<Value, String> {
    if kind == "task_plan" {
        let steps = data["steps"].as_array().ok_or(PROTOCOL)?;
        if steps.len() > 12 {
            return Err(PROTOCOL.into());
        }
        let mut active = 0;
        let mut shown = Vec::new();
        for step in steps {
            let text = step["step"].as_str().ok_or(PROTOCOL)?;
            let status = step["status"].as_str().ok_or(PROTOCOL)?;
            if text.trim().is_empty()
                || text.chars().count() > 40
                || !matches!(status, "pending" | "in_progress" | "completed")
            {
                return Err(PROTOCOL.into());
            }
            active += usize::from(status == "in_progress");
            shown.push(json!({"step":crate::redact::scrub(text),"status":status}));
        }
        if active > 1 {
            return Err(PROTOCOL.into());
        }
        return Ok(json!({"steps":shown}));
    }
    let mut shown = Map::new();
    if let Some(warning) = data.get("warning") {
        let warning = warning.as_str().ok_or(PROTOCOL)?;
        if !matches!(
            warning,
            "repeat_warning" | "verification_required" | "stopped"
        ) {
            return Err(PROTOCOL.into());
        }
        shown.insert("warning".into(), json!(warning));
    }
    if let Some(cost) = data.get("cost") {
        let cost = cost
            .as_f64()
            .filter(|c| c.is_finite() && *c >= 0.0)
            .ok_or(PROTOCOL)?;
        shown.insert("cost".into(), json!(cost));
    }
    Ok(Value::Object(shown))
}

#[cfg(test)]
mod task_tests {
    use super::*;
    #[test]
    fn task_events_are_bounded_and_do_not_publish_arbitrary_fields() {
        assert!(task_event("task_plan", &json!({"steps":[{"step":"x","status":"in_progress"},{"step":"y","status":"in_progress"}]})).is_err());
        assert!(task_event(
            "task_plan",
            &json!({"steps":[{"step":"x".repeat(41),"status":"pending"}]})
        )
        .is_err());
        let shown=task_event("task_plan", &json!({"steps":[{"step":"api_key=do-not-display","status":"pending","secret":"hidden"}]})).unwrap();
        assert!(!shown.to_string().contains("do-not-display"));
        assert!(!shown.to_string().contains("hidden"));
        assert!(task_event("task_status", &json!({"warning":"arbitrary words"})).is_err());
        assert!(task_event("task_status", &json!({"cost":-1})).is_err());
    }
}
