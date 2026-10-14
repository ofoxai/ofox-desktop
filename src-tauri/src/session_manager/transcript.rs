//! Readable transcripts: a session file's lines as what was said, by whom,
//! with the harness's tags taken out. Adapted from Magpie
//! internal/sessions/content.go and transcript.go.
//! Copyright (c) 2026 yetone, MIT; see MAGPIE_LICENSE.
//!
//! Roles: `user`, `assistant`, `thinking`, `tool_call` (with the tool's name),
//! `tool` (its result), `command` (a slash command), `command_output` and
//! `system` (what the agent or harness put in the conversation itself).
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::LazyLock;

use super::providers::{extract_text, parse_timestamp_to_ms};
use super::title::{is_context, slash_command, without_context};
use super::SessionMessage;

pub const USER: &str = "user";
pub const ASSISTANT: &str = "assistant";
pub const THINKING: &str = "thinking";
pub const TOOL_CALL: &str = "tool_call";
pub const TOOL: &str = "tool";
pub const COMMAND: &str = "command";
pub const COMMAND_OUTPUT: &str = "command_output";
pub const SYSTEM: &str = "system";

static REMINDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<system-reminder>(.*?)</system-reminder>").unwrap());
static CAVEAT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<local-command-caveat>.*?</local-command-caveat>").unwrap());
static OUTPUT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)<(?:local-command-stdout|local-command-stderr|bash-stdout|bash-stderr)>(.*?)</(?:local-command-stdout|local-command-stderr|bash-stdout|bash-stderr)>",
    )
    .unwrap()
});
static BASH_INPUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^<bash-input>(.*?)</bash-input>").unwrap());
static IMAGE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^</?image\b[^<>]*>$").unwrap());

/// A provider's own role name as one of the transcript roles.
pub fn normalize_role(role: &str) -> &'static str {
    match role.to_ascii_lowercase().as_str() {
        "user" | "human" => USER,
        "assistant" | "model" | "gemini" | "ai" => ASSISTANT,
        "thinking" | "reasoning" => THINKING,
        "tool_call" | "toolcall" => TOOL_CALL,
        "tool" | "toolresult" | "tool_result" | "function" => TOOL,
        "command" => COMMAND,
        "command_output" => COMMAND_OUTPUT,
        _ => SYSTEM,
    }
}

/// Every message's role as a transcript role.
pub fn normalized(messages: Vec<SessionMessage>) -> Vec<SessionMessage> {
    messages
        .into_iter()
        .map(|mut message| {
            message.role = normalize_role(&message.role).to_string();
            message
        })
        .collect()
}

fn message(role: &str, content: impl Into<String>, ts: Option<i64>) -> SessionMessage {
    SessionMessage {
        role: role.to_string(),
        content: content.into(),
        ts,
        name: None,
    }
}

fn named(role: &str, name: Option<&str>, content: String, ts: Option<i64>) -> SessionMessage {
    SessionMessage {
        name: name.map(str::to_string),
        ..message(role, content, ts)
    }
}

/// Collects consecutive text blocks into one message.
struct Pending {
    role: &'static str,
    parts: Vec<String>,
}

impl Pending {
    fn new(role: &'static str) -> Self {
        Self {
            role,
            parts: Vec::new(),
        }
    }

    fn push(&mut self, text: &str) {
        if !text.trim().is_empty() {
            self.parts.push(text.to_string());
        }
    }

    fn flush(&mut self, ts: Option<i64>, out: &mut Vec<SessionMessage>) {
        if self.parts.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.parts).join("\n");
        if self.role == USER {
            out.extend(user_text(&text, ts));
        } else {
            out.push(message(self.role, text.trim(), ts));
        }
    }
}

/// A user text as the harness wrote it: a slash command, its output, a
/// reminder or caveat the agent added, or words someone typed.
fn user_text(text: &str, ts: Option<i64>) -> Option<SessionMessage> {
    let text = CAVEAT.replace_all(text, "");
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(command) = slash_command(text) {
        return Some(message(COMMAND, command, ts));
    }
    if let Some(input) = BASH_INPUT.captures(text) {
        return Some(message(COMMAND, format!("! {}", input[1].trim()), ts));
    }
    if OUTPUT.is_match(text) {
        let output = OUTPUT
            .captures_iter(text)
            .map(|c| c[1].trim().to_string())
            .filter(|c| !c.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        return (!output.is_empty()).then(|| message(COMMAND_OUTPUT, output, ts));
    }
    let typed = REMINDER.replace_all(text, "");
    let typed = typed.trim();
    if typed.is_empty() {
        let reminders = REMINDER
            .captures_iter(text)
            .map(|c| c[1].trim().to_string())
            .filter(|c| !c.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        return (!reminders.is_empty()).then(|| message(SYSTEM, reminders, ts));
    }
    // what the agent put in itself (a notification, a context block)
    let role = if is_context(typed) { SYSTEM } else { USER };
    Some(message(
        role,
        if role == USER {
            without_context(typed)
        } else {
            typed.to_string()
        },
        ts,
    ))
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// A Claude Code session file as a transcript.
pub fn claude(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("Failed to open session file: {e}"))?;
    let mut out = Vec::new();
    let mut tools = HashMap::new();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(msg) = value.get("message") else {
            continue;
        };
        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);
        let content = msg.get("content").unwrap_or(&Value::Null);
        match msg.get("role").and_then(Value::as_str).unwrap_or("") {
            "user" => claude_blocks(USER, content, ts, &mut tools, &mut out),
            "assistant" => claude_blocks(ASSISTANT, content, ts, &mut tools, &mut out),
            other => {
                let text = extract_text(content);
                if !text.trim().is_empty() {
                    out.push(message(normalize_role(other), text, ts));
                }
            }
        }
    }
    Ok(out)
}

fn claude_blocks(
    role: &'static str,
    content: &Value,
    ts: Option<i64>,
    tools: &mut HashMap<String, String>,
    out: &mut Vec<SessionMessage>,
) {
    let Some(blocks) = content.as_array() else {
        let mut pending = Pending::new(role);
        pending.push(content.as_str().unwrap_or_default());
        pending.flush(ts, out);
        return;
    };
    let mut pending = Pending::new(role);
    for block in blocks {
        let field = |key| block.get(key).and_then(Value::as_str);
        match field("type").unwrap_or("") {
            "text" => pending.push(field("text").unwrap_or_default()),
            "image" => pending.push("[Image]"),
            // empty or redacted thinking says nothing
            "redacted_thinking" => {}
            "thinking" => {
                let thinking = field("thinking").unwrap_or_default().trim();
                if !thinking.is_empty() {
                    pending.flush(ts, out);
                    out.push(message(THINKING, thinking, ts));
                }
            }
            "tool_use" => {
                pending.flush(ts, out);
                let name = field("name").unwrap_or("unknown");
                if let Some(id) = field("id") {
                    tools.insert(id.to_string(), name.to_string());
                }
                let input = block.get("input").map(pretty).unwrap_or_default();
                out.push(named(TOOL_CALL, Some(name), input, ts));
            }
            "tool_result" => {
                pending.flush(ts, out);
                let text = block.get("content").map(extract_text).unwrap_or_default();
                if !text.trim().is_empty() {
                    let name = field("tool_use_id").and_then(|id| tools.get(id));
                    out.push(named(TOOL, name.map(String::as_str), text, ts));
                }
            }
            _ => {}
        }
    }
    pending.flush(ts, out);
}

/// A Codex rollout as a transcript.
pub fn codex(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let reader = super::providers::dsh::open(path)?;
    let mut out = Vec::new();
    let mut tools: HashMap<String, String> = HashMap::new();
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);
        let field = |key| payload.get(key).and_then(Value::as_str);
        let call_id = field("call_id");
        match field("type").unwrap_or("") {
            "message" => match field("role").unwrap_or("") {
                "user" => codex_user(payload.get("content"), ts, &mut out),
                role => {
                    let text = payload.get("content").map(extract_text).unwrap_or_default();
                    if !text.trim().is_empty() {
                        let role = if role == "assistant" {
                            ASSISTANT
                        } else {
                            SYSTEM
                        };
                        out.push(message(role, text, ts));
                    }
                }
            },
            "reasoning" => {
                let mut text = payload.get("summary").map(extract_text).unwrap_or_default();
                if text.trim().is_empty() {
                    text = payload.get("content").map(extract_text).unwrap_or_default();
                }
                if !text.trim().is_empty() {
                    out.push(message(THINKING, text.trim(), ts));
                }
            }
            kind @ ("function_call" | "custom_tool_call" | "local_shell_call") => {
                let name = match kind {
                    "local_shell_call" => "shell",
                    _ => field("name").unwrap_or("unknown"),
                };
                if let Some(id) = call_id {
                    tools.insert(id.to_string(), name.to_string());
                }
                let input = match kind {
                    "function_call" => {
                        let raw = field("arguments").unwrap_or_default();
                        serde_json::from_str::<Value>(raw)
                            .map(|v| pretty(&v))
                            .unwrap_or_else(|_| raw.to_string())
                    }
                    "custom_tool_call" => field("input").unwrap_or_default().to_string(),
                    _ => payload
                        .pointer("/action/command")
                        .and_then(Value::as_array)
                        .map(|parts| {
                            parts
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default(),
                };
                out.push(named(TOOL_CALL, Some(name), input, ts));
            }
            "function_call_output" | "custom_tool_call_output" => {
                let text = payload.get("output").map(codex_output).unwrap_or_default();
                if !text.trim().is_empty() {
                    let name = call_id.and_then(|id| tools.get(id));
                    out.push(named(TOOL, name.map(String::as_str), text, ts));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// A user message's items: harness context (the environment, instructions,
/// AGENTS.md, any tagged block) as `system`, the rest as what was typed.
fn codex_user(content: Option<&Value>, ts: Option<i64>, out: &mut Vec<SessionMessage>) {
    let field = |item: &'_ Value, key: &str| -> String {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    // (kind, text) of each item; a message that is one string is one text item
    let items: Vec<(String, String)> = match content {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| (field(item, "type"), field(item, "text")))
            .collect(),
        Some(Value::String(text)) => vec![("input_text".into(), text.clone())],
        _ => return,
    };
    let mut pending = Pending::new(USER);
    for (kind, text) in &items {
        match kind.as_str() {
            "input_image" | "image" => pending.push("[Image]"),
            "input_text" | "text" => {
                let trimmed = text.trim();
                if IMAGE_TAG.is_match(trimmed) {
                    continue;
                }
                if is_context(trimmed) {
                    pending.flush(ts, out);
                    out.push(message(SYSTEM, trimmed, ts));
                } else {
                    pending.push(&without_context(text));
                }
            }
            _ => {}
        }
    }
    pending.flush(ts, out);
}

/// What a Codex tool answered: words, a list of text items, or an object
/// with them in.
fn codex_output(output: &Value) -> String {
    match output {
        Value::String(text) => text.clone(),
        Value::Array(_) => extract_text(output),
        Value::Object(map) => ["content", "output"]
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_str))
            .map(str::to_string)
            .unwrap_or_else(|| output.to_string()),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genuine_tagged_questions_remain_user_messages() {
        for text in [
            "<div>fix this component</div>",
            "<user_query>why?</user_query>",
            "< 5 means what?",
        ] {
            assert_eq!(user_text(text, None).unwrap().role, USER);
            let mut parts = Vec::new();
            codex_user(Some(&serde_json::json!(text)), None, &mut parts);
            assert_eq!(parts[0].role, USER);
            assert_eq!(parts[0].content, text);
        }
    }

    #[test]
    fn provider_roles_map_to_transcript_roles() {
        for (role, want) in [
            ("user", USER),
            ("assistant", ASSISTANT),
            ("gemini", ASSISTANT),
            ("thinking", THINKING),
            ("toolResult", TOOL),
            ("tool", TOOL),
            ("tool_call", TOOL_CALL),
            ("context", SYSTEM),
            ("developer", SYSTEM),
            ("unknown", SYSTEM),
        ] {
            assert_eq!(normalize_role(role), want, "{role}");
        }
    }

    #[test]
    fn a_user_text_made_only_of_a_caveat_or_empty_output_says_nothing() {
        assert!(user_text(
            "<local-command-caveat>Caveat: x</local-command-caveat>",
            None
        )
        .is_none());
        assert!(user_text("<local-command-stdout>  </local-command-stdout>", None).is_none());
        let bash = user_text("<bash-input> ls -la</bash-input>", None).unwrap();
        assert_eq!(
            (bash.role.as_str(), bash.content.as_str()),
            (COMMAND, "! ls -la")
        );
        let output = user_text(
            "<bash-stdout>a.txt</bash-stdout><bash-stderr></bash-stderr>",
            None,
        )
        .unwrap();
        assert_eq!(
            (output.role.as_str(), output.content.as_str()),
            (COMMAND_OUTPUT, "a.txt")
        );
    }
}
