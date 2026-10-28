//! Thinking levels: which a conversation may choose for a model, and what a
//! run at a level needs (Pi's map from a level to what the provider is sent,
//! and the Anthropic compat adaptive thinking reads). From
//! `thinking_levels.json`, generated from the Pi SDK's model data by
//! `scripts/assistant-thinking-levels.mjs`; a runtime test fails when the
//! committed table no longer matches the pinned SDK.
use super::catalog::Protocol;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

/// The value of `UpdateRequest.thinking_level` that clears the choice: the
/// run then sends no thinking parameter and the provider's default applies.
pub const DEFAULT: &str = "default";

/// Offered to a model the catalog says reasons but Pi has no data for.
const WITHOUT_PI_DATA: [&str; 3] = ["low", "medium", "high"];

/// A level → what the provider is sent; `None`: the level is not offered.
pub type LevelMap = BTreeMap<String, Option<String>>;

/// The Anthropic compat fields Pi's Messages adapter reads for thinking.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnthropicCompat {
    /// Adaptive thinking with an effort instead of a token budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_adaptive_thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_temperature: Option<bool>,
}

/// What a chat model offers for thinking.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thinking {
    /// In Pi's order (`off`, `minimal`, `low`, `medium`, `high`, `xhigh`,
    /// `max`); empty when the model does not reason.
    pub levels: Vec<String>,
    #[serde(default)]
    pub thinking_level_map: Option<LevelMap>,
    #[serde(default)]
    pub anthropic_compat: Option<AnthropicCompat>,
}

#[derive(Deserialize)]
struct Table {
    models: HashMap<String, Thinking>,
}

fn table() -> &'static HashMap<String, Thinking> {
    static TABLE: OnceLock<HashMap<String, Thinking>> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str::<Table>(include_str!("thinking_levels.json"))
            .expect("the generated thinking_levels.json parses")
            .models
    })
}

/// An Ofox id as the table keys it (see the generator's `tableKey`):
/// lowercase, and Anthropic versions with hyphens (`claude-haiku-5.5` is
/// Pi's `claude-haiku-5-5`).
fn key(id: &str) -> Option<String> {
    let id = id.to_lowercase();
    let (vendor, name) = id.split_once('/')?;
    Some(match vendor {
        "anthropic" => format!("{vendor}/{}", name.replace('.', "-")),
        _ => id.clone(),
    })
}

/// The thinking a chat model offers: nothing unless the catalog lists
/// `reasoning`; then Pi's levels and map for it, or low/medium/high when Pi
/// does not know it. The Anthropic compat only for a model run with Messages.
pub fn for_model(id: &str, reasoning: bool, protocol: Option<Protocol>) -> Thinking {
    if !reasoning {
        return Thinking::default();
    }
    match key(id).and_then(|key| table().get(&key)) {
        Some(known) => Thinking {
            levels: known.levels.clone(),
            thinking_level_map: known.thinking_level_map.clone(),
            anthropic_compat: known
                .anthropic_compat
                .clone()
                .filter(|_| protocol == Some(Protocol::AnthropicMessages)),
        },
        None => Thinking {
            levels: WITHOUT_PI_DATA.map(str::to_owned).to_vec(),
            ..Thinking::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PI_ORDER: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

    fn levels(thinking: &Thinking) -> Vec<&str> {
        thinking.levels.iter().map(String::as_str).collect()
    }

    #[test]
    fn every_entry_offers_known_levels_in_pis_order() {
        assert!(table().len() > 50);
        for (key, thinking) in table() {
            let positions: Vec<usize> = thinking
                .levels
                .iter()
                .map(|level| PI_ORDER.iter().position(|known| known == level).unwrap())
                .collect();
            assert!(!positions.is_empty(), "{key}");
            assert!(positions.windows(2).all(|w| w[0] < w[1]), "{key}");
            for level in thinking.thinking_level_map.iter().flat_map(|m| m.keys()) {
                assert!(PI_ORDER.contains(&level.as_str()), "{key}: {level}");
            }
        }
    }

    #[test]
    fn claude_gets_pis_levels_map_and_compat_only_with_messages() {
        let messages = for_model(
            "anthropic/claude-haiku-5.5",
            true,
            Some(Protocol::AnthropicMessages),
        );
        assert_eq!(
            levels(&messages),
            ["off", "low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(
            messages.anthropic_compat,
            Some(AnthropicCompat {
                force_adaptive_thinking: Some(true),
                supports_temperature: Some(false),
            })
        );
        let map = messages.thinking_level_map.unwrap();
        assert_eq!(map["minimal"], None);
        assert_eq!(map["high"].as_deref(), Some("high"));
        let completions = for_model(
            "anthropic/claude-haiku-5.5",
            true,
            Some(Protocol::OpenAiCompletions),
        );
        assert!(completions.anthropic_compat.is_none());
        assert_eq!(completions.levels, messages.levels);
    }

    #[test]
    fn other_vendors_match_case_insensitively_and_keep_dots() {
        let luna = for_model("openai/gpt-6-luna", true, Some(Protocol::OpenAiResponses));
        assert_eq!(
            levels(&luna),
            ["off", "low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(
            luna.thinking_level_map.unwrap()["off"].as_deref(),
            Some("none")
        );
        let gemini = for_model(
            "google/gemini-3.1-flash-lite",
            true,
            Some(Protocol::OpenAiCompletions),
        );
        // Gemini 3 cannot turn thinking off.
        assert_eq!(levels(&gemini), ["minimal", "low", "medium", "high"]);
        let minimax = for_model("minimax/minimax-m3", true, None);
        assert_eq!(
            levels(&minimax),
            ["off", "minimal", "low", "medium", "high"]
        );
        assert!(minimax.anthropic_compat.is_none());
    }

    #[test]
    fn a_reasoning_model_pi_does_not_know_gets_low_medium_high() {
        let unknown = for_model("deepseek/deepseek-v4.1-flash", true, None);
        assert_eq!(levels(&unknown), ["low", "medium", "high"]);
        assert!(unknown.thinking_level_map.is_none());
        assert_eq!(
            levels(&for_model("unprefixed", true, None)),
            WITHOUT_PI_DATA
        );
    }

    #[test]
    fn a_model_without_reasoning_offers_no_level() {
        assert_eq!(
            for_model("openai/gpt-6-luna", false, None),
            Thinking::default()
        );
    }
}
