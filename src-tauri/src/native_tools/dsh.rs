//! Modern dsh profile/home layering and named credential routes.
//! Adapted from Magpie internal/agent/dsh.go; see MAGPIE_LICENSE.
use super::{
    common::{self, Format, Record},
    model_api, model_json,
    pi::update,
    url, NativeScopeModel,
};
use crate::services::model_fetch::FetchedModel;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const KEY: &str = "OFOX_DSH_API_KEY";
const PLUGIN: &str = "llm-pi-ai";
const DEFAULT: &str = "agent-default-model";
const ROUTES: [(&str, &str); 3] = [
    ("ofox-dsh", "openai-completions"),
    ("ofox-dsh-responses", "openai-responses"),
    ("ofox-dsh-anthropic", "anthropic-messages"),
];
#[cfg(test)]
pub(super) fn routes_for_test() -> [(&'static str, &'static str); 3] {
    ROUTES
}
type Changes = Vec<(Vec<String>, Option<Value>, bool)>;
fn own_route(route: &str) -> bool {
    ROUTES.iter().any(|(id, _)| *id == route)
}
fn route_for(model: &FetchedModel) -> Result<&'static str, String> {
    let api = model_api(crate::app_config::BindableTool::Dsh, model)?;
    Ok(ROUTES.iter().find(|(_, p)| *p == api).unwrap().0)
}

fn patch(path: &Path) -> Result<Value, String> {
    common::parse(common::read(path)?.as_deref(), Format::Patch)
}
#[cfg(test)]
pub(super) fn patch_for_test(path: &Path) -> Value {
    patch(path).unwrap()
}
/// dsh 0.1 把用户选的默认模型存在 `settings.yaml`，它盖过 profile 里 Ofox 写的模型，
/// 桌面端选的模型对新会话不起作用。0.2 第一次运行时把它迁进 profile 并改名
/// （`settings.yaml.imported`），之后模型选择和 Ofox 写的是同一处。
pub(super) const UPGRADE_HINT: &str = "dsh 版本过旧：0.1 版的 settings.yaml 会盖过 Ofox 设置的模型。请先把 dsh 升级到 0.2 以上并运行一次，再绑定";
fn require_current_layout(directory: &Path) -> Result<(), String> {
    if directory.join("settings.yaml").exists() {
        return Err(UPGRADE_HINT.into());
    }
    Ok(())
}
fn profile_paths(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let root = directory.join("profiles");
    let entries = std::fs::read_dir(&root)
        .map_err(|_| "dsh 尚未初始化，请先运行 dsh 创建 profile".to_string())?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| "无法读取 dsh profiles".to_string())?;
        if !entry
            .file_type()
            .map_err(|_| "无法读取 dsh profile 类型".to_string())?
            .is_dir()
        {
            continue;
        }
        let path = entry.path().join("cordis.patch.yml");
        if path.is_file() {
            paths.push(path);
        }
    }
    paths.sort_by_key(|p| {
        (
            p.parent()
                .and_then(Path::file_name)
                .is_none_or(|n| n != "web"),
            p.clone(),
        )
    });
    if paths.is_empty() {
        return Err("dsh 尚未初始化，请先运行 dsh 创建 profile".into());
    }
    Ok(paths)
}

fn validate_layers(value: &Value, initial: bool) -> Result<(), String> {
    if value
        .get(PLUGIN)
        .and_then(|v| v.get("config"))
        .is_some_and(|v| !v.is_object())
    {
        return Err("dsh llm-pi-ai config 格式不受支持".into());
    }
    if value
        .get(PLUGIN)
        .and_then(|v| v.pointer("/config/providers"))
        .is_some_and(|v| !v.is_object())
    {
        return Err("dsh providers 格式不受支持".into());
    }
    if initial
        && value
            .get(PLUGIN)
            .and_then(|v| v.pointer("/config/providers"))
            .is_some_and(|v| ROUTES.iter().any(|(route, _)| v.get(*route).is_some()))
    {
        return Err("dsh 已有同名 Ofox provider，未覆盖".into());
    }
    for (id, row) in value.as_object().unwrap() {
        if id.contains("credential") {
            if let Some(config) = row.get("config") {
                if config.as_object().is_some_and(|v| !v.is_empty()) {
                    return Err("dsh 使用自定义凭据存储，请先检查配置".into());
                }
            }
        }
    }
    Ok(())
}

fn route_changes(catalog: &[FetchedModel]) -> Changes {
    let mut changes = Vec::new();
    for (route, api) in ROUTES {
        let models = catalog
            .iter()
            .filter(|m| model_api(crate::app_config::BindableTool::Dsh, m).ok() == Some(api))
            .map(|m| {
                let mut value = model_json(m);
                value.as_object_mut().unwrap().remove("reasoning");
                value.as_object_mut().unwrap().remove("cost");
                value
            })
            .collect::<Vec<_>>();
        for (field, value, connection) in [
            (
                "displayName",
                json!(format!(
                    "OFox{}",
                    if api == "openai-responses" {
                        " · Responses"
                    } else if api == "anthropic-messages" {
                        " · Anthropic"
                    } else {
                        ""
                    }
                )),
                false,
            ),
            ("apiKeyEnv", json!(KEY), true),
            ("api", json!(api), true),
            ("baseURL", json!(url(api)), true),
            ("models", json!(models), false),
        ] {
            changes.push((
                common::path(&[PLUGIN, "config", "providers", route, field]),
                Some(value),
                connection,
            ));
        }
    }
    changes
}

fn model_changes(model: &FetchedModel) -> Result<Changes, String> {
    Ok(vec![
        (
            common::path(&[DEFAULT, "config", "provider"]),
            Some(route_for(model)?.into()),
            false,
        ),
        (
            common::path(&[DEFAULT, "config", "model"]),
            Some(model.id.clone().into()),
            false,
        ),
    ])
}
fn scope_name(directory: &Path, path: &Path) -> String {
    path.strip_prefix(directory)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn plan(
    directory: &Path,
    key: &str,
    model: &FetchedModel,
    catalog: &[FetchedModel],
    previous: Option<&Record>,
) -> Result<Record, String> {
    require_current_layout(directory)?;
    let mut record = previous.cloned().unwrap_or(Record {
        version: 1,
        directory: directory.into(),
        files: vec![],
    });
    let paths = if let Some(previous) = previous {
        previous
            .files
            .iter()
            .filter(|f| f.format == Format::Patch && f.path != directory.join("cordis.patch.yml"))
            .map(|f| f.path.clone())
            .collect()
    } else {
        profile_paths(directory)?
    };
    let home_path = directory.join("cordis.patch.yml");
    let home = patch(&home_path)?;
    validate_layers(&home, previous.is_none())?;
    for path in paths {
        let value = patch(&path)?;
        validate_layers(&value, previous.is_none())?;
        let mut changes = route_changes(catalog);
        changes.extend(model_changes(model)?);
        for id in [PLUGIN, DEFAULT] {
            if value.get(id).and_then(|v| v.get("disabled")) == Some(&json!(true)) {
                changes.push((common::path(&[id, "disabled"]), Some(json!(false)), true));
            }
        }
        // Plugin metadata is only added to a profile that has no such entry.
        if value.get(PLUGIN).is_none() {
            changes.push((
                common::path(&[PLUGIN, "name"]),
                Some("@deepseek-ai/dsh-llm-pi-ai".into()),
                false,
            ));
        }
        update(&mut record, path, Format::Patch, false, changes)?;
    }
    let mut home_changes = Vec::new();
    for id in [PLUGIN, DEFAULT] {
        if home.get(id).and_then(|v| v.get("disabled")) == Some(&json!(true)) {
            home_changes.push((common::path(&[id, "disabled"]), Some(json!(false)), true));
        }
    }
    if home.get(PLUGIN).and_then(|v| v.get("config")).is_some() {
        home_changes.extend(route_changes(catalog));
    }
    if home.get(DEFAULT).and_then(|v| v.get("config")).is_some() {
        home_changes.extend(model_changes(model)?);
    }
    if !home_changes.is_empty() {
        update(&mut record, home_path, Format::Patch, false, home_changes)?;
    }
    let env_path = directory.join(".env");
    let env = common::parse(common::read(&env_path)?.as_deref(), Format::Env)?;
    let credentials_path = directory.join(".credentials.yaml");
    let credentials = common::parse(common::read(&credentials_path)?.as_deref(), Format::YamlMap)?;
    validate_credentials(&credentials)?;
    if previous.is_none()
        && (env.get(KEY).is_some() || credentials.pointer(&format!("/refs/{KEY}")).is_some())
    {
        return Err("dsh 已有同名凭据，未覆盖".into());
    }
    update(
        &mut record,
        env_path,
        Format::Env,
        true,
        vec![(common::path(&[KEY]), Some(key.into()), true)],
    )?;
    let mut changes = vec![(common::path(&["refs", KEY]), Some(key.into()), true)];
    if credentials.get("version").is_none() {
        changes.push((common::path(&["version"]), Some(json!(1)), true));
    }
    update(
        &mut record,
        credentials_path,
        Format::YamlMap,
        true,
        changes,
    )?;
    Ok(record)
}

fn validate_credentials(value: &Value) -> Result<(), String> {
    if value.as_object().unwrap().is_empty() {
        return Ok(());
    }
    if value.get("version").and_then(Value::as_u64) != Some(1)
        || value.get("refs").is_some_and(|v| {
            !v.is_object() || v.as_object().unwrap().values().any(|v| !v.is_string())
        })
        || value.get("records").is_some_and(|v| !v.is_object())
        || value
            .as_object()
            .unwrap()
            .keys()
            .any(|k| !matches!(k.as_str(), "version" | "refs" | "records"))
    {
        return Err("dsh 凭据存储格式未知，未写入".into());
    }
    Ok(())
}

pub(super) fn scopes(record: &Record) -> Result<Vec<NativeScopeModel>, String> {
    require_current_layout(&record.directory)?;
    let home_path = record.directory.join("cordis.patch.yml");
    let home = patch(&home_path)?;
    validate_layers(&home, false)?;
    if home.get(PLUGIN).and_then(|v| v.get("config")).is_some()
        && !record.files.iter().any(|f| {
            f.path == home_path
                && f.owned
                    .iter()
                    .any(|o| o.path.first().is_some_and(|p| p == PLUGIN))
        })
    {
        return Err("dsh 新增了未受管的 home provider 覆盖".into());
    }
    validate_credentials(&common::parse(
        common::read(&record.directory.join(".credentials.yaml"))?.as_deref(),
        Format::YamlMap,
    )?)?;
    let home_managed = record.files.iter().any(|f| {
        f.path == home_path
            && f.owned
                .iter()
                .any(|o| o.path == common::path(&[DEFAULT, "config", "model"]))
    });
    if !home_managed && home.get(DEFAULT).and_then(|v| v.get("config")).is_some() {
        return Err("dsh 新增了未受管的 home 模型覆盖".into());
    }
    // Metadata-only home rows leave the profile's config effective.
    if home
        .get(PLUGIN)
        .is_some_and(|v| v.get("disabled") == Some(&json!(true)))
    {
        return Err("dsh home 层禁用了 llm-pi-ai".into());
    }
    let mut scopes = Vec::new();
    for file in record.files.iter().filter(|f| {
        f.format == Format::Patch
            && f.owned
                .iter()
                .any(|o| o.path == common::path(&[DEFAULT, "config", "model"]))
    }) {
        let current = patch(&file.path)?;
        let default = current
            .get(DEFAULT)
            .and_then(|v| v.get("config"))
            .ok_or_else(|| "dsh 受管模型缺失".to_string())?;
        let provider = default
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("");
        let model = default
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .ok_or_else(|| "dsh 模型未设置".to_string())?;
        if !own_route(provider) {
            return Err("dsh 当前 provider 已改变".into());
        }
        scopes.push(NativeScopeModel {
            scope: scope_name(&record.directory, &file.path),
            model_id: model.into(),
        });
    }
    if scopes.is_empty() {
        return Err("dsh 没有受管 profile".into());
    }
    Ok(scopes)
}

pub(super) fn set_models(
    record: &Record,
    _key: &str,
    scopes: &[NativeScopeModel],
    catalog: &[FetchedModel],
    one: Option<&FetchedModel>,
) -> Result<Record, String> {
    let mut next = record.clone();
    let paths = record
        .files
        .iter()
        .filter(|f| {
            f.format == Format::Patch
                && f.owned
                    .iter()
                    .any(|o| o.path == common::path(&[DEFAULT, "config", "model"]))
        })
        .map(|f| f.path.clone())
        .collect::<Vec<_>>();
    let home_model_managed = paths.contains(&record.directory.join("cordis.patch.yml"));
    for path in paths {
        let name = scope_name(&record.directory, &path);
        let model = one
            .or_else(|| {
                scopes
                    .iter()
                    .find(|s| s.scope == name)
                    .and_then(|s| catalog.iter().find(|m| m.id == s.model_id))
            })
            .ok_or_else(|| "原模型已不在目录中".to_string())?;
        let route = route_for(model)?;
        let value = patch(&path)?;
        let mut entries = value
            .pointer(&format!("/{PLUGIN}/config/providers/{route}/models"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut entry = model_json(model);
        entry.as_object_mut().unwrap().remove("cost");
        entry.as_object_mut().unwrap().remove("reasoning");
        if let Some(at) = entries.iter().position(|e| e["id"] == model.id) {
            // Keep independently edited/native capability fields outside catalog metadata.
            let map = entries[at]
                .as_object_mut()
                .ok_or_else(|| "dsh 模型配置已修改".to_string())?;
            for (key, value) in entry.as_object().unwrap() {
                map.insert(key.clone(), value.clone());
            }
        } else {
            entries.push(entry);
        }
        let mut changes = model_changes(model)?;
        changes.push((
            common::path(&[PLUGIN, "config", "providers", route, "models"]),
            Some(entries.into()),
            false,
        ));
        update(&mut next, path, Format::Patch, false, changes)?;
    }
    // A home route config may override every profile; update its catalog too.
    let home = record.directory.join("cordis.patch.yml");
    if !home_model_managed
        && record.files.iter().any(|f| {
            f.path == home
                && f.owned
                    .iter()
                    .any(|o| o.path.first().is_some_and(|p| p == PLUGIN))
        })
    {
        let model = one
            .or_else(|| {
                scopes
                    .iter()
                    .find(|s| s.scope == "cordis.patch.yml")
                    .and_then(|s| catalog.iter().find(|m| m.id == s.model_id))
            })
            .or_else(|| catalog.first())
            .ok_or_else(|| "模型目录为空".to_string())?;
        let route = route_for(model)?;
        let value = patch(&home)?;
        let mut entries = value
            .pointer(&format!("/{PLUGIN}/config/providers/{route}/models"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !entries.iter().any(|e| e["id"] == model.id) {
            let mut entry = model_json(model);
            entry.as_object_mut().unwrap().remove("reasoning");
            entry.as_object_mut().unwrap().remove("cost");
            entries.push(entry);
        }
        update(
            &mut next,
            home,
            Format::Patch,
            false,
            vec![(
                common::path(&[PLUGIN, "config", "providers", route, "models"]),
                Some(entries.into()),
                false,
            )],
        )?;
    }
    Ok(next)
}
