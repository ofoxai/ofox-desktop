//! Model-only profile matching and preview/apply reports.
//! The UI's last clicked Profile is never used as evidence of the active combination.

use crate::database::dao::model_profiles::ModelProfileRow;
use crate::model_selection::{ModelSelection, ProfileTarget, ToolModelSnapshot};
use crate::services::ofox_bind::status::BindingStatus;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) const PLAN_LIFETIME: Duration = Duration::from_secs(5 * 60);
pub(crate) const UNDO_LIFETIME: Duration = Duration::from_secs(15 * 60);
const MAX_PENDING: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProfileMatch {
    InUse,
    Partial,
    Different,
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfile {
    pub id: String,
    pub name: String,
    pub revision: String,
    pub targets: Vec<ProfileTarget>,
    pub r#match: ProfileMatch,
}

pub(crate) fn with_match(row: ModelProfileRow, snapshots: &[ToolModelSnapshot]) -> ModelProfile {
    let matched = match_targets(&row.targets, snapshots);
    ModelProfile {
        id: row.id,
        name: row.name,
        revision: row.revision,
        targets: row.targets,
        r#match: matched,
    }
}

pub(crate) fn match_targets(
    targets: &[ProfileTarget],
    snapshots: &[ToolModelSnapshot],
) -> ProfileMatch {
    let mut available = 0;
    let mut matches = 0;
    for target in targets {
        if let Some(snapshot) = snapshots
            .iter()
            .find(|snapshot| snapshot.write_target == target.target)
        {
            if snapshot.binding.status == BindingStatus::Configured {
                available += 1;
                if snapshot.managed_selection == target.selection {
                    matches += 1;
                }
            }
        }
    }
    if available == 0 {
        ProfileMatch::Unavailable
    } else if matches == targets.len() {
        ProfileMatch::InUse
    } else if matches > 0 {
        ProfileMatch::Partial
    } else {
        ProfileMatch::Different
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PreviewStatus {
    Ready,
    Unchanged,
    Skipped,
    Blocked,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    pub target: String,
    pub before: Option<ModelSelection>,
    pub after: Option<ModelSelection>,
    pub status: PreviewStatus,
    pub reason: Option<String>,
    pub scope: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePreview {
    pub plan_token: String,
    pub can_apply: bool,
    pub items: Vec<PreviewItem>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ApplyOutcome {
    Applied,
    Unchanged,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyItem {
    pub target: String,
    pub outcome: ApplyOutcome,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub status: String,
    pub items: Vec<ApplyItem>,
    pub undo_token: Option<String>,
}

pub(crate) fn report_status(items: &[ApplyItem]) -> String {
    let changed = items
        .iter()
        .any(|item| item.outcome == ApplyOutcome::Applied);
    let failed = items
        .iter()
        .any(|item| item.outcome == ApplyOutcome::Failed);
    if failed && changed {
        "partial"
    } else if failed {
        "failed"
    } else if items
        .iter()
        .any(|item| item.outcome == ApplyOutcome::Skipped)
    {
        "partial"
    } else {
        "applied"
    }
    .into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Context {
    pub apex: String,
    pub owner: Option<crate::settings::KeyOwner>,
    pub authenticated: bool,
}

#[derive(Clone)]
pub(crate) struct PreparedPlan {
    pub created_at: Instant,
    pub profile_id: String,
    pub profile_revision: String,
    pub context: Context,
    pub items: Vec<PreviewItem>,
    pub snapshots: Vec<ToolModelSnapshot>,
}

#[derive(Clone)]
pub(crate) struct UndoEntry {
    pub before: ToolModelSnapshot,
    pub after_revision: String,
}

#[derive(Clone)]
pub(crate) struct UndoPlan {
    pub created_at: Instant,
    pub context: Context,
    pub entries: Vec<UndoEntry>,
}

pub(crate) struct ExecutionSuccess {
    pub after_revision: Option<String>,
    pub skipped: Vec<String>,
}

pub(crate) struct ExecutionResult {
    pub items: Vec<ApplyItem>,
    pub undo: Vec<UndoEntry>,
    pub incomplete: bool,
}

/// The same executor is used by IPC and failure fixtures; no writer after the first failure runs.
pub(crate) async fn execute_items<F, Fut>(
    plan: &PreparedPlan,
    mut save: F,
) -> Result<ExecutionResult, String>
where
    F: FnMut(PreviewItem, ToolModelSnapshot) -> Fut,
    Fut: Future<Output = Result<ExecutionSuccess, String>>,
{
    if plan
        .items
        .iter()
        .any(|item| item.status == PreviewStatus::Blocked)
    {
        return Err("请先解决预览中阻止应用的问题。".into());
    }
    let mut items = Vec::new();
    let mut undo = Vec::new();
    let mut failed = false;
    for item in &plan.items {
        let mut result = ApplyItem {
            target: item.target.clone(),
            outcome: ApplyOutcome::Skipped,
            reason: item.reason.clone(),
        };
        if failed {
            result.reason = Some("modelProfiles.reasonStopped".into());
        } else if item.status == PreviewStatus::Blocked {
            return Err("请先解决预览中阻止应用的问题。".into());
        } else if item.status == PreviewStatus::Skipped {
            // Keep the preflight explanation.
        } else if item.status == PreviewStatus::Unchanged {
            result.outcome = ApplyOutcome::Unchanged;
        } else {
            let before = plan
                .snapshots
                .iter()
                .find(|snapshot| snapshot.write_target == item.target)
                .ok_or_else(|| "预览目标不可用。".to_string())?
                .clone();
            match save(item.clone(), before.clone()).await {
                Ok(saved) => {
                    result.outcome = ApplyOutcome::Applied;
                    if !saved.skipped.is_empty() {
                        result.reason = Some(saved.skipped.join("; "));
                        failed = true;
                    }
                    if let Some(after_revision) = saved.after_revision {
                        undo.push(UndoEntry {
                            before,
                            after_revision,
                        });
                    } else {
                        result.reason = Some("modelProfiles.reasonUndoUnavailable".into());
                        failed = true;
                    }
                }
                Err(error) => {
                    result.outcome = ApplyOutcome::Failed;
                    result.reason = Some(error);
                    failed = true;
                }
            }
        }
        items.push(result);
    }
    Ok(ExecutionResult {
        items,
        undo,
        incomplete: failed,
    })
}

static PREVIEWS: Mutex<Option<HashMap<String, PreparedPlan>>> = Mutex::new(None);
static UNDOS: Mutex<Option<HashMap<String, UndoPlan>>> = Mutex::new(None);

pub(crate) fn remember_preview(plan: PreparedPlan) -> Result<String, String> {
    let mut guard = PREVIEWS
        .lock()
        .map_err(|_| "无法创建 Profile 预览。".to_string())?;
    let plans = guard.get_or_insert_with(HashMap::new);
    plans.retain(|_, plan| plan.created_at.elapsed() < PLAN_LIFETIME);
    if plans.len() >= MAX_PENDING {
        plans.clear();
    }
    let token = uuid::Uuid::new_v4().to_string();
    plans.insert(token.clone(), plan);
    Ok(token)
}

pub(crate) fn take_preview(token: &str) -> Result<PreparedPlan, String> {
    let plan = PREVIEWS
        .lock()
        .map_err(|_| "无法读取 Profile 预览。".to_string())?
        .as_mut()
        .and_then(|plans| plans.remove(token))
        .ok_or_else(|| "Profile 预览已过期，请重新预览。".to_string())?;
    if plan.created_at.elapsed() >= PLAN_LIFETIME {
        return Err("Profile 预览已过期，请重新预览。".into());
    }
    Ok(plan)
}

pub(crate) fn remember_undo(plan: UndoPlan) -> Result<String, String> {
    let mut guard = UNDOS.lock().map_err(|_| "无法创建模型撤销。".to_string())?;
    let plans = guard.get_or_insert_with(HashMap::new);
    plans.retain(|_, plan| plan.created_at.elapsed() < UNDO_LIFETIME);
    if plans.len() >= MAX_PENDING {
        plans.clear();
    }
    let token = uuid::Uuid::new_v4().to_string();
    plans.insert(token.clone(), plan);
    Ok(token)
}

pub(crate) fn take_undo(token: &str) -> Result<UndoPlan, String> {
    let plan = UNDOS
        .lock()
        .map_err(|_| "无法读取模型撤销。".to_string())?
        .as_mut()
        .and_then(|plans| plans.remove(token))
        .ok_or_else(|| "模型撤销已过期，请刷新。".to_string())?;
    if plan.created_at.elapsed() >= UNDO_LIFETIME {
        return Err("模型撤销已过期，请刷新。".into());
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_selection::ObservedSelection;
    use crate::services::ofox_bind::status::ToolBindingStatus;

    fn snapshot(target: &str, selection: ModelSelection) -> ToolModelSnapshot {
        ToolModelSnapshot {
            tool_id: target.into(),
            write_target: target.into(),
            revision: "v1".into(),
            binding: ToolBindingStatus::configured(),
            managed_selection: selection,
            // An App's recent model is intentionally different from its writable default.
            observed_selection: ObservedSelection {
                model: Some("observed-app-model".into()),
                source: "appRecent".into(),
            },
            can_write: true,
            scope: vec![],
            scope_models: vec![],
            effort: None,
        }
    }

    #[test]
    fn matching_uses_managed_values_and_order_instead_of_last_clicked_or_observed() {
        let targets = vec![ProfileTarget {
            target: "codex".into(),
            selection: ModelSelection::Single {
                model: "configured-default".into(),
            },
        }];
        let snapshots = vec![snapshot("codex", targets[0].selection.clone())];
        assert_eq!(match_targets(&targets, &snapshots), ProfileMatch::InUse);
        let set = ModelSelection::Models {
            models: vec!["a".into(), "b".into()],
        };
        let changed = ModelSelection::Models {
            models: vec!["b".into(), "a".into()],
        };
        assert_eq!(
            match_targets(
                &[ProfileTarget {
                    target: "workbuddy".into(),
                    selection: set
                }],
                &[snapshot("workbuddy", changed)]
            ),
            ProfileMatch::Different
        );
    }

    #[test]
    fn missing_target_and_partial_success_are_not_full_application() {
        let targets = vec![
            ProfileTarget {
                target: "codex".into(),
                selection: ModelSelection::Single { model: "a".into() },
            },
            ProfileTarget {
                target: "pi".into(),
                selection: ModelSelection::Single { model: "b".into() },
            },
        ];
        assert_eq!(match_targets(&targets, &[]), ProfileMatch::Unavailable);
        assert_eq!(
            match_targets(&targets, &[snapshot("codex", targets[0].selection.clone())]),
            ProfileMatch::Partial
        );
        assert_eq!(
            report_status(&[
                ApplyItem {
                    target: "codex".into(),
                    outcome: ApplyOutcome::Applied,
                    reason: None
                },
                ApplyItem {
                    target: "pi".into(),
                    outcome: ApplyOutcome::Failed,
                    reason: Some("write failed".into())
                },
            ]),
            "partial"
        );
    }

    #[test]
    fn preview_tokens_expire_and_can_only_be_consumed_once() {
        let plan = PreparedPlan {
            created_at: Instant::now(),
            profile_id: "id".into(),
            profile_revision: "v1".into(),
            context: Context {
                apex: "ofox.ai".into(),
                owner: None,
                authenticated: false,
            },
            items: vec![],
            snapshots: vec![],
        };
        let token = remember_preview(plan.clone()).unwrap();
        assert!(take_preview(&token).is_ok());
        assert!(take_preview(&token).is_err());
        let token = remember_preview(PreparedPlan {
            created_at: Instant::now() - PLAN_LIFETIME,
            ..plan
        })
        .unwrap();
        assert!(take_preview(&token).is_err());
    }

    fn execution_plan() -> PreparedPlan {
        let ids = ["codex", "pi", "gemini"];
        PreparedPlan {
            created_at: Instant::now(),
            profile_id: "id".into(),
            profile_revision: "v1".into(),
            context: Context {
                apex: "ofox.ai".into(),
                owner: None,
                authenticated: true,
            },
            items: ids
                .iter()
                .map(|id| PreviewItem {
                    target: (*id).into(),
                    before: Some(ModelSelection::Single {
                        model: "old".into(),
                    }),
                    after: Some(ModelSelection::Single {
                        model: "new".into(),
                    }),
                    status: PreviewStatus::Ready,
                    reason: None,
                    scope: vec![],
                })
                .collect(),
            snapshots: ids
                .iter()
                .map(|id| {
                    snapshot(
                        id,
                        ModelSelection::Single {
                            model: "old".into(),
                        },
                    )
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn first_failure_stops_later_writers_and_undo_only_captures_committed_targets() {
        let plan = execution_plan();
        let calls = std::sync::Arc::new(Mutex::new(Vec::new()));
        let recording = calls.clone();
        let result = execute_items(&plan, move |item, _before| {
            recording.lock().unwrap().push(item.target.clone());
            async move {
                if item.target == "pi" {
                    return Err("write failed".into());
                }
                Ok(ExecutionSuccess {
                    after_revision: Some("after".into()),
                    skipped: vec![],
                })
            }
        })
        .await
        .unwrap();
        assert_eq!(*calls.lock().unwrap(), vec!["codex", "pi"]);
        assert_eq!(
            result
                .items
                .iter()
                .map(|item| item.outcome.clone())
                .collect::<Vec<_>>(),
            vec![
                ApplyOutcome::Applied,
                ApplyOutcome::Failed,
                ApplyOutcome::Skipped
            ]
        );
        assert_eq!(report_status(&result.items), "partial");
        assert_eq!(result.undo.len(), 1);
        assert_eq!(
            result.undo[0].before.managed_selection,
            ModelSelection::Single {
                model: "old".into()
            }
        );
        assert_eq!(result.undo[0].after_revision, "after");
    }

    #[tokio::test]
    async fn partially_saved_collection_is_not_full_application_and_stops_remaining_targets() {
        let mut plan = execution_plan();
        plan.items[0].target = "opencode".into();
        plan.snapshots[0].write_target = "opencode".into();
        let calls = std::sync::Arc::new(Mutex::new(Vec::new()));
        let recording = calls.clone();
        let result = execute_items(&plan, move |item, _| {
            recording.lock().unwrap().push(item.target);
            async {
                Ok(ExecutionSuccess {
                    after_revision: Some("after".into()),
                    skipped: vec!["gone: unavailable".into()],
                })
            }
        })
        .await
        .unwrap();
        assert_eq!(*calls.lock().unwrap(), vec!["opencode"]);
        assert!(result.incomplete);
        assert_eq!(result.undo.len(), 1);
        assert_eq!(report_status(&result.items), "partial");
    }

    #[tokio::test]
    async fn blocked_preview_never_writes_and_missing_receipt_never_creates_undo() {
        let mut blocked = execution_plan();
        blocked.items[2].status = PreviewStatus::Blocked;
        let mut writes = 0;
        let result = execute_items(&blocked, |_, _| {
            writes += 1;
            std::future::ready(Ok(ExecutionSuccess {
                after_revision: None,
                skipped: vec![],
            }))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(writes, 0);

        let calls = std::sync::Arc::new(Mutex::new(Vec::new()));
        let recording = calls.clone();
        let result = execute_items(&execution_plan(), move |item, _| {
            recording.lock().unwrap().push(item.target);
            async {
                Ok(ExecutionSuccess {
                    after_revision: None,
                    skipped: vec![],
                })
            }
        })
        .await
        .unwrap();
        assert_eq!(*calls.lock().unwrap(), vec!["codex"]);
        assert!(result.incomplete);
        assert!(result.undo.is_empty());
        assert_eq!(result.items[0].outcome, ApplyOutcome::Applied);
        assert_eq!(
            result.items[0].reason.as_deref(),
            Some("modelProfiles.reasonUndoUnavailable")
        );
    }
}
