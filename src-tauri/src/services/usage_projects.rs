//! Positive native session/project associations from the same cached snapshot
//! as a usage query. A proxy's coincident SID is not a native import identity.
use super::session_analytics_core::{self, Project};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOption {
    pub id: String,
    pub label: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Association {
    pub session_key: String,
    pub project_id: Option<String>,
}
pub(crate) struct ProjectIndex {
    pub snapshot_version: String,
    pub options: Vec<ProjectOption>,
    sessions: BTreeMap<(String, String), Association>,
}
impl ProjectIndex {
    pub fn from_input(input: &super::session_analytics::AnalyticsInput) -> Result<Self, AppError> {
        let identities: Vec<_> = input.sessions.iter().map(|s| s.identity.clone()).collect();
        let roots = session_analytics_core::resolve_roots(&identities)
            .map_err(|_| AppError::Database("usage.query.invalidSessionRoots".into()))?;
        let descriptors: BTreeMap<_, _> = input
            .sessions
            .iter()
            .map(|s| (s.identity.key.as_str(), s))
            .collect();
        let mut options = BTreeMap::new();
        let mut sessions = BTreeMap::new();
        for descriptor in &input.sessions {
            let root = roots
                .get(descriptor.identity.key.as_str())
                .ok_or_else(|| AppError::Database("usage.query.invalidSessionRoots".into()))?;
            let root_descriptor = descriptors[root.key.as_str()];
            let unverified = descriptor
                .message_reasons
                .iter()
                .chain(&root_descriptor.message_reasons)
                .any(|r| {
                    matches!(
                        r.as_str(),
                        "invalidSessionRoot" | "sessionParentUnavailable"
                    )
                });
            let project_id = if unverified {
                None
            } else {
                match &root.project {
                    Project::Known(id) => Some(id.clone()),
                    Project::Unknown => Some(session_analytics_core::UNKNOWN_PROJECT_ID.into()),
                    Project::Unparsed => None,
                }
            };
            if let Some(id) = &project_id {
                options.entry(id.clone()).or_insert_with(|| ProjectOption {
                    id: id.clone(),
                    label: root_descriptor
                        .project_label
                        .clone()
                        .unwrap_or_else(|| id.clone()),
                    path: root_descriptor.project_path.clone(),
                });
            }
            sessions.insert(
                (
                    descriptor.identity.tool_id.clone(),
                    descriptor.native_session_id.clone(),
                ),
                Association {
                    session_key: root.key.clone(),
                    project_id,
                },
            );
        }
        Ok(Self {
            snapshot_version: input.snapshot_version.clone(),
            options: options.into_values().collect(),
            sessions,
        })
    }
    pub fn associate(
        &self,
        tool: &str,
        provider: &str,
        source: Option<&str>,
        native_sid: Option<&str>,
    ) -> Option<&Association> {
        let (owned_provider, owned_source) = match tool {
            "claude" => ("_session", "session_log"),
            "codex" => ("_codex_session", "codex_session"),
            "gemini" => ("_gemini_session", "gemini_session"),
            "opencode" => ("_opencode_session", "opencode_session"),
            "workbuddy" => ("_workbuddy_session", "workbuddy_session"),
            "pi" => ("_pi_session", "pi_session"),
            "dsh" => ("_dsh_session", "dsh_session"),
            "openclaw" => ("_openclaw_session", "openclaw_session"),
            _ => return None,
        };
        if provider != owned_provider || source != Some(owned_source) {
            return None;
        }
        self.sessions.get(&(tool.into(), native_sid?.into()))
    }
}
