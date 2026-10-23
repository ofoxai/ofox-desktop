//! Bindable resource targets. Client aliases never create a second physical write.
pub struct SkillTarget {
    pub id: &'static str,
    pub label: &'static str,
    /// Reads ~/.agents/skills as well as its own folder.
    pub reads_shared: bool,
}

pub const TARGETS: &[SkillTarget] = &[
    SkillTarget {
        id: "claude",
        label: "Claude Code",
        reads_shared: false,
    },
    SkillTarget {
        id: "codex",
        label: "Codex / ChatGPT",
        reads_shared: true,
    },
    SkillTarget {
        id: "gemini",
        label: "Gemini CLI",
        reads_shared: true,
    },
    SkillTarget {
        id: "opencode",
        label: "OpenCode",
        reads_shared: true,
    },
    SkillTarget {
        id: "hermes",
        label: "Hermes",
        reads_shared: false,
    },
    SkillTarget {
        id: "openclaw",
        label: "OpenClaw",
        reads_shared: false,
    },
    SkillTarget {
        id: "pi",
        label: "Pi",
        reads_shared: false,
    },
    SkillTarget {
        id: "dsh",
        label: "DeepSeek Harness",
        reads_shared: true,
    },
    SkillTarget {
        id: "workbuddy",
        label: "WorkBuddy",
        reads_shared: false,
    },
];

pub const TOOLS: &[&str] = &[
    "claude",
    "codex",
    "opencode",
    "openclaw",
    "hermes",
    "gemini",
    "workbuddy",
    "dsh",
    "pi",
];
pub const CLIENTS: &[&str] = &[
    "claude",
    "codex",
    "chatgpt",
    "opencode",
    "openclaw",
    "hermes",
    "gemini",
    "workbuddy",
    "dsh",
    "pi",
];
pub fn canonical(id: &str) -> &str {
    if id == "chatgpt" {
        "codex"
    } else {
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_bindable_client_has_exactly_one_resource_target() {
        use std::{collections::BTreeSet, str::FromStr};
        let targets: BTreeSet<_> = TARGETS.iter().map(|target| target.id).collect();
        assert_eq!(targets, TOOLS.iter().copied().collect());
        assert_eq!(targets.len(), 9);
        assert_eq!(CLIENTS.len(), 10);
        for client in CLIENTS {
            assert_eq!(
                crate::app_config::BindableTool::from_str(client)
                    .unwrap()
                    .as_str(),
                canonical(client)
            );
            assert!(targets.contains(canonical(client)));
        }
    }
}
