//! Body-free message facts. Only classification reads content; the index never
//! retains prompts, replies, tool input, or credentials.
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptFacts {
    pub session_id: Option<String>,
    pub project_dir: Option<String>,
    pub project_parsed: bool,
    pub parent_session_id: Option<String>,
    pub is_main: bool,
    pub messages: BTreeMap<String, MessageFact>,
    pub reasons: BTreeSet<String>,
    #[serde(default)]
    pub tools: super::session_analytics::interactions::CallScan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageFact {
    pub identity: String,
    pub at_ms: Option<i64>,
    pub role: String,
    pub native_identity: bool,
}

pub struct TranscriptScan {
    provider: String,
    source_key: String,
    facts: TranscriptFacts,
}

impl TranscriptScan {
    pub fn new(provider: &str, source_key: &str, subagent: bool) -> Result<Self, AppError> {
        if !matches!(provider, "claude" | "codex") || source_key.is_empty() {
            return Err(invalid("unsupportedTranscriptReader"));
        }
        Ok(Self {
            provider: provider.into(),
            source_key: source_key.into(),
            facts: TranscriptFacts {
                is_main: !subagent,
                ..Default::default()
            },
        })
    }

    pub fn line(&mut self, ordinal: i64, value: &Value) -> Result<(), AppError> {
        if ordinal <= 0 {
            return Err(invalid("invalidMessageOrdinal"));
        }
        let kind = value.get("type").and_then(Value::as_str);
        let timestamp = value
            .get("timestamp")
            .and_then(crate::session_manager::providers::parse_timestamp_to_ms);
        if self.provider == "claude" {
            self.session(value.get("sessionId"))?;
            self.project(value.get("cwd"));
            if value.get("isMeta").and_then(Value::as_bool) == Some(true)
                || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
            {
                return Ok(());
            }
            let Some(message) = value.get("message") else {
                return Ok(());
            };
            let role = match kind {
                Some("user") => "user",
                Some("assistant") => "assistant",
                None => message.get("role").and_then(Value::as_str).unwrap_or(""),
                _ => return Ok(()),
            };
            if role == "assistant" {
                self.facts.tools.blocks(message.get("content"), timestamp);
            }
            if !matches!(role, "user" | "assistant") || !visible(message.get("content"), role) {
                return Ok(());
            }
            let id = if role == "assistant" {
                message.get("id")
            } else {
                value.get("uuid")
            };
            self.message(ordinal, role, id, timestamp);
        } else if kind == Some("session_meta") {
            let Some(payload) = value.get("payload") else {
                self.facts
                    .reasons
                    .insert("sessionMetadataUnavailable".into());
                return Ok(());
            };
            self.session(payload.get("id"))?;
            self.project(payload.get("cwd"));
            match payload.get("source") {
                Some(Value::String(source))
                    if matches!(source.as_str(), "cli" | "vscode" | "exec" | "appServer") => {}
                Some(Value::Object(source)) if source.contains_key("subagent") => {
                    self.facts.is_main = false;
                    let parent = payload
                        .pointer("/source/subagent/thread_spawn/parent_thread_id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty() && id.len() <= 512);
                    if let Some(parent) = parent {
                        if self
                            .facts
                            .parent_session_id
                            .as_deref()
                            .is_some_and(|old| old != parent)
                        {
                            return Err(invalid("conflictingSessionParent"));
                        }
                        self.facts.parent_session_id = Some(parent.to_owned());
                    } else {
                        self.facts.reasons.insert("sessionParentUnavailable".into());
                    }
                }
                _ => {
                    self.facts.is_main = false;
                    self.facts.reasons.insert("sessionRoleNotRecorded".into());
                }
            }
        } else if kind == Some("response_item") {
            let Some(payload) = value.get("payload") else {
                return Ok(());
            };
            if payload.get("type").and_then(Value::as_str) != Some("message") {
                return Ok(());
            }
            let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
            if matches!(role, "user" | "assistant") && visible(payload.get("content"), role) {
                self.message(ordinal, role, payload.get("id"), timestamp);
            }
        }
        Ok(())
    }

    fn session(&mut self, value: Option<&Value>) -> Result<(), AppError> {
        let Some(id) = value
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 512)
        else {
            return Ok(());
        };
        if self
            .facts
            .session_id
            .as_deref()
            .is_some_and(|old| old != id)
        {
            return Err(invalid("conflictingSessionIdentity"));
        }
        self.facts.session_id = Some(id.to_owned());
        Ok(())
    }

    fn project(&mut self, value: Option<&Value>) {
        if self.facts.reasons.contains("conflictingSessionProject") {
            return;
        }
        let project = match value {
            Some(Value::String(path)) => Some(path.as_str())
                .filter(|p| !p.trim().is_empty())
                .map(str::to_owned),
            Some(Value::Null) => None,
            _ => return,
        };
        if self.facts.project_parsed && self.facts.project_dir != project {
            self.facts.project_parsed = false;
            self.facts.project_dir = None;
            self.facts
                .reasons
                .insert("conflictingSessionProject".into());
            return;
        }
        self.facts.project_parsed = true;
        self.facts.project_dir = project;
    }

    fn message(&mut self, ordinal: i64, role: &str, id: Option<&Value>, at_ms: Option<i64>) {
        let native = id
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 512);
        let identity = match native {
            Some(id) => format!("{role}:{id}"),
            None => format!("local:{}:{ordinal}", self.source_key),
        };
        self.facts
            .messages
            .entry(identity.clone())
            .and_modify(|old| {
                old.at_ms = match (old.at_ms, at_ms) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            })
            .or_insert(MessageFact {
                identity,
                at_ms,
                role: role.into(),
                native_identity: native.is_some(),
            });
    }

    pub fn finish(mut self) -> TranscriptFacts {
        if self.facts.session_id.is_none() {
            self.facts.is_main = false;
            self.facts
                .reasons
                .insert("sessionIdentityUnavailable".into());
        }
        if !self.facts.project_parsed {
            self.facts.reasons.insert("projectNotRecorded".into());
        }
        if self
            .facts
            .messages
            .values()
            .any(|message| message.at_ms.is_none())
        {
            self.facts.reasons.insert("messageTimeUnavailable".into());
        }
        if self
            .facts
            .messages
            .values()
            .any(|message| !message.native_identity)
        {
            self.facts
                .reasons
                .insert("messageNativeIdentityUnavailable".into());
        }
        self.facts
    }
}

fn invalid(code: &str) -> AppError {
    AppError::InvalidInput(format!("usage.analytics.{code}"))
}

pub(crate) fn visible(content: Option<&Value>, role: &str) -> bool {
    let mut text = String::new();
    let mut image = false;
    match content {
        Some(Value::String(value)) => text.push_str(value),
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text" | "input_text" | "output_text") => {
                        if let Some(value) = block.get("text").and_then(Value::as_str) {
                            text.push_str(value);
                            text.push('\n');
                        }
                    }
                    Some("image" | "input_image" | "image_url") => image = true,
                    _ => {}
                }
            }
        }
        _ => {}
    }
    image
        || if role == "user" {
            crate::session_manager::title::clean_prompt(&text).is_some()
        } else {
            !text.trim().is_empty()
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scan(provider: &str, rows: &[Value]) -> TranscriptFacts {
        let mut scan = TranscriptScan::new(provider, "source-a", false).unwrap();
        for (index, row) in rows.iter().enumerate() {
            scan.line(index as i64 + 1, row).unwrap();
        }
        scan.finish()
    }

    #[test]
    fn analytics_claude_message_facts_drop_context_tools_thinking_but_keep_html_and_images() {
        let result = scan(
            "claude",
            &[
                json!({"type":"user","sessionId":"s","cwd":"/project","uuid":"context","timestamp":"2026-10-15T00:00:00Z","message":{"role":"user","content":"<environment_context>cwd: /private</environment_context>"}}),
                json!({"type":"user","uuid":"html","timestamp":"2026-10-15T00:00:01Z","message":{"role":"user","content":"<html>这个页面为什么空白？</html>"}}),
                json!({"type":"user","uuid":"image","timestamp":"2026-10-15T00:00:02Z","message":{"role":"user","content":[{"type":"image","source":{"data":"not-retained"}}]}}),
                json!({"type":"user","uuid":"tool-result","timestamp":"2026-10-15T00:00:03Z","message":{"role":"user","content":[{"type":"tool_result","content":"tool output"}]}}),
                json!({"type":"assistant","timestamp":"2026-10-15T00:00:04Z","message":{"id":"thinking","role":"assistant","content":[{"type":"thinking","thinking":"private reasoning"}]}}),
                json!({"type":"assistant","timestamp":"2026-10-15T00:00:05Z","message":{"id":"reply","role":"assistant","content":[{"type":"text","text":"answer"}]}}),
            ],
        );
        assert_eq!(result.session_id.as_deref(), Some("s"));
        assert_eq!(result.project_dir.as_deref(), Some("/project"));
        assert!(result.project_parsed);
        assert_eq!(result.messages.len(), 3);
        assert_eq!(
            result
                .messages
                .values()
                .map(|v| v.role.as_str())
                .collect::<Vec<_>>(),
            ["assistant", "user", "user"]
        );
        assert!(result.reasons.is_empty());
        assert!(!format!("{result:?}").contains("not-retained"));
        assert!(!format!("{result:?}").contains("answer"));
    }

    #[test]
    fn analytics_claude_streamed_revision_counts_once_at_first_visible_time() {
        let result = scan(
            "claude",
            &[
                json!({"type":"assistant","sessionId":"s","timestamp":"2026-10-15T00:00:02Z","message":{"id":"reply","role":"assistant","content":[{"type":"text","text":"part"}]}}),
                json!({"type":"assistant","timestamp":"2026-10-15T00:00:06Z","message":{"id":"reply","role":"assistant","content":[{"type":"text","text":"final"}]}}),
            ],
        );
        assert_eq!(result.messages.len(), 1);
        assert_eq!(
            result.messages.values().next().unwrap().at_ms,
            Some(1_792_022_402_000)
        );
    }

    #[test]
    fn analytics_codex_context_and_event_mirrors_do_not_double_message_count() {
        let result = scan(
            "codex",
            &[
                json!({"type":"session_meta","payload":{"id":"s","cwd":"/project","source":"cli"}}),
                json!({"type":"event_msg","timestamp":"2026-10-15T00:00:01Z","payload":{"type":"user_message","message":"question"}}),
                json!({"type":"response_item","timestamp":"2026-10-15T00:00:01Z","payload":{"id":"context","type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd: /private</environment_context>"}]}}),
                json!({"type":"response_item","timestamp":"2026-10-15T00:00:02Z","payload":{"id":"user","type":"message","role":"user","content":[{"type":"input_text","text":"<html>what failed?</html>"}]}}),
                json!({"type":"response_item","timestamp":"2026-10-15T00:00:03Z","payload":{"id":"tool","type":"function_call","name":"shell","arguments":"private"}}),
                json!({"type":"response_item","timestamp":"2026-10-15T00:00:04Z","payload":{"id":"reply","type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}}),
            ],
        );
        assert_eq!(result.messages.len(), 2);
        assert!(result.is_main);
        assert!(result.reasons.is_empty());
    }

    #[test]
    fn analytics_codex_native_thread_spawn_relation_is_preserved_without_guessing_root() {
        let result = scan(
            "codex",
            &[
                json!({"type":"session_meta","payload":{"id":"child","cwd":"/child","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent","depth":1}}}}}),
                json!({"type":"response_item","payload":{"id":"child-msg","type":"message","role":"user","content":[{"type":"input_text","text":"task"}]}}),
            ],
        );
        assert!(!result.is_main);
        assert_eq!(result.parent_session_id.as_deref(), Some("parent"));
        assert_eq!(result.session_id.as_deref(), Some("child"));
        assert!(result.reasons.contains("messageTimeUnavailable"));
        assert_eq!(result.messages.values().next().unwrap().at_ms, None);
    }

    #[test]
    fn analytics_missing_metadata_or_native_message_id_is_explicitly_partial() {
        let result = scan(
            "codex",
            &[
                json!({"type":"session_meta","payload":{"id":"s","source":"cli"}}),
                json!({"type":"response_item","timestamp":"2026-10-15T00:00:01Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}}),
            ],
        );
        assert!(!result.project_parsed);
        assert!(result.reasons.contains("projectNotRecorded"));
        assert!(result.reasons.contains("messageNativeIdentityUnavailable"));
        let message = result.messages.values().next().unwrap();
        assert!(!message.native_identity);
        assert!(message.identity.contains("source-a"));
        let unknown = scan(
            "codex",
            &[json!({"type":"session_meta","payload":{"id":"s","cwd":null,"source":"cli"}})],
        );
        assert!(unknown.project_parsed);
        assert_eq!(unknown.project_dir, None);
    }

    #[test]
    fn analytics_conflicting_session_metadata_and_unsupported_readers_fail_explicitly() {
        let mut scan = TranscriptScan::new("codex", "source-a", false).unwrap();
        scan.line(
            1,
            &json!({"type":"session_meta","payload":{"id":"first","cwd":"/one"}}),
        )
        .unwrap();
        assert!(scan
            .line(
                2,
                &json!({"type":"session_meta","payload":{"id":"second","cwd":"/two"}})
            )
            .is_err());
        assert!(TranscriptScan::new("unknown", "source-a", false).is_err());
    }

    #[test]
    fn analytics_claude_and_codex_keep_significant_cwd_whitespace() {
        let claude = scan(
            "claude",
            &[
                json!({"type":"user","sessionId":"s","cwd":"/one/project ","uuid":"u","message":{"role":"user","content":"question"}}),
            ],
        );
        assert_eq!(claude.project_dir.as_deref(), Some("/one/project "));
        let codex = scan(
            "codex",
            &[
                json!({"type":"session_meta","payload":{"id":"s","cwd":"/one/project ","source":"cli"}}),
            ],
        );
        assert_eq!(codex.project_dir.as_deref(), Some("/one/project "));
        let conflict = scan(
            "codex",
            &[
                json!({"type":"session_meta","payload":{"id":"s","cwd":"/one/project","source":"cli"}}),
                json!({"type":"session_meta","payload":{"id":"s","cwd":"/one/project ","source":"cli"}}),
            ],
        );
        assert!(!conflict.project_parsed);
        assert!(conflict.reasons.contains("conflictingSessionProject"));
    }

    #[test]
    fn analytics_codex_without_session_metadata_cannot_claim_main_session_activity() {
        let result = scan(
            "codex",
            &[
                json!({"type":"response_item","timestamp":"2026-10-15T00:00:01Z","payload":{"id":"a","type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}}),
            ],
        );
        assert!(!result.is_main);
        assert!(result.reasons.contains("sessionIdentityUnavailable"));
        assert_eq!(result.messages.len(), 1);
    }
}

#[cfg(test)]
mod tool_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn assistant_tool_only_blocks_are_counted_without_becoming_visible_messages() {
        let mut scan = TranscriptScan::new("claude", "fixture", false).unwrap();
        let row = json!({"type":"assistant","sessionId":"s","timestamp":1000,"message":{"id":"m","role":"assistant","content":[{"type":"tool_use","id":"call","name":"Skill","input":{"skill":"fizzy"}}]}});
        scan.line(1, &row).unwrap();
        scan.line(2, &row).unwrap();
        let result = scan.finish();
        assert!(result.messages.is_empty());
        assert_eq!(result.tools.records.len(), 1);
        assert_eq!(result.tools.records["call"].skill.as_deref(), Some("fizzy"));
    }
}
