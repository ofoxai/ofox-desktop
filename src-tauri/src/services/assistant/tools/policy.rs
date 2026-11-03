//! Pure authorization policy, modeled on AionUi's built-in agent: reads never
//! ask, `AutoEdit` also lets configuration changes through, `FullAuto` lets
//! everything through. Protected stores stay denied in every mode. Command
//! classification is computed by the host, never accepted from model arguments.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    #[default]
    Default,
    AutoEdit,
    FullAuto,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Read,
    WriteConfig,
    Exec,
}

#[derive(Clone, Copy, Debug)]
pub struct Operation {
    pub category: Category,
    pub read_only_command: bool,
    pub forbidden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Confirm,
    Deny,
}

pub fn decide(enabled: bool, mode: PermissionMode, op: Operation) -> Decision {
    use Decision::*;
    if !enabled || op.forbidden {
        return Deny;
    }
    let allowed = match op.category {
        Category::Read => true,
        Category::Exec => op.read_only_command || mode == PermissionMode::FullAuto,
        Category::WriteConfig => mode != PermissionMode::Default,
    };
    if allowed {
        Allow
    } else {
        Confirm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_policy_matrix() {
        use Decision::*;
        use PermissionMode::*;
        let op = |category, read_only_command| Operation {
            category,
            read_only_command,
            forbidden: false,
        };
        for (operation, expected) in [
            (op(Category::Read, false), [Allow, Allow, Allow]),
            (op(Category::Exec, true), [Allow, Allow, Allow]),
            (op(Category::WriteConfig, false), [Confirm, Allow, Allow]),
            (op(Category::Exec, false), [Confirm, Confirm, Allow]),
        ] {
            for (mode, expected) in [Default, AutoEdit, FullAuto].into_iter().zip(expected) {
                assert_eq!(
                    decide(true, mode, operation),
                    expected,
                    "{mode:?} {operation:?}"
                );
                assert_eq!(decide(false, mode, operation), Deny);
                let forbidden = Operation {
                    forbidden: true,
                    ..operation
                };
                assert_eq!(decide(true, mode, forbidden), Deny);
            }
        }
        assert_eq!(PermissionMode::default(), Default);
    }
}
