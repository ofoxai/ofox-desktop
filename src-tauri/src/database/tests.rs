//! 数据库模块测试
//!
//! 包含 Schema 迁移和基本功能的测试。

use super::*;
use crate::app_config::MultiAppConfig;
use crate::provider::{Provider, ProviderManager};
use indexmap::IndexMap;
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::HashMap;
use tempfile::NamedTempFile;

const LEGACY_SCHEMA_SQL: &str = r#"
    CREATE TABLE providers (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        settings_config TEXT NOT NULL,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE provider_endpoints (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        provider_id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        url TEXT NOT NULL
    );
    CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        server_config TEXT NOT NULL
    );
    CREATE TABLE prompts (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        content TEXT NOT NULL,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE skills (
        key TEXT PRIMARY KEY,
        installed BOOLEAN NOT NULL DEFAULT 0
    );
    CREATE TABLE skill_repos (
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        PRIMARY KEY (owner, name)
    );
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT
    );
"#;

// v3.8.x（schema v1）的真实表结构快照：用于验证从 v3.8.* 升级到当前版本的迁移链路
// 参考：tag v3.8.3 的 src-tauri/src/database/schema.rs
const V3_8_SCHEMA_V1_SQL: &str = r#"
    CREATE TABLE providers (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        settings_config TEXT NOT NULL,
        website_url TEXT,
        category TEXT,
        created_at INTEGER,
        sort_index INTEGER,
        notes TEXT,
        icon TEXT,
        icon_color TEXT,
        meta TEXT NOT NULL DEFAULT '{}',
        is_current BOOLEAN NOT NULL DEFAULT 0,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE provider_endpoints (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        provider_id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        url TEXT NOT NULL,
        added_at INTEGER,
        FOREIGN KEY (provider_id, app_type) REFERENCES providers(id, app_type) ON DELETE CASCADE
    );
    CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        server_config TEXT NOT NULL,
        description TEXT,
        homepage TEXT,
        docs TEXT,
        tags TEXT NOT NULL DEFAULT '[]',
        enabled_claude BOOLEAN NOT NULL DEFAULT 0,
        enabled_codex BOOLEAN NOT NULL DEFAULT 0,
        enabled_gemini BOOLEAN NOT NULL DEFAULT 0
    );
    CREATE TABLE prompts (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        content TEXT NOT NULL,
        description TEXT,
        enabled BOOLEAN NOT NULL DEFAULT 1,
        created_at INTEGER,
        updated_at INTEGER,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE skills (
        key TEXT PRIMARY KEY,
        installed BOOLEAN NOT NULL DEFAULT 0,
        installed_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE skill_repos (
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        branch TEXT NOT NULL DEFAULT 'main',
        enabled BOOLEAN NOT NULL DEFAULT 1,
        PRIMARY KEY (owner, name)
    );
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT
    );
"#;

#[derive(Debug)]
struct ColumnInfo {
    r#type: String,
    notnull: i64,
    default: Option<String>,
}

fn get_column_info(conn: &Connection, table: &str, column: &str) -> ColumnInfo {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\");"))
        .expect("prepare pragma");
    let mut rows = stmt.query([]).expect("query pragma");
    while let Some(row) = rows.next().expect("read row") {
        let column_name: String = row.get(1).expect("name");
        if column_name.eq_ignore_ascii_case(column) {
            return ColumnInfo {
                r#type: row.get::<_, String>(2).expect("type"),
                notnull: row.get::<_, i64>(3).expect("notnull"),
                default: row.get::<_, Option<String>>(4).ok().flatten(),
            };
        }
    }
    panic!("column {table}.{column} not found");
}

fn normalize_default(default: &Option<String>) -> Option<String> {
    default
        .as_ref()
        .map(|s| s.trim_matches('\'').trim_matches('"').to_string())
}

#[test]
fn schema_migration_sets_user_version_when_missing() {
    let conn = Connection::open_in_memory().expect("open memory db");

    Database::create_tables_on_conn(&conn).expect("create tables");
    assert_eq!(
        Database::get_user_version(&conn).expect("read version before"),
        0
    );

    Database::apply_schema_migrations_on_conn(&conn).expect("apply migration");

    assert_eq!(
        Database::get_user_version(&conn).expect("read version after"),
        SCHEMA_VERSION
    );
}

#[test]
fn schema_migration_rejects_future_version() {
    let conn = Connection::open_in_memory().expect("open memory db");
    Database::create_tables_on_conn(&conn).expect("create tables");
    Database::set_user_version(&conn, SCHEMA_VERSION + 1).expect("set future version");

    let err =
        Database::apply_schema_migrations_on_conn(&conn).expect_err("should reject higher version");
    assert!(
        err.to_string().contains("数据库版本过新"),
        "unexpected error: {err}"
    );
}

#[test]
fn schema_migration_adds_missing_columns_for_providers() {
    let conn = Connection::open_in_memory().expect("open memory db");

    // 创建旧版 providers 表，缺少新增列
    conn.execute_batch(LEGACY_SCHEMA_SQL)
        .expect("seed old schema");

    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    // 验证关键新增列已补齐
    for (table, column) in [
        ("providers", "meta"),
        ("providers", "is_current"),
        ("provider_endpoints", "added_at"),
        ("mcp_servers", "enabled_gemini"),
        ("prompts", "updated_at"),
        ("skills", "installed_at"),
        ("skill_repos", "enabled"),
    ] {
        assert!(
            Database::has_column(&conn, table, column).expect("check column"),
            "{table}.{column} should exist after migration"
        );
    }

    // 验证 meta 列约束保持一致
    let meta = get_column_info(&conn, "providers", "meta");
    assert_eq!(meta.notnull, 1, "meta should be NOT NULL");
    assert_eq!(
        normalize_default(&meta.default).as_deref(),
        Some("{}"),
        "meta default should be '{{}}'"
    );

    assert_eq!(
        Database::get_user_version(&conn).expect("version after migration"),
        SCHEMA_VERSION
    );
}

#[test]
fn schema_migration_aligns_column_defaults_and_types() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(LEGACY_SCHEMA_SQL)
        .expect("seed old schema");

    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    let is_current = get_column_info(&conn, "providers", "is_current");
    assert_eq!(is_current.r#type, "BOOLEAN");
    assert_eq!(is_current.notnull, 1);
    assert_eq!(normalize_default(&is_current.default).as_deref(), Some("0"));

    let tags = get_column_info(&conn, "mcp_servers", "tags");
    assert_eq!(tags.r#type, "TEXT");
    assert_eq!(tags.notnull, 1);
    assert_eq!(normalize_default(&tags.default).as_deref(), Some("[]"));

    let enabled = get_column_info(&conn, "prompts", "enabled");
    assert_eq!(enabled.r#type, "BOOLEAN");
    assert_eq!(enabled.notnull, 1);
    assert_eq!(normalize_default(&enabled.default).as_deref(), Some("1"));

    let installed_at = get_column_info(&conn, "skills", "installed_at");
    assert_eq!(installed_at.r#type, "INTEGER");
    assert_eq!(installed_at.notnull, 1);
    assert_eq!(
        normalize_default(&installed_at.default).as_deref(),
        Some("0")
    );

    let branch = get_column_info(&conn, "skill_repos", "branch");
    assert_eq!(branch.r#type, "TEXT");
    assert_eq!(normalize_default(&branch.default).as_deref(), Some("main"));

    let skill_repo_enabled = get_column_info(&conn, "skill_repos", "enabled");
    assert_eq!(skill_repo_enabled.r#type, "BOOLEAN");
    assert_eq!(skill_repo_enabled.notnull, 1);
    assert_eq!(
        normalize_default(&skill_repo_enabled.default).as_deref(),
        Some("1")
    );
}

#[test]
fn schema_create_tables_include_pricing_model_columns() {
    let conn = Connection::open_in_memory().expect("open memory db");
    Database::create_tables_on_conn(&conn).expect("create tables");

    let multiplier = get_column_info(&conn, "proxy_config", "default_cost_multiplier");
    assert_eq!(multiplier.r#type, "TEXT");
    assert_eq!(multiplier.notnull, 1);
    assert_eq!(normalize_default(&multiplier.default).as_deref(), Some("1"));

    let pricing_source = get_column_info(&conn, "proxy_config", "pricing_model_source");
    assert_eq!(pricing_source.r#type, "TEXT");
    assert_eq!(pricing_source.notnull, 1);
    assert_eq!(
        normalize_default(&pricing_source.default).as_deref(),
        Some("response")
    );

    let request_model = get_column_info(&conn, "proxy_request_logs", "request_model");
    assert_eq!(request_model.r#type, "TEXT");
    assert_eq!(request_model.notnull, 0);
}

#[test]
fn schema_migration_v4_adds_pricing_model_columns() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(
        r#"
        CREATE TABLE providers (
            id TEXT NOT NULL,
            app_type TEXT NOT NULL,
            name TEXT NOT NULL,
            settings_config TEXT NOT NULL DEFAULT '{}',
            meta TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (id, app_type)
        );
        CREATE TABLE proxy_config (app_type TEXT PRIMARY KEY);
        CREATE TABLE proxy_request_logs (request_id TEXT PRIMARY KEY, model TEXT NOT NULL);
        CREATE TABLE mcp_servers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            server_config TEXT NOT NULL,
            enabled_claude INTEGER NOT NULL DEFAULT 0,
            enabled_codex INTEGER NOT NULL DEFAULT 0,
            enabled_gemini INTEGER NOT NULL DEFAULT 0,
            enabled_opencode INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .expect("seed v4 schema");

    Database::set_user_version(&conn, 4).expect("set user_version=4");
    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    let multiplier = get_column_info(&conn, "proxy_config", "default_cost_multiplier");
    assert_eq!(multiplier.r#type, "TEXT");
    assert_eq!(multiplier.notnull, 1);
    assert_eq!(normalize_default(&multiplier.default).as_deref(), Some("1"));

    let pricing_source = get_column_info(&conn, "proxy_config", "pricing_model_source");
    assert_eq!(pricing_source.r#type, "TEXT");
    assert_eq!(pricing_source.notnull, 1);
    assert_eq!(
        normalize_default(&pricing_source.default).as_deref(),
        Some("response")
    );

    let request_model = get_column_info(&conn, "proxy_request_logs", "request_model");
    assert_eq!(request_model.r#type, "TEXT");
    assert_eq!(request_model.notnull, 0);

    assert_eq!(
        Database::get_user_version(&conn).expect("version after migration"),
        SCHEMA_VERSION
    );
}

#[test]
fn schema_create_tables_repairs_legacy_proxy_config_singleton_to_per_app() {
    let conn = Connection::open_in_memory().expect("open memory db");

    // 模拟测试版 v2：user_version=2，但 proxy_config 仍是单例结构（无 app_type）
    Database::set_user_version(&conn, 2).expect("set user_version");
    conn.execute_batch(
        r#"
        CREATE TABLE proxy_config (
            id INTEGER PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 0,
            listen_address TEXT NOT NULL DEFAULT '127.0.0.1',
            listen_port INTEGER NOT NULL DEFAULT 5000,
            max_retries INTEGER NOT NULL DEFAULT 3,
            request_timeout INTEGER NOT NULL DEFAULT 300,
            enable_logging INTEGER NOT NULL DEFAULT 1,
            target_app TEXT NOT NULL DEFAULT 'claude',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        INSERT INTO proxy_config (id, enabled) VALUES (1, 1);
        "#,
    )
    .expect("seed legacy proxy_config");

    Database::create_tables_on_conn(&conn).expect("create tables should repair proxy_config");

    assert!(
        Database::has_column(&conn, "proxy_config", "app_type").expect("check app_type"),
        "proxy_config should be migrated to per-app structure"
    );

    let count: i32 = conn
        .query_row("SELECT COUNT(*) FROM proxy_config", [], |r| r.get(0))
        .expect("count rows");
    assert_eq!(count, 3, "per-app proxy_config should have 3 rows");

    // 新结构下应能按 app_type 查询
    let _: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM proxy_config WHERE app_type = 'claude'",
            [],
            |r| r.get(0),
        )
        .expect("query by app_type");
}

#[test]
fn migration_from_v3_8_schema_v1_to_current_schema_v3() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute("PRAGMA foreign_keys = ON;", [])
        .expect("enable foreign keys");

    // 模拟 v3.8.* 用户的数据库（schema v1）
    conn.execute_batch(V3_8_SCHEMA_V1_SQL)
        .expect("seed v3.8 schema v1");
    Database::set_user_version(&conn, 1).expect("set user_version=1");

    // 插入一条旧版 Provider + Skill（用于验证迁移不会破坏既有数据）
    conn.execute(
        "INSERT INTO providers (
            id, app_type, name, settings_config, website_url, category,
            created_at, sort_index, notes, icon, icon_color, meta, is_current
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            "p1",
            "claude",
            "Test Provider",
            serde_json::to_string(&json!({ "anthropicApiKey": "sk-test" })).unwrap(),
            Option::<String>::None,
            Option::<String>::None,
            Option::<i64>::None,
            Option::<usize>::None,
            Option::<String>::None,
            Option::<String>::None,
            Option::<String>::None,
            "{}",
            1,
        ],
    )
    .expect("seed provider");

    conn.execute(
        "INSERT INTO skills (key, installed, installed_at) VALUES (?1, ?2, ?3)",
        params!["claude:demo-skill", 1, 1700000000i64],
    )
    .expect("seed legacy skill");

    // 按应用启动流程：先 create_tables（补齐新增表），再 apply_schema_migrations（按 user_version 迁移）
    Database::create_tables_on_conn(&conn).expect("create tables");
    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    assert_eq!(
        Database::get_user_version(&conn).expect("user_version after migration"),
        SCHEMA_VERSION
    );

    // v1 -> v2：providers 新增字段必须补齐
    for column in [
        "cost_multiplier",
        "limit_daily_usd",
        "limit_monthly_usd",
        "provider_type",
        "in_failover_queue",
    ] {
        assert!(
            Database::has_column(&conn, "providers", column).expect("check column"),
            "providers.{column} should exist after migration"
        );
    }

    // 旧 provider 不应丢失，且新增字段应有默认值
    let provider_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM providers WHERE id = 'p1' AND app_type = 'claude'",
            [],
            |r| r.get(0),
        )
        .expect("count providers");
    assert_eq!(provider_count, 1);

    let cost_multiplier: String = conn
        .query_row(
            "SELECT cost_multiplier FROM providers WHERE id = 'p1' AND app_type = 'claude'",
            [],
            |r| r.get(0),
        )
        .expect("read cost_multiplier");
    assert_eq!(cost_multiplier, "1.0");

    // v2 -> v3：skills 表重建为统一结构，并设置 pending 标记（后续由启动时扫描文件系统重建数据）
    assert!(
        Database::has_column(&conn, "skills", "enabled_claude").expect("check skills v3 column"),
        "skills table should be migrated to v3 structure"
    );
    let skills_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
        .expect("count skills");
    assert_eq!(skills_count, 0, "skills table should be rebuilt empty");

    let pending: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'skills_ssot_migration_pending'",
            [],
            |r| r.get(0),
        )
        .ok();
    assert!(
        matches!(pending.as_deref(), Some("true") | Some("1")),
        "skills_ssot_migration_pending should be set after v2->v3 migration"
    );
    let snapshot: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'skills_ssot_migration_snapshot'",
            [],
            |r| r.get(0),
        )
        .ok();
    let snapshot = snapshot.expect("skills migration snapshot should be recorded");
    let snapshot_rows: serde_json::Value =
        serde_json::from_str(&snapshot).expect("parse skills migration snapshot");
    assert!(
        snapshot_rows
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| {
                row.get("directory").and_then(|v| v.as_str()) == Some("demo-skill")
                    && row.get("app_type").and_then(|v| v.as_str()) == Some("claude")
            })),
        "skills migration snapshot should preserve legacy app mapping"
    );

    // v3.9+ 新增：proxy_config 三行 seed 必须存在（否则 UI 会查不到默认值）
    let proxy_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM proxy_config", [], |r| r.get(0))
        .expect("count proxy_config rows");
    assert_eq!(proxy_rows, 3);

    // model_pricing 应具备默认数据（迁移时会 seed）
    let pricing_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM model_pricing", [], |r| r.get(0))
        .expect("count model_pricing rows");
    assert!(pricing_rows > 0, "model_pricing should be seeded");
}

#[test]
fn schema_dry_run_does_not_write_to_disk() {
    // Create minimal valid config for migration
    let mut apps = HashMap::new();
    apps.insert("claude".to_string(), ProviderManager::default());

    let config = MultiAppConfig {
        version: 2,
        apps,
        mcp: Default::default(),
        prompts: Default::default(),
        skills: Default::default(),
        common_config_snippets: Default::default(),
        claude_common_config_snippet: None,
    };

    // Dry-run should succeed without any file I/O errors
    let result = Database::migrate_from_json_dry_run(&config);
    assert!(
        result.is_ok(),
        "Dry-run should succeed with valid config: {result:?}"
    );
}

#[test]
fn dry_run_validates_schema_compatibility() {
    // Create config with actual provider data
    let mut providers = IndexMap::new();
    providers.insert(
        "test-provider".to_string(),
        Provider {
            id: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            settings_config: json!({
                "anthropicApiKey": "sk-test-123",
            }),
            website_url: None,
            category: None,
            created_at: Some(1234567890),
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        },
    );

    let manager = ProviderManager {
        providers,
        current: "test-provider".to_string(),
    };

    let mut apps = HashMap::new();
    apps.insert("claude".to_string(), manager);

    let config = MultiAppConfig {
        version: 2,
        apps,
        mcp: Default::default(),
        prompts: Default::default(),
        skills: Default::default(),
        common_config_snippets: Default::default(),
        claude_common_config_snippet: None,
    };

    // Dry-run should validate the full migration path
    let result = Database::migrate_from_json_dry_run(&config);
    assert!(
        result.is_ok(),
        "Dry-run should succeed with provider data: {result:?}"
    );
}

#[test]
fn schema_model_pricing_is_seeded_on_init() {
    let db = Database::memory().expect("create memory db");

    let conn = db.conn.lock().expect("lock conn");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM model_pricing", [], |row| row.get(0))
        .expect("count pricing");

    assert!(
        count > 0,
        "模型定价数据应该在初始化时自动填充，实际数量: {}",
        count
    );

    // 验证包含 Claude 模型
    let claude_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_pricing WHERE model_id LIKE 'claude-%'",
            [],
            |row| row.get(0),
        )
        .expect("check claude");
    assert!(
        claude_count > 0,
        "应该包含 Claude 模型定价，实际数量: {}",
        claude_count
    );

    // 验证包含 GPT 模型
    let gpt_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_pricing WHERE model_id LIKE 'gpt-%'",
            [],
            |row| row.get(0),
        )
        .expect("check gpt");
    assert!(
        gpt_count > 0,
        "应该包含 GPT 模型定价，实际数量: {}",
        gpt_count
    );

    // 验证包含 Gemini 模型
    let gemini_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_pricing WHERE model_id LIKE 'gemini-%'",
            [],
            |row| row.get(0),
        )
        .expect("check gemini");
    assert!(
        gemini_count > 0,
        "应该包含 Gemini 模型定价，实际数量: {}",
        gemini_count
    );
}

#[test]
fn ensure_incremental_auto_vacuum_rebuilds_existing_file_db() {
    let temp = NamedTempFile::new().expect("create temp db file");
    let path = temp.path().to_path_buf();

    let conn = Connection::open(&path).expect("open temp db");
    conn.execute("PRAGMA auto_vacuum = NONE;", [])
        .expect("set none auto_vacuum");
    Database::create_tables_on_conn(&conn).expect("create tables");

    assert_eq!(
        Database::get_auto_vacuum_mode(&conn).expect("auto_vacuum before rebuild"),
        0,
        "existing file db should start with NONE auto_vacuum"
    );

    let rebuilt =
        Database::ensure_incremental_auto_vacuum_on_conn(&conn).expect("enable incremental mode");
    assert!(rebuilt, "existing db should require rebuild via VACUUM");
    drop(conn);

    let reopened = Connection::open(&path).expect("reopen temp db");
    assert_eq!(
        Database::get_auto_vacuum_mode(&reopened).expect("auto_vacuum after rebuild"),
        2,
        "file db should persist INCREMENTAL auto_vacuum after VACUUM rebuild"
    );
}

/// The usage tables exactly as schema v10 created them (`create_tables_on_conn`
/// at v10): the v10 -> v11 migration reads and rewrites these.
const V10_USAGE_SCHEMA_SQL: &str = r#"
    CREATE TABLE proxy_request_logs (
        request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, app_type TEXT NOT NULL, model TEXT NOT NULL,
        request_model TEXT,
        input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
        cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
        input_cost_usd TEXT NOT NULL DEFAULT '0', output_cost_usd TEXT NOT NULL DEFAULT '0',
        cache_read_cost_usd TEXT NOT NULL DEFAULT '0', cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
        total_cost_usd TEXT NOT NULL DEFAULT '0', latency_ms INTEGER NOT NULL, first_token_ms INTEGER,
        duration_ms INTEGER, status_code INTEGER NOT NULL, error_message TEXT, session_id TEXT,
        provider_type TEXT, is_streaming INTEGER NOT NULL DEFAULT 0,
        cost_multiplier TEXT NOT NULL DEFAULT '1.0', created_at INTEGER NOT NULL,
        data_source TEXT NOT NULL DEFAULT 'proxy'
    );
    CREATE INDEX idx_request_logs_session ON proxy_request_logs(session_id);
    CREATE TABLE usage_daily_rollups (
        date TEXT NOT NULL, app_type TEXT NOT NULL, provider_id TEXT NOT NULL, model TEXT NOT NULL,
        request_count INTEGER NOT NULL DEFAULT 0, success_count INTEGER NOT NULL DEFAULT 0,
        input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
        cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
        total_cost_usd TEXT NOT NULL DEFAULT '0', avg_latency_ms INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (date, app_type, provider_id, model)
    );
"#;

/// v10 rows as v10 wrote them: Codex/Gemini input still counts the cached tokens.
fn seed_v10_usage_rows(conn: &Connection) {
    // (request_id, app_type, data_source, input, cache_read)
    let rows = [
        ("codex-session", "codex", "codex_session", 1000, 800),
        ("gemini-session", "gemini", "gemini_session", 100, 300),
        ("codex-proxy", "codex", "proxy", 500, 100),
        ("gemini-proxy", "gemini", "proxy", 900, 600),
        ("claude-proxy", "claude", "proxy", 3, 5000),
        ("claude-session", "claude", "session_log", 3, 5000),
    ];
    for (id, app, source, input, cache_read) in rows {
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                input_tokens, output_tokens, cache_read_tokens, latency_ms, status_code,
                created_at, data_source)
             VALUES (?1, 'p', ?2, 'm', ?3, 10, ?4, 0, 200, ?5, ?6)",
            // recent, so the startup prune leaves the rows in place
            params![
                id,
                app,
                input,
                cache_read,
                chrono::Utc::now().timestamp(),
                source
            ],
        )
        .expect("seed v10 log row");
    }
    for (app, input, cache_read) in [
        ("codex", 5000, 4000),
        ("gemini", 10, 50),
        ("claude", 7, 9000),
    ] {
        conn.execute(
            "INSERT INTO usage_daily_rollups (date, app_type, provider_id, model,
                request_count, input_tokens, cache_read_tokens)
             VALUES ('2026-01-01', ?1, 'p', 'm', 1, ?2, ?3)",
            params![app, input, cache_read],
        )
        .expect("seed v10 rollup row");
    }
}

fn input_tokens_of(conn: &Connection, request_id: &str) -> i64 {
    conn.query_row(
        "SELECT input_tokens FROM proxy_request_logs WHERE request_id = ?1",
        [request_id],
        |row| row.get(0),
    )
    .expect("read input tokens")
}

fn rollup_input_tokens_of(conn: &Connection, app_type: &str) -> i64 {
    conn.query_row(
        "SELECT input_tokens FROM usage_daily_rollups WHERE app_type = ?1",
        [app_type],
        |row| row.get(0),
    )
    .expect("read rollup input tokens")
}

#[test]
fn schema_migration_v10_to_v11_stores_fresh_input_and_adds_usage_tables() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(V10_USAGE_SCHEMA_SQL)
        .expect("v10 schema");
    seed_v10_usage_rows(&conn);
    Database::set_user_version(&conn, 10).expect("set user_version=10");

    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    assert_eq!(Database::get_user_version(&conn).unwrap(), 11);
    // Codex/Gemini (session logs and proxy rows) lose the cached part, clamped at 0
    assert_eq!(input_tokens_of(&conn, "codex-session"), 200);
    assert_eq!(input_tokens_of(&conn, "gemini-session"), 0);
    assert_eq!(input_tokens_of(&conn, "codex-proxy"), 400);
    assert_eq!(input_tokens_of(&conn, "gemini-proxy"), 300);
    // Claude already reported fresh input
    assert_eq!(input_tokens_of(&conn, "claude-proxy"), 3);
    assert_eq!(input_tokens_of(&conn, "claude-session"), 3);
    assert_eq!(rollup_input_tokens_of(&conn, "codex"), 1000);
    assert_eq!(rollup_input_tokens_of(&conn, "gemini"), 0);
    assert_eq!(rollup_input_tokens_of(&conn, "claude"), 7);

    for table in [
        "session_usage_archive",
        "session_meta_cache",
        "balance_samples",
        "maintenance_flags",
    ] {
        assert!(
            Database::table_exists(&conn, table).unwrap(),
            "{table} should exist"
        );
    }
    for (key, value) in [
        ("recompute_claude_costs", "pending"),
        ("backfill_session_archive", "pending"),
    ] {
        let stored: String = conn
            .query_row(
                "SELECT value FROM maintenance_flags WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .expect("flag row");
        assert_eq!(stored, value, "{key}");
    }
    let index: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'idx_request_logs_app_session'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(index, 1);
    let archive_cols: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('session_usage_archive')")
            .unwrap();
        let cols = stmt.query_map([], |row| row.get(0)).unwrap();
        cols.map(|c| c.unwrap()).collect()
    };
    assert_eq!(
        archive_cols,
        [
            "app_type",
            "session_id",
            "model",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_creation_tokens",
            "request_count",
            "cost_usd",
            "first_at",
            "last_at"
        ]
    );
}

#[test]
fn schema_migration_v10_to_v11_rolls_back_whole_when_a_step_fails() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(V10_USAGE_SCHEMA_SQL)
        .expect("v10 schema");
    seed_v10_usage_rows(&conn);
    // a conflicting object makes the migration's index creation fail
    conn.execute_batch("CREATE TABLE idx_request_logs_app_session (x INTEGER);")
        .expect("conflicting name");
    Database::set_user_version(&conn, 10).expect("set user_version=10");

    assert!(Database::apply_schema_migrations_on_conn(&conn).is_err());

    assert_eq!(Database::get_user_version(&conn).unwrap(), 10);
    assert_eq!(input_tokens_of(&conn, "codex-session"), 1000);
    assert!(!Database::table_exists(&conn, "maintenance_flags").unwrap());
}

mod v10_upgrade {
    use super::*;
    use serial_test::serial;

    /// Calls of `session` counted by live rows plus the archive.
    fn session_calls(db: &Database, session: &str) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT (SELECT COUNT(*) FROM proxy_request_logs WHERE session_id = ?1)
                  + (SELECT COALESCE(SUM(request_count), 0) FROM session_usage_archive
                     WHERE session_id = ?1)",
            [session],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    #[serial]
    fn restoring_the_pre_migration_backup_counts_each_call_once_and_reprices_claude() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db_path = get_app_config_dir().join("ofox-desktop.db");
        std::fs::create_dir_all(get_app_config_dir()).unwrap();
        {
            let conn = Connection::open(&db_path).expect("create v10 db");
            conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")
                .unwrap();
            conn.execute_batch(V10_USAGE_SCHEMA_SQL)
                .expect("v10 schema");
            let now = chrono::Utc::now().timestamp();
            // v10 priced Claude input as (3 - 5000 -> 0): no input cost
            for (id, created_at) in [("claude-old", now - 40 * 86400), ("claude-new", now)] {
                conn.execute(
                    "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                        input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                        input_cost_usd, total_cost_usd, latency_ms, status_code, session_id,
                        created_at, data_source)
                     VALUES (?1, '_session', 'claude', 'claude-opus-4-7', 3, 150, 5000, 10000,
                        '0', '0.06', 0, 200, 's-old', ?2, 'session_log')",
                    params![id, created_at],
                )
                .unwrap();
            }
            Database::set_user_version(&conn, 10).unwrap();
        }
        // upgrade: backup, migrate, prune (claude-old moves into the archive)
        let db = Database::init().expect("init upgrades v10");
        let backups: Vec<String> = std::fs::read_dir(get_app_config_dir().join("backups"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        crate::services::usage_maintenance::run_pending(&db).unwrap();
        assert_eq!(session_calls(&db, "s-old"), 2);

        db.restore_from_backup(&backups[0])
            .expect("restore v10 backup");

        // claude-old is live again; the archive must not still hold it
        assert_eq!(session_calls(&db, "s-old"), 2);
        crate::services::usage_maintenance::run_pending(&db).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let input_cost: String = conn
                .query_row(
                    "SELECT input_cost_usd FROM proxy_request_logs WHERE request_id = 'claude-old'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                input_cost, "0.000015",
                "restored Claude rows are priced again"
            );
        }
        db.rollup_and_prune(30).unwrap();
        assert_eq!(session_calls(&db, "s-old"), 2);
    }

    #[test]
    #[serial]
    fn init_backs_up_a_v10_database_before_normalizing_it() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db_path = get_app_config_dir().join("ofox-desktop.db");
        std::fs::create_dir_all(get_app_config_dir()).unwrap();
        {
            let conn = Connection::open(&db_path).expect("create v10 db");
            // as v10 left it: incremental auto-vacuum already on, so init's only
            // backup is the pre-migration one
            conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")
                .unwrap();
            conn.execute_batch(V10_USAGE_SCHEMA_SQL)
                .expect("v10 schema");
            seed_v10_usage_rows(&conn);
            Database::set_user_version(&conn, 10).unwrap();
        }

        let db = Database::init().expect("init upgrades v10");

        {
            let conn = db.conn.lock().unwrap();
            assert_eq!(Database::get_user_version(&conn).unwrap(), 11);
            assert_eq!(input_tokens_of(&conn, "codex-session"), 200);
        }
        let backups: Vec<_> = std::fs::read_dir(get_app_config_dir().join("backups"))
            .expect("a backups dir")
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        let backup = Connection::open(&backups[0]).unwrap();
        assert_eq!(Database::get_user_version(&backup).unwrap(), 10);
        assert_eq!(input_tokens_of(&backup, "codex-session"), 1000);
    }
}

#[cfg(unix)]
mod private_files {
    use super::*;
    use crate::config::{file_mode, set_file_mode};
    use serial_test::serial;
    use std::path::PathBuf;

    fn db_path() -> PathBuf {
        get_app_config_dir().join("ofox-desktop.db")
    }

    #[test]
    #[serial]
    fn init_creates_the_app_dir_0700_and_the_database_0600() {
        let _sandbox = crate::test_sandbox::sandbox();
        assert!(!get_app_config_dir().exists(), "the sandbox home is fresh");

        let _db = Database::init().expect("init");

        assert_eq!(file_mode(&get_app_config_dir()), Some(0o700));
        assert_eq!(file_mode(&db_path()), Some(0o600));
    }

    #[test]
    #[serial]
    fn init_leaves_every_sqlite_file_it_made_owner_only() {
        let _sandbox = crate::test_sandbox::sandbox();

        let _db = Database::init().expect("init");

        for suffix in ["", "-journal", "-wal", "-shm"] {
            let mut name = db_path().into_os_string();
            name.push(suffix);
            let path = PathBuf::from(name);
            if path.exists() {
                assert_eq!(file_mode(&path), Some(0o600), "{}", path.display());
            }
        }
    }

    #[test]
    #[serial]
    fn init_tightens_a_database_an_older_version_left_world_readable() {
        let _sandbox = crate::test_sandbox::sandbox();
        drop(Database::init().expect("first init"));
        set_file_mode(&db_path(), 0o644).unwrap();

        let _db = Database::init().expect("second init");

        assert_eq!(file_mode(&db_path()), Some(0o600));
    }

    #[test]
    #[serial]
    fn init_keeps_an_existing_app_dir_as_the_user_has_it() {
        let _sandbox = crate::test_sandbox::sandbox();
        let dir = get_app_config_dir();
        std::fs::create_dir_all(&dir).unwrap();
        set_file_mode(&dir, 0o755).unwrap();

        let _db = Database::init().expect("init");

        assert_eq!(file_mode(&dir), Some(0o755));
        assert_eq!(file_mode(&db_path()), Some(0o600));
    }

    #[test]
    fn restricting_the_database_files_covers_the_journal_wal_and_shm() {
        let dir = tempfile::tempdir().expect("tempdir");
        let main = dir.path().join("ofox-desktop.db");
        let present = ["", "-journal", "-wal", "-shm"].map(|suffix| {
            let mut name = main.clone().into_os_string();
            name.push(suffix);
            PathBuf::from(name)
        });
        for path in &present {
            std::fs::write(path, b"x").unwrap();
            set_file_mode(path, 0o644).unwrap();
        }
        let unrelated = dir.path().join("config.json");
        std::fs::write(&unrelated, b"{}").unwrap();
        set_file_mode(&unrelated, 0o644).unwrap();

        restrict_database_files(&main);

        for path in &present {
            assert_eq!(file_mode(path), Some(0o600), "{}", path.display());
        }
        assert_eq!(file_mode(&unrelated), Some(0o644));
    }

    #[test]
    fn restricting_the_database_files_does_not_create_missing_ones() {
        let dir = tempfile::tempdir().expect("tempdir");
        let main = dir.path().join("ofox-desktop.db");
        std::fs::write(&main, b"x").unwrap();

        restrict_database_files(&main);

        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "only the main file exists"
        );
    }

    #[test]
    #[serial]
    fn a_backup_lands_in_a_0700_dir_as_a_0600_file() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::init().expect("init");

        let backup = db.backup_database_file().unwrap().expect("a backup");

        let backup_dir = get_app_config_dir().join("backups");
        assert_eq!(file_mode(&backup_dir), Some(0o700));
        assert_eq!(file_mode(&backup), Some(0o600));
    }

    #[test]
    #[serial]
    fn a_backup_tightens_a_backups_dir_an_older_version_made_0755() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::init().expect("init");
        let backup_dir = get_app_config_dir().join("backups");
        std::fs::create_dir_all(&backup_dir).unwrap();
        set_file_mode(&backup_dir, 0o755).unwrap();

        db.backup_database_file().unwrap().expect("a backup");

        assert_eq!(file_mode(&backup_dir), Some(0o700));
    }
}
