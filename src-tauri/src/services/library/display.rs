//! Small, non-secret display hints. Never stringify a native definition for UI.
use serde_json::Value;

/// Only maintained public package identities and exact vendor hosts earn an icon.
/// Arbitrary args, URL paths/query, environment values and executable paths stay private.
pub(crate) fn mcp_display(spec: &Value) -> (Option<String>, Option<String>) {
    if spec.get("url").is_some() {
        let Some(host) = super::inventory::mcp_service_host(spec) else {
            return (None, None);
        };
        let icon = match host.as_str() {
            "mcp.context7.com" => Some("context7"),
            "mcp.deepwiki.com" => Some("deepwiki"),
            "learn.microsoft.com" => Some("microsoft-learn"),
            "api.githubcopilot.com" => Some("github"),
            _ => None,
        };
        return (Some(host), icon.map(str::to_owned));
    }
    let Some(runner) = spec.get("command").and_then(Value::as_str) else {
        return (None, None);
    };
    if !["npx", "uvx", "node", "python", "python3", "codegraph"].contains(&runner) {
        return (None, None);
    }
    let package = spec.get("args").and_then(Value::as_array).and_then(|args| {
        // Inspect only the runner's first positional argument, never options' values.
        let first = match runner {
            "npx" => args
                .iter()
                .find(|arg| arg.as_str() != Some("-y") && arg.as_str() != Some("--yes")),
            "uvx" => args.first(),
            _ => None,
        }?
        .as_str()?;
        let (name, version) = match first.rsplit_once('@') {
            Some((name, version)) if !name.is_empty() => (name, Some(version)),
            _ => (first, None),
        };
        if version.is_some_and(|version| {
            version.is_empty()
                || !version
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ".^~_-".contains(c))
        }) {
            return None;
        }
        match (runner, name) {
            ("npx", "@upstash/context7-mcp") => Some((name, "context7")),
            ("npx", "chrome-devtools-mcp") => Some((name, "chrome-devtools")),
            ("npx", "@playwright/mcp") => Some((name, "playwright")),
            ("npx", "@modelcontextprotocol/server-filesystem") => Some((name, "filesystem")),
            ("uvx", "mcp-server-fetch") => Some((name, "fetch")),
            _ => None,
        }
    });
    let summary = package.map_or_else(
        || runner.to_owned(),
        |(name, _)| format!("{runner} · {name}"),
    );
    (Some(summary), package.map(|(_, icon)| icon.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn does_not_mistake_paths_shells_flags_or_names_for_a_package_identity() {
        for spec in [
            json!({"command":"sh","args":["-c","npx @playwright/mcp"]}),
            json!({"command":"/private/person/npx","args":["@playwright/mcp"]}),
            json!({"command":"npx","args":["--token","@playwright/mcp"]}),
            json!({"command":"npx","args":["@playwright/mcp@https://private.example/token"]}),
            json!({"command":"npx","args":["@playwright/mcp-evil"]}),
            json!({"url":"https://private:secret@mcp.context7.com/mcp"}),
        ] {
            let (summary, icon) = mcp_display(&spec);
            assert!(icon.is_none());
            assert!(summary.is_none() || summary.as_deref() == Some("npx"));
        }
    }
}
