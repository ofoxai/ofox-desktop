//! Pure authorization policy. Scope and command classification are computed
//! by the host, never accepted from model arguments.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    ReadOnly,
    Ask,
    #[default]
    AutoConfig,
    FullAccess,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Read,
    WriteConfig,
    Exec,
    Network,
    Paid,
}

#[derive(Clone, Copy, Debug)]
pub struct Operation {
    pub category: Category,
    pub in_scope: bool,
    pub reversible: bool,
    pub structured: bool,
    pub read_only_command: bool,
    pub forbidden: bool,
    pub irreversible: bool,
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
    let read =
        op.category == Category::Read || (op.category == Category::Exec && op.read_only_command);
    if mode == PermissionMode::ReadOnly && !read {
        return Deny;
    }
    if op.irreversible {
        return Confirm;
    }
    if mode == PermissionMode::FullAccess {
        return Allow;
    }
    if !op.in_scope {
        return Confirm;
    }
    if read
        || (mode == PermissionMode::AutoConfig
            && op.category == Category::WriteConfig
            && op.structured
            && op.reversible)
    {
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
        let modes = [ReadOnly, Ask, AutoConfig, FullAccess];
        // Scope approval never overrides a forbidden path or a read-only mode.
        for (operation, expected) in [
            (
                Operation {
                    category: Category::Read,
                    in_scope: true,
                    reversible: false,
                    structured: false,
                    read_only_command: false,
                    forbidden: false,
                    irreversible: false,
                },
                [Allow, Allow, Allow, Allow],
            ),
            (
                Operation {
                    category: Category::Read,
                    in_scope: false,
                    reversible: false,
                    structured: false,
                    read_only_command: false,
                    forbidden: false,
                    irreversible: false,
                },
                [Confirm, Confirm, Confirm, Allow],
            ),
            (
                Operation {
                    category: Category::WriteConfig,
                    in_scope: true,
                    reversible: true,
                    structured: true,
                    read_only_command: false,
                    forbidden: false,
                    irreversible: false,
                },
                [Deny, Confirm, Allow, Allow],
            ),
            (
                Operation {
                    category: Category::WriteConfig,
                    in_scope: true,
                    reversible: true,
                    structured: false,
                    read_only_command: false,
                    forbidden: false,
                    irreversible: false,
                },
                [Deny, Confirm, Confirm, Allow],
            ),
            (
                Operation {
                    category: Category::Exec,
                    in_scope: true,
                    reversible: false,
                    structured: false,
                    read_only_command: true,
                    forbidden: false,
                    irreversible: false,
                },
                [Allow, Allow, Allow, Allow],
            ),
            (
                Operation {
                    category: Category::Exec,
                    in_scope: true,
                    reversible: false,
                    structured: false,
                    read_only_command: false,
                    forbidden: false,
                    irreversible: false,
                },
                [Deny, Confirm, Confirm, Allow],
            ),
            (
                Operation {
                    category: Category::Exec,
                    in_scope: true,
                    reversible: false,
                    structured: true,
                    read_only_command: false,
                    forbidden: false,
                    irreversible: true,
                },
                [Deny, Confirm, Confirm, Confirm],
            ),
        ] {
            for (mode, expected) in modes.into_iter().zip(expected) {
                assert_eq!(
                    decide(true, mode, operation),
                    expected,
                    "{mode:?} {operation:?}"
                );
                assert_eq!(decide(false, mode, operation), Deny);
                assert_eq!(
                    decide(
                        true,
                        mode,
                        Operation {
                            forbidden: true,
                            ..operation
                        }
                    ),
                    Deny
                );
            }
        }
        assert_eq!(PermissionMode::default(), AutoConfig);
    }
}
