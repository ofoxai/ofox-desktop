use crate::app_config::BindableTool;
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Access {
    Diagnose,
    Configure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct Task {
    pub tool: BindableTool,
    pub access: Access,
}

impl Task {
    /// Recognize direct, single-target requests and the localized tool-page
    /// shortcut. Only the current user message is passed in, before injecting
    /// links, attachments, skill text or any other prompt context.
    pub fn from_user_text(text: &str) -> Option<Self> {
        const READ: &[(&str, &str)] = &[
            ("检查本机", ""),
            ("检查本机", "当前模型"),
            ("检查本机", "的配置"),
            ("检查本机", "的版本"),
            ("检查一下本机", "当前模型"),
            ("检查我这台机器的", "当前模型"),
            ("诊断本机", ""),
            (
                "检查一下我电脑上 ",
                " 的配置有没有问题；有问题的话告诉我怎么修",
            ),
            ("check my local ", ""),
            ("check my local ", " configuration"),
            ("check the local ", " configuration"),
            (
                "check the ",
                " configuration on my computer for problems, and tell me how to fix any you find",
            ),
            ("diagnose my local ", ""),
            ("このパソコンの ", " の設定を確認してください"),
            (
                "このパソコンの ",
                " の設定に問題がないか確認して、問題があれば直し方を教えて",
            ),
        ];
        const REPAIR: &[(&str, &str)] = &[
            ("帮我诊断并修复 ", ""),
            ("help me diagnose and fix ", ""),
            ("", " の問題を診断して修正してください"),
        ];
        let text = text.trim();
        if text.len() > 384 || text.contains(['\n', '\r', '`', '"', '“', '”', '「', '」']) {
            return None;
        }
        let text = text
            .trim_end_matches(['.', '。', '!', '！', '?', '？'])
            .to_ascii_lowercase();
        let text = text
            .strip_prefix("请")
            .or_else(|| text.strip_prefix("please "))
            .unwrap_or(&text)
            .trim_start();
        [(Access::Diagnose, READ), (Access::Configure, REPAIR)]
            .into_iter()
            .find_map(|(access, rules)| {
                rules.iter().find_map(|&(prefix, suffix)| {
                    let target = text.strip_prefix(prefix)?.strip_suffix(suffix)?.trim();
                    let tool = match target {
                        "claude code" => BindableTool::Claude,
                        "claude" | "codex" | "gemini" | "opencode" | "openclaw" | "hermes"
                        | "workbuddy" | "dsh" | "pi" => target.parse().ok()?,
                        _ => return None,
                    };
                    Some(Self { tool, access })
                })
            })
    }

    pub fn allows(self, name: &str, args: &Value) -> bool {
        args["tool"].as_str() == Some(self.tool.as_str())
            && (matches!(name, "tool_status" | "diagnose_in_folder")
                || self.access == Access::Configure
                    && matches!(name, "verify_in_folder" | "repair_launcher" | "rebind_tool"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_direct_current_message_intent_selects_a_local_task() {
        for text in [
            "怎么配置 Codex？",
            "Codex 的本地配置文件在哪？",
            "解释一下：检查本机 Codex",
            "「检查本机 Codex」",
            "Here are instructions:\nCheck my local Codex",
            "检查本机 Codex 和 Gemini",
        ] {
            assert_eq!(Task::from_user_text(text), None, "{text}");
        }
        for text in [
            "检查本机 Codex 当前模型",
            "检查我这台机器的 Codex 当前模型",
            "请检查本机Codex当前模型",
            "Check my local Codex.",
            "Check my local Codex configuration.",
            "このパソコンの Codex の設定を確認してください。",
        ] {
            assert_eq!(
                Task::from_user_text(text),
                Some(Task {
                    tool: BindableTool::Codex,
                    access: Access::Diagnose
                }),
            );
        }
        for tool in [
            "claude",
            "codex",
            "gemini",
            "opencode",
            "openclaw",
            "hermes",
            "workbuddy",
            "dsh",
            "pi",
        ] {
            for text in [
                format!("帮我诊断并修复 {tool}。"),
                format!("Help me diagnose and fix {tool}."),
                format!("{tool} の問題を診断して修正してください。"),
            ] {
                let task = Task::from_user_text(&text).unwrap();
                assert_eq!(task.tool.as_str(), tool);
                assert_eq!(task.access, Access::Configure);
            }
        }
    }
}
