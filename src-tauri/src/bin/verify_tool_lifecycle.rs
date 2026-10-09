//! Read-only host probe for tool installation evidence. Never installs, binds,
//! restores, unbinds, or fetches remote latest versions.

use cc_switch_lib::{get_tool_install_capabilities, get_tool_versions};
use serde_json::{json, Value};

const TOOLS: &[&str] = &[
    "claude",
    "codex",
    "chatgpt",
    "gemini",
    "opencode",
    "openclaw",
    "hermes",
    "workbuddy",
];

fn parse_tools(args: impl IntoIterator<Item = String>) -> Result<Option<Vec<String>>, String> {
    let mut args = args.into_iter();
    let mut tools = Vec::new();
    while let Some(arg) = args.next() {
        if arg != "--tool" {
            return Err(format!("Unknown argument: {arg}. Use --help for usage."));
        }
        let tool = args.next().ok_or("--tool requires a tool ID")?;
        if !TOOLS.contains(&tool.as_str()) {
            return Err(format!("Unknown tool: {tool}"));
        }
        if !tools.contains(&tool) {
            tools.push(tool);
        }
    }
    Ok((!tools.is_empty()).then_some(tools))
}

/// Exclude raw process diagnostics: a misbehaving CLI can print credentials
/// even for --version. This report contains evidence, not configuration values.
fn public_observation(row: &Value) -> Value {
    let has_error = !row["error"].is_null();
    let diagnostic = match row["installationStatus"].as_str() {
        Some("notInstalled") => "notFound",
        Some("installed") if has_error => "versionCheckFailed",
        Some("installed") => "ok",
        _ => "detectionFailed",
    };
    json!({
        "name": row["name"],
        "installationStatus": row["installationStatus"],
        "installationKind": row["installationKind"],
        "version": row["version"],
        "envType": row["env_type"],
        "wslDistro": row["wsl_distro"],
        "diagnostic": diagnostic,
    })
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!(
            "Usage: verify_tool_lifecycle [--tool TOOL]...\n\
             TOOL: {}\n\
             Probes local installation evidence only. No config writes or remote update checks.\n\
             Exit 0 means detection completed; inspect each installationStatus for the result.",
            TOOLS.join(", ")
        );
        return;
    }
    let tools = match parse_tools(args) {
        Ok(tools) => tools,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let versions = match get_tool_versions(tools, None, Some(false)).await {
        Ok(versions) => versions,
        Err(_) => {
            eprintln!("Tool installation probe failed.");
            std::process::exit(1);
        }
    };
    let rows = serde_json::to_value(versions).expect("serialize tool evidence");
    let observations: Vec<Value> = rows
        .as_array()
        .expect("tool evidence array")
        .iter()
        .map(public_observation)
        .collect();
    let report = json!({
        "schemaVersion": 1,
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "automaticInstallTools": get_tool_install_capabilities(),
        "tools": observations,
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_tool_selection_rejects_typos_and_deduplicates() {
        assert_eq!(parse_tools(Vec::new()).unwrap(), None);
        assert_eq!(
            parse_tools(["--tool", "codex", "--tool", "codex"].map(String::from)).unwrap(),
            Some(vec!["codex".into()])
        );
        assert!(parse_tools(["--tool", "codeex"].map(String::from)).is_err());
        assert!(parse_tools(["--tool"].map(String::from)).is_err());
        assert!(parse_tools(["--install"].map(String::from)).is_err());
    }

    #[test]
    fn public_report_keeps_broken_installation_evidence_without_process_secrets() {
        let observation = public_observation(&json!({
            "name": "codex",
            "installationStatus": "installed",
            "installationKind": "cli",
            "version": null,
            "error": "bad CLI leaked sk-test-private and config contents",
            "executable_path": "C:\\private\\codex.cmd",
            "env_type": "windows",
            "wsl_distro": null,
        }));
        assert_eq!(observation["installationStatus"], "installed");
        assert_eq!(observation["diagnostic"], "versionCheckFailed");
        assert!(!observation.to_string().contains("sk-test-private"));
        assert!(!observation.to_string().contains("C:\\private"));
        assert_eq!(
            public_observation(&json!({"installationStatus":"unknown","error":"WSL unreachable"}))
                ["diagnostic"],
            "detectionFailed"
        );
        assert_eq!(
            public_observation(&json!({"installationStatus":"notInstalled","error":null}))
                ["diagnostic"],
            "notFound"
        );
    }
}
