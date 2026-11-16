//! Opt-in journaling inside existing writers' configuration locks. Normal app
//! operations have no active context and retain their existing behavior.
use super::{
    db_state,
    journal::Journal,
    scope::{self, sensitive_path, Access, Scope},
};
use crate::database::Database;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::watch;

tokio::task_local! {pub static ACTIVE: Arc<Context>;}
pub struct Context {
    pub journal: Mutex<Journal>,
    pub tool: String,
    pub apex: Option<String>,
    pub operation: String,
    pub scope: Scope,
    pub cancel: watch::Receiver<bool>,
    pub deadline: tokio::time::Instant,
    pub error: Mutex<Option<String>>,
    pub observed: AtomicBool,
}
pub struct ChangeGuard<'a> {
    context: Arc<Context>,
    db: Option<&'a Database>,
    id: String,
}
impl Drop for ChangeGuard<'_> {
    fn drop(&mut self) {
        let state = self.db.map_or(Ok(Value::Null), |db| {
            db_state::capture(db, &self.context.tool)
        });
        let result = state.and_then(|value| {
            self.context
                .journal
                .lock()
                .map_err(|_| "assistant_journal_unavailable".to_string())?
                .finish(&self.id, value)
        });
        if let Err(error) = result {
            if let Ok(mut slot) = self.context.error.lock() {
                *slot = Some(error);
            }
        }
    }
}
pub fn check_active() -> Result<(), String> {
    let Ok(context) = ACTIVE.try_with(Arc::clone) else {
        return Ok(());
    };
    if *context.cancel.borrow() || tokio::time::Instant::now() >= context.deadline {
        return Err("assistant_cancelled".into());
    }
    if context
        .apex
        .as_ref()
        .is_some_and(|apex| apex != crate::ofox_apex::current_apex())
    {
        return Err("assistant_identity_changed".into());
    }
    Ok(())
}

pub fn begin<'a>(
    db: Option<&'a Database>,
    tool: &str,
    paths: Vec<PathBuf>,
) -> Result<Option<ChangeGuard<'a>>, String> {
    begin_with(db, tool, paths, None)
}

/// Launcher repair writes host-chosen files in one folder of the private root.
/// Only that folder, reached without a symlink, skips the private overlay.
pub fn begin_launcher(
    tool: &str,
    launchers: &Path,
    paths: Vec<PathBuf>,
) -> Result<Option<ChangeGuard<'static>>, String> {
    begin_with(None, tool, paths, Some(launchers))
}

fn begin_with<'a>(
    db: Option<&'a Database>,
    tool: &str,
    paths: Vec<PathBuf>,
    launchers: Option<&Path>,
) -> Result<Option<ChangeGuard<'a>>, String> {
    let Ok(context) = ACTIVE.try_with(Arc::clone) else {
        return Ok(None);
    };
    check_active()?;
    if context.tool != tool {
        return Err("assistant_invalid_tool".into());
    }
    if *context.cancel.borrow() || tokio::time::Instant::now() >= context.deadline {
        return Err("assistant_cancelled".into());
    }
    for path in &paths {
        if denied(&context.scope, path, launchers)? {
            return Err("assistant_path_denied".into());
        }
    }
    let state = db.map_or(Ok(Value::Null), |db| db_state::capture(db, tool))?;
    let id = context
        .journal
        .lock()
        .map_err(|_| "assistant_journal_unavailable")?
        .prepare(&context.operation, &paths, state)?;
    context.observed.store(true, Ordering::SeqCst);
    Ok(Some(ChangeGuard { context, db, id }))
}

fn denied(scope: &Scope, path: &Path, launchers: Option<&Path>) -> Result<bool, String> {
    let real = scope::resolve(path)?;
    if sensitive_path(&path.to_string_lossy()) || sensitive_path(&real.to_string_lossy()) {
        return Ok(true);
    }
    if let Some(dir) = launchers {
        let linked = std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink());
        if !linked && path.starts_with(dir) && real.starts_with(scope::resolve(dir)?) {
            return Ok(false);
        }
    }
    Ok(scope.write_forbidden(path) || scope.check(path)?.access == Access::Forbidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app_config::BindableTool, services::model_fetch::FetchedModel};
    fn context(home: &std::path::Path, tool: &str) -> Arc<Context> {
        let (_, cancel) = watch::channel(false);
        Arc::new(Context {
            journal: Mutex::new(
                Journal::open(&home.join("changes"), &[3; 32], "isolated/test").unwrap(),
            ),
            tool: tool.into(),
            apex: None,
            operation: "rebind_tool".into(),
            scope: Scope::new(home.into(), vec![home.into()], vec![]).unwrap(),
            cancel,
            deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(60),
            error: Mutex::new(None),
            observed: AtomicBool::new(false),
        })
    }
    fn protected_context(
        home: &std::path::Path,
        tool: &str,
        private: &std::path::Path,
    ) -> Arc<Context> {
        let mut ctx = Arc::try_unwrap(context(home, tool)).ok().unwrap();
        ctx.scope.protect(private, &[], vec![]).unwrap();
        Arc::new(ctx)
    }
    #[tokio::test]
    async fn host_launcher_repair_writes_inside_the_private_root() {
        let tmp = tempfile::tempdir().unwrap();
        let private = tmp.path().join(".ofox-desktop");
        std::fs::create_dir_all(private.join("bin")).unwrap();
        let ctx = protected_context(tmp.path(), "claude", &private);
        let bin = private.join("bin");
        let result = ACTIVE
            .scope(ctx, async move {
                begin_launcher("claude", &bin, vec![bin.join("claude")]).map(|g| g.is_some())
            })
            .await;
        assert_eq!(result, Ok(true));
    }
    #[tokio::test]
    async fn host_writes_into_private_state_stay_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let private = tmp.path().join(".ofox-desktop");
        std::fs::create_dir_all(private.join("bin")).unwrap();
        let ctx = protected_context(tmp.path(), "claude", &private);
        let bin = private.join("bin");
        let state = private.join("agent-skills.json");
        let result = ACTIVE
            .scope(ctx, async move {
                (
                    begin(None, "claude", vec![bin.join("claude")]).map(|g| g.is_some()),
                    begin_launcher("claude", &bin, vec![state]).map(|g| g.is_some()),
                )
            })
            .await;
        assert_eq!(result.0, Err("assistant_path_denied".into()));
        assert_eq!(result.1, Err("assistant_path_denied".into()));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn launcher_writes_through_a_symlinked_folder_stay_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let private = tmp.path().join(".ofox-desktop");
        std::fs::create_dir_all(private.join("assistant")).unwrap();
        std::os::unix::fs::symlink(private.join("assistant"), private.join("bin")).unwrap();
        let ctx = protected_context(tmp.path(), "claude", &private);
        let bin = private.join("bin");
        let result = ACTIVE
            .scope(ctx, async move {
                begin_launcher("claude", &bin, vec![bin.join("claude")]).map(|g| g.is_some())
            })
            .await;
        assert_eq!(result, Err("assistant_path_denied".into()));
    }
    #[tokio::test]
    async fn host_writes_still_refuse_sensitive_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let private = tmp.path().join(".ofox-desktop");
        std::fs::create_dir_all(&private).unwrap();
        let ctx = protected_context(tmp.path(), "claude", &private);
        let key = tmp.path().join(".ssh/config");
        let result = ACTIVE
            .scope(ctx, async move {
                begin(None, "claude", vec![key]).map(|g| g.is_some())
            })
            .await;
        assert_eq!(result, Err("assistant_path_denied".into()));
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn native_binding_journals_credentials_and_db_under_both_region_choices() {
        for apex in ["ofox.ai", "ofox.io"] {
            let sandbox = crate::test_sandbox::sandbox();
            let home = sandbox.home();
            crate::settings::mutate_settings(|s| s.ofox_apex = Some(apex.into())).unwrap();
            let db = Database::memory().unwrap();
            let folder = home.join(".pi/agent");
            std::fs::create_dir_all(&folder).unwrap();
            let auth =
                r#"{"personal":{"type":"api_key","key":"sk-test-original-credential-123456"}}"#;
            std::fs::write(folder.join("auth.json"), auth).unwrap();
            let model = FetchedModel {
                id: "test/model".into(),
                name: None,
                owned_by: None,
                pricing_prompt: None,
                supported_endpoints: Some(vec!["/v1/chat/completions".into()]),
                supported_parameters: Some(vec!["tools".into()]),
                input_modalities: None,
                output_modalities: None,
            };
            let ctx = context(home, "pi");
            ACTIVE
                .scope(
                    ctx.clone(),
                    crate::native_tools::bind_with_catalog(
                        &db,
                        BindableTool::Pi,
                        "sk-of-test-journal-key",
                        &model,
                        std::slice::from_ref(&model),
                    ),
                )
                .await
                .unwrap();
            assert!(ctx.observed.load(Ordering::SeqCst));
            assert!(ctx.error.lock().unwrap().is_none());
            assert!(std::fs::read_to_string(folder.join("models.json"))
                .unwrap()
                .contains(apex));
            let bytes = std::fs::read(home.join("changes/journal.enc")).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("original-credential"));
            drop(ctx);
            let mut journal =
                Journal::open(&home.join("changes"), &[3; 32], "isolated/test").unwrap();
            journal
                .restore_with(
                    |before| db_state::capture(&db, before["scope"].as_str().unwrap()),
                    |before, after| db_state::restore(&db, before, after),
                )
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(folder.join("auth.json")).unwrap(),
                auth
            );
            assert!(!folder.join("models.json").exists());
            assert!(db.get_bind_record("pi").unwrap().is_none());
        }
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn classic_and_workbuddy_bindings_restore_original_files_and_state_on_both_apexes() {
        for apex in ["ofox.ai", "ofox.io"] {
            for tool in ["claude", "workbuddy"] {
                let sandbox = crate::test_sandbox::sandbox();
                let home = sandbox.home();
                crate::settings::mutate_settings(|s| s.ofox_apex = Some(apex.into())).unwrap();
                let db = Database::memory().unwrap();
                let file = if tool == "claude" {
                    home.join(".claude/settings.json")
                } else {
                    crate::workbuddy_config::models_path()
                };
                let original = if tool == "claude" {
                    r#"{"env":{"ANTHROPIC_API_KEY":"third-party-credential"},"theme":"dark"}"#
                } else {
                    r#"[{"id":"personal","apiKey":"third-party-credential"}]"#
                };
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(&file, original).unwrap();
                if tool == "claude" {
                    db.save_provider("claude",&crate::provider::Provider::with_id("ofox-claude".into(),"Ofox".into(),serde_json::json!({"env":{"ANTHROPIC_AUTH_TOKEN":"","ANTHROPIC_MODEL":"test/model"}}),None)).unwrap();
                }
                let before = db_state::capture(&db, tool).unwrap();
                let ctx = context(home, tool);
                ACTIVE
                    .scope(ctx.clone(), async {
                        if tool == "claude" {
                            crate::services::ofox_bind::bind(
                                &db,
                                crate::services::ofox_bind::Tool::Claude,
                                "claude",
                                "sk-of-test",
                            )
                            .await
                        } else {
                            crate::workbuddy_config::sync_selected_models(
                                &db,
                                "sk-of-test",
                                &[crate::workbuddy_config::WorkBuddyModelSelection {
                                    id: "test/model".into(),
                                    name: "Test".into(),
                                    supports_tool_call: true,
                                    supports_images: false,
                                    supports_reasoning: false,
                                    supported_endpoints: None,
                                }],
                            )
                            .await
                        }
                    })
                    .await
                    .unwrap();
                assert!(ctx.error.lock().unwrap().is_none());
                assert!(ctx.observed.load(Ordering::SeqCst));
                assert!(std::fs::read_to_string(&file).unwrap().contains(apex));
                drop(ctx);
                let mut journal =
                    Journal::open(&home.join("changes"), &[3; 32], "isolated/test").unwrap();
                journal
                    .restore_with(
                        |state| db_state::capture(&db, state["scope"].as_str().unwrap()),
                        |before, after| db_state::restore(&db, before, after),
                    )
                    .unwrap();
                assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
                assert_eq!(db_state::capture(&db, tool).unwrap(), before);
            }
        }
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn first_resolution_and_region_switch_fence_a_frozen_transaction() {
        for apex in ["ofox.ai", "ofox.io"] {
            let sandbox = crate::test_sandbox::sandbox();
            let home = sandbox.home();
            assert!(!crate::settings::get_settings()
                .ofox_apex_resolved
                .unwrap_or(false));
            crate::settings::mutate_settings(|s| {
                s.ofox_apex = Some(apex.into());
                s.ofox_apex_resolved = Some(true);
            })
            .unwrap();
            let mut ctx = context(home, "pi");
            Arc::get_mut(&mut ctx).unwrap().apex = Some(apex.into());
            ACTIVE
                .scope(ctx.clone(), async { check_active() })
                .await
                .unwrap();
            crate::settings::mutate_settings(|s| {
                s.ofox_apex = Some(
                    if apex == "ofox.ai" {
                        "ofox.io"
                    } else {
                        "ofox.ai"
                    }
                    .into(),
                )
            })
            .unwrap();
            assert_eq!(
                ACTIVE
                    .scope(ctx.clone(), async {
                        begin(None, "pi", vec![home.join("config")]).map(|_| ())
                    })
                    .await
                    .unwrap_err(),
                "assistant_identity_changed"
            );
            assert!(ctx.journal.lock().unwrap().changes().is_empty());
            assert!(!home.join("config").exists());
        }
    }

    #[tokio::test]
    async fn cancelled_transaction_refuses_snapshot_and_mutation() {
        let home = tempfile::tempdir().unwrap();
        let ctx = context(home.path(), "pi");
        let (cancel, rx) = watch::channel(true);
        let mut context = Arc::try_unwrap(ctx).ok().unwrap();
        context.cancel = rx;
        let ctx = Arc::new(context);
        let result = ACTIVE
            .scope(ctx.clone(), async {
                begin(None, "pi", vec![home.path().join("file")]).map(|_| ())
            })
            .await;
        assert_eq!(result.unwrap_err(), "assistant_cancelled");
        assert!(ctx.journal.lock().unwrap().changes().is_empty());
        drop(cancel);
    }
}
