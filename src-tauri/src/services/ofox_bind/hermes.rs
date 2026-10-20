//! Hermes 的绑定配置：`config.yaml` 里 `custom_providers` 中名为 `ofox-hermes` 的
//! 条目，以及 `model` 段落的路由字段（`provider` / `default`）。其它 provider、
//! skills、`.env`、`auth.json` 一律不动。写回时只改有变化的顶层段落，其余段落的
//! 注释和格式原样保留。

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{json, Value};
use serde_yaml::{Mapping, Value as Yaml};

use crate::config::FileTxn;
use crate::hermes_config::{self, get_hermes_config_path};
use crate::ofox_apex::mentions_ofox_gateway;

use super::json_file::{self, JsonPath};
use super::ownership::{self, Leaf, Leaves};
use super::plan::{read_text, FileEdit, RestorePlan};

const LABEL: &str = "Hermes 的 config.yaml";
const PROVIDER_NAME: &str = "ofox-hermes";
const PROVIDERS: &str = "custom_providers";
/// Hermes v12+ 的 `providers:` 字典；Hermes 升级时会把 `custom_providers` 迁移进来。
const DICT: &str = "providers";
const MODEL: &str = "model";
/// `model` 段落里由绑定改写、解绑时还原的路由字段；上下文长度等其它字段不动。
const ROUTING_KEYS: [&str; 2] = ["provider", "default"];
const SECTIONS: &[&str] = &[PROVIDERS, DICT, MODEL];
/// 从这个版本起 Hermes 的正式格式是 `providers:` 字典，不再迁移 `custom_providers`。
const DICT_SCHEMA_VERSION: u64 = 12;
const TOKEN: JsonPath = &["api_key"];
/// `custom_providers` 列表里的 Ofox 条目（按 `name` 找）在报告和记录里的名字。
const LIST_ENTRY: &str = "custom_providers[ofox-hermes]";
/// 解绑时还原成绑定前的值（原来没有就删掉）；用户之后改过的按规则留着。
pub(crate) const LEAVES: &[Leaf] = &[
    Leaf::route(&[LIST_ENTRY], ownership::ofox_entry),
    Leaf::route(&[DICT, PROVIDER_NAME], ownership::ofox_entry),
    Leaf::route(&[MODEL, "provider"], routes_to_ofox),
    Leaf::model(&[MODEL, "default"], &[MODEL, "provider"]),
];

pub(crate) fn config_path() -> PathBuf {
    get_hermes_config_path()
}

fn parse(text: Option<&str>) -> Result<Yaml, String> {
    let value = hermes_config::parse_config_text(text.unwrap_or_default()).map_err(|e| {
        format!(
            "{LABEL} 不是有效的 YAML，请先修复后再操作：{}",
            crate::redact::scrub(&e.to_string())
        )
    })?;
    match value {
        Yaml::Mapping(_) => Ok(value),
        // 只有注释的文件。
        Yaml::Null => Ok(Yaml::Mapping(Mapping::new())),
        _ => Err(format!("{LABEL} 的根节点必须是映射，请先修复后再操作")),
    }
}

fn root(config: &mut Yaml) -> &mut Mapping {
    config.as_mapping_mut().expect("parse returns a mapping")
}

pub(crate) fn validate(text: &str) -> Result<(), String> {
    parse(Some(text)).map(drop)
}

fn entry_position(config: &Yaml) -> Option<usize> {
    config
        .get(PROVIDERS)?
        .as_sequence()?
        .iter()
        .position(|provider| provider.get("name").and_then(Yaml::as_str) == Some(PROVIDER_NAME))
}

fn dict_entry(config: &Yaml) -> Option<&Yaml> {
    config.get(DICT)?.as_mapping()?.get(PROVIDER_NAME)
}

/// 配置里已有的 Ofox 条目（列表或字典写法）。
fn existing_entry(config: &Yaml) -> Option<&Yaml> {
    match entry_position(config) {
        Some(index) => config.get(PROVIDERS)?.as_sequence()?.get(index),
        None => dict_entry(config),
    }
}

fn uses_dict_schema(config: &Yaml) -> bool {
    config
        .get("_config_version")
        .and_then(Yaml::as_u64)
        .is_some_and(|version| version >= DICT_SCHEMA_VERSION)
}

/// 去掉 `providers:` 字典里的 Ofox 条目（其它条目保持顺序），返回是否去掉了。
fn remove_dict_entry(config: &mut Yaml) -> bool {
    config
        .get_mut(DICT)
        .and_then(Yaml::as_mapping_mut)
        .is_some_and(|dict| dict.shift_remove(PROVIDER_NAME).is_some())
}

/// `ofox-hermes` 条目（列表或字典里）指向 Ofox 网关——当前（或旧版本）绑定留下的配置。
pub(crate) fn is_ofox_bound(text: &str) -> bool {
    parse(Some(text)).is_ok_and(|config| entry_points_to_ofox(&config))
}

fn entry_points_to_ofox(config: &Yaml) -> bool {
    let list_url =
        entry_position(config).and_then(|index| config[PROVIDERS][index].get("base_url"));
    // Hermes 迁移时按 `base_url` / `url` / `api` 取地址，写成 `api`。
    let dict_url = dict_entry(config).and_then(|entry| {
        ["api", "base_url", "url"]
            .iter()
            .find_map(|key| entry.get(*key))
    });
    [list_url, dict_url]
        .into_iter()
        .flatten()
        .filter_map(Yaml::as_str)
        .any(mentions_ofox_gateway)
}

fn routes_to_ofox(provider: &Value) -> bool {
    provider.as_str() == Some(PROVIDER_NAME)
}

/// config.yaml 里各受管字段现在的值。
pub(crate) fn leaves(text: Option<&str>) -> Result<Leaves, String> {
    let config = parse(text)?;
    let to_json = |yaml: Option<&Yaml>| match yaml {
        Some(yaml) => serde_json::to_value(yaml)
            .map_err(|_| format!("{LABEL} 的 Ofox 条目无法读取，请先修复后再操作")),
        None => Ok(Value::Null),
    };
    let model = config.get(MODEL).filter(|model| model.is_mapping());
    let projected = json!({
        LIST_ENTRY: to_json(existing_list_entry(&config))?,
        DICT: { PROVIDER_NAME: to_json(dict_entry(&config))? },
        MODEL: {
            "provider": to_json(model.and_then(|model| model.get("provider")))?,
            "default": to_json(model.and_then(|model| model.get("default")))?,
        },
    });
    Ok(ownership::leaves_at(&projected, LEAVES))
}

fn existing_list_entry(config: &Yaml) -> Option<&Yaml> {
    config
        .get(PROVIDERS)?
        .as_sequence()?
        .get(entry_position(config)?)
}

/// 只把值有变化的段落写回 `raw`。
fn render_changed(raw: &str, before: &Yaml, after: &Yaml) -> Result<String, String> {
    let mut text = raw.to_string();
    for section in SECTIONS {
        let target = after.get(*section);
        if target != before.get(*section) {
            text =
                hermes_config::render_section(&text, section, target).map_err(|e| e.to_string())?;
        }
    }
    Ok(text)
}

/// 由 DB 里 `ofox-hermes` 模板生成绑定后的配置：写入 Ofox 条目，路由指向它。
///
/// 条目在哪就更新哪；新条目在 v12+ 配置里写进 `providers:` 字典，否则写进
/// `custom_providers` 列表（未标版本的配置之后由 Hermes 自己迁移）。
///
/// `select_route`：绑定、保存模型时要让路由指向 Ofox 条目；其余（换 key、修复）
/// 路由已经指向 Ofox 就不动，用户在 agent 里选的模型要留着。
pub(crate) fn bound_config(
    current: Option<&str>,
    template: &Value,
    api_key: &str,
    select_route: bool,
) -> Result<String, String> {
    let before = parse(current)?;
    let mut entry = template.clone();
    json_file::set(&mut entry, TOKEN, api_key.into());
    let routed_to_ofox = before
        .get(MODEL)
        .and_then(|model| model.get("provider"))
        .and_then(Yaml::as_str)
        == Some(PROVIDER_NAME);
    // 用户在 agent 里加的字段、模型留着；保存模型时以模板的模型列表为准。
    let existing = existing_entry(&before)
        .and_then(|entry| serde_json::to_value(entry).ok())
        .map(|mut existing| {
            if select_route {
                if let Some(existing) = existing.as_object_mut() {
                    existing.remove("models");
                }
            }
            existing
        });
    let upsert = json_file::merge_provider_entry(existing.as_ref(), entry.clone());
    let mut after = before.clone();
    let use_list = entry_position(&before).is_some()
        || (dict_entry(&before).is_none() && !uses_dict_schema(&before));
    if use_list {
        let providers =
            hermes_config::upsert_custom_provider(&before, PROVIDER_NAME, upsert.clone())
                .map_err(|e| e.to_string())?;
        root(&mut after).insert(PROVIDERS.into(), providers);
        // Hermes 先读列表；字典里再留一份 Ofox 条目只会重复。
        remove_dict_entry(&mut after);
    } else {
        let mut dict = match before.get(DICT) {
            None => Mapping::new(),
            Some(Yaml::Mapping(dict)) => dict.clone(),
            Some(_) => return Err(format!("{LABEL} 的 {DICT} 段落格式无效，请先修复后再操作")),
        };
        let updated = hermes_config::provider_dict_entry(
            dict.get(PROVIDER_NAME),
            PROVIDER_NAME,
            upsert.clone(),
        )
        .map_err(|e| e.to_string())?;
        dict.insert(PROVIDER_NAME.into(), updated);
        root(&mut after).insert(DICT.into(), Yaml::Mapping(dict));
    }
    if select_route || !routed_to_ofox {
        let current_model = hermes_config::model_config_of(&before)
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let model = hermes_config::switch_defaults(current_model, PROVIDER_NAME, &entry);
        let model = serde_json::to_value(model)
            .map_err(|e| e.to_string())
            .and_then(|model| hermes_config::json_to_yaml(&model).map_err(|e| e.to_string()))?;
        root(&mut after).insert(MODEL.into(), model);
    }
    render_changed(current.unwrap_or_default(), &before, &after)
}

/// 绑定和切换模型共用。
pub(crate) fn write_bound(
    template: &Value,
    api_key: &str,
    select_route: bool,
    txn: &mut FileTxn,
) -> Result<(), String> {
    let path = config_path();
    let bound = bound_config(
        read_text(&path)?.as_deref(),
        template,
        api_key,
        select_route,
    )?;
    txn.write(&path, bound.as_bytes())
        .map_err(|e| format!("写入 Hermes 配置失败：{e}"))
}

/// 还原后的 `custom_providers`：Ofox 条目放回绑定前的位置（原来没有就去掉），
/// 其它条目保持当前状态。
fn restore_providers(
    original: &Yaml,
    current: &Yaml,
    keep: &BTreeSet<String>,
    restored_keys: &mut Vec<String>,
    removed_keys: &mut Vec<String>,
) -> Option<Yaml> {
    if keep.contains(LIST_ENTRY) {
        return current.get(PROVIDERS).cloned();
    }
    let entry_key = LIST_ENTRY.to_string();
    let mut providers = current
        .get(PROVIDERS)
        .and_then(Yaml::as_sequence)
        .cloned()
        .unwrap_or_default();
    let before = providers.len();
    providers.retain(|provider| provider.get("name").and_then(Yaml::as_str) != Some(PROVIDER_NAME));
    let had_entry = providers.len() != before;
    match entry_position(original) {
        Some(index) => {
            let entry = original[PROVIDERS][index].clone();
            providers.insert(index.min(providers.len()), entry);
            restored_keys.push(entry_key);
        }
        None if had_entry => removed_keys.push(entry_key),
        None => {}
    }
    if !providers.is_empty() {
        return Some(Yaml::Sequence(providers));
    }
    match (original.get(PROVIDERS), current.get(PROVIDERS)) {
        // Hermes 迁移时删掉了这个段落：不要再造一个空列表出来。
        (_, None) => None,
        (Some(Yaml::Sequence(_)), _) => Some(Yaml::Sequence(providers)),
        (other, _) => other.cloned(),
    }
}

/// 还原后的 `providers:` 字典：Ofox 条目放回绑定前的位置（原来没有就去掉，
/// 包括 Hermes 从列表迁移过来的），其它条目保持当前状态和顺序。
fn restore_dict(
    original: &Yaml,
    current: &Yaml,
    keep: &BTreeSet<String>,
    restored_keys: &mut Vec<String>,
    removed_keys: &mut Vec<String>,
) -> Option<Yaml> {
    let entry_key = format!("{DICT}.{PROVIDER_NAME}");
    if keep.contains(&entry_key) {
        return current.get(DICT).cloned();
    }
    let Some(current_dict) = current.get(DICT).and_then(Yaml::as_mapping) else {
        return current.get(DICT).cloned();
    };
    let mut dict = current_dict.clone();
    let had_entry = dict.shift_remove(PROVIDER_NAME).is_some();
    let original_entry = original
        .get(DICT)
        .and_then(Yaml::as_mapping)
        .and_then(|dict| {
            let index = dict
                .keys()
                .position(|key| key.as_str() == Some(PROVIDER_NAME))?;
            Some((index, dict.get(PROVIDER_NAME)?.clone()))
        });
    match original_entry {
        Some((index, entry)) => {
            let mut entries: Vec<_> = dict.into_iter().collect();
            entries.insert(index.min(entries.len()), (PROVIDER_NAME.into(), entry));
            dict = entries.into_iter().collect();
            restored_keys.push(entry_key);
        }
        None if had_entry => removed_keys.push(entry_key),
        None => {}
    }
    if dict.is_empty() && original.get(DICT).is_none() {
        return None;
    }
    Some(Yaml::Mapping(dict))
}

/// 还原后的 `model` 段落：路由字段回到绑定前（`keep` 里用户改过的除外），其它字段
/// 保持当前状态。
fn restore_model(
    original: &Yaml,
    current: &Yaml,
    keep: &BTreeSet<String>,
    restored_keys: &mut Vec<String>,
    removed_keys: &mut Vec<String>,
) -> Option<Yaml> {
    let original_model = original.get(MODEL);
    let kept = |key: &str| keep.contains(&format!("{MODEL}.{key}"));
    let keeps_any = ROUTING_KEYS.into_iter().any(kept);
    let Some(current_model) = current.get(MODEL).and_then(Yaml::as_mapping) else {
        return if keeps_any {
            current.get(MODEL).cloned()
        } else {
            original_model.cloned()
        };
    };
    if original_model.is_some_and(|model| !model.is_mapping()) && !keeps_any {
        return original_model.cloned();
    }
    let mut model = current_model.clone();
    for key in ROUTING_KEYS.into_iter().filter(|key| !kept(key)) {
        match original_model.and_then(|model| model.get(key)) {
            Some(value) => {
                model.insert(key.into(), value.clone());
                restored_keys.push(format!("{MODEL}.{key}"));
            }
            None => {
                if model.remove(key).is_some() {
                    removed_keys.push(format!("{MODEL}.{key}"));
                }
            }
        }
    }
    if model.is_empty() && original_model.is_none() {
        return None;
    }
    Some(Yaml::Mapping(model))
}

fn set_section(config: &mut Yaml, key: &str, value: Option<Yaml>) {
    match value {
        Some(value) => {
            root(config).insert(key.into(), value);
        }
        None => {
            root(config).remove(key);
        }
    }
}

/// 受管内容还原成绑定前的样子（`keep` 里用户改过的除外）。语义上等于绑定前时写回
/// 原文本；否则只改有变化的段落，和绑定前一样的段落原样搬回（含注释）。
pub(crate) fn plan_restore(
    original: Option<&str>,
    current: Option<&str>,
    keep: &BTreeSet<String>,
) -> Result<RestorePlan, String> {
    let original_value = parse(original)?;
    let current_value = parse(current)?;
    let mut restored_keys = Vec::new();
    let mut removed_keys = Vec::new();
    let mut restored = current_value.clone();
    let providers = restore_providers(
        &original_value,
        &current_value,
        keep,
        &mut restored_keys,
        &mut removed_keys,
    );
    set_section(&mut restored, PROVIDERS, providers);
    let dict = restore_dict(
        &original_value,
        &current_value,
        keep,
        &mut restored_keys,
        &mut removed_keys,
    );
    set_section(&mut restored, DICT, dict);
    let model = restore_model(
        &original_value,
        &current_value,
        keep,
        &mut restored_keys,
        &mut removed_keys,
    );
    set_section(&mut restored, MODEL, model);

    let unowned_text = |source: Option<&str>| -> Result<String, String> {
        let mut text = source.unwrap_or_default().to_string();
        for section in SECTIONS {
            text =
                hermes_config::render_section(&text, section, None).map_err(|e| e.to_string())?;
        }
        Ok(text)
    };
    if restored == original_value && unowned_text(current)? == unowned_text(original)? {
        return Ok(RestorePlan {
            content: original.map(str::to_string),
            exact: true,
            restored_keys,
            removed_keys,
            kept_keys: Vec::new(),
        });
    }
    let mut text = current.unwrap_or_default().to_string();
    for section in SECTIONS {
        let target = restored.get(*section);
        if target == current_value.get(*section) {
            continue;
        }
        let copied = match (target, original) {
            (Some(value), Some(original_text)) if original_value.get(*section) == Some(value) => {
                hermes_config::copy_section(&text, original_text, section)
            }
            _ => None,
        };
        text = match copied {
            Some(text) => text,
            None => {
                hermes_config::render_section(&text, section, target).map_err(|e| e.to_string())?
            }
        };
    }
    Ok(RestorePlan {
        content: Some(text),
        exact: false,
        restored_keys,
        removed_keys,
        kept_keys: Vec::new(),
    })
}

/// 旧版本（`__ofoxDirectBackupVersion` 外层）记下的绑定前路由。
fn legacy_routing(legacy: Option<&Value>) -> Option<[Option<&str>; 2]> {
    let snapshot = legacy
        .filter(|record| record.get("__ofoxDirectBackupVersion").is_some())?
        .get("runtimeDefault")?;
    Some(ROUTING_KEYS.map(|key| snapshot.get(key).and_then(Value::as_str)))
}

/// 没有绑定前快照时的尽力清理：删掉 Ofox 条目；旧版本记过绑定前的路由就还原它，
/// 否则只去掉指向 Ofox 的路由。
pub(crate) fn legacy_edits(legacy: Option<&Value>) -> Result<Vec<FileEdit>, String> {
    let path = config_path();
    let Some(text) = read_text(&path)? else {
        return Ok(Vec::new());
    };
    let before = parse(Some(&text))?;
    let mut after = before.clone();
    let mut restored_keys = Vec::new();
    let mut removed_keys = Vec::new();
    if let Some(index) = entry_position(&after) {
        if let Some(providers) = after.get_mut(PROVIDERS).and_then(Yaml::as_sequence_mut) {
            providers.remove(index);
            removed_keys.push(LIST_ENTRY.to_string());
        }
    }
    if remove_dict_entry(&mut after) {
        removed_keys.push(format!("{DICT}.{PROVIDER_NAME}"));
    }
    let routes_to_ofox = before
        .get(MODEL)
        .and_then(|model| model.get("provider"))
        .and_then(Yaml::as_str)
        == Some(PROVIDER_NAME);
    let routing = legacy_routing(legacy).or(routes_to_ofox.then_some([None, None]));
    let model = after.get_mut(MODEL).and_then(Yaml::as_mapping_mut);
    if let (Some(routing), Some(model)) = (routing, model) {
        for (key, value) in ROUTING_KEYS.into_iter().zip(routing) {
            match value {
                Some(value) => {
                    if model.get(key).and_then(Yaml::as_str) != Some(value) {
                        model.insert(key.into(), value.into());
                        restored_keys.push(format!("{MODEL}.{key}"));
                    }
                }
                None => {
                    if model.remove(key).is_some() {
                        removed_keys.push(format!("{MODEL}.{key}"));
                    }
                }
            }
        }
    }
    if after == before {
        return Ok(Vec::new());
    }
    Ok(vec![FileEdit {
        content: Some(render_changed(&text, &before, &after)?),
        path,
        restored_keys,
        removed_keys,
    }])
}

/// 推理强度在 `agent.reasoning_effort`；绑定只管 [`SECTIONS`]，这一段单独写。
const AGENT: &str = "agent";
const EFFORT_FIELD: &str = "reasoning_effort";

/// config.yaml 的推理强度改成 `effort`，`None` 删掉、交给 Hermes（它默认 medium）；
/// `agent` 段里别的设置和其余段落原样保留。
pub(crate) fn with_effort(current: Option<&str>, effort: Option<&str>) -> Result<String, String> {
    let before = parse(current)?;
    let mut agent = match before.get(AGENT) {
        None | Some(Yaml::Null) => Mapping::new(),
        Some(Yaml::Mapping(agent)) => agent.clone(),
        Some(_) => return Err(format!("{LABEL} 的 {AGENT} 段落格式无效，请先修复后再操作")),
    };
    match effort {
        Some(level) => {
            agent.insert(EFFORT_FIELD.into(), level.into());
        }
        None => {
            agent.remove(EFFORT_FIELD);
        }
    }
    let after = (!agent.is_empty()).then_some(Yaml::Mapping(agent));
    if after.as_ref() == before.get(AGENT) {
        return Ok(current.unwrap_or_default().to_string());
    }
    hermes_config::render_section(current.unwrap_or_default(), AGENT, after.as_ref())
        .map_err(|e| e.to_string())
}

/// config.yaml 现在的推理强度；没有或读不了都当作默认。
pub(crate) fn effort() -> Option<String> {
    let text = read_text(&config_path()).ok()??;
    parse(Some(&text))
        .ok()?
        .get(AGENT)?
        .get(EFFORT_FIELD)?
        .as_str()
        .map(Into::into)
}

pub(crate) fn write_effort(effort: Option<&str>, txn: &mut FileTxn) -> Result<(), String> {
    let path = config_path();
    let updated = with_effort(read_text(&path)?.as_deref(), effort)?;
    txn.write(&path, updated.as_bytes())
        .map_err(|e| format!("写入 Hermes 配置失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const USER_CONFIG: &str = "# Hermes config\nmodel:\n  default: deepseek-chat\n  provider: deepseek\n  context_length: 32000\ncustom_providers:\n# my relay\n- name: deepseek\n  base_url: https://api.deepseek.com/v1\n  api_key: sk-ds\nskills:\n  enabled: true\n";

    #[test]
    fn unparseable_yaml_error_never_echoes_the_key_in_it() {
        let broken = "api_key: sk-of-abc123456789\nmodel: [unclosed\n";

        let error = parse(Some(broken)).unwrap_err();

        assert!(!error.contains("abc123456789"), "{error}");
    }

    fn template() -> Value {
        json!({
            "name": "ofox",
            "base_url": "https://api.ofox.ai/v1",
            "api_key": "",
            "api_mode": "chat_completions",
            "models": { "openai/gpt-x": {} },
        })
    }

    fn yaml(text: &str) -> Yaml {
        parse(Some(text)).unwrap()
    }

    #[test]
    fn bind_adds_the_entry_and_routes_to_it() {
        let bound = bound_config(Some(USER_CONFIG), &template(), "sk-of-K", true).unwrap();
        let config = yaml(&bound);
        let index = entry_position(&config).unwrap();
        assert_eq!(
            config[PROVIDERS][index]["api_key"].as_str(),
            Some("sk-of-K")
        );
        assert_eq!(config[PROVIDERS][0]["name"].as_str(), Some("deepseek"));
        assert_eq!(config[MODEL]["provider"].as_str(), Some(PROVIDER_NAME));
        assert_eq!(config[MODEL]["default"].as_str(), Some("openai/gpt-x"));
        assert_eq!(config[MODEL]["context_length"].as_u64(), Some(32000));
        assert!(bound.contains("skills:\n  enabled: true\n"));
        assert!(is_ofox_bound(&bound));
        assert!(!is_ofox_bound(USER_CONFIG));
    }

    #[test]
    fn unbind_preserves_mcp_text_edits_that_do_not_change_yaml_values() {
        let original = format!("{USER_CONFIG}mcp_servers:\n  demo:\n    command: server\n");
        let bound = bound_config(Some(&original), &template(), "sk-of-K", true).unwrap();
        let edited = bound
            .replace("mcp_servers:\n", "# current MCP note\nmcp_servers:\n")
            .replace("command: server", "command: \"server\"");
        let plan = plan_restore(Some(&original), Some(&edited), &BTreeSet::new()).unwrap();
        assert!(plan
            .content
            .unwrap()
            .contains("# current MCP note\nmcp_servers:\n  demo:\n    command: \"server\"\n"));
    }

    #[test]
    fn restore_brings_back_the_commented_file() {
        let bound = bound_config(Some(USER_CONFIG), &template(), "sk-of-K", true).unwrap();
        let plan = plan_restore(Some(USER_CONFIG), Some(&bound), &BTreeSet::new()).unwrap();
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(USER_CONFIG));
    }

    #[test]
    fn restore_keeps_other_edits_and_copies_untouched_sections_verbatim() {
        let bound = bound_config(Some(USER_CONFIG), &template(), "sk-of-K", true).unwrap();
        // 绑定期间用户改了上下文长度，还加了新段落。
        let edited = bound.replace("context_length: 32000", "context_length: 64000")
            + "memory:\n  enabled: true\n";
        let plan = plan_restore(Some(USER_CONFIG), Some(&edited), &BTreeSet::new()).unwrap();
        assert!(!plan.exact);
        let text = plan.content.unwrap();
        assert!(
            text.contains("custom_providers:\n# my relay\n- name: deepseek\n"),
            "{text}"
        );
        let config = yaml(&text);
        assert_eq!(config[MODEL]["provider"].as_str(), Some("deepseek"));
        assert_eq!(config[MODEL]["default"].as_str(), Some("deepseek-chat"));
        assert_eq!(config[MODEL]["context_length"].as_u64(), Some(64000));
        assert_eq!(config["memory"]["enabled"].as_bool(), Some(true));
        assert!(entry_position(&config).is_none());
    }

    /// Hermes' 11 → 12 migration moved an earlier Ofox entry into `providers:`.
    const MIGRATED_CONFIG: &str = "# Hermes config\nmodel:\n  default: qwen/qwen-flash\n  provider: ofox-hermes\n  context_length: 32000\nskills:\n  enabled: true\nproviders:\n  deepseek:\n    api: https://api.deepseek.com/v1\n    name: deepseek\n    api_key: sk-ds\n  ofox-hermes:\n    api: https://api.ofox.ai/v1\n    name: ofox-hermes\n    api_key: sk-of-OLD\n    models:\n      qwen/qwen-flash: {}\n    default_model: qwen/qwen-flash\n    transport: chat_completions\n_config_version: 50\n";

    const VERSIONED_CONFIG: &str = "model:\n  default: deepseek-chat\n  provider: deepseek\nproviders:\n  deepseek:\n    api: https://api.deepseek.com/v1\n    api_key: sk-ds\n_config_version: 50\n";

    fn assert_dict_entry(config: &Yaml, api_key: &str) {
        let entry = &config[DICT][PROVIDER_NAME];
        assert_eq!(entry["api"].as_str(), Some("https://api.ofox.ai/v1"));
        assert_eq!(entry["api_key"].as_str(), Some(api_key));
        assert_eq!(entry["transport"].as_str(), Some("chat_completions"));
        assert_eq!(entry["default_model"].as_str(), Some("openai/gpt-x"));
        assert!(entry["models"].get("openai/gpt-x").is_some());
        assert!(entry["models"].get("qwen/qwen-flash").is_none());
        for legacy in ["base_url", "api_mode", "model"] {
            assert!(entry.get(legacy).is_none(), "{legacy} left in {entry:?}");
        }
    }

    #[test]
    fn bind_updates_an_entry_hermes_migrated_into_the_providers_dict() {
        assert!(is_ofox_bound(MIGRATED_CONFIG));
        let bound = bound_config(Some(MIGRATED_CONFIG), &template(), "sk-of-K", true).unwrap();
        let config = yaml(&bound);
        assert_dict_entry(&config, "sk-of-K");
        assert!(config.get(PROVIDERS).is_none());
        assert_eq!(
            config[DICT]["deepseek"]["api"].as_str(),
            Some("https://api.deepseek.com/v1")
        );
        assert_eq!(config[MODEL]["provider"].as_str(), Some(PROVIDER_NAME));
        assert_eq!(config[MODEL]["default"].as_str(), Some("openai/gpt-x"));
        assert_eq!(config["_config_version"].as_u64(), Some(50));
        assert!(bound.contains("skills:\n  enabled: true\n"));
        assert!(is_ofox_bound(&bound));
    }

    #[test]
    fn bind_adds_a_new_entry_to_the_providers_dict_of_a_versioned_config() {
        let bound = bound_config(Some(VERSIONED_CONFIG), &template(), "sk-of-K", true).unwrap();
        let config = yaml(&bound);
        assert_dict_entry(&config, "sk-of-K");
        assert!(config.get(PROVIDERS).is_none());
        assert!(config[DICT].get("deepseek").is_some());
        assert!(is_ofox_bound(&bound));
        assert!(!is_ofox_bound(VERSIONED_CONFIG));
    }

    #[test]
    fn bind_keeps_a_single_ofox_entry_when_both_sections_hold_one() {
        let both = "model:\n  provider: ofox-hermes\ncustom_providers:\n- name: ofox-hermes\n  base_url: https://api.ofox.ai/v1\n  api_key: sk-of-OLD\nproviders:\n  ofox-hermes:\n    api: https://api.ofox.ai/v1\n    api_key: sk-of-OLD\n  other:\n    api: https://example.com/v1\n_config_version: 50\n";
        let bound = bound_config(Some(both), &template(), "sk-of-K", true).unwrap();
        let config = yaml(&bound);
        let index = entry_position(&config).unwrap();
        assert_eq!(
            config[PROVIDERS][index]["api_key"].as_str(),
            Some("sk-of-K")
        );
        assert!(config[DICT].get(PROVIDER_NAME).is_none());
        assert!(config[DICT].get("other").is_some());
    }

    #[test]
    fn restore_after_a_dict_bind_brings_back_the_file() {
        let bound = bound_config(Some(VERSIONED_CONFIG), &template(), "sk-of-K", true).unwrap();
        let plan = plan_restore(Some(VERSIONED_CONFIG), Some(&bound), &BTreeSet::new()).unwrap();
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(VERSIONED_CONFIG));
    }

    #[test]
    fn restore_after_hermes_migrated_the_binding_leaves_no_ofox_entry() {
        // Bound on the legacy list; Hermes then migrated everything into the dict and
        // the user added another provider after the Ofox entry.
        let migrated = "# Hermes config\nmodel:\n  default: openai/gpt-x\n  provider: ofox-hermes\n  context_length: 32000\nskills:\n  enabled: true\nproviders:\n  deepseek:\n    api: https://api.deepseek.com/v1\n    name: deepseek\n    api_key: sk-ds\n  ofox-hermes:\n    api: https://api.ofox.ai/v1\n    name: ofox-hermes\n    api_key: sk-of-K\n  zeta:\n    api: https://zeta.example/v1\n_config_version: 50\n";
        let plan = plan_restore(Some(USER_CONFIG), Some(migrated), &BTreeSet::new()).unwrap();
        let text = plan.content.unwrap();
        let config = yaml(&text);
        assert!(config.get(PROVIDERS).is_none(), "{text}");
        let keys: Vec<_> = config[DICT]
            .as_mapping()
            .unwrap()
            .keys()
            .filter_map(Yaml::as_str)
            .collect();
        assert_eq!(keys, ["deepseek", "zeta"]);
        assert_eq!(config[MODEL]["provider"].as_str(), Some("deepseek"));
        assert_eq!(config[MODEL]["default"].as_str(), Some("deepseek-chat"));
        assert!(!text.contains("sk-of-K"));
        assert!(plan
            .removed_keys
            .contains(&format!("{DICT}.{PROVIDER_NAME}")));
    }

    #[test]
    fn dict_entry_pointing_elsewhere_is_not_an_ofox_binding() {
        let relay = VERSIONED_CONFIG.replace("deepseek:", "ofox-hermes:");
        assert!(!is_ofox_bound(&relay));
    }

    #[test]
    fn the_list_entry_name_is_the_one_in_reports() {
        assert_eq!(LIST_ENTRY, format!("{PROVIDERS}[{PROVIDER_NAME}]"));
    }

    #[test]
    fn restore_deletes_a_file_bind_created() {
        let bound = bound_config(None, &template(), "sk-of-K", true).unwrap();
        assert_eq!(
            plan_restore(None, Some(&bound), &BTreeSet::new())
                .unwrap()
                .content,
            None
        );
    }
}
