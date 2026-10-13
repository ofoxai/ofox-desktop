//! 工具的默认路由指向哪个服务商条目，以及这个条目是不是还连着 Ofox 网关。
//!
//! 用户可以在 agent 里选自己配的条目（比如自带 key 的 `ofox`）。只要它的地址还是
//! Ofox 网关，用的就还是 Ofox 的接口，不算离开 Ofox。

use serde_json::Value;

use crate::ofox_apex::mentions_ofox_gateway;

fn gateway(url: Option<&Value>) -> bool {
    url.and_then(Value::as_str)
        .is_some_and(mentions_ofox_gateway)
}

/// Codex：`model_providers.<name>.base_url`。
pub(super) fn codex_is_gateway(config: &Value, name: &str) -> bool {
    gateway(
        config
            .get("model_providers")
            .and_then(|providers| providers.get(name))
            .and_then(|entry| entry.get("base_url")),
    )
}

/// OpenCode：`provider.<id>.options.baseURL`。
pub(super) fn opencode_is_gateway(config: &Value, id: &str) -> bool {
    gateway(
        config
            .get("provider")
            .and_then(|providers| providers.get(id))
            .and_then(|entry| entry.get("options"))
            .and_then(|options| options.get("baseURL")),
    )
}

/// OpenClaw：`models.providers.<id>.baseUrl`。
pub(super) fn openclaw_is_gateway(config: &Value, id: &str) -> bool {
    gateway(
        config
            .get("models")
            .and_then(|models| models.get("providers"))
            .and_then(|providers| providers.get(id))
            .and_then(|entry| entry.get("baseUrl")),
    )
}

/// Hermes：`custom_providers` 列表里 `name` 相同的条目，或 `providers:` 字典里的条目；
/// 地址在 `base_url`（列表写法）或 `api`（字典写法）。
pub(super) fn hermes_is_gateway(config: &Value, name: &str) -> bool {
    let listed = config
        .get("custom_providers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name));
    let keyed = config
        .get("providers")
        .and_then(|providers| providers.get(name));
    [listed, keyed]
        .into_iter()
        .flatten()
        .any(|entry| gateway(entry.get("base_url")) || gateway(entry.get("api")))
}
