//! Gemini CLI 的绑定配置：`.env` 里的三个接入变量（按行改，注释和其它变量原样
//! 保留）和 `settings.json` 的 `security.auth.selectedType`。`oauth_creds.json`
//! 和其它设置一律不动。

use std::path::PathBuf;

use serde_json::Value;

use crate::config::FileTxn;
use crate::gemini_config::{
    get_gemini_env_path, get_gemini_settings_path, normalize_gemini_env_model,
};
use crate::ofox_apex::mentions_ofox_gateway;

use super::env_file;
use super::json_file::{self, JsonPath};
use super::plan::{read_text, FileEdit, RestorePlan};

const API_KEY: &str = "GEMINI_API_KEY";
const BASE_URL: &str = "GOOGLE_GEMINI_BASE_URL";
const MODEL: &str = "GEMINI_MODEL";
/// 解绑时一律还原成绑定前的那一行（原来没有就删掉）。
const ENV_KEYS: &[&str] = &[API_KEY, BASE_URL, MODEL];
const SETTINGS_LABEL: &str = "Gemini 的 settings.json";
const SELECTED_TYPE: JsonPath = &["security", "auth", "selectedType"];
const SETTINGS_CONTAINERS: &[JsonPath] = &[&["security", "auth"], &["security"]];
const API_KEY_AUTH: &str = "gemini-api-key";
const GOOGLE_LOGIN_AUTH: &str = "oauth-personal";
/// `.env` 里有 key，只给本人读写。
const ENV_MODE: u32 = 0o600;

pub(crate) fn env_path() -> PathBuf {
    get_gemini_env_path()
}

pub(crate) fn settings_path() -> PathBuf {
    get_gemini_settings_path()
}

pub(crate) fn validate_settings(text: &str) -> Result<(), String> {
    json_file::parse_object(Some(text), SETTINGS_LABEL).map(drop)
}

/// `.env` 的 `GOOGLE_GEMINI_BASE_URL` 指向 Ofox 网关——当前（或旧版本）绑定留下的配置。
pub(crate) fn is_ofox_bound(env: &str) -> bool {
    env_file::get_value(env, BASE_URL).is_some_and(|url| mentions_ofox_gateway(&url))
}

fn template_env<'a>(template: &'a Value, key: &str) -> Option<&'a str> {
    template
        .get("env")?
        .get(key)?
        .as_str()
        .filter(|value| !value.trim().is_empty())
}

/// 按 DB 里 `ofox-gemini` 模板的 settings_config 生成绑定后的 `.env`。
pub(crate) fn bound_env(current: &str, template: &Value, api_key: &str) -> Result<String, String> {
    let base_url = template_env(template, BASE_URL)
        .ok_or_else(|| "Ofox Gemini 模板缺少 GOOGLE_GEMINI_BASE_URL".to_string())?;
    let model = template_env(template, MODEL).map(normalize_gemini_env_model);
    let env = env_file::set_value(current, API_KEY, Some(api_key));
    let env = env_file::set_value(&env, BASE_URL, Some(base_url));
    Ok(env_file::set_value(&env, MODEL, model.as_deref()))
}

/// 让 Gemini CLI 用 `.env` 里的 key 登录。已经是 API key 登录时返回 `None`，
/// 不重写文件（保留用户的注释和格式）。
pub(crate) fn bound_settings(current: Option<&str>) -> Result<Option<String>, String> {
    let mut value = json_file::parse_object(current, SETTINGS_LABEL)?;
    if json_file::get(&value, SELECTED_TYPE).and_then(Value::as_str) == Some(API_KEY_AUTH) {
        return Ok(None);
    }
    json_file::set(&mut value, SELECTED_TYPE, API_KEY_AUTH.into());
    serde_json::to_string_pretty(&value)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// 绑定和切换模型共用。两个文件都算好再写，任何一个解析失败都不动磁盘。
pub(crate) fn write_bound(
    template: &Value,
    api_key: &str,
    txn: &mut FileTxn,
) -> Result<(), String> {
    let (env_path, settings_path) = (env_path(), settings_path());
    let env = bound_env(
        &read_text(&env_path)?.unwrap_or_default(),
        template,
        api_key,
    )?;
    let settings = bound_settings(read_text(&settings_path)?.as_deref())?;
    txn.write(&env_path, env.as_bytes())
        .and_then(|()| txn.set_mode(&env_path, ENV_MODE))
        .and_then(|()| match settings {
            Some(text) => txn.write(&settings_path, text.as_bytes()),
            None => Ok(()),
        })
        .map_err(|e| format!("写入 Gemini 配置失败：{e}"))
}

pub(crate) fn plan_restore_env(original: Option<&str>, current: Option<&str>) -> RestorePlan {
    env_file::plan_restore(original, current, ENV_KEYS)
}

pub(crate) fn plan_restore_settings(
    original: Option<&str>,
    current: Option<&str>,
) -> Result<RestorePlan, String> {
    json_file::plan_restore(
        original,
        current,
        SETTINGS_LABEL,
        &[SELECTED_TYPE],
        SETTINGS_CONTAINERS,
    )
}

/// 没有绑定前快照时的尽力清理：去掉 Ofox 的地址、key 和 Ofox 的模型名。用户
/// 自己还有 key 就保持 API key 登录，否则回到 Google 账号登录。
pub(crate) fn legacy_edits() -> Result<Vec<FileEdit>, String> {
    let env_path = env_path();
    let Some(env) = read_text(&env_path)? else {
        return Ok(Vec::new());
    };
    let points_to_ofox = is_ofox_bound(&env);
    let ofox_key = env_file::get_value(&env, API_KEY).is_some_and(|key| key.starts_with("sk-of-"));
    let mut doomed = Vec::new();
    if points_to_ofox {
        doomed.extend([BASE_URL, MODEL]);
    }
    // 只删 Ofox 的 key；用户自己的 Google key 留着。
    if ofox_key {
        doomed.push(API_KEY);
    }
    let mut cleaned = env.clone();
    let mut removed_keys = Vec::new();
    for key in doomed {
        if env_file::find_line(&cleaned, key).is_some() {
            cleaned = env_file::set_line(&cleaned, key, None);
            removed_keys.push(key.to_string());
        }
    }
    if removed_keys.is_empty() {
        return Ok(Vec::new());
    }

    let own_key_left = env_file::get_value(&cleaned, API_KEY).is_some_and(|key| !key.is_empty());
    let mut edits = vec![FileEdit {
        path: env_path,
        content: (!cleaned.trim().is_empty()).then_some(cleaned),
        restored_keys: Vec::new(),
        removed_keys,
    }];
    let settings_path = settings_path();
    if let (false, Some(text)) = (own_key_left, read_text(&settings_path)?) {
        let mut value = json_file::parse_object(Some(&text), SETTINGS_LABEL)?;
        if json_file::get(&value, SELECTED_TYPE).and_then(Value::as_str) == Some(API_KEY_AUTH) {
            json_file::set(&mut value, SELECTED_TYPE, GOOGLE_LOGIN_AUTH.into());
            edits.push(FileEdit {
                path: settings_path,
                content: Some(serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?),
                restored_keys: vec![SELECTED_TYPE.join(".")],
                removed_keys: Vec::new(),
            });
        }
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const USER_ENV: &str =
        "# personal key\nGEMINI_API_KEY=AIza-mine\nHTTPS_PROXY=http://proxy:8080\n";

    fn template(model: Option<&str>) -> Value {
        let mut env = json!({
            "GOOGLE_GEMINI_BASE_URL": "https://api.ofox.ai/gemini",
            "GEMINI_API_KEY": "",
        });
        if let Some(model) = model {
            env["GEMINI_MODEL"] = json!(model);
        }
        json!({ "env": env })
    }

    #[test]
    fn bind_edits_only_the_connection_lines() {
        let bound = bound_env(USER_ENV, &template(Some("google/gemini-x")), "sk-of-K").unwrap();
        assert_eq!(
            bound,
            "# personal key\nGEMINI_API_KEY=sk-of-K\nHTTPS_PROXY=http://proxy:8080\nGOOGLE_GEMINI_BASE_URL=https://api.ofox.ai/gemini\nGEMINI_MODEL=gemini-x\n"
        );
        assert!(is_ofox_bound(&bound));
        assert!(!is_ofox_bound(USER_ENV));
    }

    #[test]
    fn restore_puts_the_env_back_byte_for_byte() {
        let bound = bound_env(USER_ENV, &template(Some("gemini-x")), "sk-of-K").unwrap();
        let plan = plan_restore_env(Some(USER_ENV), Some(&bound));
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(USER_ENV));
    }

    #[test]
    fn google_login_comes_back_after_unbind() {
        let original = "{\n  \"security\": { \"auth\": { \"selectedType\": \"oauth-personal\" } },\n  \"theme\": \"dark\"\n}";
        let bound = bound_settings(Some(original)).unwrap().unwrap();
        let value: Value = serde_json::from_str(&bound).unwrap();
        assert_eq!(value["security"]["auth"]["selectedType"], API_KEY_AUTH);
        let plan = plan_restore_settings(Some(original), Some(&bound)).unwrap();
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(original));
    }

    #[test]
    fn api_key_login_settings_are_not_rewritten() {
        let original = "{\n  // keep me\n  \"security\": { \"auth\": { \"selectedType\": \"gemini-api-key\" } }\n}";
        assert_eq!(bound_settings(Some(original)).unwrap(), None);
    }

    #[test]
    fn bind_refuses_settings_it_cannot_parse() {
        assert!(bound_settings(Some("{ oops")).is_err());
        assert!(validate_settings("[1]").is_err());
    }
}
