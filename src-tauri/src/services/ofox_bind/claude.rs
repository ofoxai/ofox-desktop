//! Claude Code 的绑定配置：只改 `settings.json` 里 `env` 下的接入字段，
//! `mcpServers`、hooks、permissions、其它 env 一律不动。
//!
//! 模型写成 `env.ANTHROPIC_DEFAULT_MODEL`：它比 `/model` 记住的 `model` 优先级低，
//! 用户在 Claude Code 里换的模型下次启动还在，桌面端读回来跟着显示。

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;

use crate::config::{get_claude_settings_path, FileTxn};
use crate::ofox_apex::mentions_ofox_gateway;

use super::json_file::{self, JsonPath, KeyStyle};
use super::ownership::{self, Leaf, Leaves};
use super::plan::{read_text, FileEdit, RestorePlan};

const LABEL: &str = "Claude 的 settings.json";
const BASE_URL: JsonPath = &["env", "ANTHROPIC_BASE_URL"];
const AUTH_TOKEN: JsonPath = &["env", "ANTHROPIC_AUTH_TOKEN"];
/// 模板（DB）里存选中模型的位置。旧版本也把它写进配置文件，但它比 `/model` 记住的
/// `model` 优先，用户在 Claude Code 里换的模型下次启动就被盖回去，所以绑定时拿掉。
const MODEL: JsonPath = &["env", "ANTHROPIC_MODEL"];
/// 选中的模型写成默认值：`/model` 记住的 `model` 比它优先。
const DEFAULT_MODEL: JsonPath = &["env", "ANTHROPIC_DEFAULT_MODEL"];
/// 让 `/model` 从 Ofox 的 `/v1/models` 列出全部 Claude 模型。
const DISCOVERY: JsonPath = &["env", "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY"];
/// `/model` 里的 Opus、Sonnet、Haiku、Fable（记住后是 `opus` 这样的别名）各自对应
/// 的模型：写成 Ofox 上这一家族最新的 id，不然 Claude Code 发的是 Ofox 不认的原生 id。
const FAMILIES: &[(&str, JsonPath)] = &[
    ("opus", &["env", "ANTHROPIC_DEFAULT_OPUS_MODEL"]),
    ("sonnet", &["env", "ANTHROPIC_DEFAULT_SONNET_MODEL"]),
    ("haiku", &["env", "ANTHROPIC_DEFAULT_HAIKU_MODEL"]),
    ("fable", &["env", "ANTHROPIC_DEFAULT_FABLE_MODEL"]),
];
/// 中转站常用的 x-api-key。绑定期间拿掉，免得和 Ofox 的 key 一起发给 Ofox。
const API_KEY: JsonPath = &["env", "ANTHROPIC_API_KEY"];
/// 用户在 agent 里「记住」的模型（`/model`），比默认模型优先。绑定、在应用里保存模型
/// 时，原来有就改成 Ofox 的模型（见 [`write_bound`]），没有就不加。
const REMEMBERED_MODEL: JsonPath = &["model"];
/// 解绑时还原成绑定前的值（原来没有就删掉）；用户之后改过的按规则留着。
pub(crate) const LEAVES: &[Leaf] = &[
    Leaf::route(BASE_URL, ownership::ofox_url),
    Leaf::route(AUTH_TOKEN, ownership::ofox_key),
    Leaf::model(MODEL, BASE_URL),
    Leaf::model(DEFAULT_MODEL, BASE_URL),
    Leaf::preference(DISCOVERY),
    Leaf::model(FAMILIES[0].1, BASE_URL),
    Leaf::model(FAMILIES[1].1, BASE_URL),
    Leaf::model(FAMILIES[2].1, BASE_URL),
    Leaf::model(FAMILIES[3].1, BASE_URL),
    Leaf::route(API_KEY, ownership::ofox_key),
    Leaf::model(REMEMBERED_MODEL, BASE_URL),
];
const CONTAINERS: &[JsonPath] = &[&["env"]];

/// 家族别名 → Ofox 上这一家族版本最高的模型 id。只认 `anthropic/claude-<家族>-<版本>`
/// （版本只有数字和点），带日期、`-thinking` 之类后缀的不算。
pub(crate) fn family_models<'a>(
    ids: impl IntoIterator<Item = &'a str>,
) -> Vec<(&'static str, String)> {
    let mut newest: Vec<(&'static str, Vec<u32>, &'a str)> = Vec::new();
    for id in ids {
        let Some(rest) = id.strip_prefix("anthropic/claude-") else {
            continue;
        };
        let Some((family, version)) = rest.split_once('-') else {
            continue;
        };
        let Some(&(family, _)) = FAMILIES.iter().find(|(name, _)| *name == family) else {
            continue;
        };
        let Ok(version) = version
            .split('.')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
        else {
            continue;
        };
        match newest.iter_mut().find(|(name, _, _)| *name == family) {
            Some(entry) if entry.1 >= version => {}
            Some(entry) => *entry = (family, version, id),
            None => newest.push((family, version, id)),
        }
    }
    FAMILIES
        .iter()
        .filter_map(|(family, _)| {
            newest
                .iter()
                .find(|(name, _, _)| name == family)
                .map(|(_, _, id)| (*family, id.to_string()))
        })
        .collect()
}

/// 按最近拉到的 Ofox 模型目录算的家族映射；还没拉过就是空的。
pub(crate) fn catalog_families() -> Vec<(&'static str, String)> {
    let ids = crate::services::model_fetch::cached_ofox_anthropic_model_ids();
    family_models(ids.iter().map(String::as_str))
}

pub(crate) fn settings_path() -> PathBuf {
    get_claude_settings_path()
}

pub(crate) fn validate(text: &str) -> Result<(), String> {
    json_file::validate(text, LABEL)
}

/// `env.ANTHROPIC_BASE_URL` 指向 Ofox 网关——当前（或旧版本）绑定留下的配置。
pub(crate) fn is_ofox_bound(text: &str) -> bool {
    json_file::parse_object(Some(text), LABEL)
        .ok()
        .and_then(|value| {
            json_file::get(&value, BASE_URL)?
                .as_str()
                .map(mentions_ofox_gateway)
        })
        .unwrap_or(false)
}

fn template_str(template: &Value, path: JsonPath) -> Option<&str> {
    json_file::get(template, path)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

/// 模型相关的 env：去掉旧的 `ANTHROPIC_MODEL`，模板里的模型写成默认模型，没设过就打开
/// 模型发现，家族别名指向 Ofox 的 id（`families` 是 [`family_models`] 的结果）。目录里没有
/// 某个家族时，不是 Ofox id 的旧映射（中转站留下的）去掉，免得 Claude Code 拿它请求 Ofox。
fn apply_model_env(value: &mut Value, template: &Value, families: &[(&'static str, String)]) {
    json_file::remove(value, MODEL);
    match template_str(template, MODEL) {
        Some(model) => json_file::set(value, DEFAULT_MODEL, model.into()),
        None => {
            json_file::remove(value, DEFAULT_MODEL);
        }
    }
    // 用户自己设过（比如关掉）就留着。
    if json_file::get(value, DISCOVERY).is_none() {
        json_file::set(value, DISCOVERY, "1".into());
    }
    for (family, path) in FAMILIES {
        match families.iter().find(|(name, _)| name == family) {
            Some((_, model)) => json_file::set(value, path, model.as_str().into()),
            None => {
                if json_file::get(value, path)
                    .and_then(Value::as_str)
                    .is_some_and(|model| !model.starts_with("anthropic/"))
                {
                    json_file::remove(value, path);
                }
            }
        }
    }
}

/// 按 DB 里 `ofox-claude` 模板的 settings_config 生成绑定后的 settings.json。
/// 拿不到模型目录时 `families` 传空，Ofox 写过的别名映射不动。
pub(crate) fn bound_settings(
    current: Option<&str>,
    template: &Value,
    api_key: &str,
    families: &[(&'static str, String)],
) -> Result<String, String> {
    let base_url = template_str(template, BASE_URL)
        .ok_or_else(|| "Ofox Claude 模板缺少 ANTHROPIC_BASE_URL".to_string())?;
    let before = json_file::parse_object(current, LABEL)?;
    let mut value = before.clone();
    json_file::set(&mut value, BASE_URL, base_url.into());
    json_file::set(&mut value, AUTH_TOKEN, api_key.into());
    apply_model_env(&mut value, template, families);
    json_file::remove(&mut value, API_KEY);
    json_file::render_changed(current.unwrap_or_default(), KeyStyle::Json, &before, &value)
}

/// 绑定和切换模型共用。内容没变就不写。
///
/// `select_route`：在应用里选了模型时，用户在 agent 里「记住」的 `model` 也要跟着改，
/// 不然它会盖过刚选的默认模型。没有这个字段就不加。
pub(crate) fn write_bound(
    template: &Value,
    api_key: &str,
    select_route: bool,
    families: &[(&'static str, String)],
    txn: &mut FileTxn,
) -> Result<(), String> {
    let path = settings_path();
    let current = read_text(&path)?;
    let mut bound = bound_settings(current.as_deref(), template, api_key, families)?;
    if let (true, Some(model)) = (select_route, template_str(template, MODEL)) {
        let before = json_file::parse_object(Some(&bound), LABEL)?;
        if json_file::get(&before, REMEMBERED_MODEL).is_some() {
            let mut after = before.clone();
            json_file::set(&mut after, REMEMBERED_MODEL, model.into());
            bound = json_file::render_changed(&bound, KeyStyle::Json, &before, &after)?;
        }
    }
    if current.as_deref() == Some(bound.as_str()) {
        return Ok(());
    }
    txn.write(&path, bound.as_bytes())
        .map_err(|e| format!("写入 Claude 配置失败：{e}"))
}

/// 已绑定的配置按模板规整模型相关的 env（见 [`apply_model_env`]）：旧版本写的
/// `ANTHROPIC_MODEL` 迁成默认模型，家族别名跟上最新的模型目录。不用改时是 `None`。
pub(crate) fn normalized(
    text: &str,
    template: &Value,
    families: &[(&'static str, String)],
) -> Result<Option<String>, String> {
    let before = json_file::parse_object(Some(text), LABEL)?;
    let mut after = before.clone();
    apply_model_env(&mut after, template, families);
    if after == before {
        return Ok(None);
    }
    json_file::render_changed(text, KeyStyle::Json, &before, &after).map(Some)
}

/// Claude Code 实际会用的 Ofox 模型：`/model` 记住的 `model`（家族别名按 env 里的
/// 映射、再按模型目录换成 Ofox 的 id），没有就是旧版本写的 `ANTHROPIC_MODEL`，再没有
/// 就是 Ofox 写的默认模型。`model` 是换不成 Ofox id 的值（原生 id 之类）时返回
/// `None`：桌面端表示不了它，不跟。
pub(crate) fn live_model(
    text: &str,
    families: &[(&'static str, String)],
) -> Result<Option<String>, String> {
    let value = json_file::parse_object(Some(text), LABEL)?;
    let env = |path: JsonPath| template_str(&value, path).map(str::to_string);
    let picked = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty() && !model.eq_ignore_ascii_case("default"));
    if let Some(model) = picked {
        if model.contains('/') {
            return Ok(Some(model.to_string()));
        }
        let alias = model.strip_suffix("[1m]").unwrap_or(model).to_lowercase();
        let Some(&(family, path)) = FAMILIES.iter().find(|(name, _)| *name == alias) else {
            return Ok(None);
        };
        return Ok(env(path).or_else(|| {
            families
                .iter()
                .find(|(name, _)| *name == family)
                .map(|(_, id)| id.clone())
        }));
    }
    Ok(env(MODEL).or_else(|| env(DEFAULT_MODEL)))
}

pub(crate) fn leaves(text: Option<&str>) -> Result<Leaves, String> {
    json_file::leaves(text, LABEL, LEAVES)
}

pub(crate) fn plan_restore(
    original: Option<&str>,
    current: Option<&str>,
    keep: &BTreeSet<String>,
) -> Result<RestorePlan, String> {
    json_file::plan_restore(
        original,
        current,
        LABEL,
        KeyStyle::Json,
        LEAVES,
        CONTAINERS,
        keep,
    )
}

/// 没有绑定前快照时的尽力清理：去掉 Ofox 的地址、key 和 Ofox 的模型名，
/// 回到 Claude Code 自己的登录方式。
pub(crate) fn legacy_edits() -> Result<Vec<FileEdit>, String> {
    let path = settings_path();
    let Some(text) = read_text(&path)? else {
        return Ok(Vec::new());
    };
    let before = json_file::parse_object(Some(&text), LABEL)?;
    let mut value = before.clone();
    let points_to_ofox = is_ofox_bound(&text);
    let ofox_token = json_file::get(&value, AUTH_TOKEN)
        .and_then(Value::as_str)
        .is_some_and(|token| token.starts_with("sk-of-"));
    let mut doomed = Vec::new();
    if points_to_ofox {
        doomed.extend([BASE_URL, MODEL, DEFAULT_MODEL, DISCOVERY]);
        doomed.extend(FAMILIES.iter().map(|(_, path)| *path));
    }
    if points_to_ofox || ofox_token {
        doomed.push(AUTH_TOKEN);
    }
    let mut removed_keys = Vec::new();
    for path in doomed {
        if json_file::remove(&mut value, path) {
            removed_keys.push(path.join("."));
        }
    }
    if removed_keys.is_empty() {
        return Ok(Vec::new());
    }
    if json_file::get(&value, &["env"])
        .and_then(Value::as_object)
        .is_some_and(serde_json::Map::is_empty)
    {
        json_file::remove(&mut value, &["env"]);
    }
    Ok(vec![FileEdit {
        path,
        content: Some(json_file::render_changed(
            &text,
            KeyStyle::Json,
            &before,
            &value,
        )?),
        restored_keys: Vec::new(),
        removed_keys,
    }])
}

/// 推理强度；Claude Code 不让 settings.json 的 `effortLevel` 存 max，max 要放环境变量。
const EFFORT: JsonPath = &["effortLevel"];
const EFFORT_ENV: JsonPath = &["env", "CLAUDE_CODE_EFFORT_LEVEL"];

/// settings.json 的推理强度改成 `effort`，`None` 两处都删掉、交给 Claude Code。
pub(crate) fn with_effort(current: Option<&str>, effort: Option<&str>) -> Result<String, String> {
    let before = json_file::parse_object(current, LABEL)?;
    let mut after = before.clone();
    json_file::remove(&mut after, EFFORT);
    json_file::remove(&mut after, EFFORT_ENV);
    match effort {
        Some("max") => json_file::set(&mut after, EFFORT_ENV, "max".into()),
        Some(level) => json_file::set(&mut after, EFFORT, level.into()),
        None => {}
    }
    json_file::render_changed(current.unwrap_or_default(), KeyStyle::Json, &before, &after)
}

/// settings.json 现在的推理强度（环境变量里的 max 优先）；没有或读不了都当作默认。
pub(crate) fn effort() -> Option<String> {
    let text = read_text(&settings_path()).ok()??;
    let value = json_file::parse_object(Some(&text), LABEL).ok()?;
    [EFFORT_ENV, EFFORT]
        .iter()
        .find_map(|path| json_file::get(&value, path)?.as_str().map(Into::into))
}

pub(crate) fn write_effort(effort: Option<&str>, txn: &mut FileTxn) -> Result<(), String> {
    let path = settings_path();
    let updated = with_effort(read_text(&path)?.as_deref(), effort)?;
    txn.write(&path, updated.as_bytes())
        .map_err(|e| format!("写入 Claude 配置失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const RELAY: &str = "{\n  \"env\": {\n    \"ANTHROPIC_BASE_URL\": \"https://relay.example\",\n    \"ANTHROPIC_API_KEY\": \"sk-relay\",\n    \"ANTHROPIC_MODEL\": \"claude-opus-x\",\n    \"DISABLE_TELEMETRY\": \"1\"\n  },\n  \"statusLine\": {\n    \"type\": \"command\"\n  }\n}\n";

    fn template(model: Option<&str>) -> Value {
        let mut env = json!({
            "ANTHROPIC_BASE_URL": "https://api.ofox.ai/anthropic",
            "ANTHROPIC_AUTH_TOKEN": "",
        });
        if let Some(model) = model {
            env["ANTHROPIC_MODEL"] = json!(model);
        }
        json!({ "env": env })
    }

    fn parsed(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn families() -> Vec<(&'static str, String)> {
        family_models([
            "anthropic/claude-haiku-4.5",
            "anthropic/claude-haiku-5.5",
            "anthropic/claude-opus-4.8",
            "anthropic/claude-opus-5",
            "anthropic/claude-sonnet-5.5",
            "openai/gpt-6",
        ])
    }

    #[test]
    fn bind_points_to_ofox_and_drops_the_relay_key() {
        let bound =
            bound_settings(Some(RELAY), &template(Some("anthropic/x")), "sk-of-K", &[]).unwrap();
        let value = parsed(&bound);
        assert_eq!(
            value["env"]["ANTHROPIC_BASE_URL"],
            "https://api.ofox.ai/anthropic"
        );
        assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-of-K");
        assert!(value["env"].get("ANTHROPIC_API_KEY").is_none());
        assert_eq!(value["env"]["DISABLE_TELEMETRY"], "1");
        assert_eq!(value["statusLine"]["type"], "command");
        assert!(is_ofox_bound(&bound));
        assert!(!is_ofox_bound(RELAY));
    }

    #[test]
    fn bind_writes_the_model_as_a_default_the_model_picker_can_override() {
        let bound =
            bound_settings(Some(RELAY), &template(Some("anthropic/x")), "sk-of-K", &[]).unwrap();
        let env = &parsed(&bound)["env"];
        assert_eq!(env["ANTHROPIC_DEFAULT_MODEL"], "anthropic/x");
        assert!(env.get("ANTHROPIC_MODEL").is_none());
    }

    #[test]
    fn bind_lets_claude_code_list_the_ofox_models() {
        let bound = bound_settings(Some(RELAY), &template(None), "sk-of-K", &[]).unwrap();
        assert_eq!(
            parsed(&bound)["env"]["CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY"],
            "1"
        );
    }

    #[test]
    fn bind_maps_the_family_aliases_to_the_newest_ofox_models() {
        let bound = bound_settings(Some(RELAY), &template(None), "sk-of-K", &families()).unwrap();
        let env = &parsed(&bound)["env"];
        assert_eq!(
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
            "anthropic/claude-haiku-5.5"
        );
        assert_eq!(
            env["ANTHROPIC_DEFAULT_OPUS_MODEL"],
            "anthropic/claude-opus-5"
        );
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            "anthropic/claude-sonnet-5.5"
        );
        assert!(env.get("ANTHROPIC_DEFAULT_FABLE_MODEL").is_none());
    }

    #[test]
    fn bind_without_a_catalog_keeps_ofox_aliases_and_drops_relay_ones() {
        let before = bound_settings(Some(RELAY), &template(None), "sk-of-K", &families()).unwrap();
        let mut relay = parsed(&before);
        relay["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"] = json!("relay-fable");
        let after =
            bound_settings(Some(&relay.to_string()), &template(None), "sk-of-K", &[]).unwrap();
        let env = &parsed(&after)["env"];
        assert_eq!(
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
            "anthropic/claude-haiku-5.5"
        );
        assert!(env.get("ANTHROPIC_DEFAULT_FABLE_MODEL").is_none());
    }

    #[test]
    fn family_models_skip_dated_and_suffixed_ids() {
        let families = family_models([
            "anthropic/claude-haiku-5.5",
            "anthropic/claude-haiku-6-thinking",
            "anthropic/claude-haiku-7-20270101",
            "claude-haiku-9",
        ]);
        assert_eq!(
            families,
            [("haiku", "anthropic/claude-haiku-5.5".to_string())]
        );
    }

    #[test]
    fn family_models_compare_versions_numerically() {
        let families = family_models(["anthropic/claude-opus-4.10", "anthropic/claude-opus-4.9"]);
        assert_eq!(
            families,
            [("opus", "anthropic/claude-opus-4.10".to_string())]
        );
    }

    #[test]
    fn bind_without_a_model_removes_the_old_model() {
        let bound = bound_settings(Some(RELAY), &template(None), "sk-of-K", &[]).unwrap();
        let env = &parsed(&bound)["env"];
        assert!(env.get("ANTHROPIC_MODEL").is_none());
        assert!(env.get("ANTHROPIC_DEFAULT_MODEL").is_none());
    }

    #[test]
    fn restore_puts_the_relay_setup_back_byte_for_byte() {
        let bound = bound_settings(
            Some(RELAY),
            &template(Some("anthropic/x")),
            "sk-of-K",
            &families(),
        )
        .unwrap();
        let plan = plan_restore(Some(RELAY), Some(&bound), &BTreeSet::new()).unwrap();
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(RELAY));
    }

    #[test]
    fn bind_refuses_settings_it_cannot_parse() {
        assert!(bound_settings(Some("{ oops"), &template(None), "sk-of-K", &[]).is_err());
        assert!(validate("{ oops").is_err());
    }

    #[test]
    fn live_model_follows_the_model_picked_in_claude_code() {
        let text = json!({
            "model": "anthropic/claude-opus-4.7",
            "env": { "ANTHROPIC_DEFAULT_MODEL": "anthropic/claude-sonnet-5.5" }
        })
        .to_string();
        assert_eq!(
            live_model(&text, &[]).unwrap().as_deref(),
            Some("anthropic/claude-opus-4.7")
        );
    }

    #[test]
    fn live_model_resolves_a_family_alias_through_the_env_then_the_catalog() {
        let text = json!({
            "model": "haiku",
            "env": { "ANTHROPIC_DEFAULT_HAIKU_MODEL": "anthropic/claude-haiku-5.1" }
        })
        .to_string();
        assert_eq!(
            live_model(&text, &families()).unwrap().as_deref(),
            Some("anthropic/claude-haiku-5.1")
        );
        let text = json!({ "model": "Haiku[1m]", "env": {} }).to_string();
        assert_eq!(
            live_model(&text, &families()).unwrap().as_deref(),
            Some("anthropic/claude-haiku-5.5")
        );
    }

    #[test]
    fn live_model_ignores_a_model_it_cannot_map_to_ofox() {
        let text = json!({
            "model": "claude-haiku-5-5",
            "env": { "ANTHROPIC_DEFAULT_MODEL": "anthropic/claude-sonnet-5.5" }
        })
        .to_string();
        assert_eq!(live_model(&text, &families()).unwrap(), None);
    }

    #[test]
    fn live_model_falls_back_to_the_default_when_claude_code_uses_its_default() {
        let text = json!({
            "model": "default",
            "env": { "ANTHROPIC_DEFAULT_MODEL": "anthropic/claude-sonnet-5.5" }
        })
        .to_string();
        assert_eq!(
            live_model(&text, &[]).unwrap().as_deref(),
            Some("anthropic/claude-sonnet-5.5")
        );
    }

    #[test]
    fn normalized_moves_the_old_env_model_to_the_default() {
        let text = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.ofox.ai/anthropic",
                "ANTHROPIC_MODEL": "anthropic/old"
            }
        })
        .to_string();
        let updated = normalized(&text, &template(Some("anthropic/old")), &[])
            .unwrap()
            .unwrap();
        let env = &parsed(&updated)["env"];
        assert!(env.get("ANTHROPIC_MODEL").is_none());
        assert_eq!(env["ANTHROPIC_DEFAULT_MODEL"], "anthropic/old");
        assert_eq!(env["CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY"], "1");
        assert_eq!(
            normalized(&updated, &template(Some("anthropic/old")), &[]).unwrap(),
            None
        );
    }
}
