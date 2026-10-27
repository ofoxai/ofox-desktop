//! Honest support projection. Permission approval never invents an adapter or
//! an installation prerequisite. Runtime tool results remain authoritative.
use super::policy::{decide, Decision, OperationClass, PermissionMode};
use serde_json::{json, Value};

pub fn tools() -> Value {
    let tools:Vec<Value> = crate::model_selection::TOOLS.iter().map(|tool| {
        let desktop = matches!(*tool,"chatgpt"|"workbuddy");
        let partial = crate::app_config::BindableTool::native(tool).is_none() && *tool != "workbuddy";
        let actions:Vec<Value> = ["config_status","diagnose_tool","bind_tool","set_tool_model","install_tool","update_tool"].iter().map(|action| {
            let lifecycle = matches!(*action,"install_tool"|"update_tool");
            let handoff = lifecycle && (desktop || (*action == "install_tool" && cfg!(all(target_os="macos", not(target_arch="aarch64")))));
            let unsupported = lifecycle && !cfg!(any(target_os="macos",target_os="windows"));
            let operation = if *action == "config_status" { OperationClass::Read } else { OperationClass::Configure };
            let mut modes = serde_json::Map::new();
            for (name,mode) in [("readOnly",PermissionMode::ReadOnly),("ask",PermissionMode::Ask),("autoConfig",PermissionMode::AutoConfig),("fullAccess",PermissionMode::FullAccess)] {
                let support = if unsupported { "unsupported" } else if handoff { "toolsPage" } else { match decide(true,mode,operation) { Decision::Allow=>"automatic",Decision::Confirm=>"confirmation",Decision::Deny=>"unsupported" } };
                modes.insert(name.into(),json!(support));
            }
            let note = if handoff {"nativeInstaller"} else if unsupported {"platformUnavailable"} else if *action=="config_status" && partial {"staticPartial"} else if *action=="diagnose_tool" && *tool=="workbuddy" {"staticOnly"} else if *action=="diagnose_tool" && partial {"diagnosticsPartial"} else if *action=="diagnose_tool" {"fixedDiagnostics"} else if *action=="install_tool" && *tool!="hermes" {"installPrerequisites"} else if *action=="update_tool" {"originalManager"} else {"managedConfiguration"};
            json!({"action":action,"supportByMode":modes,"note":note,"partial":partial && matches!(*action,"config_status"|"diagnose_tool")})
        }).collect();
        json!({"tool":tool,"actions":actions})
    }).collect();
    json!({"version":1,"tools":tools,"arbitraryCommand":{"readOnly":"unsupported","ask":"confirmation","autoConfig":"confirmation","fullAccess":"automatic"},"resourceChanges":"toolsPage"})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_siblings_disclose_partial_checks_permissions_and_handoffs() {
        let c = tools();
        assert_eq!(
            c["tools"].as_array().unwrap().len(),
            crate::model_selection::TOOLS.len()
        );
        for tool in c["tools"].as_array().unwrap() {
            for action in tool["actions"].as_array().unwrap() {
                assert!(action["note"].is_string());
                if action["action"] == "diagnose_tool" {
                    assert_eq!(action["supportByMode"]["readOnly"], "unsupported");
                }
                if matches!(tool["tool"].as_str(), Some("chatgpt" | "workbuddy"))
                    && action["action"] == "install_tool"
                {
                    assert_eq!(action["supportByMode"]["fullAccess"], "toolsPage");
                }
            }
        }
        assert_eq!(c["arbitraryCommand"]["autoConfig"], "confirmation");
    }
}
