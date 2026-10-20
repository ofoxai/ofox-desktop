//! Gemini CLI 的绑定配置：`.env` 里的 key 和地址（按行改，注释和其它变量原样
//! 保留），`settings.json` 的 `security.auth.selectedType` 和默认模型 `model.name`。
//! 模型只写 `model.name`：那是用户在 Gemini 里按 Tab「记住」时写的地方，`.env` 的
//! `GEMINI_MODEL` 优先级比它高，写在那里会盖掉用户自己记住的模型。`oauth_creds.json`
//! 和其它设置一律不动。

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;

use crate::config::FileTxn;
use crate::gemini_config::{
    get_gemini_env_path, get_gemini_settings_path, normalize_gemini_env_model,
};
use crate::ofox_apex::mentions_ofox_gateway;

use super::env_file;
use super::json_file::{self, JsonPath, KeyStyle};
use super::ownership::{self, Leaf, Leaves};
use super::plan::{read_text, FileEdit, RestorePlan};
use super::record::ManagedFile;

const API_KEY: &str = "GEMINI_API_KEY";
const BASE_URL: &str = "GOOGLE_GEMINI_BASE_URL";
/// 旧版本把模型写在这里。现在绑定时删掉这一行；解绑时还原成绑定前的那一行。
const MODEL: &str = "GEMINI_MODEL";
/// 解绑时还原成绑定前的那一行（原来没有就删掉）；用户之后改过的按规则留着。
pub(crate) const ENV_LEAVES: &[Leaf] = &[
    Leaf::route(&[API_KEY], ownership::ofox_key),
    Leaf::route(&[BASE_URL], ownership::ofox_url),
    Leaf::model(&[MODEL], &[BASE_URL]),
];
const SETTINGS_LABEL: &str = "Gemini 的 settings.json";
const SELECTED_TYPE: JsonPath = &["security", "auth", "selectedType"];
/// 登录方式：绑定后用户自己换了登录方式就留着。记住的模型跟着 `.env` 里的地址走。
pub(crate) const SETTINGS_LEAVES: &[Leaf] = &[
    Leaf::preference(SELECTED_TYPE),
    Leaf::model_in(REMEMBERED_MODEL, ManagedFile::GeminiEnv.id(), &[BASE_URL]),
];
/// Gemini 的默认模型，也是用户在 agent 里「记住」的模型。绑定、在应用里保存模型时
/// 写成 Ofox 的模型（见 [`write_bound`]）。
const REMEMBERED_MODEL: JsonPath = &["model", "name"];
const SETTINGS_CONTAINERS: &[JsonPath] = &[&["security", "auth"], &["security"], &["model"]];
const API_KEY_AUTH: &str = "gemini-api-key";
const GOOGLE_LOGIN_AUTH: &str = "oauth-personal";
/// `.env` 里有 key，只给本人读写。
pub(super) const ENV_MODE: u32 = 0o600;

pub(crate) fn env_path() -> PathBuf {
    get_gemini_env_path()
}

pub(crate) fn settings_path() -> PathBuf {
    get_gemini_settings_path()
}

pub(crate) fn validate_settings(text: &str) -> Result<(), String> {
    json_file::validate(text, SETTINGS_LABEL)
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

/// 按 DB 里 `ofox-gemini` 模板的 settings_config 生成绑定后的 `.env`。模型不写这里，
/// 已有的 `GEMINI_MODEL` 行删掉。
pub(crate) fn bound_env(current: &str, template: &Value, api_key: &str) -> Result<String, String> {
    let base_url = template_env(template, BASE_URL)
        .ok_or_else(|| "Ofox Gemini 模板缺少 GOOGLE_GEMINI_BASE_URL".to_string())?;
    let env = env_file::set_value(current, API_KEY, Some(api_key));
    let env = env_file::set_value(&env, BASE_URL, Some(base_url));
    Ok(env_file::set_value(&env, MODEL, None))
}

/// 让 Gemini CLI 用 `.env` 里的 key 登录。已经是 API key 登录时返回 `None`，不写文件。
pub(crate) fn bound_settings(current: Option<&str>) -> Result<Option<String>, String> {
    let before = json_file::parse_object(current, SETTINGS_LABEL)?;
    if json_file::get(&before, SELECTED_TYPE).and_then(Value::as_str) == Some(API_KEY_AUTH) {
        return Ok(None);
    }
    let mut value = before.clone();
    json_file::set(&mut value, SELECTED_TYPE, API_KEY_AUTH.into());
    json_file::render_changed(current.unwrap_or_default(), KeyStyle::Json, &before, &value)
        .map(Some)
}

/// 绑定和切换模型共用。两个文件都算好再写，任何一个解析失败都不动磁盘。
///
/// `select_route`：绑定、在应用里选了模型时把 `model.name` 写成模板里的模型（没有就
/// 新建）。换 key、修复时不动它：那时它可能是用户刚在 agent 里记住的模型。settings 整个
/// 不在（被删了）时也写，补回已保存的模型。
pub(crate) fn write_bound(
    template: &Value,
    api_key: &str,
    select_route: bool,
    txn: &mut FileTxn,
) -> Result<(), String> {
    let (env_path, settings_path) = (env_path(), settings_path());
    let env = bound_env(
        &read_text(&env_path)?.unwrap_or_default(),
        template,
        api_key,
    )?;
    let current_settings = read_text(&settings_path)?;
    let mut settings = bound_settings(current_settings.as_deref())?;
    if let (true, Some(model)) = (
        select_route || current_settings.is_none(),
        template_env(template, MODEL),
    ) {
        let text = settings.as_deref().or(current_settings.as_deref());
        let before = json_file::parse_object(text, SETTINGS_LABEL)?;
        let mut after = before.clone();
        json_file::set(
            &mut after,
            REMEMBERED_MODEL,
            normalize_gemini_env_model(model).into(),
        );
        if after != before {
            settings = Some(json_file::render_changed(
                text.unwrap_or_default(),
                KeyStyle::Json,
                &before,
                &after,
            )?);
        }
    }
    txn.write_with_mode(&env_path, env.as_bytes(), ENV_MODE)
        .and_then(|()| match settings {
            Some(text) => txn.write(&settings_path, text.as_bytes()),
            None => Ok(()),
        })
        .map_err(|e| format!("写入 Gemini 配置失败：{e}"))
}

/// `auto`、`pro` 这类别名由 Gemini 自己解析，不当成用户选的具体模型。
pub(crate) fn is_concrete_model(model: &str) -> bool {
    !model.is_empty()
        && !model.starts_with("auto")
        && !matches!(model, "pro" | "flash" | "flash-lite")
}

/// settings 里的默认模型（`model.name`），只认具体的模型名。
pub(crate) fn live_model(settings: &str) -> Result<Option<String>, String> {
    let value = json_file::parse_object(Some(settings), SETTINGS_LABEL)?;
    Ok(json_file::get(&value, REMEMBERED_MODEL)
        .and_then(Value::as_str)
        .map(|model| normalize_gemini_env_model(model).trim().to_string())
        .filter(|model| is_concrete_model(model)))
}

/// 旧版本把模型写在 `.env` 的 `GEMINI_MODEL`，它会盖掉用户在 Gemini 里记住的模型。
/// 去掉这一行；`model.name` 还没有具体模型时把它搬过去，保持正在用的模型不变。
/// 不用改时是 `None`；否则是新的 `.env`，以及要改时新的 settings。
pub(crate) fn migrated(
    env: &str,
    settings: Option<&str>,
) -> Result<Option<(String, Option<String>)>, String> {
    let Some(env_model) = env_file::get_value(env, MODEL) else {
        return Ok(None);
    };
    let cleaned = env_file::set_value(env, MODEL, None);
    let env_model = normalize_gemini_env_model(&env_model).trim().to_string();
    let remembered = settings.map(live_model).transpose()?.flatten();
    if remembered.is_some() || !is_concrete_model(&env_model) {
        return Ok(Some((cleaned, None)));
    }
    let before = json_file::parse_object(settings, SETTINGS_LABEL)?;
    let mut after = before.clone();
    json_file::set(&mut after, REMEMBERED_MODEL, env_model.into());
    let settings = json_file::render_changed(
        settings.unwrap_or_default(),
        KeyStyle::Json,
        &before,
        &after,
    )?;
    Ok(Some((cleaned, Some(settings))))
}

/// [`migrated`] 要写的文件、内容和权限。
pub(crate) fn migration_edits() -> Result<Vec<(PathBuf, String, Option<u32>)>, String> {
    let (env_path, settings_path) = (env_path(), settings_path());
    let Some(env) = read_text(&env_path)? else {
        return Ok(Vec::new());
    };
    let Some((env, settings)) = migrated(&env, read_text(&settings_path)?.as_deref())? else {
        return Ok(Vec::new());
    };
    // `.env` 里有 key，和绑定时一样只给自己读写。
    let mut edits = vec![(env_path, env, Some(ENV_MODE))];
    edits.extend(settings.map(|text| (settings_path, text, None)));
    Ok(edits)
}

/// `.env` 里各受管变量现在的值（按行读，没有「解析失败」）。
pub(crate) fn env_leaves(text: Option<&str>) -> Leaves {
    let text = text.unwrap_or_default();
    ENV_LEAVES
        .iter()
        .map(|leaf| {
            let value = env_file::get_value(text, &leaf.id()).map_or(Value::Null, Value::String);
            (leaf.id(), value)
        })
        .collect()
}

pub(crate) fn settings_leaves(text: Option<&str>) -> Result<Leaves, String> {
    json_file::leaves(text, SETTINGS_LABEL, SETTINGS_LEAVES)
}

pub(crate) fn plan_restore_env(
    original: Option<&str>,
    current: Option<&str>,
    keep: &BTreeSet<String>,
) -> RestorePlan {
    let keys: Vec<&str> = ENV_LEAVES.iter().map(|leaf| leaf.path[0]).collect();
    env_file::plan_restore(original, current, &keys, keep)
}

pub(crate) fn plan_restore_settings(
    original: Option<&str>,
    current: Option<&str>,
    keep: &BTreeSet<String>,
) -> Result<RestorePlan, String> {
    json_file::plan_restore(
        original,
        current,
        SETTINGS_LABEL,
        KeyStyle::Json,
        SETTINGS_LEAVES,
        SETTINGS_CONTAINERS,
        keep,
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
        let before = json_file::parse_object(Some(&text), SETTINGS_LABEL)?;
        if json_file::get(&before, SELECTED_TYPE).and_then(Value::as_str) == Some(API_KEY_AUTH) {
            let mut value = before.clone();
            json_file::set(&mut value, SELECTED_TYPE, GOOGLE_LOGIN_AUTH.into());
            edits.push(FileEdit {
                path: settings_path,
                content: Some(json_file::render_changed(
                    &text,
                    KeyStyle::Json,
                    &before,
                    &value,
                )?),
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
            "# personal key\nGEMINI_API_KEY=sk-of-K\nHTTPS_PROXY=http://proxy:8080\nGOOGLE_GEMINI_BASE_URL=https://api.ofox.ai/gemini\n"
        );
        assert!(is_ofox_bound(&bound));
        assert!(!is_ofox_bound(USER_ENV));
    }

    #[test]
    fn restore_puts_the_env_back_byte_for_byte() {
        let original = "# personal key\nGEMINI_API_KEY=AIza-mine\nGEMINI_MODEL=mine\nHTTPS_PROXY=http://proxy:8080\n";
        let bound = bound_env(original, &template(Some("gemini-x")), "sk-of-K").unwrap();
        assert!(!bound.contains("GEMINI_MODEL"));
        let plan = plan_restore_env(Some(original), Some(&bound), &BTreeSet::new());
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(original));
    }

    #[test]
    fn migration_moves_the_env_model_into_settings_unless_one_is_remembered() {
        let env = "GEMINI_API_KEY=sk-of-K\nGEMINI_MODEL=google/gemini-x\n";
        let (cleaned, settings) = migrated(env, Some("{}")).unwrap().unwrap();
        assert_eq!(cleaned, "GEMINI_API_KEY=sk-of-K\n");
        assert_eq!(
            live_model(&settings.unwrap()).unwrap().as_deref(),
            Some("gemini-x")
        );
        let remembered = "{ \"model\": { \"name\": \"gemini-mine\" } }";
        assert_eq!(
            migrated(env, Some(remembered)).unwrap(),
            Some(("GEMINI_API_KEY=sk-of-K\n".into(), None))
        );
        let alias = "{ \"model\": { \"name\": \"auto\" } }";
        assert!(migrated(env, Some(alias)).unwrap().unwrap().1.is_some());
        assert_eq!(migrated("GEMINI_API_KEY=sk-of-K\n", None).unwrap(), None);
    }

    #[test]
    fn google_login_comes_back_after_unbind() {
        let original = "{\n  \"security\": { \"auth\": { \"selectedType\": \"oauth-personal\" } },\n  \"theme\": \"dark\"\n}";
        let bound = bound_settings(Some(original)).unwrap().unwrap();
        let value: Value = serde_json::from_str(&bound).unwrap();
        assert_eq!(value["security"]["auth"]["selectedType"], API_KEY_AUTH);
        let plan = plan_restore_settings(Some(original), Some(&bound), &BTreeSet::new()).unwrap();
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
