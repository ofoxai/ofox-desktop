//! 解绑时哪些受管字段还归 Ofox（和 magpie 的 Disconnect 一样）。
//!
//! Ofox 每次写配置都记下自己写了什么（[`super::record::BindEnvelope::written`]）。
//! 解绑时逐个字段比对：还是 Ofox 写的样子、或者一看就是 Ofox 的，才还原成绑定前；
//! 绑定之后用户自己改过的值留给用户。字段被删掉了（没有值）就补回绑定前的值——
//! 和 magpie 的 Disconnect 一样，空着的字段回到连接前的样子。没有这份记录的旧绑定
//! 照旧全部还原。

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::ofox_apex::mentions_ofox_gateway;

/// 一个受管文件里各受管字段的值，按字段路径（[`Leaf::id`]）；文件里没有的字段是 `Null`。
pub(crate) type Leaves = BTreeMap<String, Value>;

/// 用户改过的字段解绑时还要不要还原。
#[derive(Debug, Clone, Copy)]
pub(crate) enum LeafPolicy {
    /// 接入方式（服务商、地址、key）：还是 Ofox 写的样子，或者参数判断它是 Ofox 的
    /// （`ofox` 服务商、Ofox 网关地址、`sk-of-` key），就还原；否则是用户自己的，留着。
    Route(fn(&Value) -> bool),
    /// 模型：还是 Ofox 写的样子，或者它所属的接入方式（`route`）要还原，就还原——
    /// Ofox 的模型名不能留在用户自己的服务商下面。
    Model { route: RouteRef },
    /// 偏好（推理强度等）：只有还是 Ofox 写的样子才还原。
    Preference,
}

/// 模型所属的接入方式：同一个文件里的字段，或者（`file` 是另一个受管文件的
/// [`super::record::ManagedFile::id`]）别的文件里的，比如 Gemini settings.json 里的模型
/// 跟着 `.env` 里的地址走。
#[derive(Debug, Clone, Copy)]
pub(crate) struct RouteRef {
    pub file: Option<&'static str>,
    pub path: &'static [&'static str],
}

/// 一个受管字段：从文件根出发的路径和它的还原规则。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Leaf {
    pub path: &'static [&'static str],
    pub policy: LeafPolicy,
}

impl Leaf {
    pub(crate) const fn route(path: &'static [&'static str], ofox: fn(&Value) -> bool) -> Self {
        Self {
            path,
            policy: LeafPolicy::Route(ofox),
        }
    }

    pub(crate) const fn model(
        path: &'static [&'static str],
        route: &'static [&'static str],
    ) -> Self {
        Self {
            path,
            policy: LeafPolicy::Model {
                route: RouteRef {
                    file: None,
                    path: route,
                },
            },
        }
    }

    /// 模型的接入方式在另一个受管文件（`file`）里。
    pub(crate) const fn model_in(
        path: &'static [&'static str],
        file: &'static str,
        route: &'static [&'static str],
    ) -> Self {
        Self {
            path,
            policy: LeafPolicy::Model {
                route: RouteRef {
                    file: Some(file),
                    path: route,
                },
            },
        }
    }

    pub(crate) const fn preference(path: &'static [&'static str]) -> Self {
        Self {
            path,
            policy: LeafPolicy::Preference,
        }
    }

    /// 报告和记录里的名字，例如 `env.ANTHROPIC_BASE_URL`。
    pub(crate) fn id(&self) -> String {
        self.path.join(".")
    }
}

/// 按路径从 JSON（或转成 JSON 的 TOML）里取出各受管字段。
pub(crate) fn leaves_at(value: &Value, leaves: &[Leaf]) -> Leaves {
    leaves
        .iter()
        .map(|leaf| {
            let found = leaf.path.iter().try_fold(value, |node, key| node.get(key));
            (leaf.id(), found.cloned().unwrap_or(Value::Null))
        })
        .collect()
}

pub(crate) fn ofox_url(value: &Value) -> bool {
    value.as_str().is_some_and(mentions_ofox_gateway)
}

pub(crate) fn ofox_key(value: &Value) -> bool {
    value.as_str().is_some_and(|key| key.starts_with("sk-of-"))
}

/// 服务商条目：里面有 Ofox 网关地址或 Ofox 的 key。
pub(crate) fn ofox_entry(value: &Value) -> bool {
    match value {
        Value::String(_) => ofox_url(value) || ofox_key(value),
        Value::Array(items) => items.iter().any(ofox_entry),
        Value::Object(fields) => fields.values().any(ofox_entry),
        _ => false,
    }
}

/// 记进绑定记录的样子：key、token 之类只存 SHA-256 摘要——绑定记录在本机数据库里，
/// 不能有明文 key。比对时当前值也换成这个样子再比。
pub(crate) fn recorded(id: &str, value: &Value) -> Value {
    digest_secrets(id.rsplit('.').next().unwrap_or(id), value)
}

fn digest_secrets(name: &str, value: &Value) -> Value {
    match value {
        Value::String(text) if secret_name(name) || looks_secret(text) => {
            Value::String(format!("sha256:{:x}", Sha256::digest(text.as_bytes())))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| digest_secrets(name, item))
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), digest_secrets(key, item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["key", "token", "secret", "password"]
        .iter()
        .any(|word| name.contains(word))
}

fn looks_secret(text: &str) -> bool {
    matches!(crate::redact::scrub(text), Cow::Owned(_))
}

/// 一次写入之后更新 `written`（`before` / `after` 是写之前、之后读到的受管字段）：
/// - 这次改了的字段，记成写入后的值；
/// - 没改的沿用之前的记录：Ofox 重写时保留下来的用户的值不能算成 Ofox 写的；
/// - 之前没记过、又和绑定前（`original`）一样的也记上——那就是 Ofox 留下的样子。
///   和绑定前不一样又没记过的（旧版本绑定的）不记，解绑时照旧还原。
pub(crate) fn record_write(
    written: &mut Leaves,
    before: &Leaves,
    after: &Leaves,
    original: Option<&Leaves>,
) {
    for (id, now) in after {
        let changed = before.get(id) != Some(now);
        let as_original = original.and_then(|original| original.get(id)) == Some(now);
        if changed || (!written.contains_key(id) && as_original) {
            written.insert(id.clone(), recorded(id, now));
        }
    }
}

/// 解绑时一个受管文件的样子。
pub(crate) struct FileState<'a> {
    /// [`super::record::ManagedFile::id`]。
    pub file: &'static str,
    pub rules: &'static [Leaf],
    pub current: Leaves,
    /// 绑定前的值。
    pub original: Leaves,
    /// Ofox 最近写的；没有（旧记录）就全部还原。
    pub written: Option<&'a Leaves>,
}

/// 解绑时每个文件里留给用户、不还原的字段（按文件 id）：用户改过、又和绑定前不一样
/// 的。`written` 里没记的字段、现在没有值的字段都还原。模型跟着它的接入方式走，
/// 接入方式可以在同一个工具的另一个文件里。
pub(crate) fn kept(files: &[FileState]) -> BTreeMap<&'static str, BTreeSet<String>> {
    files
        .iter()
        .map(|state| {
            let kept = state
                .rules
                .iter()
                .filter(|leaf| !restores(files, state, leaf))
                .map(Leaf::id)
                .filter(|id| state.current.get(id) != state.original.get(id))
                .collect();
            (state.file, kept)
        })
        .collect()
}

fn restores(files: &[FileState], state: &FileState, leaf: &Leaf) -> bool {
    let Some(written) = state.written else {
        return true;
    };
    let id = leaf.id();
    let now = state.current.get(&id).unwrap_or(&Value::Null);
    let as_written = written
        .get(&id)
        .is_none_or(|was| recorded(&id, now) == *was);
    // 删掉的字段补回绑定前的值（原来也没有就还是没有）。
    now.is_null()
        || as_written
        || match leaf.policy {
            LeafPolicy::Route(ofox) => ofox(now),
            LeafPolicy::Model { route } => route_restored(files, state, route),
            LeafPolicy::Preference => false,
        }
}

/// 模型所属的接入方式要不要还原。那个文件已经不在了（解绑不会再建它），或者找不到
/// 这个字段，都按还原算：Ofox 的模型不留给用户。
fn route_restored(files: &[FileState], state: &FileState, route: RouteRef) -> bool {
    let holder = match route.file {
        None => Some(state),
        Some(file) => files.iter().find(|other| other.file == file),
    };
    holder.is_none_or(|holder| {
        holder
            .rules
            .iter()
            .find(|leaf| leaf.path == route.path)
            .is_none_or(|leaf| restores(files, holder, leaf))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LEAVES: &[Leaf] = &[
        Leaf::route(&["provider"], |value| value.as_str() == Some("ofox")),
        Leaf::model(&["model"], &["provider"]),
        Leaf::preference(&["effort"]),
    ];

    fn leaves(provider: &str, model: &str, effort: &str) -> Leaves {
        leaves_at(
            &json!({ "provider": provider, "model": model, "effort": effort }),
            LEAVES,
        )
    }

    /// 绑定前：用户自己的服务商、模型和推理强度。
    fn original() -> Leaves {
        leaves("mine", "my-model", "medium")
    }

    fn kept_in(current: Leaves, written: &Leaves) -> BTreeSet<String> {
        let state = FileState {
            file: "f",
            rules: LEAVES,
            current,
            original: original(),
            written: Some(written),
        };
        kept(&[state]).remove("f").unwrap()
    }

    fn written() -> Leaves {
        let mut written = Leaves::new();
        record_write(
            &mut written,
            &leaves("mine", "my-model", "medium"),
            &leaves("ofox", "ofox/model", "high"),
            None,
        );
        written
    }

    #[test]
    fn only_values_the_user_changed_are_kept() {
        let kept = |current| kept_in(current, &written());

        assert!(kept(leaves("ofox", "ofox/model", "high")).is_empty());
        assert_eq!(
            kept(leaves("ofox", "ofox/model", "low")),
            ["effort".into()].into()
        );
        // A model picked in the agent goes with the Ofox provider it runs on.
        assert!(kept(leaves("ofox", "ofox/other", "high")).is_empty());
        // The user's own provider keeps the model they picked for it …
        assert_eq!(
            kept(leaves("relay", "relay-model", "high")),
            ["model".into(), "provider".into()].into()
        );
        // … but never Ofox's model id.
        assert_eq!(
            kept(leaves("relay", "ofox/model", "high")),
            ["provider".into()].into()
        );
    }

    #[test]
    fn an_ofox_shaped_route_is_restored_even_if_it_changed() {
        let mut written = Leaves::new();
        record_write(
            &mut written,
            &leaves_at(&json!({}), LEAVES),
            &leaves("relay", "m", "high"),
            None,
        );

        assert!(!kept_in(leaves("ofox", "m", "high"), &written).contains("provider"));
    }

    #[test]
    fn a_field_left_absent_goes_back_to_its_value_before_binding() {
        let current = leaves_at(&json!({ "model": "relay-model" }), LEAVES);

        let kept = kept_in(current, &written());

        assert!(!kept.contains("provider"), "{kept:?}");
        assert!(!kept.contains("effort"), "{kept:?}");
    }

    #[test]
    fn a_model_follows_its_route_in_another_file() {
        const ROUTE: &[Leaf] = &[Leaf::route(&["url"], |url| url == "ofox")];
        const MODEL: &[Leaf] = &[Leaf::model_in(&["model"], "env", &["url"])];
        let state = |file, rules, current: Value, original: Value, written| FileState {
            file,
            rules,
            current: leaves_at(&current, rules),
            original: leaves_at(&original, rules),
            written: Some(written),
        };
        let route_written = leaves_at(&json!({ "url": "ofox" }), ROUTE);
        let model_written = leaves_at(&json!({ "model": "ofox/m" }), MODEL);
        let unbind = |url: &str| {
            kept(&[
                state(
                    "env",
                    ROUTE,
                    json!({ "url": url }),
                    json!({ "url": "mine" }),
                    &route_written,
                ),
                state(
                    "settings",
                    MODEL,
                    json!({ "model": "picked" }),
                    json!({ "model": "before" }),
                    &model_written,
                ),
            ])
        };

        assert!(unbind("ofox")["settings"].is_empty());
        assert_eq!(unbind("relay")["settings"], ["model".to_string()].into());
    }

    #[test]
    fn unrecorded_fields_are_restored_as_before() {
        assert!(kept_in(leaves("relay", "x", "low"), &Leaves::new()).is_empty());
    }

    #[test]
    fn a_value_ofox_kept_for_the_user_on_a_rewrite_stays_theirs() {
        let mut written = written();
        let tuned = leaves("ofox", "ofox/model", "low");
        record_write(&mut written, &tuned, &tuned, None);

        assert_eq!(kept_in(tuned, &written), ["effort".into()].into());
    }

    #[test]
    fn a_first_record_takes_unchanged_fields_only_when_they_are_as_before_binding() {
        let original = leaves("mine", "my-model", "medium");
        let mut written = Leaves::new();
        // An older version bound it; the user has since changed the effort.
        record_write(
            &mut written,
            &leaves("ofox", "ofox/model", "low"),
            &leaves("ofox", "ofox/new", "low"),
            Some(&original),
        );
        assert!(!written.contains_key("effort"));

        let mut fresh = Leaves::new();
        let same = leaves("ofox", "ofox/model", "medium");
        record_write(&mut fresh, &same, &same, Some(&original));
        assert_eq!(fresh["effort"], json!("medium"));
    }

    #[test]
    fn keys_are_recorded_as_digests_only() {
        let value = json!({
            "base_url": "https://api.ofox.ai/v1",
            "experimental_bearer_token": "sk-of-secret123456",
            "headers": { "x": "Bearer abcdefghijklmnopqrstuvwxyz0123" },
        });

        let text = recorded("model_providers.ofox", &value).to_string();

        assert!(!text.contains("secret123456"), "{text}");
        assert!(!text.contains("abcdefghijklmnopqrstuvwxyz0123"), "{text}");
        assert!(text.contains("https://api.ofox.ai/v1"), "{text}");
        assert_ne!(
            recorded("GEMINI_API_KEY", &json!("sk-of-a")),
            recorded("GEMINI_API_KEY", &json!("sk-of-b"))
        );
        assert_eq!(recorded("model", &json!("gpt-x")), json!("gpt-x"));
    }
}
