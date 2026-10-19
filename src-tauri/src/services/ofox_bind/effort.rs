//! A bound tool's reasoning effort (推理强度), shown and changed next to its
//! model. Codex, Claude Code and Hermes each keep it in one key of their own
//! config, as magpie sets it. Ofox's catalog only says whether a model reasons,
//! not which levels it takes, so these are the levels each tool accepts.
//! "Default" (`None`) removes the key and leaves the choice to the tool.

use serde::Serialize;

use super::{claude, codex, hermes, Tool};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffortState {
    /// What the config says; `None` is the tool's own default.
    pub value: Option<String>,
    /// What can be picked, weakest first.
    pub levels: Vec<String>,
    /// The level the tool falls back to when none is set, where it is known.
    pub default_level: Option<String>,
}

const CODEX: &[&str] = &["low", "medium", "high", "xhigh"];
const CLAUDE: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const HERMES: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh"];

/// The levels a tool takes and the one it uses unset, or `None` without effort.
fn levels(tool: Tool) -> Option<(&'static [&'static str], Option<&'static str>)> {
    match tool {
        Tool::Codex => Some((CODEX, Some("medium"))),
        // Claude Code picks its own per model; there is no one level to name.
        Tool::Claude => Some((CLAUDE, None)),
        Tool::Hermes => Some((HERMES, Some("medium"))),
        Tool::Gemini | Tool::OpenCode | Tool::OpenClaw => None,
    }
}

/// The effort as the tool's config has it now; `None` for a tool without one.
pub(crate) fn state(tool: Tool) -> Option<EffortState> {
    let (levels, default_level) = levels(tool)?;
    let value = match tool {
        Tool::Codex => codex::effort(),
        Tool::Claude => claude::effort(),
        Tool::Hermes => hermes::effort(),
        Tool::Gemini | Tool::OpenCode | Tool::OpenClaw => None,
    };
    Some(EffortState {
        value,
        levels: levels.iter().map(|level| (*level).to_string()).collect(),
        default_level: default_level.map(Into::into),
    })
}

/// A level this tool takes, or `None` for its default; anything else is refused.
pub(crate) fn validate(tool: Tool, effort: Option<&str>) -> Result<Option<String>, String> {
    let (levels, _) =
        levels(tool).ok_or_else(|| "这个工具不能在 Ofox 里设置推理强度。".to_string())?;
    match effort.map(str::trim).filter(|effort| !effort.is_empty()) {
        None => Ok(None),
        Some(level) if levels.contains(&level) => Ok(Some(level.into())),
        Some(_) => Err("这个工具不支持该推理强度。".into()),
    }
}
