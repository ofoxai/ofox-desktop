use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    ReadOnly,
    #[default]
    Ask,
    AutoConfig,
    FullAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationClass {
    Read,
    Configure,
    Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Confirm,
    Deny,
}

pub fn decide(configuration: bool, mode: PermissionMode, operation: OperationClass) -> Decision {
    if !configuration {
        return Decision::Deny;
    }
    match (mode, operation) {
        (_, OperationClass::Read) | (PermissionMode::FullAccess, _) => Decision::Allow,
        (PermissionMode::ReadOnly, _) => Decision::Deny,
        (PermissionMode::AutoConfig, OperationClass::Configure) => Decision::Allow,
        _ => Decision::Confirm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_assistant_cannot_gain_host_tools_from_a_permission_argument() {
        for mode in [
            PermissionMode::ReadOnly,
            PermissionMode::Ask,
            PermissionMode::AutoConfig,
            PermissionMode::FullAccess,
        ] {
            assert_eq!(
                decide(false, mode, OperationClass::Configure),
                Decision::Deny
            );
            assert_eq!(decide(false, mode, OperationClass::Command), Decision::Deny);
        }
    }

    #[test]
    fn read_only_never_mutates_and_auto_config_does_not_allow_arbitrary_shell() {
        assert_eq!(
            decide(true, PermissionMode::ReadOnly, OperationClass::Configure),
            Decision::Deny
        );
        assert_eq!(
            decide(true, PermissionMode::ReadOnly, OperationClass::Command),
            Decision::Deny
        );
        assert_eq!(
            decide(true, PermissionMode::AutoConfig, OperationClass::Configure),
            Decision::Allow
        );
        assert_eq!(
            decide(true, PermissionMode::AutoConfig, OperationClass::Command),
            Decision::Confirm
        );
    }
}
