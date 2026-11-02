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
        _ => None,
    };
    let mut args = Map::new();
    if let Some((key, value)) =
        shown.and_then(|key| Some((key, data["args"][key].as_str()?.to_owned())))
    {
        args.insert(key.into(), json!(value));
    }
    Ok(json!({"name": name, "toolCallId": call, "args": args}))
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
