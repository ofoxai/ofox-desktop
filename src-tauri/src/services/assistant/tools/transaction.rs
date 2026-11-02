//! Opt-in journaling inside existing writers' configuration locks. Normal app
//! operations have no active context and retain their existing behavior.
use super::{
    db_state,
    journal::Journal,
    scope::{Access, Scope},
};
use crate::database::Database;
use serde_json::Value;
use std::{
    path::PathBuf,
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
        if context.scope.check(path)?.access == Access::Forbidden {
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
