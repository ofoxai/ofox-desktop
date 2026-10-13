//! ChatGPT 桌面 App 的 Codex 模式不把用户在界面里选的模型写进 `~/.codex/config.toml`，
//! 而是记在它自己的状态文件 `~/.codex/.codex-global-state.json` 里：
//! `electron-persisted-atom-state` 下的 `composer-recent-model-configurations-v1`，
//! 第一项就是最近一次选的。主页的 ChatGPT 行读它，才能跟上用户在 App 里的切换。
//! 文件是 App 的内部状态，格式没有承诺，所以读不到或形状不对时一律当作「没有」。

use super::read_text;

const ATOM_STATE: &str = "electron-persisted-atom-state";
const RECENT: &str = "composer-recent-model-configurations-v1";

/// 最近一次选的模型 id；没有、为空或形状不对时是 `None`。
fn parse_recent_model(text: &str) -> Option<String> {
    let state: serde_json::Value = serde_json::from_str(text).ok()?;
    let model = state
        .get(ATOM_STATE)?
        .get(RECENT)?
        .as_array()?
        .first()?
        .get("model")?
        .as_str()?
        .trim();
    (!model.is_empty()).then(|| model.to_string())
}

/// ChatGPT 桌面 App 里最近选的模型。读不了文件时返回 `None`，不报错：这只是展示用。
pub(crate) fn recent_model() -> Option<String> {
    let path = crate::codex_config::get_codex_config_dir().join(".codex-global-state.json");
    parse_recent_model(&read_text(&path).ok().flatten()?)
}

#[cfg(test)]
mod tests {
    use super::parse_recent_model;

    fn state(recent: &str) -> String {
        format!(
            r#"{{"other":1,"electron-persisted-atom-state":{{"composer-recent-model-configurations-v1":{recent}}}}}"#
        )
    }

    #[test]
    fn the_first_recent_model_is_the_current_one() {
        let text = state(r#"[{"model":"gpt-5.6-sol","effort":"high"},{"model":"gpt-6-luna"}]"#);
        assert_eq!(parse_recent_model(&text).as_deref(), Some("gpt-5.6-sol"));
    }

    #[test]
    fn an_empty_blank_or_oddly_shaped_list_means_no_model() {
        assert_eq!(parse_recent_model(&state("[]")), None);
        assert_eq!(parse_recent_model(&state(r#"[{"model":"  "}]"#)), None);
        assert_eq!(parse_recent_model(&state(r#"[{"model":5}]"#)), None);
        assert_eq!(parse_recent_model(&state(r#"[{"effort":"high"}]"#)), None);
        assert_eq!(parse_recent_model(&state(r#""x""#)), None);
    }

    #[test]
    fn missing_keys_and_broken_json_mean_no_model() {
        assert_eq!(
            parse_recent_model(r#"{"electron-persisted-atom-state":{}}"#),
            None
        );
        assert_eq!(parse_recent_model("{}"), None);
        assert_eq!(parse_recent_model("not json"), None);
        assert_eq!(parse_recent_model(""), None);
    }
}
