use std::sync::Arc;

use cc_switch_lib::{AppState, Database, MultiAppConfig};

// One implementation for the unit and integration harnesses: see
// `src/test_sandbox.rs` for what a sandbox sets, clears and restores.
#[allow(unused_imports)] // not every test binary asks for the guard type
pub use cc_switch_lib::test_sandbox::{sandbox, Sandbox};

/// 创建测试用的 AppState，包含一个空的数据库
#[allow(dead_code)]
pub fn create_test_state() -> Result<AppState, Box<dyn std::error::Error>> {
    let db = Arc::new(Database::init()?);
    Ok(AppState::new(db))
}

/// 创建测试用的 AppState，并从 MultiAppConfig 迁移数据
#[allow(dead_code)]
pub fn create_test_state_with_config(
    config: &MultiAppConfig,
) -> Result<AppState, Box<dyn std::error::Error>> {
    let db = Arc::new(Database::init()?);
    db.migrate_from_json(config)?;
    Ok(AppState::new(db))
}
