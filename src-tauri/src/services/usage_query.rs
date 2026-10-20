//! Versioned, normalized analytics queries. Legacy second-based APIs keep their contract.
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageQueryV1 {
    pub version: u8,
    pub start_ms: Option<i64>,
    pub end_ms: i64,
    #[serde(default)]
    pub tool_ids: Vec<String>,
    #[serde(default)]
    pub model_ids: Vec<String>,
    #[serde(default)]
    pub project_ids: Vec<String>,
    #[serde(default)]
    pub source_ids: Vec<String>,
}

impl UsageQueryV1 {
    pub fn normalized(mut self) -> Result<Self, AppError> {
        if self.version != 1 || self.start_ms.is_some_and(|start| start > self.end_ms) {
            return Err(AppError::InvalidInput("usage.query.invalid".into()));
        }
        for ids in [
            &mut self.tool_ids,
            &mut self.model_ids,
            &mut self.project_ids,
            &mut self.source_ids,
        ] {
            if ids.len() > 256 || ids.iter().any(|id| id.is_empty() || id.len() > 512) {
                return Err(AppError::InvalidInput("usage.query.invalidIds".into()));
            }
            ids.sort();
            ids.dedup();
        }
        Ok(self)
    }

    pub fn fingerprint(&self) -> Result<String, AppError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|_| AppError::InvalidInput("usage.query.invalid".into()))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryReceipt {
    pub applied_query: UsageQueryV1,
    pub query_fingerprint: String,
    pub snapshot_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub state: String,
    pub reasons: Vec<String>,
    pub time_basis: String,
    pub detail_start_ms: Option<i64>,
    pub detail_end_ms: Option<i64>,
    pub aggregate_only: bool,
    pub excluded_rollup_days: u64,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHealth {
    pub source_id: String,
    pub tool_id: String,
    pub status: String,
    pub last_attempt_at_ms: Option<i64>,
    pub last_success_at_ms: Option<i64>,
    pub error_code: Option<String>,
    #[serde(default)]
    pub last_scan: Option<SourceScan>,
    #[serde(default)]
    pub retained_detail_range: Option<RetainedDetailRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceScan {
    pub files_scanned: u32,
    pub imported: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetainedDetailRange {
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCapability {
    pub tool_id: String,
    pub source_ids: Vec<String>,
    pub usage_supported: bool,
    pub session_analytics_supported: bool,
    pub project_filter_supported: bool,
    pub reason: Option<String>,
}

pub fn source_capabilities() -> Vec<SourceCapability> {
    [
        ("claude", Some("session_log")),
        ("codex", Some("codex_session")),
        ("gemini", Some("gemini_session")),
        ("opencode", Some("opencode_session")),
        ("chatgpt", None),
        ("workbuddy", Some("workbuddy_session")),
        ("hermes", Some("hermes_session")),
        ("pi", Some("pi_session")),
        ("dsh", Some("dsh_session")),
        ("openclaw", Some("openclaw_session")),
    ]
    .into_iter()
    .map(|(tool, source)| SourceCapability {
        tool_id: tool.into(),
        source_ids: source.into_iter().map(str::to_owned).collect(),
        usage_supported: source.is_some(),
        session_analytics_supported: source.is_some(),
        project_filter_supported: source.is_some(),
        reason: source.is_none().then(|| {
            if tool == "chatgpt" {
                "codexModeSharesCodexRecords".into()
            } else {
                "usageReaderUnavailable".into()
            }
        }),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bindable_tool_but_chatgpt_has_usage_and_session_analytics() {
        for capability in source_capabilities() {
            let supported = capability.tool_id != "chatgpt";
            assert_eq!(
                capability.usage_supported, supported,
                "{}",
                capability.tool_id
            );
            assert_eq!(
                capability.session_analytics_supported, supported,
                "{}",
                capability.tool_id
            );
            assert_eq!(
                capability.project_filter_supported, supported,
                "{}",
                capability.tool_id
            );
        }
        let hermes = source_capabilities()
            .into_iter()
            .find(|c| c.tool_id == "hermes")
            .unwrap();
        assert_eq!(hermes.source_ids, ["hermes_session"]);
        assert_eq!(hermes.reason, None);
    }

    #[test]
    fn query_fingerprint_is_order_independent() {
        let query = UsageQueryV1 {
            version: 1,
            start_ms: Some(1),
            end_ms: 2,
            tool_ids: vec!["gemini".into(), "claude".into(), "claude".into()],
            model_ids: vec![],
            project_ids: vec![],
            source_ids: vec![],
        };
        let normalized = query.normalized().unwrap();
        assert_eq!(normalized.tool_ids, ["claude", "gemini"]);
        assert_eq!(
            normalized.fingerprint().unwrap(),
            normalized
                .clone()
                .normalized()
                .unwrap()
                .fingerprint()
                .unwrap()
        );
        let mut project = normalized;
        project.project_ids.push("unknown".into());
        assert_eq!(project.normalized().unwrap().project_ids, ["unknown"]);
    }
}
