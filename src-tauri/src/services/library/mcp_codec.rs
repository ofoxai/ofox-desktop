//! Pure native MCP codecs. Never execute commands or resolve credentials.
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnedFields {
    pub tool: String,
    pub server_id: String,
    pub fields: Vec<(Vec<String>, Value)>,
    pub created_paths: Vec<Vec<String>>,
    /// Explicitly adopted server container, distinct from parents OFox created.
    #[serde(default)]
    pub adopted_server: bool,
}

fn env_name(value: &str) -> Option<&str> {
    let name = value.strip_prefix("${")?.strip_suffix('}')?;
    let mut bytes = name.bytes();
    if !bytes
        .next()
        .is_some_and(|c| c == b'_' || c.is_ascii_alphabetic())
        || !bytes.all(|c| c == b'_' || c.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(name)
}
fn reference(value: &str, tool: &str) -> Result<String, AppError> {
    let mut rest = value;
    let mut result = String::new();
    let mut count = 0;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let end = rest[start..]
            .find('}')
            .ok_or_else(|| error("credentialsNeedMigration"))?
            + start;
        let token = &rest[start..=end];
        let name = env_name(token).ok_or_else(|| error("credentialsNeedMigration"))?;
        result.push_str(&if tool == "opencode" {
            format!("{{env:{name}}}")
        } else {
            token.to_string()
        });
        rest = &rest[end + 1..];
        count += 1;
    }
    result.push_str(rest);
    if count == 0 {
        return Err(error("credentialsNeedMigration"));
    }
    Ok(result)
}

fn optional_reference(value: &str, tool: &str) -> Result<String, AppError> {
    if value.contains("${") {
        reference(value, tool)
    } else {
        Ok(value.to_string())
    }
}
fn native_url(value: &str, tool: &str) -> Result<String, AppError> {
    let native = optional_reference(value, tool)?;
    if value.contains("${") && tool == "codex" {
        return Err(error("referenceUnsupported"));
    }
    if env_name(value).is_some() {
        return Ok(native);
    }
    let mut skeleton = value.to_string();
    while let Some(start) = skeleton.find("${") {
        let end = skeleton[start..]
            .find('}')
            .ok_or_else(|| error("referenceUnsupported"))?
            + start;
        skeleton.replace_range(start..=end, "ofox-env.invalid");
    }
    let parsed = url::Url::parse(&skeleton).map_err(|_| error("invalidMcp"))?;
    if !["http", "https"].contains(&parsed.scheme())
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(error("credentialsNeedMigration"));
    }
    Ok(native)
}
fn header_names<'a>(maps: impl IntoIterator<Item = &'a Value>) -> Result<(), AppError> {
    let mut seen = std::collections::BTreeSet::new();
    for map in maps {
        for name in map.as_object().ok_or_else(|| error("invalidMcp"))?.keys() {
            if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err()
                || !seen.insert(name.to_ascii_lowercase())
            {
                return Err(error("headerConflict"));
            }
        }
    }
    Ok(())
}

fn string_list(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|values| values.iter().all(Value::is_string))
}
fn nonnegative(value: &Value) -> bool {
    value.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0)
}
fn optional_field(
    tool: &str,
    transport: &str,
    key: &str,
    value: &Value,
) -> Result<Value, AppError> {
    let remote = transport != "stdio";
    let valid = match (tool, key) {
        ("claude", "timeout") => value.as_u64().is_some_and(|n| n >= 1000),
        ("gemini" | "hermes", "timeout") => nonnegative(value),
        ("opencode", "timeout") => value.as_u64().is_some_and(|n| n > 0),
        ("codex", "startup_timeout_sec" | "tool_timeout_sec") | ("hermes", "connect_timeout") => {
            nonnegative(value)
        }
        ("codex", "enabled_tools" | "disabled_tools")
        | ("gemini", "includeTools" | "excludeTools") => string_list(value),
        ("gemini", "trust") => value.is_boolean(),
        ("opencode", "oauth") if remote => {
            if value == &json!(false) {
                true
            } else if let Some(options) = value.as_object() {
                for (key, field) in options {
                    if key == "clientSecret" {
                        return Err(error("credentialsNeedMigration"));
                    }
                    if !match key.as_str() {
                        "clientId" | "scope" | "redirectUri" => field.is_string(),
                        "callbackPort" => field.as_u64().is_some_and(|n| (1..=65535).contains(&n)),
                        _ => false,
                    } {
                        return Err(error("fieldUnsupported"));
                    }
                }
                true
            } else {
                false
            }
        }
        ("hermes", "auth") if remote => value.as_str() == Some("oauth"),
        // OAuth authorization remains native. Secret/client registration options
        // need a separate migration contract, never a lossy cross-tool conversion.
        ("hermes", "oauth") if remote => {
            value.as_object().is_some_and(|options| options.is_empty())
        }
        ("hermes", "tools") => value.as_object().is_some_and(|options| {
            options.iter().all(|(key, field)| match key.as_str() {
                "include" | "exclude" => field.is_string() || string_list(field),
                "resources" | "prompts" => field.is_boolean(),
                _ => false,
            })
        }),
        ("hermes", "sampling") => value.as_object().is_some_and(|options| {
            options.iter().all(|(key, field)| match key.as_str() {
                "enabled" | "expose_client_tools" => field.is_boolean(),
                "max_rpm" | "max_tokens_cap" => field.as_u64().is_some_and(|n| n > 0),
                "max_tool_rounds" => field.as_u64().is_some(),
                "timeout" => nonnegative(field),
                "model" => field.is_string(),
                "allowed_models" => string_list(field),
                "log_level" => field.as_str().is_some_and(|level| {
                    ["DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"].contains(&level)
                }),
                _ => false,
            })
        }),
        _ => return Err(error("fieldUnsupported")),
    };
    if !valid {
        return Err(error("invalidMcp"));
    }
    if tool == "opencode" && key == "oauth" {
        if let Some(options) = value.as_object() {
            let mut mapped = options.clone();
            for field in mapped.values_mut() {
                if let Some(text) = field.as_str() {
                    *field = json!(optional_reference(text, tool)?);
                }
            }
            return Ok(json!(mapped));
        }
    }
    if tool == "hermes" && key == "timeout" {
        return Ok(json!(value.as_f64().expect("validated number") / 1000.0));
    }
    Ok(value.clone())
}

fn verify_native_headers(server: &Value, tool: &str) -> Result<(), AppError> {
    if tool == "codex" {
        header_names(
            ["http_headers", "env_http_headers"]
                .iter()
                .filter_map(|key| server.get(*key)),
        )?;
        if server.get("bearer_token_env_var").is_some()
            && ["http_headers", "env_http_headers"]
                .iter()
                .filter_map(|key| server.get(*key).and_then(Value::as_object))
                .any(|headers| {
                    headers
                        .keys()
                        .any(|key| key.eq_ignore_ascii_case("authorization"))
                })
        {
            return Err(error("headerConflict"));
        }
    } else if let Some(headers) = server.get("headers") {
        header_names([headers])?;
    }
    Ok(())
}

pub fn native_spec(tool: &str, spec: &Value) -> Result<Value, AppError> {
    if !["claude", "codex", "gemini", "opencode", "hermes"].contains(&tool) {
        return Err(error("targetUnsupported"));
    }
    let obj = spec.as_object().ok_or_else(|| error("invalidMcp"))?;
    if obj.get("type").is_some_and(|value| !value.is_string()) {
        return Err(error("invalidMcp"));
    }
    let transport = obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(if obj.contains_key("url") {
            "http"
        } else {
            "stdio"
        });
    if !["stdio", "http", "sse"].contains(&transport) || (tool == "codex" && transport == "sse") {
        return Err(error("transportUnsupported"));
    }
    let mut native = Map::new();
    let common = ["type", "command", "args", "cwd", "env", "url", "headers"];
    for (key, value) in obj {
        if common.contains(&key.as_str()) || key == "enabled" {
            continue;
        }
        let allowed = match tool {
            "claude" => ["timeout"].contains(&key.as_str()),
            "codex" => [
                "startup_timeout_sec",
                "tool_timeout_sec",
                "enabled_tools",
                "disabled_tools",
            ]
            .contains(&key.as_str()),
            "gemini" => {
                ["timeout", "trust", "includeTools", "excludeTools"].contains(&key.as_str())
            }
            "opencode" => ["timeout", "oauth"].contains(&key.as_str()),
            "hermes" => [
                "timeout",
                "connect_timeout",
                "tools",
                "sampling",
                "auth",
                "oauth",
            ]
            .contains(&key.as_str()),
            _ => false,
        };
        if !allowed {
            return Err(error("fieldUnsupported"));
        }
        native.insert(key.clone(), optional_field(tool, transport, key, value)?);
    }
    if transport == "stdio" {
        if obj.contains_key("url") || obj.contains_key("headers") {
            return Err(error("invalidMcp"));
        }
        let command = obj
            .get("command")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| error("invalidMcp"))?;
        let args = match obj.get("args") {
            Some(value) => value
                .as_array()
                .filter(|a| a.iter().all(Value::is_string))
                .cloned()
                .ok_or_else(|| error("invalidMcp"))?,
            None => vec![],
        };
        if tool == "codex"
            && (command.contains("${")
                || args.iter().any(|arg| arg.as_str().unwrap().contains("${")))
        {
            return Err(error("referenceUnsupported"));
        }
        if tool == "opencode" {
            let mut array = vec![json!(optional_reference(command, tool)?)];
            for arg in args {
                array.push(json!(optional_reference(
                    arg.as_str().expect("validated string"),
                    tool
                )?));
            }
            native.insert("type".into(), json!("local"));
            native.insert("command".into(), json!(array));
            native.insert("enabled".into(), json!(true));
        } else {
            native.insert("command".into(), json!(command));
            if !args.is_empty() {
                native.insert("args".into(), json!(args));
            }
            if ["claude", "gemini"].contains(&tool) {
                native.insert("type".into(), json!("stdio"));
            }
        }
        if let Some(cwd) = obj.get("cwd") {
            if tool == "claude" {
                return Err(error("fieldUnsupported"));
            }
            if !cwd.is_string() {
                return Err(error("invalidMcp"));
            }
            let cwd = cwd.as_str().expect("validated string");
            if tool == "codex" && cwd.contains("${") {
                return Err(error("referenceUnsupported"));
            }
            native.insert("cwd".into(), json!(optional_reference(cwd, tool)?));
        }
        if let Some(env) = obj.get("env") {
            let mut vars = Vec::new();
            let mut mapped = Map::new();
            for (key, value) in env.as_object().ok_or_else(|| error("invalidMcp"))? {
                let value = value.as_str().ok_or_else(|| error("invalidMcp"))?;
                if tool == "codex" {
                    // Codex passes a variable through by name or takes a literal; it cannot expand.
                    if env_name(value) == Some(key.as_str()) {
                        vars.push(json!(key));
                    } else if value.contains("${") {
                        return Err(error("referenceUnsupported"));
                    } else {
                        mapped.insert(key.clone(), json!(value));
                    }
                } else {
                    mapped.insert(key.clone(), json!(optional_reference(value, tool)?));
                }
            }
            if !vars.is_empty() {
                native.insert("env_vars".into(), json!(vars));
            }
            if !mapped.is_empty() {
                native.insert(
                    if tool == "opencode" {
                        "environment"
                    } else {
                        "env"
                    }
                    .into(),
                    json!(mapped),
                );
            }
        }
    } else {
        if obj.contains_key("command")
            || obj.contains_key("args")
            || obj.contains_key("env")
            || obj.contains_key("cwd")
        {
            return Err(error("invalidMcp"));
        }
        let url = obj
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| error("invalidMcp"))?;
        native.insert("url".into(), json!(native_url(url, tool)?));
        match tool {
            "claude" | "gemini" => {
                native.insert("type".into(), json!(transport));
            }
            "opencode" => {
                native.insert("type".into(), json!("remote"));
                native.insert("enabled".into(), json!(true));
            }
            "hermes" if transport == "sse" => {
                native.insert("transport".into(), json!("sse"));
            }
            _ => {}
        }
        if let Some(headers) = obj.get("headers") {
            header_names([headers])?;
            let mut mapped = Map::new();
            let mut by_name = Map::new();
            for (key, value) in headers.as_object().ok_or_else(|| error("invalidMcp"))? {
                let value = value.as_str().ok_or_else(|| error("invalidMcp"))?;
                if tool != "codex" {
                    mapped.insert(key.clone(), json!(optional_reference(value, tool)?));
                    continue;
                }
                if key.eq_ignore_ascii_case("authorization") {
                    if let Some(name) = value.strip_prefix("Bearer ").and_then(env_name) {
                        native.insert("bearer_token_env_var".into(), json!(name));
                        continue;
                    }
                }
                if let Some(name) = env_name(value) {
                    by_name.insert(key.clone(), json!(name));
                } else if value.contains("${") {
                    return Err(error("referenceUnsupported"));
                } else {
                    mapped.insert(key.clone(), json!(value));
                }
            }
            if !mapped.is_empty() {
                native.insert(
                    if tool == "codex" {
                        "http_headers"
                    } else {
                        "headers"
                    }
                    .into(),
                    json!(mapped),
                );
            }
            if !by_name.is_empty() {
                native.insert("env_http_headers".into(), json!(by_name));
            }
        }
    }
    if tool == "hermes" {
        native.insert("enabled".into(), json!(true));
    }
    Ok(json!(native))
}

pub fn canonical_spec(tool: &str, native: &Value) -> Result<Value, AppError> {
    let mut map = native
        .as_object()
        .ok_or_else(|| error("nativeInvalid"))?
        .clone();
    if tool == "opencode" {
        match map.get("type").and_then(Value::as_str) {
            Some("local") => {
                let command = map
                    .remove("command")
                    .and_then(|v| v.as_array().cloned())
                    .filter(|a| !a.is_empty() && a.iter().all(Value::is_string))
                    .ok_or_else(|| error("nativeInvalid"))?;
                map.insert("command".into(), command[0].clone());
                if command.len() > 1 {
                    map.insert("args".into(), json!(&command[1..]));
                }
                if let Some(env) = map.remove("environment") {
                    map.insert("env".into(), env);
                }
                map.insert("type".into(), json!("stdio"));
            }
            Some("remote") => {
                map.insert("type".into(), json!("http"));
            }
            _ => return Err(error("nativeInvalid")),
        }
        fn canonical_references(value: &mut Value) {
            match value {
                Value::String(text) => *text = text.replace("{env:", "${"),
                Value::Array(values) => values.iter_mut().for_each(canonical_references),
                Value::Object(map) => map.values_mut().for_each(canonical_references),
                _ => {}
            }
        }
        for value in map.values_mut() {
            canonical_references(value);
        }
    }
    if tool == "codex" {
        header_names(
            ["http_headers", "env_http_headers"]
                .iter()
                .filter_map(|key| map.get(*key)),
        )
        .map_err(|_| error("credentialFallbackRequiresMigration"))?;
        let bearer = map.remove("bearer_token_env_var");
        if let Some(bearer) = &bearer {
            let name = bearer.as_str().ok_or_else(|| error("nativeInvalid"))?;
            if env_name(&format!("${{{name}}}")).is_none() {
                return Err(error("nativeInvalid"));
            }
            if ["http_headers", "env_http_headers"]
                .iter()
                .filter_map(|key| map.get(*key).and_then(Value::as_object))
                .any(|headers| {
                    headers
                        .keys()
                        .any(|key| key.eq_ignore_ascii_case("authorization"))
                })
            {
                return Err(error("credentialFallbackRequiresMigration"));
            }
        }
        if let Some(headers) = map.remove("http_headers") {
            map.insert("headers".into(), headers);
        }
        if let Some(headers) = map.remove("env_http_headers") {
            let target = map
                .entry("headers")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| error("nativeInvalid"))?;
            for (key, value) in headers.as_object().ok_or_else(|| error("nativeInvalid"))? {
                if target.contains_key(key) {
                    return Err(error("credentialFallbackRequiresMigration"));
                }
                target.insert(
                    key.clone(),
                    json!(format!(
                        "${{{}}}",
                        value.as_str().ok_or_else(|| error("nativeInvalid"))?
                    )),
                );
            }
        }
        if let Some(bearer) = bearer {
            let headers = map
                .entry("headers")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| error("nativeInvalid"))?;
            headers.insert(
                "Authorization".into(),
                json!(format!(
                    "Bearer ${{{}}}",
                    bearer.as_str().expect("validated name")
                )),
            );
        }
        if let Some(vars) = map.remove("env_vars") {
            let target = map
                .entry("env")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| error("nativeInvalid"))?;
            for value in vars.as_array().ok_or_else(|| error("nativeInvalid"))? {
                let name = value.as_str().ok_or_else(|| error("nativeInvalid"))?;
                target
                    .entry(name.to_string())
                    .or_insert_with(|| json!(format!("${{{name}}}")));
            }
        }
    }
    if tool == "gemini" {
        if let Some(url) = map.remove("httpUrl") {
            map.insert("url".into(), url);
            map.insert("type".into(), json!("http"));
        }
    }
    if tool == "hermes" {
        if let Some(timeout) = map.get_mut("timeout") {
            let seconds = timeout
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or_else(|| error("nativeInvalid"))?;
            let milliseconds = seconds * 1000.0;
            if !milliseconds.is_finite() {
                return Err(error("nativeInvalid"));
            }
            *timeout = if milliseconds.fract() == 0.0 && milliseconds <= 9_007_199_254_740_991.0 {
                json!(milliseconds as u64)
            } else {
                json!(milliseconds)
            };
        }
        if map
            .remove("transport")
            .and_then(|v| v.as_str().map(str::to_owned))
            .as_deref()
            == Some("sse")
        {
            map.insert("type".into(), json!("sse"));
        }
    }
    normalize_map(&mut map);
    Ok(json!(map))
}

/// The implied transport is spelled out and empty env/args/headers are dropped,
/// so two spellings of one definition compare equal. `enabled` is a tool's own
/// switch, not part of the definition.
fn normalize_map(map: &mut Map<String, Value>) {
    map.remove("enabled");
    if !map.contains_key("type") {
        let transport = if map.contains_key("url") {
            "http"
        } else {
            "stdio"
        };
        map.insert("type".into(), json!(transport));
    }
    for key in ["env", "args", "headers"] {
        if map.get(key).is_some_and(|v| {
            v.as_object().is_some_and(|m| m.is_empty())
                || v.as_array().is_some_and(|a| a.is_empty())
        }) {
            map.remove(key);
        }
    }
}
/// Env or header values in plain text, or a URL query (keys often ride there).
pub fn has_literal_values(spec: &Value) -> bool {
    let literal = |key: &str| {
        spec.get(key).and_then(Value::as_object).is_some_and(|map| {
            map.values()
                .any(|value| value.as_str().is_some_and(|text| !text.contains("${")))
        })
    };
    literal("env")
        || literal("headers")
        || spec
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| url.contains('?'))
}
/// A local server an app ships inside its own bundle (Codex's node_repl and
/// computer-use). The app rewrites it on update, so the library leaves it alone.
pub fn app_owned(spec: &Value) -> bool {
    if spec.get("url").is_some() {
        return false;
    }
    let command = spec
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .replace('\\', "/")
        .to_lowercase();
    [
        ".app/contents/",
        "/windowsapps/",
        "/cua_node/",
        "/openai/codex/runtimes/",
    ]
    .iter()
    .any(|marker| command.contains(marker))
}
pub(crate) fn normalize_spec(spec: &Value) -> Value {
    let Some(map) = spec.as_object() else {
        return spec.clone();
    };
    let mut map = map.clone();
    normalize_map(&mut map);
    Value::Object(map)
}

pub fn section(tool: &str) -> &'static str {
    match tool {
        "codex" => "mcp_servers",
        "hermes" => "mcp_servers",
        "opencode" => "mcp",
        _ => "mcpServers",
    }
}
pub fn parse_native(text: &str, tool: &str) -> Result<Value, AppError> {
    let body = if tool == "hermes" {
        text.strip_prefix('\u{feff}').unwrap_or(text)
    } else {
        text
    };
    let value = if tool == "codex" {
        let value: toml::Value = toml::from_str(body).map_err(|_| error("nativeInvalid"))?;
        serde_json::to_value(value).map_err(|_| error("nativeInvalid"))?
    } else if tool == "hermes" && body.trim().is_empty() {
        // An empty YAML document is a config with nothing set.
        json!({})
    } else if tool == "hermes" {
        super::yaml_paths::validate_native_mcp(body)?;
        serde_yaml::from_str::<Value>(body).map_err(|_| error("nativeInvalid"))?
    } else {
        match tool {
            "claude" => super::native_json::parse(body, false, false, true)?,
            "gemini" => super::native_json::parse(body, true, false, false)?,
            "opencode" => super::native_json::parse(body, true, true, false)?,
            _ => return Err(error("targetUnsupported")),
        }
    };
    if !value.is_object() {
        return Err(error("nativeInvalid"));
    }
    if value.get(section(tool)).is_some_and(|v| !v.is_object()) {
        return Err(error("nativeInvalid"));
    }
    Ok(value)
}
fn leaf<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(root, |value, key| value.get(key))
}
fn flatten(value: &Value, path: Vec<String>, fields: &mut Vec<(Vec<String>, Value)>) {
    if let Some(map) = value.as_object() {
        for (key, value) in map {
            let mut child = path.clone();
            child.push(key.clone());
            flatten(value, child, fields);
        }
    } else {
        fields.push((path, value.clone()));
    }
}
pub fn validate_owned(owned: &OwnedFields) -> Result<(), AppError> {
    if !["claude", "codex", "gemini", "opencode", "hermes"].contains(&owned.tool.as_str())
        || owned.server_id.is_empty()
        || owned.fields.is_empty()
    {
        return Err(error("stateUnreadable"));
    }
    let mut paths = std::collections::BTreeSet::new();
    for (path, value) in &owned.fields {
        if path.len() < 3
            || path[0] != section(&owned.tool)
            || path[1] != owned.server_id
            || value.is_object()
            || !paths.insert(path)
        {
            return Err(error("stateUnreadable"));
        }
    }
    for path in &owned.created_paths {
        // The servers section itself, when this write created it, or a path inside the server.
        let section_only = path.len() == 1 && path[0] == section(&owned.tool);
        if !section_only
            && (path.len() < 2 || path[0] != section(&owned.tool) || path[1] != owned.server_id)
        {
            return Err(error("stateUnreadable"));
        }
    }
    Ok(())
}
pub fn verify_owned_text(text: &str, owned: &OwnedFields) -> Result<(), AppError> {
    validate_owned(owned)?;
    let root = parse_native(text, &owned.tool)?;
    verify_native_headers(&root[section(&owned.tool)][&owned.server_id], &owned.tool)?;
    for (path, value) in &owned.fields {
        if leaf(&root, path) != Some(value) {
            return Err(error("mcpFieldsConflict"));
        }
    }
    Ok(())
}
fn remove_semantic(root: &mut Value, path: &[String]) {
    if path.len() == 1 {
        if let Some(map) = root.as_object_mut() {
            map.remove(&path[0]);
        }
    } else if let Some(child) = root.get_mut(&path[0]) {
        remove_semantic(child, &path[1..]);
    }
}

/// Adoption owns only matching desired leaves. It never rewrites the native text.
pub fn adopt_server(
    text: &str,
    tool: &str,
    id: &str,
    spec: &Value,
) -> Result<OwnedFields, AppError> {
    let root = parse_native(text, tool)?;
    let native = root
        .get(section(tool))
        .and_then(|servers| servers.get(id))
        .ok_or_else(|| error("mcpNameConflict"))?;
    verify_native_headers(native, tool)?;
    let mut fields = vec![];
    flatten(spec, vec![section(tool).into(), id.into()], &mut fields);
    let owned = OwnedFields {
        tool: tool.into(),
        server_id: id.into(),
        fields,
        created_paths: vec![],
        adopted_server: true,
    };
    verify_owned_text(text, &owned)?;
    Ok(owned)
}

fn withdrawal_parents(owned: &OwnedFields) -> Vec<Vec<String>> {
    let mut paths = owned.created_paths.clone();
    if owned.adopted_server {
        for (field, _) in &owned.fields {
            for count in 2..field.len() {
                let parent = field[..count].to_vec();
                if !paths.contains(&parent) {
                    paths.push(parent);
                }
            }
        }
    }
    paths.sort_by_key(|path| std::cmp::Reverse(path.len()));
    paths
}

pub fn patch_server(
    current: Option<&str>,
    tool: &str,
    id: &str,
    owned: Option<&OwnedFields>,
    spec: Option<&Value>,
) -> Result<(String, Option<OwnedFields>), AppError> {
    if id.is_empty()
        || id.len() > 256
        || !["claude", "codex", "gemini", "opencode", "hermes"].contains(&tool)
    {
        return Err(error("invalidMcp"));
    }
    let empty = if tool == "codex" { "" } else { "{}" };
    let text = current.unwrap_or(empty);
    let root = parse_native(text, tool)?;
    if owned.is_none() && root.get(section(tool)).and_then(|v| v.get(id)).is_some() {
        return Err(error("mcpNameConflict"));
    }
    let mut old = BTreeMap::new();
    if let Some(owned) = owned {
        validate_owned(owned)?;
        if owned.tool != tool || owned.server_id != id {
            return Err(error("ownershipChanged"));
        }
        verify_native_headers(&root[section(tool)][id], tool)?;
        for (path, value) in &owned.fields {
            if leaf(&root, path) != Some(value) {
                return Err(error("mcpFieldsConflict"));
            }
            old.insert(path.clone(), value.clone());
        }
    }
    if spec.is_none() {
        if let Some(owned) = owned {
            let mut remaining = root.clone();
            for (path, _) in &owned.fields {
                remove_semantic(&mut remaining, path);
            }
            for path in withdrawal_parents(owned) {
                if leaf(&remaining, &path)
                    .is_some_and(|value| value.as_object().is_some_and(|map| map.is_empty()))
                {
                    remove_semantic(&mut remaining, &path);
                }
            }
            // Removing the launch fields would strand the user's extensions in
            // an unusable server. Keep both the file and ownership proof intact.
            if remaining
                .get(section(tool))
                .and_then(|v| v.get(id))
                .is_some()
            {
                return Err(error("withdrawHasUserFields"));
            }
        }
    }
    let mut fields = vec![];
    if let Some(spec) = spec {
        flatten(spec, vec![section(tool).into(), id.into()], &mut fields);
    }
    for (path, _) in &fields {
        if !old.contains_key(path) && leaf(&root, path).is_some() {
            return Err(error("mcpFieldsConflict"));
        }
    }
    let new: BTreeMap<_, _> = fields.iter().cloned().collect();
    let edits: Vec<_> = old
        .keys()
        .chain(new.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|path| leaf(&root, path) != new.get(*path))
        .map(|path| (path.clone(), new.get(path).cloned()))
        .collect();
    let mut created_paths = owned
        .map(|owned| owned.created_paths.clone())
        .unwrap_or_default();
    for (path, _) in &fields {
        for count in 1..path.len() {
            let parent = path[..count].to_vec();
            if leaf(&root, &parent).is_none() && !created_paths.contains(&parent) {
                created_paths.push(parent);
            }
        }
    }
    let render = |text: &str, edits: &[(Vec<String>, Option<Value>)]| -> Result<String, AppError> {
        if tool == "codex" {
            edit_toml(text, edits)
        } else if tool == "hermes" {
            super::yaml_paths::edit(text, edits)
        } else {
            crate::json5_sections::edit_object_paths(
                text,
                crate::json5_sections::KeyStyle::Json,
                edits,
            )
        }
    };
    let mut result = render(text, &edits)?;
    let mut cleanup = owned
        .map(|owned| owned.created_paths.clone())
        .unwrap_or_default();
    if let Some(owned) = owned.filter(|owned| owned.adopted_server) {
        cleanup = withdrawal_parents(owned);
    }
    cleanup.sort_by_key(|path| std::cmp::Reverse(path.len()));
    for path in cleanup {
        if spec.is_some() && new.keys().any(|field| field.starts_with(&path)) {
            continue;
        }
        let root = parse_native(&result, tool)?;
        if leaf(&root, &path)
            .is_some_and(|value| value.as_object().is_some_and(|map| map.is_empty()))
        {
            result = render(&result, &[(path.clone(), None)])?;
            created_paths.retain(|created| created != &path);
        }
    }
    let after = parse_native(&result, tool)?;
    if spec.is_some() {
        verify_native_headers(&after[section(tool)][id], tool)?;
    }
    for (path, value) in &edits {
        if leaf(&after, path) != value.as_ref() {
            return Err(error("verifyFailed"));
        }
    }
    let receipt = spec.map(|_| OwnedFields {
        tool: tool.into(),
        server_id: id.into(),
        fields,
        created_paths,
        adopted_server: owned.is_some_and(|owned| owned.adopted_server),
    });
    Ok((result, receipt))
}

fn toml_value(value: &Value) -> Result<toml_edit::Value, AppError> {
    Ok(match value {
        Value::String(v) => toml_edit::Value::from(v.as_str()),
        Value::Bool(v) => toml_edit::Value::from(*v),
        Value::Number(v) => {
            if let Some(v) = v.as_i64() {
                toml_edit::Value::from(v)
            } else if let Some(v) = v.as_f64() {
                toml_edit::Value::from(v)
            } else {
                return Err(error("invalidMcp"));
            }
        }
        Value::Array(values) => {
            let mut array = toml_edit::Array::new();
            for value in values {
                array.push(toml_value(value)?);
            }
            toml_edit::Value::Array(array)
        }
        _ => return Err(error("invalidMcp")),
    })
}
fn preserve_toml_array_comments(value: &toml_edit::Value, comments: &mut String) {
    if let Some(array) = value.as_array() {
        for value in array.iter() {
            for text in [value.decor().prefix(), value.decor().suffix()]
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str())
            {
                if text.contains('#') {
                    comments.push_str(text);
                    if !comments.ends_with('\n') {
                        comments.push('\n');
                    }
                }
            }
            preserve_toml_array_comments(value, comments);
        }
        if let Some(text) = array.trailing().as_str() {
            if text.contains('#') {
                comments.push_str(text);
                if !comments.ends_with('\n') {
                    comments.push('\n');
                }
            }
        }
    }
}

fn toml_path(
    table: &mut dyn toml_edit::TableLike,
    path: &[String],
    value: Option<&Value>,
    comments: &mut String,
) -> Result<(), AppError> {
    let key = &path[0];
    if path.len() > 1 {
        if !table.contains_key(key) {
            if value.is_none() {
                return Ok(());
            }
            table.insert(key, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let child = table
            .get_mut(key)
            .and_then(toml_edit::Item::as_table_like_mut)
            .ok_or_else(|| error("nativeInvalid"))?;
        return toml_path(child, &path[1..], value, comments);
    }
    if let Some(value) = value {
        let mut new = toml_value(value)?;
        if let Some(old) = table.get(key).and_then(toml_edit::Item::as_value) {
            preserve_toml_array_comments(old, comments);
            *new.decor_mut() = old.decor().clone();
        }
        table.insert(key, toml_edit::Item::Value(new));
    } else {
        if let Some((key, item)) = table.get_key_value(key) {
            let mut decorations = vec![key.leaf_decor(), key.dotted_decor()];
            if let Some(value) = item.as_value() {
                preserve_toml_array_comments(value, comments);
                decorations.push(value.decor());
            } else if let Some(table) = item.as_table() {
                decorations.push(table.decor());
            }
            for decoration in decorations {
                for text in [decoration.prefix(), decoration.suffix()]
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str())
                {
                    if text.contains('#') {
                        comments.push_str(text);
                        if !comments.ends_with('\n') {
                            comments.push('\n');
                        }
                    }
                }
            }
        }
        table.remove(key);
    }
    Ok(())
}
fn edit_toml(text: &str, edits: &[(Vec<String>, Option<Value>)]) -> Result<String, AppError> {
    let (bom, body) = text
        .strip_prefix('\u{feff}')
        .map_or(("", text), |body| ("\u{feff}", body));
    let mut doc: toml_edit::DocumentMut = body.parse().map_err(|_| error("nativeInvalid"))?;
    if doc.to_string() != body {
        return Err(error("nativeInvalid"));
    }
    let mut comments = String::new();
    for (path, value) in edits {
        toml_path(doc.as_table_mut(), path, value.as_ref(), &mut comments)?;
    }
    if !comments.is_empty() {
        doc.set_trailing(format!(
            "{}\n{comments}",
            doc.trailing().as_str().unwrap_or("")
        ));
    }
    let result = doc.to_string();
    let result = if body.contains("\r\n") {
        result.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        result
    };
    Ok(format!("{bom}{result}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn opencode_oauth_references_round_trip_in_native_syntax() {
        let native = json!({"type":"remote", "url":"https://example.org", "oauth":{"clientId":"{env:CLIENT_ID}","scope":"{env:SCOPES}", "redirectUri":"http://localhost/{env:PATH}"}});
        let canonical = canonical_spec("opencode", &native).unwrap();
        assert_eq!(
            native_spec("opencode", &canonical).unwrap()["oauth"],
            native["oauth"]
        );
    }
    #[test]
    fn timeout_has_one_canonical_unit_and_optional_fields_obey_native_contracts() {
        let spec = json!({"command":"server", "timeout":30_000});
        let hermes = native_spec("hermes", &spec).unwrap();
        assert_eq!(hermes["timeout"].as_f64(), Some(30.0));
        assert_eq!(
            canonical_spec("hermes", &hermes).unwrap()["timeout"].as_f64(),
            Some(30_000.0)
        );
        assert_eq!(native_spec("gemini", &spec).unwrap()["timeout"], 30_000);
        let imported =
            canonical_spec("hermes", &json!({"command":"server", "timeout":30})).unwrap();
        assert!(native_spec("claude", &imported).is_ok());
        assert!(native_spec("opencode", &imported).is_ok());
        for (tool, spec) in [
            ("claude", json!({"command":"s", "cwd":"/work"})),
            ("claude", json!({"command":"s", "timeout":999})),
            ("gemini", json!({"command":"s", "trust":"false"})),
            ("gemini", json!({"command":"s", "includeTools":[4]})),
            ("codex", json!({"command":"s", "tool_timeout_sec":-1})),
            ("codex", json!({"command":"s", "enabled_tools":true})),
            ("opencode", json!({"command":"s", "timeout":"oops"})),
            (
                "opencode",
                json!({"url":"https://example.org", "oauth":true}),
            ),
            ("opencode", json!({"command":"s", "oauth":false})),
            (
                "opencode",
                json!({"url":"https://example.org", "oauth":{"client_id":"a"}}),
            ),
            ("hermes", json!({"command":"s", "sampling":false})),
            ("hermes", json!({"command":"s", "roots":[]})),
            (
                "hermes",
                json!({"url":"https://example.org", "auth":"bearer"}),
            ),
            (
                "hermes",
                json!({"url":"https://example.org", "oauth":{"clientId":"a"}}),
            ),
        ] {
            assert!(native_spec(tool, &spec).is_err(), "accepted {tool}: {spec}");
        }
        assert!(native_spec("opencode", &json!({"command":"s", "cwd":"/work"})).is_ok());
        assert!(native_spec(
            "opencode",
            &json!({"url":"https://example.org", "oauth":false})
        )
        .is_ok());
    }

    #[test]
    fn user_added_header_alias_prevents_false_applied_state() {
        let spec = native_spec(
            "codex",
            &json!({"url":"https://example.org", "headers":{"Authorization":"Bearer ${TOKEN}"}}),
        )
        .unwrap();
        let (text, receipt) = patch_server(None, "codex", "s", None, Some(&spec)).unwrap();
        let extended = format!("{text}\n[mcp_servers.s.http_headers]\nauthorization = 'user'\n");
        assert!(verify_owned_text(&extended, receipt.as_ref().unwrap()).is_err());
        assert!(
            patch_server(Some(&extended), "codex", "s", receipt.as_ref(), Some(&spec)).is_err()
        );
    }
    use super::*;
    #[test]
    fn references_round_trip_in_commands_args_and_dynamic_urls() {
        let canonical = json!({"command":"${BIN}","args":["--path=${DIR}"],"cwd":"${WORK_DIR}"});
        let native = native_spec("opencode", &canonical).unwrap();
        assert_eq!(native["command"], json!(["{env:BIN}", "--path={env:DIR}"]));
        assert_eq!(native["cwd"], "{env:WORK_DIR}");
        assert_eq!(
            canonical_spec("opencode", &native).unwrap(),
            json!({"type":"stdio","command":"${BIN}","args":["--path=${DIR}"],"cwd":"${WORK_DIR}"})
        );
        let remote =
            json!({"type":"http","url":"${MCP_URL}","headers":{"Authorization":"Bearer ${TOKEN}"}});
        for tool in ["claude", "gemini", "opencode", "hermes"] {
            let native = native_spec(tool, &remote).unwrap();
            assert_eq!(
                native["url"],
                if tool == "opencode" {
                    "{env:MCP_URL}"
                } else {
                    "${MCP_URL}"
                }
            );
            assert_eq!(canonical_spec(tool, &native).unwrap()["url"], "${MCP_URL}");
        }
        assert!(native_spec("codex", &remote).is_err());
        let bearer=native_spec("codex",&json!({"type":"http","url":"https://example.org/mcp","headers":{"Authorization":"Bearer ${TOKEN}"}})).unwrap();
        assert_eq!(
            canonical_spec("codex", &bearer).unwrap()["headers"]["Authorization"],
            "Bearer ${TOKEN}"
        );
    }
    #[test]
    fn header_case_aliases_cannot_lose_native_credential_precedence() {
        assert!(canonical_spec("codex",&json!({"url":"https://example.org/mcp","http_headers":{"authorization":"private-fallback"},"env_http_headers":{"Authorization":"TOKEN"}})).is_err());
        for tool in ["claude", "codex", "gemini", "opencode", "hermes"] {
            assert!(native_spec(tool,&json!({"type":"http","url":"https://example.org/mcp","headers":{"authorization":"${ONE}","Authorization":"${TWO}"}})).is_err());
        }
    }

    #[test]
    fn non_string_transport_is_never_inferred_as_stdio() {
        for tool in ["claude", "codex", "gemini", "opencode", "hermes"] {
            for value in [json!(false), json!(17), json!({}), json!(null)] {
                assert!(native_spec(tool, &json!({"type":value,"command":"server"})).is_err());
            }
        }
    }

    #[test]
    fn hermes_yaml_11_ambiguous_strings_and_keys_are_quoted() {
        for text in [
            "on",
            "YES",
            "n",
            "off",
            "0123",
            "-0123",
            "+0123",
            "0o77",
            "1:20",
            "1:20.5",
            "2026-10-15",
        ] {
            let native = native_spec(
                "hermes",
                &json!({"command":text,"args":[text],"env":{text:"${TOKEN}"}}),
            )
            .unwrap();
            let (applied, _) = patch_server(None, "hermes", "demo", None, Some(&native)).unwrap();
            assert!(
                applied.contains(&format!("\"command\": \"{text}\"")),
                "{text}: {applied}"
            );
            assert!(
                applied.contains(&format!("\"{text}\": ")),
                "{text}: {applied}"
            );
        }
    }
    #[test]
    fn hermes_yaml_11_ambiguous_native_mcp_scalars_require_manual_migration() {
        for text in [
            "on",
            "YES",
            "n",
            "off",
            "0123",
            "-0123",
            "+0123",
            "0o77",
            "1:20",
            "1:20.5",
            "2026-10-15",
        ] {
            assert!(
                parse_native(
                    &format!("mcp_servers:\n  demo:\n    command: {text}\n"),
                    "hermes"
                )
                .is_err(),
                "{text}"
            );
            assert!(
                parse_native(
                    &format!("mcp_servers:\n  demo:\n    env:\n      {text}: value\n"),
                    "hermes"
                )
                .is_err(),
                "key {text}"
            );
        }
        assert!(parse_native(
            "model: on\nmcp_servers:\n  demo:\n    command: server\n",
            "hermes"
        )
        .is_ok());
    }

    #[test]
    fn withdrawing_a_user_extended_server_refuses_an_invalid_native_entry() {
        for tool in ["claude", "codex", "gemini", "opencode", "hermes"] {
            let spec = native_spec(tool, &json!({"command":"server"})).unwrap();
            let (applied, owned) = patch_server(None, tool, "demo", None, Some(&spec)).unwrap();
            let edit = vec![(
                vec![section(tool).into(), "demo".into(), "timeout".into()],
                Some(json!(13)),
            )];
            let current = if tool == "codex" {
                edit_toml(&applied, &edit).unwrap()
            } else if tool == "hermes" {
                super::super::yaml_paths::edit(&applied, &edit).unwrap()
            } else {
                crate::json5_sections::edit_object_paths(
                    &applied,
                    crate::json5_sections::KeyStyle::Json,
                    &edit,
                )
                .unwrap()
            };
            assert!(
                patch_server(Some(&current), tool, "demo", owned.as_ref(), None).is_err(),
                "{tool} leaves timeout without command/url"
            );
        }
    }

    #[test]
    fn hermes_args_survive_updates_and_withdrawal() {
        let original = "model: personal\nmcp_servers:\n  demo:\n    command: server\n    args:\n    - a\n    - b\n";
        let spec = native_spec("hermes", &json!({"command":"server","args":["a","b"]})).unwrap();
        let (_, owned) = patch_server(None, "hermes", "demo", None, Some(&spec)).unwrap();
        // Treat the unrelated model as user-owned and the recorded MCP fields as applied.
        let original = format!("{original}    enabled: true\n");
        let updated_spec =
            native_spec("hermes", &json!({"command":"server","args":["x","b"]})).unwrap();
        let (updated, owned) = patch_server(
            Some(&original),
            "hermes",
            "demo",
            owned.as_ref(),
            Some(&updated_spec),
        )
        .unwrap();
        assert_eq!(
            parse_native(&updated, "hermes").unwrap()["mcp_servers"]["demo"]["args"],
            json!(["x", "b"])
        );
        let (removed, _) =
            patch_server(Some(&updated), "hermes", "demo", owned.as_ref(), None).unwrap();
        assert_eq!(
            parse_native(&removed, "hermes").unwrap()["model"],
            "personal"
        );
        assert!(patch_server(Some(&removed), "hermes", "demo", None, Some(&spec)).is_ok());
    }

    #[test]
    fn native_grammars_are_narrower_than_json5() {
        for (tool, text) in [
            ("claude", "{\"x\":1,}"),
            ("claude", "{/* comment */\"x\":1}"),
            ("gemini", "{\"x\":1,}"),
            ("gemini", "\u{feff}{\"x\":1}"),
            ("opencode", "{x:'value'}"),
        ] {
            assert!(parse_native(text, tool).is_err(), "{tool}: {text}");
        }
        assert!(parse_native("{/* comment */\"x\":1}", "gemini").is_ok());
        assert!(parse_native("{/* comment */\"x\":1,}", "opencode").is_ok());
        assert!(parse_native("\u{feff}{\"x\":1}", "claude").is_ok());
        assert!(parse_native("\u{feff}\u{feff}{\"x\":1}", "claude").is_err());
    }

    #[test]
    fn codex_import_keeps_explicit_env_precedence_and_refuses_header_fallback_loss() {
        let spec=canonical_spec("codex",&json!({"command":"server","env":{"TOKEN":"private-explicit"},"env_vars":["TOKEN","OTHER"]})).unwrap();
        assert_eq!(spec["env"]["TOKEN"], "private-explicit");
        assert_eq!(spec["env"]["OTHER"], "${OTHER}");
        assert!(canonical_spec("codex",&json!({"url":"https://example.org/mcp","http_headers":{"Authorization":"private-fallback"},"env_http_headers":{"Authorization":"TOKEN"}})).is_err());
    }

    #[test]
    fn toml_table_comments_and_array_comments_survive_update_and_withdraw() {
        for tool in ["gemini", "codex"] {
            let native = native_spec(tool, &json!({"command":"server","args":["a","b"]})).unwrap();
            let (applied, owned) = patch_server(None, tool, "demo", None, Some(&native)).unwrap();
            let commented = if tool == "codex" {
                applied
                    .replace(
                        "[mcp_servers.demo]",
                        "# table note\n[mcp_servers.demo] # table suffix",
                    )
                    .replace("[\"a\", \"b\"]", "[\"a\", # array note\n \"b\"]")
            } else {
                applied.replace("\"a\",", "\"a\", /* array note */")
            };
            assert!(commented.contains("array note"), "{tool}: {commented}");
            let (same, owned) = patch_server(
                Some(&commented),
                tool,
                "demo",
                owned.as_ref(),
                Some(&native),
            )
            .unwrap_or_else(|e| panic!("{tool}: {e}; {commented}"));
            assert!(same.contains("array note"), "{tool}: {same}");
            let changed = native_spec(tool, &json!({"command":"server","args":["x","b"]})).unwrap();
            let (changed, owned) =
                patch_server(Some(&same), tool, "demo", owned.as_ref(), Some(&changed)).unwrap();
            assert!(changed.contains("array note"), "{tool}: {changed}");
            let (withdrawn, _) = patch_server(Some(&changed), tool, "demo", owned.as_ref(), None)
                .unwrap_or_else(|e| panic!("{tool}: {e}; {changed}"));
            assert!(withdrawn.contains("array note"));
            if tool == "codex" {
                assert!(withdrawn.contains("table note"));
                assert!(withdrawn.contains("table suffix"));
            }
        }
    }

    #[test]
    fn all_five_codecs_can_apply_withdraw_and_reapply() {
        for tool in ["claude", "codex", "gemini", "opencode", "hermes"] {
            let native = native_spec(
                tool,
                &json!({"command":"server","env":{"TOKEN":"${TOKEN}"}}),
            )
            .unwrap();
            let (applied, owned) = patch_server(None, tool, "demo", None, Some(&native)).unwrap();
            let (withdrawn, _) =
                patch_server(Some(&applied), tool, "demo", owned.as_ref(), None).unwrap();
            assert!(
                patch_server(Some(&withdrawn), tool, "demo", None, Some(&native)).is_ok(),
                "{tool}: {withdrawn}"
            );
        }
    }

    #[test]
    fn native_toml_and_yaml_round_trips_preserve_model_and_user_mcp() {
        for (tool,original) in [
            ("codex", "model = \"personal\"\n# personal server\n[mcp_servers.other]\ncommand = \"mine\"\n"),
            ("hermes", "model: personal\nmcp_servers:\n  # personal server\n  other:\n    command: mine\n"),
        ] {
            let native=native_spec(tool,&json!({"command":"server","env":{"TOKEN":"${TOKEN}"}})).unwrap();
            let (applied,receipt)=patch_server(Some(original),tool,"demo",None,Some(&native)).unwrap();
            let (removed,_)=patch_server(Some(&applied),tool,"demo",receipt.as_ref(),None).unwrap();
            let root=parse_native(&removed,tool).unwrap();
            assert_eq!(root["model"],"personal");
            assert_eq!(root[section(tool)]["other"]["command"],"mine");
            assert!(removed.contains("# personal server"));
            assert!(root[section(tool)]["demo"].get("command").is_none());
        }
    }

    #[test]
    fn codecs_translate_environment_references_without_reading_secrets() {
        let spec = json!({"type":"http","url":"https://example.org/mcp","headers":{"Authorization":"Bearer ${MCP_TOKEN}"}});
        assert_eq!(native_spec("codex",&json!({"type":"http","url":"https://example.org/mcp","headers":{"Authorization":"${MCP_TOKEN}"}})).unwrap()["env_http_headers"]["Authorization"],"MCP_TOKEN");
        assert_eq!(
            native_spec("codex", &spec).unwrap()["bearer_token_env_var"],
            "MCP_TOKEN"
        );
        assert_eq!(
            native_spec("opencode", &spec).unwrap()["headers"]["Authorization"],
            "Bearer {env:MCP_TOKEN}"
        );
        assert_eq!(native_spec("gemini", &spec).unwrap()["type"], "http");
        assert!(native_spec(
            "codex",
            &json!({"type":"sse","url":"https://example.org/sse"})
        )
        .is_err());
        assert_eq!(
            native_spec(
                "hermes",
                &json!({"type":"sse","url":"https://example.org/sse"})
            )
            .unwrap()["transport"],
            "sse"
        );
        let stdio = json!({"command":"server","env":{"API_KEY":"${API_KEY}"}});
        assert_eq!(
            native_spec("codex", &stdio).unwrap()["env_vars"],
            json!(["API_KEY"])
        );
        assert!(native_spec(
            "codex",
            &json!({"command":"server","env":{"API_KEY":"${OTHER}"}})
        )
        .is_err());
    }

    #[test]
    fn unowned_same_name_is_a_conflict_even_for_identical_values() {
        let current = r#"{"mcpServers":{"demo":{"command":"user"}}}"#;
        assert!(patch_server(
            Some(current),
            "claude",
            "demo",
            None,
            Some(&json!({"command":"user"}))
        )
        .is_err());
        assert!(patch_server(Some(current), "claude", "demo", None, None).is_err());
    }

    #[test]
    fn owned_update_and_withdraw_preserve_added_fields_and_comments() {
        let original = "{\"model\":\"personal\",\"mcpServers\":{// neighbor\n \"other\":{\"command\":\"mine\"}}}";
        let (applied, receipt) = patch_server(
            Some(original),
            "gemini",
            "demo",
            None,
            Some(&json!({"command":"server","env":{"TOKEN":"${TOKEN}"}})),
        )
        .unwrap();
        let current = crate::json5_sections::edit_object_paths(
            &applied,
            crate::json5_sections::KeyStyle::Json,
            &[(
                vec!["mcpServers".into(), "demo".into(), "timeout".into()],
                Some(json!(13)),
            )],
        )
        .unwrap();
        let (updated, receipt2) = patch_server(
            Some(&current),
            "gemini",
            "demo",
            receipt.as_ref(),
            Some(&json!({"command":"new","env":{"TOKEN":"${TOKEN}"}})),
        )
        .unwrap();
        assert!(updated.contains("// neighbor"));
        assert_eq!(
            json5::from_str::<Value>(&updated).unwrap()["mcpServers"]["demo"]["timeout"],
            13
        );
        assert!(patch_server(Some(&updated), "gemini", "demo", receipt2.as_ref(), None).is_err());
        verify_owned_text(&updated, receipt2.as_ref().unwrap()).unwrap();
        assert_eq!(
            parse_native(&updated, "gemini").unwrap()["model"],
            "personal"
        );
        assert!(patch_server(
            Some(&updated.replace("new", "manual")),
            "gemini",
            "demo",
            receipt2.as_ref(),
            None
        )
        .is_err());
    }

    #[test]
    fn corrupt_roots_and_containers_never_become_empty_config() {
        for current in ["broken", "[]", r#"{"mcpServers":[]} "#] {
            assert!(patch_server(
                Some(current),
                "claude",
                "demo",
                None,
                Some(&json!({"command":"server"}))
            )
            .is_err());
        }
    }

    #[test]
    fn literal_values_are_written_as_is_for_every_tool() {
        let local = json!({"type":"stdio","command":"npx","args":["-y","shadcn"],"env":{"GITHUB_TOKEN":"ghp-literal"}});
        let remote = json!({"type":"http","url":"https://mcp.context7.com/mcp","headers":{"CONTEXT7_API_KEY":"ctx-literal"}});
        for tool in ["claude", "codex", "gemini", "opencode", "hermes"] {
            for spec in [&local, &remote] {
                let native = native_spec(tool, spec).unwrap_or_else(|e| panic!("{tool}: {e}"));
                assert_eq!(canonical_spec(tool, &native).unwrap(), *spec, "{tool}");
            }
        }
        let codex = native_spec("codex", &local).unwrap();
        assert_eq!(codex["env"]["GITHUB_TOKEN"], "ghp-literal");
        assert!(codex.get("env_vars").is_none());
        let codex = native_spec("codex", &remote).unwrap();
        assert_eq!(codex["http_headers"]["CONTEXT7_API_KEY"], "ctx-literal");
        assert!(codex.get("env_http_headers").is_none());
        assert_eq!(
            native_spec("opencode", &local).unwrap()["environment"]["GITHUB_TOKEN"],
            "ghp-literal"
        );
    }
    #[test]
    fn codex_splits_literal_and_referenced_values_and_rejects_mixtures() {
        let local =
            json!({"type":"stdio","command":"server","env":{"PLAIN":"value","TOKEN":"${TOKEN}"}});
        let native = native_spec("codex", &local).unwrap();
        assert_eq!(native["env"], json!({"PLAIN":"value"}));
        assert_eq!(native["env_vars"], json!(["TOKEN"]));
        assert_eq!(canonical_spec("codex", &native).unwrap(), local);
        let remote = json!({"type":"http","url":"https://example.org/mcp","headers":{"X-Plain":"value","X-Token":"${TOKEN}","Authorization":"Bearer literal"}});
        let native = native_spec("codex", &remote).unwrap();
        assert_eq!(
            native["http_headers"],
            json!({"X-Plain":"value","Authorization":"Bearer literal"})
        );
        assert_eq!(native["env_http_headers"], json!({"X-Token":"TOKEN"}));
        assert_eq!(canonical_spec("codex", &native).unwrap(), remote);
        for spec in [
            json!({"command":"server","env":{"PATH_LIST":"/bin:${HOME}"}}),
            json!({"type":"http","url":"https://example.org/mcp","headers":{"X-Token":"pre-${TOKEN}"}}),
        ] {
            assert!(native_spec("codex", &spec).is_err(), "accepted {spec}");
        }
        assert_eq!(
            native_spec(
                "opencode",
                &json!({"command":"server","env":{"PATH_LIST":"/bin:${HOME}"}})
            )
            .unwrap()["environment"]["PATH_LIST"],
            "/bin:{env:HOME}"
        );
    }
    #[test]
    fn urls_may_carry_a_query_but_not_user_info() {
        for tool in ["claude", "codex", "gemini", "opencode", "hermes"] {
            let native = native_spec(
                tool,
                &json!({"type":"http","url":"https://x.test/mcp?key=abc"}),
            )
            .unwrap_or_else(|e| panic!("{tool}: {e}"));
            assert_eq!(native["url"], "https://x.test/mcp?key=abc");
            assert!(
                native_spec(tool, &json!({"type":"http","url":"https://u:p@x.test/mcp"})).is_err()
            );
        }
    }
    #[test]
    fn codex_literal_tables_are_withdrawn_completely() {
        let original = "model = \"personal\"\n";
        let native = native_spec(
            "codex",
            &json!({"type":"stdio","command":"server","env":{"PLAIN":"value","TOKEN":"${TOKEN}"}}),
        )
        .unwrap();
        let (applied, receipt) =
            patch_server(Some(original), "codex", "demo", None, Some(&native)).unwrap();
        let root = parse_native(&applied, "codex").unwrap();
        assert_eq!(root["mcp_servers"]["demo"]["env"]["PLAIN"], "value");
        verify_owned_text(&applied, receipt.as_ref().unwrap()).unwrap();
        let (removed, _) =
            patch_server(Some(&applied), "codex", "demo", receipt.as_ref(), None).unwrap();
        let root = parse_native(&removed, "codex").unwrap();
        assert!(root
            .get("mcp_servers")
            .and_then(|m| m.get("demo"))
            .is_none());
        assert!(!removed.contains("value"));
    }

    #[test]
    fn withdrawing_restores_each_config_byte_for_byte() {
        let gemini = "{\n  \"security\": {\n    \"auth\": {\n      \"selectedType\": \"gemini-api-key\"\n    }\n  },\n  \"model\": {\n    \"name\": \"gemini-2.5-pro\"\n  }\n}\n";
        let hermes =
            "model:\n  default: personal\n# trailing comment\nagent:\n  reasoning_effort: high\n";
        let opencode = "{\n  // keep me\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {\n    \"context7\": {\n      \"type\": \"remote\",\n      \"url\": \"https://mcp.context7.com/mcp\"\n    }\n  }\n}\n";
        let codex = "model = \"personal\"\n\n[mcp_servers]\n\n[mcp_servers.node_repl]\nargs = []\ncommand = \"/Applications/ChatGPT.app/node_repl\"\nstartup_timeout_sec = 120\n\n[mcp_servers.node_repl.env]\nB_KEY = \"2\"\nA_KEY = \"1\"\n";
        let claude = "{\n  \"mcpServers\": {},\n  \"theme\": \"dark\"\n}\n";
        let spec = json!({"type":"stdio","command":"npx","args":["-y","@modelcontextprotocol/server-memory"],"env":{"MEMORY_FILE_PATH":"/tmp/memory.json"}});
        let mut failures = vec![];
        for (tool, original) in [
            ("gemini", gemini),
            ("hermes", hermes),
            ("opencode", opencode),
            ("codex", codex),
            ("claude", claude),
        ] {
            let native = native_spec(tool, &spec).unwrap();
            let (applied, receipt) =
                patch_server(Some(original), tool, "ofox-test", None, Some(&native)).unwrap();
            assert!(applied.contains("ofox-test"), "{tool}");
            let (removed, _) =
                patch_server(Some(&applied), tool, "ofox-test", receipt.as_ref(), None).unwrap();
            if removed != original {
                failures.push(format!("{tool}:\n{removed:?}"));
            }
        }
        assert!(
            failures.is_empty(),
            "not restored:\n{}",
            failures.join("\n")
        );
    }
}
