//! Codex（以及共用 `~/.codex` 的 ChatGPT 桌面版 Codex 模式）的绑定配置。
//!
//! 绑定只改 `config.toml`，**从不读写 `auth.json`**：Ofox 服务商通过
//! `experimental_bearer_token` 自带 key、`requires_openai_auth = false`。
//! 旧做法是把 key 写进 auth.json 并设 `requires_openai_auth = true`；用 ChatGPT
//! 登录的用户 auth.json 里有 `auth_mode = "chatgpt"`，Codex 会优先按它取令牌，
//! 结果把 ChatGPT 的 access_token 发给了 Ofox 网关。
//!
//! 解绑只还原「接入方式」这几个字段（[`MANAGED_KEYS`] + `[model_providers.ofox]`），
//! MCP、项目、插件等其余内容一律不动。

use std::path::{Path, PathBuf};

use serde_json::Value;
use toml_edit::{DocumentMut, Item, Table, TableLike};

use crate::codex_config::{get_codex_auth_path, get_codex_config_path};
use crate::config::FileTxn;
use crate::ofox_apex::mentions_ofox_gateway;

use super::plan::{read_text, FileEdit, RestorePlan};

const OFOX_PROVIDER: &str = "ofox";
const PROVIDERS_TABLE: &str = "model_providers";
/// Ofox 管理的顶层字段：解绑时一律还原成绑定前的值（原来没有就删掉）。
const MANAGED_KEYS: &[&str] = &[
    "model_provider",
    "model",
    "model_reasoning_effort",
    "disable_response_storage",
];
/// 绑定之后归用户的字段：ChatGPT 和 Codex 会把界面里选的推理强度写回 config.toml，
/// Ofox 再写配置（比如切换模型）时保留，只在没有时写模板的默认值。
const USER_TUNED_KEYS: &[&str] = &["model_reasoning_effort"];
/// config.toml 里有 key，只给本人读写。
const BOUND_CONFIG_MODE: u32 = 0o600;

fn parse_doc(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>()
        .map_err(|e| format!("~/.codex/config.toml 不是有效的 TOML，请先修复后再操作：{e}"))
}

fn child_table<'a>(
    parent: &'a mut dyn TableLike,
    key: &str,
) -> Result<&'a mut dyn TableLike, String> {
    if parent.get(key).is_none() {
        let mut table = Table::new();
        table.set_implicit(true);
        parent.insert(key, Item::Table(table));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| format!("config.toml 里的 {key} 不是表"))
}

fn ofox_provider(doc: &DocumentMut) -> Option<&dyn TableLike> {
    doc.get(PROVIDERS_TABLE)?
        .as_table_like()?
        .get(OFOX_PROVIDER)?
        .as_table_like()
}

/// `[model_providers.ofox]` 指向 Ofox 网关——当前（或旧版本）绑定留下的配置。
pub(crate) fn is_ofox_bound(config: &str) -> bool {
    let Ok(doc) = config.parse::<DocumentMut>() else {
        return false;
    };
    ofox_provider(&doc)
        .and_then(|provider| provider.get("base_url"))
        .and_then(Item::as_str)
        .is_some_and(mentions_ofox_gateway)
}

/// 由 DB 里 `ofox-codex` 模板的 config 生成绑定补丁：Ofox 服务商自带 key，不依赖
/// auth.json。旧模板里的 `requires_openai_auth = true` 在这里一并纠正。
pub(crate) fn bound_patch(template_config: &str, api_key: &str) -> Result<DocumentMut, String> {
    let mut doc = parse_doc(template_config)?;
    let providers = child_table(doc.as_table_mut(), PROVIDERS_TABLE)?;
    let provider = child_table(providers, OFOX_PROVIDER)?;
    provider.remove("env_key");
    provider.remove("auth");
    provider.insert("requires_openai_auth", toml_edit::value(false));
    provider.insert("experimental_bearer_token", toml_edit::value(api_key));
    Ok(doc)
}

/// 把补丁合并进用户当前的 config.toml：受管字段覆盖，`[model_providers.ofox]`
/// 整表替换（用户自己写过的同名表不能和 key 混在一起），其余内容不动。
pub(crate) fn apply_patch(current: &str, patch: &DocumentMut) -> Result<String, String> {
    let mut doc = parse_doc(current)?;
    let bound = doc.get("model_provider").and_then(Item::as_str) == Some(OFOX_PROVIDER);
    for (key, item) in patch.as_table().iter() {
        if bound && USER_TUNED_KEYS.contains(&key) && doc.contains_key(key) {
            continue;
        }
        if key == PROVIDERS_TABLE {
            let patch_providers = item
                .as_table_like()
                .ok_or_else(|| "Ofox 模板里的 model_providers 不是表".to_string())?;
            let providers = child_table(doc.as_table_mut(), PROVIDERS_TABLE)?;
            for (name, provider) in patch_providers.iter() {
                providers.insert(name, provider.clone());
            }
        } else {
            doc.insert(key, item.clone());
        }
    }
    Ok(doc.to_string())
}

/// 按模板写绑定后的 config.toml（绑定和切换模型共用），不碰 auth.json。
pub(crate) fn write_bound_config(
    template_config: &str,
    api_key: &str,
    txn: &mut FileTxn,
) -> Result<(), String> {
    let path = get_codex_config_path();
    let current = read_text(&path)?.unwrap_or_default();
    let merged = apply_patch(&current, &bound_patch(template_config, api_key)?)?;
    txn.write(&path, merged.as_bytes())
        .and_then(|()| txn.set_mode(&path, BOUND_CONFIG_MODE))
        .map_err(|e| format!("写入 Codex 配置失败：{e}"))
}

pub(crate) fn validate(text: &str) -> Result<(), String> {
    parse_doc(text).map(drop)
}

fn same_toml(a: &str, b: &str) -> bool {
    match (
        toml::from_str::<toml::Table>(a),
        toml::from_str::<toml::Table>(b),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// 受管字段一律还原成绑定前的值（原来没有就删掉），即使绑定期间被改过；
/// 其余内容保持当前状态。
pub(crate) fn plan_restore(
    original: Option<&str>,
    current: Option<&str>,
) -> Result<RestorePlan, String> {
    let original_doc = parse_doc(original.unwrap_or_default())?;
    let mut doc = parse_doc(current.unwrap_or_default())?;
    let mut restored_keys = Vec::new();
    let mut removed_keys = Vec::new();

    for key in MANAGED_KEYS {
        match original_doc.get(key) {
            Some(item) => {
                doc.insert(key, item.clone());
                restored_keys.push((*key).to_string());
            }
            None => {
                if doc.remove(key).is_some() {
                    removed_keys.push((*key).to_string());
                }
            }
        }
    }

    let provider_key = format!("{PROVIDERS_TABLE}.{OFOX_PROVIDER}");
    let original_provider = original_doc
        .get(PROVIDERS_TABLE)
        .and_then(Item::as_table_like)
        .and_then(|providers| providers.get(OFOX_PROVIDER));
    match original_provider {
        Some(item) => {
            child_table(doc.as_table_mut(), PROVIDERS_TABLE)?.insert(OFOX_PROVIDER, item.clone());
            restored_keys.push(provider_key);
        }
        None => {
            let removed = doc
                .get_mut(PROVIDERS_TABLE)
                .and_then(Item::as_table_like_mut)
                .and_then(|providers| providers.remove(OFOX_PROVIDER));
            if removed.is_some() {
                removed_keys.push(provider_key);
            }
        }
    }
    // Ofox 建出来的空 [model_providers] 一并去掉。
    let providers_now_empty = doc
        .get(PROVIDERS_TABLE)
        .and_then(Item::as_table_like)
        .is_some_and(|providers| providers.is_empty());
    if original_doc.get(PROVIDERS_TABLE).is_none() && providers_now_empty {
        doc.remove(PROVIDERS_TABLE);
    }

    let restored = doc.to_string();
    let same = same_toml(&restored, original.unwrap_or_default());
    Ok(RestorePlan::finish(
        restored,
        original,
        same,
        restored_keys,
        removed_keys,
    ))
}

fn is_ofox_key(value: &str, ofox_key: Option<&str>) -> bool {
    value.starts_with("sk-of-") || ofox_key.is_some_and(|key| key == value)
}

/// 旧版本绑定会把 Ofox 的 key 写进 auth.json。去掉它：有 ChatGPT 登录信息的
/// 置为 `null`（Codex 自己就是这么存的），否则删掉字段，剩下空对象就删文件。
/// 返回 `None` 表示不需要改。
pub(crate) fn strip_ofox_key_from_auth(
    auth_text: &str,
    ofox_key: Option<&str>,
) -> Result<Option<Option<String>>, String> {
    let mut auth: Value =
        serde_json::from_str(auth_text).map_err(|e| format!("auth.json 不是有效 JSON：{e}"))?;
    let Some(object) = auth.as_object_mut() else {
        return Ok(None);
    };
    let holds_ofox_key = object
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .is_some_and(|key| is_ofox_key(key, ofox_key));
    if !holds_ofox_key {
        return Ok(None);
    }
    if object.contains_key("tokens") || object.contains_key("auth_mode") {
        object.insert("OPENAI_API_KEY".to_string(), Value::Null);
    } else {
        object.remove("OPENAI_API_KEY");
    }
    if object.is_empty() {
        return Ok(Some(None));
    }
    let text = serde_json::to_string_pretty(&auth).map_err(|e| e.to_string())?;
    Ok(Some(Some(text)))
}

fn apply_auth_edit(txn: &mut FileTxn, path: &Path, edit: Option<String>) -> Result<(), String> {
    match edit {
        Some(text) => txn.write(path, text.as_bytes()),
        None => txn.remove(path),
    }
    .map_err(|e| format!("更新 Codex auth.json 失败：{e}"))
}

/// 旧版本绑定的安装：key 在 auth.json 里、`requires_openai_auth = true`。改成新
/// 形态（key 放进 Ofox 服务商），并从 auth.json 里拿掉 Ofox 的 key。可重复执行；
/// 找不到 key 时什么都不改。返回是否做了迁移。
///
/// `stored_key` 只在确实要迁移、且 auth.json 里没有 Ofox key 时才调用——读钥匙串
/// 可能弹系统授权窗口，不能在每次启动时无条件去读。
pub(crate) fn migrate_legacy_shape(
    stored_key: impl FnOnce() -> Option<String>,
) -> Result<bool, String> {
    let config_path = get_codex_config_path();
    let Some(config) = read_text(&config_path)? else {
        return Ok(false);
    };
    if !is_ofox_bound(&config) {
        return Ok(false);
    }
    let doc = parse_doc(&config)?;
    let provider = ofox_provider(&doc).expect("is_ofox_bound checked the provider");
    let has_bearer = provider
        .get("experimental_bearer_token")
        .and_then(Item::as_str)
        .is_some_and(|token| !token.is_empty());
    let needs_openai_auth =
        provider.get("requires_openai_auth").and_then(Item::as_bool) != Some(false);
    if has_bearer && !needs_openai_auth {
        return Ok(false);
    }

    let auth_path = get_codex_auth_path();
    let auth_text = read_text(&auth_path)?;
    let auth_key = auth_text
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|auth| auth.get("OPENAI_API_KEY")?.as_str().map(str::to_string))
        .filter(|key| key.starts_with("sk-of-"));
    let key = auth_key.or_else(stored_key);
    if key.is_none() && !needs_openai_auth {
        // 泄露已经堵住，只是还没有 key；等下次绑定补上。
        return Ok(false);
    }

    let mut migrated = parse_doc(&config)?;
    let providers = child_table(migrated.as_table_mut(), PROVIDERS_TABLE)?;
    let provider = child_table(providers, OFOX_PROVIDER)?;
    provider.insert("requires_openai_auth", toml_edit::value(false));
    if let Some(key) = key.as_deref() {
        provider.insert("experimental_bearer_token", toml_edit::value(key));
    }

    let mut txn = FileTxn::new();
    let result = (|| {
        txn.write(&config_path, migrated.to_string().as_bytes())
            .and_then(|()| txn.set_mode(&config_path, BOUND_CONFIG_MODE))
            .map_err(|e| format!("迁移 Codex 配置失败：{e}"))?;
        if let (Some(text), Some(key)) = (auth_text.as_deref(), key.as_deref()) {
            if let Some(edit) = strip_ofox_key_from_auth(text, Some(key))? {
                apply_auth_edit(&mut txn, &auth_path, edit)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        return Err(super::rollback_with(txn, error));
    }
    match key {
        Some(_) => log::info!(
            "[ofox_bind] 已把旧版 Codex 绑定迁移为服务商自带 key，auth.json 不再存 Ofox key"
        ),
        // 没有 key 也要先堵住泄露：Ofox 服务商不再用 ChatGPT 的登录信息。
        None => log::warn!(
            "[ofox_bind] Codex 旧版绑定缺少可用的 Ofox key：已关闭 requires_openai_auth，重新绑定后恢复可用"
        ),
    }
    Ok(true)
}

/// 没有绑定前快照时的尽力清理：去掉 Ofox 写进 config.toml 的接入字段，以及旧版本
/// 留在 auth.json 里的 Ofox key，回到 Codex 默认登录。
pub(crate) fn legacy_edits() -> Result<Vec<FileEdit>, String> {
    let mut edits = Vec::new();
    let config_path = get_codex_config_path();
    if let Some(config) = read_text(&config_path)? {
        if let (Some(content), removed_keys) = legacy_cleanup_config(&config)? {
            edits.push(FileEdit {
                path: config_path,
                content: Some(content),
                restored_keys: Vec::new(),
                removed_keys,
            });
        }
    }
    let auth_path = get_codex_auth_path();
    if let Some(auth) = read_text(&auth_path)? {
        if let Some(content) = strip_ofox_key_from_auth(&auth, None)? {
            edits.push(FileEdit {
                path: auth_path,
                content,
                restored_keys: Vec::new(),
                removed_keys: vec!["OPENAI_API_KEY".to_string()],
            });
        }
    }
    Ok(edits)
}

/// 返回新的 config 内容（`None` 表示无需改动）和被删掉的字段。
fn legacy_cleanup_config(config: &str) -> Result<(Option<String>, Vec<String>), String> {
    let mut doc = parse_doc(config)?;
    let mut removed = Vec::new();
    let provider_is_ofox = is_ofox_bound(config);
    if provider_is_ofox {
        if let Some(providers) = doc
            .get_mut(PROVIDERS_TABLE)
            .and_then(Item::as_table_like_mut)
        {
            providers.remove(OFOX_PROVIDER);
            removed.push(format!("{PROVIDERS_TABLE}.{OFOX_PROVIDER}"));
            if providers.is_empty() {
                doc.remove(PROVIDERS_TABLE);
            }
        }
    }
    let selected_ofox = doc.get("model_provider").and_then(Item::as_str) == Some(OFOX_PROVIDER);
    if selected_ofox {
        doc.remove("model_provider");
        removed.push("model_provider".to_string());
    }
    if selected_ofox || provider_is_ofox {
        // 模型名、推理强度、响应存储都是 Ofox 绑定时写的；原值已无从得知，交给 Codex 默认。
        if doc.remove("model").is_some() {
            removed.push("model".to_string());
        }
        if doc.get("model_reasoning_effort").and_then(Item::as_str) == Some("high") {
            doc.remove("model_reasoning_effort");
            removed.push("model_reasoning_effort".to_string());
        }
        if doc.get("disable_response_storage").and_then(Item::as_bool) == Some(true) {
            doc.remove("disable_response_storage");
            removed.push("disable_response_storage".to_string());
        }
    }
    if removed.is_empty() {
        return Ok((None, removed));
    }
    Ok((Some(doc.to_string()), removed))
}

pub(crate) fn config_path() -> PathBuf {
    get_codex_config_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = "model_provider = \"ofox\"\nmodel = \"openai/gpt-6-luna\"\nmodel_reasoning_effort = \"high\"\ndisable_response_storage = true\n\n[model_providers.ofox]\nname = \"ofox\"\nbase_url = \"https://api.ofox.ai/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n";

    const USER_CONFIG: &str = "# my codex setup\nmodel = \"gpt-5-codex\"\nmodel_reasoning_effort = \"medium\"\n\n[projects.\"/work\"]\ntrust_level = \"trusted\"\n\n[mcp_servers.docs]\ncommand = \"docs-mcp\"\n";

    fn bound(current: &str) -> String {
        apply_patch(current, &bound_patch(TEMPLATE, "sk-of-TEST").unwrap()).unwrap()
    }

    #[test]
    fn bound_provider_carries_its_own_key_and_never_needs_openai_auth() {
        let text = bound(USER_CONFIG);
        let doc: toml::Table = toml::from_str(&text).unwrap();
        let provider = &doc["model_providers"]["ofox"];
        assert_eq!(
            provider["experimental_bearer_token"].as_str(),
            Some("sk-of-TEST")
        );
        assert_eq!(provider["requires_openai_auth"].as_bool(), Some(false));
        assert_eq!(doc["model_provider"].as_str(), Some("ofox"));
        assert_eq!(
            doc["mcp_servers"]["docs"]["command"].as_str(),
            Some("docs-mcp")
        );
        assert_eq!(
            doc["projects"]["/work"]["trust_level"].as_str(),
            Some("trusted")
        );
    }

    #[test]
    fn binding_replaces_a_user_defined_ofox_provider_wholesale() {
        let current = "[model_providers.ofox]\nbase_url = \"https://old\"\nenv_key = \"MY_KEY\"\n";
        let doc: toml::Table = toml::from_str(&bound(current)).unwrap();
        assert!(doc["model_providers"]["ofox"].get("env_key").is_none());
    }

    #[test]
    fn restore_returns_the_original_bytes_after_bind_and_model_change() {
        let mut current = bound(USER_CONFIG);
        current = current.replace("openai/gpt-6-luna", "anthropic/claude-x");
        let plan = plan_restore(Some(USER_CONFIG), Some(&current)).unwrap();
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(USER_CONFIG));
    }

    #[test]
    fn restore_reverts_managed_keys_even_if_edited_and_keeps_new_mcp() {
        let mut current = bound(USER_CONFIG);
        current = current.replace(
            "model_reasoning_effort = \"high\"",
            "model_reasoning_effort = \"low\"",
        );
        current.push_str("\n[mcp_servers.added_while_bound]\ncommand = \"x\"\n");
        let plan = plan_restore(Some(USER_CONFIG), Some(&current)).unwrap();
        assert!(!plan.exact);
        let doc: toml::Table = toml::from_str(plan.content.as_deref().unwrap()).unwrap();
        assert_eq!(doc["model"].as_str(), Some("gpt-5-codex"));
        assert_eq!(doc["model_reasoning_effort"].as_str(), Some("medium"));
        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model_providers").is_none());
        assert!(doc.get("disable_response_storage").is_none());
        assert_eq!(
            doc["mcp_servers"]["added_while_bound"]["command"].as_str(),
            Some("x")
        );
    }

    #[test]
    fn restore_deletes_a_config_that_bind_created() {
        let plan = plan_restore(None, Some(&bound(""))).unwrap();
        assert_eq!(plan.content, None);
        assert!(plan.exact);
    }

    #[test]
    fn restore_puts_back_a_pre_existing_ofox_provider() {
        let original = "[model_providers.ofox]\nbase_url = \"https://my-relay/v1\"\n";
        let plan = plan_restore(Some(original), Some(&bound(original))).unwrap();
        assert_eq!(plan.content.as_deref(), Some(original));
    }

    #[test]
    fn auth_strip_nulls_key_for_chatgpt_login_and_removes_otherwise() {
        let oauth =
            r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":"sk-of-OLD","tokens":{"access_token":"a"}}"#;
        let edited = strip_ofox_key_from_auth(oauth, None)
            .unwrap()
            .unwrap()
            .unwrap();
        let value: Value = serde_json::from_str(&edited).unwrap();
        assert!(value["OPENAI_API_KEY"].is_null());
        assert_eq!(value["tokens"]["access_token"], "a");

        let only_key = r#"{"OPENAI_API_KEY":"sk-of-OLD"}"#;
        assert_eq!(
            strip_ofox_key_from_auth(only_key, None).unwrap(),
            Some(None)
        );

        let user_key = r#"{"OPENAI_API_KEY":"sk-proj-mine"}"#;
        assert_eq!(strip_ofox_key_from_auth(user_key, None).unwrap(), None);
    }

    #[test]
    fn legacy_cleanup_removes_ofox_connection_fields_only() {
        let legacy = bound(USER_CONFIG);
        let (cleaned, removed) = legacy_cleanup_config(&legacy).unwrap();
        let doc: toml::Table = toml::from_str(&cleaned.unwrap()).unwrap();
        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model").is_none());
        assert!(doc.get("model_providers").is_none());
        assert_eq!(
            doc["mcp_servers"]["docs"]["command"].as_str(),
            Some("docs-mcp")
        );
        assert!(removed.contains(&"model_provider".to_string()));
        assert_eq!(legacy_cleanup_config(USER_CONFIG).unwrap(), (None, vec![]));
    }

    #[test]
    fn ofox_fingerprint_requires_a_gateway_url() {
        assert!(is_ofox_bound(&bound("")));
        assert!(!is_ofox_bound(
            "[model_providers.ofox]\nbase_url = \"https://my-relay/v1\"\n"
        ));
        assert!(!is_ofox_bound(USER_CONFIG));
    }
}
