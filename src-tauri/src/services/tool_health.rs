//! Periodic tool-health probe.
//!
//! For every tool the user has bound (`AppSettings.bound_tools`), we send
//! a `max_tokens=1` completion via [`commands::manage_tool::ofox_ping_model_internal`]
//! at the user-configured interval (`"off" | "1h" | "6h" | "24h"`). Results
//! land in [`ToolHealthCache`]; ConsolePage subscribes to the
//! `ofox-tool-health-updated` event and reads the cache.
//!
//! Why a real `max_tokens=1` request and not a free `/health` endpoint?
//! OFox's gateway has no per-tool health probe today, and a 1-token
//! completion is the cheapest way to verify the *exact* code path the
//! tool will take in production (auth header shape, model existence,
//! routing). Cost is small but non-zero — surface this in UI text.
//!
//! The probe respects three skip conditions:
//!   1. OFox auth not active → skip (we'll come back on next tick)
//!   2. No bound tools → cache cleared, no requests
//!   3. Per-tool: active model is empty → mark `Skipped` without firing

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use futures::future::join_all;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;

use crate::app_config::AppType;
use crate::commands::manage_tool::{
    ofox_ping_model_internal, read_active_model_for, PingResult,
};
use crate::ofox_auth::OfoxAuthManager;
use crate::store::AppState;

/// Tauri event broadcast after every health-check round.
pub const TOOL_HEALTH_UPDATED_EVENT: &str = "ofox-tool-health-updated";

/// Bucket for the latest probe result of one tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolHealthSnapshot {
    pub tool_id: String,
    pub status: ToolHealthStatus,
    /// `None` for skipped (no request was made).
    pub latency_ms: Option<u64>,
    /// Unix-ms.
    pub checked_at: i64,
    pub status_code: Option<u16>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolHealthStatus {
    Ok,
    Fail,
    Skipped,
}

/// Process-wide health-check cache. Cheap to clone (`Arc`); writers grab
/// the `RwLock` for the duration of one round.
#[derive(Debug, Clone, Default)]
pub struct ToolHealthCache(pub Arc<RwLock<HashMap<String, ToolHealthSnapshot>>>);

impl ToolHealthCache {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }

    pub async fn snapshot(&self) -> HashMap<String, ToolHealthSnapshot> {
        self.0.read().await.clone()
    }

    pub async fn clear(&self) {
        self.0.write().await.clear();
    }

    async fn replace(&self, snap: HashMap<String, ToolHealthSnapshot>) {
        let mut g = self.0.write().await;
        *g = snap;
    }
}

/// Parse the user-facing interval string into a duration. `"off"` and any
/// unrecognized value return `None`, signalling the loop should sleep
/// indefinitely until reconfigured.
pub fn parse_interval(raw: &str) -> Option<Duration> {
    match raw {
        "1h" => Some(Duration::from_secs(60 * 60)),
        "6h" => Some(Duration::from_secs(6 * 60 * 60)),
        "24h" => Some(Duration::from_secs(24 * 60 * 60)),
        _ => None, // "off" or garbage — caller treats as paused.
    }
}

/// Run one health-check round: probe every bound tool concurrently, write
/// the results into `cache`, and emit `ofox-tool-health-updated`.
///
/// Returns the snapshot map so callers can also use it directly (tests).
pub async fn run_health_check_round(
    app: &AppHandle,
    state: &AppState,
    manager: &Arc<RwLock<OfoxAuthManager>>,
    cache: &ToolHealthCache,
    bound_tools: &[String],
    now_ms: i64,
) -> HashMap<String, ToolHealthSnapshot> {
    if bound_tools.is_empty() {
        cache.clear().await;
        let _ = app.emit(TOOL_HEALTH_UPDATED_EVENT, &HashMap::<String, ToolHealthSnapshot>::new());
        return HashMap::new();
    }

    // Prepare per-tool model lookup BEFORE concurrent probes:
    //   - empty model → record `Skipped`, no HTTP request
    //   - DB error    → record `Skipped` with the error string (we don't
    //                   want to spam Fail when the cause is local)
    //   - present     → schedule the actual probe
    enum Plan {
        Skip { reason: String },
        Probe { model: String },
    }
    let mut plans: Vec<(String, Plan)> = Vec::with_capacity(bound_tools.len());
    for tool_id in bound_tools {
        let app_type = match AppType::from_str(tool_id) {
            Ok(t) => t,
            Err(e) => {
                plans.push((
                    tool_id.clone(),
                    Plan::Skip {
                        reason: format!("无法识别工具类型: {e}"),
                    },
                ));
                continue;
            }
        };
        match read_active_model_for(state, &app_type) {
            Ok(model) if model.trim().is_empty() => plans.push((
                tool_id.clone(),
                Plan::Skip {
                    reason: "未配置模型".to_string(),
                },
            )),
            Ok(model) => plans.push((tool_id.clone(), Plan::Probe { model })),
            Err(e) => plans.push((
                tool_id.clone(),
                Plan::Skip {
                    reason: format!("读取模型失败: {e}"),
                },
            )),
        }
    }

    // Probe in parallel — each future is independent, so total wall-clock
    // ≈ slowest probe (rather than sum). Skipped entries don't even spawn
    // an HTTP request.
    let futures = plans.into_iter().map(|(tool_id, plan)| async move {
        match plan {
            Plan::Skip { reason } => ToolHealthSnapshot {
                tool_id: tool_id.clone(),
                status: ToolHealthStatus::Skipped,
                latency_ms: None,
                checked_at: now_ms,
                status_code: None,
                error: Some(reason),
            },
            Plan::Probe { model } => {
                let res: PingResult =
                    ofox_ping_model_internal(state, manager, &tool_id, &model).await;
                ToolHealthSnapshot {
                    tool_id,
                    status: if res.success {
                        ToolHealthStatus::Ok
                    } else {
                        ToolHealthStatus::Fail
                    },
                    latency_ms: Some(res.latency_ms),
                    checked_at: now_ms,
                    status_code: res.status_code,
                    error: res.error,
                }
            }
        }
    });

    let results = join_all(futures).await;

    let snap: HashMap<String, ToolHealthSnapshot> = results
        .into_iter()
        .map(|s| (s.tool_id.clone(), s))
        .collect();

    cache.replace(snap.clone()).await;
    if let Err(e) = app.emit(TOOL_HEALTH_UPDATED_EVENT, &snap) {
        log::warn!("[tool_health] emit {} failed: {}", TOOL_HEALTH_UPDATED_EVENT, e);
    }
    log::info!(
        "[tool_health] round complete: {} tool(s)",
        snap.len()
    );
    snap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_interval_known_values() {
        assert_eq!(parse_interval("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_interval("6h"), Some(Duration::from_secs(21600)));
        assert_eq!(parse_interval("24h"), Some(Duration::from_secs(86400)));
    }

    #[test]
    fn parse_interval_off_returns_none() {
        assert_eq!(parse_interval("off"), None);
    }

    #[test]
    fn parse_interval_garbage_returns_none() {
        assert_eq!(parse_interval(""), None);
        assert_eq!(parse_interval("2h"), None);
        assert_eq!(parse_interval("forever"), None);
    }

    #[tokio::test]
    async fn cache_clear_resets_state() {
        let cache = ToolHealthCache::new();
        let mut map = HashMap::new();
        map.insert(
            "claude".to_string(),
            ToolHealthSnapshot {
                tool_id: "claude".to_string(),
                status: ToolHealthStatus::Ok,
                latency_ms: Some(10),
                checked_at: 0,
                status_code: Some(200),
                error: None,
            },
        );
        cache.replace(map).await;
        assert_eq!(cache.snapshot().await.len(), 1);
        cache.clear().await;
        assert!(cache.snapshot().await.is_empty());
    }
}
