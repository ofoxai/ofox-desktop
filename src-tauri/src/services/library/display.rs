//! Small display hints. Env and header values never leave; args are shown with
//! credential-looking values masked.
use serde_json::Value;

const MASK: &str = "••••";

fn secret_flag(flag: &str) -> bool {
    let name = flag.trim_start_matches('-').to_ascii_lowercase();
    flag.starts_with('-')
        && [
            "token",
            "key",
            "secret",
            "password",
            "passwd",
            "auth",
            "credential",
            "bearer",
            "cookie",
            "session",
        ]
        .iter()
        .any(|word| name.contains(word))
}
fn secret_value(arg: &str) -> bool {
    [
        "sk-",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "github_pat_",
        "glpat-",
        "xoxb-",
        "xoxp-",
        "AKIA",
    ]
    .iter()
    .any(|prefix| arg.starts_with(prefix))
        || (arg.len() >= 32
            && arg.bytes().any(|b| b.is_ascii_digit())
            && arg.bytes().any(|b| b.is_ascii_alphabetic())
            && arg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
}
/// `host[:port]/path`, with user info dropped and a query reduced to `?…`.
fn url_text(raw: &str, scheme: bool) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    let host = url.host_str()?;
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let path = url
        .path()
        .trim_end_matches('/')
        .split('/')
        .map(|segment| if secret_value(segment) { MASK } else { segment })
        .collect::<Vec<_>>()
        .join("/");
    let query = if url.query().is_some() { "?…" } else { "" };
    let prefix = if scheme {
        format!("{}://", url.scheme())
    } else {
        String::new()
    };
    Some(format!("{prefix}{host}{port}{path}{query}"))
}
/// The server as it reads in its config: `command args…` or `host/path`.
pub(crate) fn mcp_command_line(spec: &Value) -> Option<String> {
    if let Some(url) = spec.get("url").and_then(Value::as_str) {
        return url_text(url, false);
    }
    let home = crate::config::get_home_dir().to_string_lossy().into_owned();
    let tidy = |text: &str| match text.strip_prefix(home.as_str()) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => text.to_string(),
    };
    let mut words = vec![tidy(spec.get("command").and_then(Value::as_str)?)];
    let mut hide_next = false;
    for arg in spec
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let word = if hide_next {
            MASK.to_string()
        } else if let Some((flag, _)) = arg.split_once('=').filter(|(flag, _)| secret_flag(flag)) {
            format!("{flag}={MASK}")
        } else if secret_value(arg) {
            MASK.to_string()
        } else if arg.contains("://") {
            url_text(arg, true).unwrap_or_else(|| MASK.to_string())
        } else {
            tidy(arg)
        };
        hide_next = !arg.contains('=') && secret_flag(arg);
        words.push(if word.chars().any(char::is_whitespace) {
            format!("'{word}'")
        } else {
            word
        });
    }
    Some(words.join(" "))
}

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
    #[test]
    #[serial_test::serial]
    fn command_line_reads_like_the_config_with_secrets_masked() {
        let home = crate::config::get_home_dir().to_string_lossy().into_owned();
        for (spec, line) in [
            (
                json!({"command":"npx","args":["-y","@modelcontextprotocol/server-filesystem","~/Documents",format!("{home}/Desktop")]}),
                "npx -y @modelcontextprotocol/server-filesystem ~/Documents ~/Desktop",
            ),
            (
                json!({"command":"auggie","args":["--mcp","--mcp-auto-workspace"],"env":{"AUGMENT_SESSION_AUTH":"private-env"}}),
                "auggie --mcp --mcp-auto-workspace",
            ),
            (
                json!({"command":"server","args":["--token","private-a","--api-key=private-b","--name","My Docs"]}),
                "server --token •••• --api-key=•••• --name 'My Docs'",
            ),
            (
                json!({"command":"server","args":["sk-private123456","https://u:p@x.test/a?key=private-c"]}),
                "server •••• https://x.test/a?…",
            ),
            (
                json!({"type":"http","url":"https://mcp.context7.com/mcp","headers":{"KEY":"private-h"}}),
                "mcp.context7.com/mcp",
            ),
            (
                json!({"type":"sse","url":"https://user:pw@x.test:8443/sse?key=private-q"}),
                "x.test:8443/sse?…",
            ),
            (
                json!({"url":"https://mcp.example.com/ghp_private0123/sse"}),
                "mcp.example.com/••••/sse",
            ),
        ] {
            assert_eq!(mcp_command_line(&spec).as_deref(), Some(line));
        }
        assert!(mcp_command_line(&json!({"type":"stdio"})).is_none());
    }
}
