//! OfoxAI 种子数据集成测试
//!
//! 验证 `init_default_ofox_providers` 正确写入 6 条种子、幂等性、
//! 以及 `is_builtin_seed_id` / `has_non_official_seed_provider` 的行为。

use cc_switch_lib::AppType;

#[path = "support.rs"]
mod support;
use support::{create_test_state, ensure_test_home, reset_test_fs, test_mutex};

// --------------------------------------------------------------------------
// 辅助：所有 6 个 AppType
// --------------------------------------------------------------------------
const ALL_APP_TYPES: &[AppType] = &[
    AppType::Claude,
    AppType::Codex,
    AppType::Gemini,
    AppType::OpenCode,
    AppType::OpenClaw,
    AppType::Hermes,
];

const OFOX_SEED_IDS: &[&str] = &[
    "ofox-claude",
    "ofox-codex",
    "ofox-gemini",
    "ofox-opencode",
    "ofox-openclaw",
    "ofox-hermes",
];

// --------------------------------------------------------------------------
// 测试：种子写入 6 条记录
// --------------------------------------------------------------------------
#[test]
fn init_ofox_providers_inserts_six_seeds() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");
    let count = state
        .db
        .init_default_ofox_providers()
        .expect("init ofox providers");

    assert_eq!(count, 6, "should insert exactly 6 OfoxAI seeds");

    // 验证每个 app type 都能查到对应的 OfoxAI provider
    for (app_type, expected_id) in ALL_APP_TYPES.iter().zip(OFOX_SEED_IDS.iter()) {
        let provider = state
            .db
            .get_provider_by_id(expected_id, app_type.as_str())
            .expect("query provider")
            .unwrap_or_else(|| {
                panic!(
                    "OfoxAI seed '{}' should exist for {}",
                    expected_id,
                    app_type.as_str()
                )
            });

        assert_eq!(provider.name, "OfoxAI");
        assert_eq!(provider.icon.as_deref(), Some("ofox"));
        assert_eq!(provider.icon_color.as_deref(), Some("#D97706"));
        assert_eq!(provider.category.as_deref(), Some("aggregator"));

        // meta 中 providerType 应为 "ofox"
        let meta = provider.meta.as_ref().expect("meta should be present");
        assert_eq!(
            meta.provider_type.as_deref(),
            Some("ofox"),
            "providerType should be 'ofox' for {}",
            expected_id
        );
    }
}

// --------------------------------------------------------------------------
// 测试：幂等性——调用两次只写入一次
// --------------------------------------------------------------------------
#[test]
fn init_ofox_providers_is_idempotent() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    let first = state.db.init_default_ofox_providers().expect("first init");
    assert_eq!(first, 6);

    let second = state.db.init_default_ofox_providers().expect("second init");
    assert_eq!(second, 0, "second call should be a no-op (flag is set)");

    // 确认仍然只有 6 条
    for (app_type, expected_id) in ALL_APP_TYPES.iter().zip(OFOX_SEED_IDS.iter()) {
        assert!(
            state
                .db
                .get_provider_by_id(expected_id, app_type.as_str())
                .expect("query")
                .is_some(),
            "seed {} should still exist",
            expected_id
        );
    }
}

// --------------------------------------------------------------------------
// 测试：has_non_official_seed_provider 在只有内置种子时返回 false
// --------------------------------------------------------------------------
#[test]
fn has_non_official_seed_returns_false_with_only_seeds() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    // 写入官方种子 + OfoxAI 种子
    state
        .db
        .init_default_official_providers()
        .expect("init official");
    state.db.init_default_ofox_providers().expect("init ofox");

    // 对 Claude/Codex/Gemini（有官方+OfoxAI 种子），应返回 false
    for app_type in &[AppType::Claude, AppType::Codex, AppType::Gemini] {
        assert!(
            !state
                .db
                .has_non_official_seed_provider(app_type.as_str())
                .expect("check"),
            "{} should have no non-seed providers",
            app_type.as_str()
        );
    }

    // 对 OpenCode/OpenClaw/Hermes（只有 OfoxAI 种子），也应返回 false
    for app_type in &[AppType::OpenCode, AppType::OpenClaw, AppType::Hermes] {
        assert!(
            !state
                .db
                .has_non_official_seed_provider(app_type.as_str())
                .expect("check"),
            "{} should have no non-seed providers",
            app_type.as_str()
        );
    }
}

// --------------------------------------------------------------------------
// 测试：有用户创建的供应商时 has_non_official_seed_provider 返回 true
// --------------------------------------------------------------------------
#[test]
fn has_non_official_seed_returns_true_with_user_provider() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    state
        .db
        .init_default_official_providers()
        .expect("init official");
    state.db.init_default_ofox_providers().expect("init ofox");

    // 手动添加一个用户创建的供应商
    let user_provider = cc_switch_lib::Provider::with_id(
        "my-custom-provider".to_string(),
        "My Custom".to_string(),
        serde_json::json!({"env": {"ANTHROPIC_BASE_URL": "https://example.com"}}),
        None,
    );
    state
        .db
        .save_provider(AppType::Claude.as_str(), &user_provider)
        .expect("save user provider");

    assert!(
        state
            .db
            .has_non_official_seed_provider(AppType::Claude.as_str())
            .expect("check"),
        "Claude should now have a non-seed provider"
    );

    // 其他 app type 仍然只有种子
    assert!(
        !state
            .db
            .has_non_official_seed_provider(AppType::Codex.as_str())
            .expect("check"),
        "Codex should still have no non-seed providers"
    );
}

// --------------------------------------------------------------------------
// 测试：种子的 settings_config_json 可解析为有效 JSON
// --------------------------------------------------------------------------
#[test]
fn ofox_seed_settings_config_is_valid_json() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");
    state.db.init_default_ofox_providers().expect("init ofox");

    for (app_type, expected_id) in ALL_APP_TYPES.iter().zip(OFOX_SEED_IDS.iter()) {
        let provider = state
            .db
            .get_provider_by_id(expected_id, app_type.as_str())
            .expect("query")
            .unwrap_or_else(|| panic!("{} should exist", expected_id));

        // settings_config 应为非 null 的 JSON 对象
        assert!(
            provider.settings_config.is_object(),
            "settings_config for {} should be a JSON object, got: {}",
            expected_id,
            provider.settings_config
        );
    }
}
