//! Explicit audio tools share the conversation's run ledger, never its turns.
mod host;
mod http;
mod job;
use super::{audio_input::Info, catalog::AudioPricing, types::*};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const INPUT_UNAVAILABLE: &str = "assistant_audio_input_unavailable";
pub const INVALID: &str = "assistant_audio_invalid_params";
pub const UNKNOWN: &str = "assistant_audio_outcome_unknown";
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    pub pricing: AudioPricing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<super::speech::OutputInfo>,
    pub input: Option<Info>,
    pub text: Option<String>,
    #[serde(default)]
    pub usage: Value,
    pub request_id: Option<String>,
    pub request_id_source: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultRecord {
    pub operation_id: String,
    pub run_id: String,
    #[serde(flatten)]
    pub media: MediaRecord,
}
pub(super) fn results(operations: &[Operation]) -> Vec<ResultRecord> {
    operations
        .iter()
        .filter_map(|op| {
            let media = op.media.as_ref().filter(|m| {
                matches!(m.kind.as_str(), "transcription" | "speech") && m.audio.is_some()
            })?;
            Some(ResultRecord {
                operation_id: op.id.clone(),
                run_id: op.run_id.clone(),
                media: media.clone(),
            })
        })
        .collect()
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscribeRequest {
    pub session_id: String,
    pub expected_owner: String,
    pub expected_generation: u64,
    pub input_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub model_id: String,
    pub language: Option<String>,
}
impl TranscribeRequest {
    pub(super) fn validate(&self) -> Result<(), String> {
        for id in [
            &self.session_id,
            &self.input_id,
            &self.run_id,
            &self.operation_id,
        ] {
            validate_id(id)?;
        }
        if self
            .language
            .as_deref()
            .is_some_and(|v| !["zh", "en", "ja"].contains(&v))
        {
            return Err(INVALID.into());
        }
        Ok(())
    }
}

pub(super) fn cost(prices: &AudioPricing, usage: &Value) -> Option<Decimal> {
    use super::catalog::AudioBillingUnit;
    let rate = |s: &str| s.parse::<Decimal>().ok().filter(|n| *n >= Decimal::ZERO);
    match prices.unit {
        AudioBillingUnit::Seconds if usage["type"] == "duration" => {
            let seconds = usage["seconds"]
                .as_number()?
                .to_string()
                .parse::<Decimal>()
                .ok()
                .filter(|n| *n >= Decimal::ZERO)?;
            seconds.checked_mul(rate(prices.input_audio_per_second.as_deref()?)?)
        }
        AudioBillingUnit::Tokens if usage["type"] == "tokens" => {
            let input = usage["input_tokens"].as_u64()?;
            let output = usage["output_tokens"].as_u64()?;
            let text_rate = rate(prices.input_text_per_million.as_deref()?)?;
            let audio_rate = rate(prices.input_audio_per_million.as_deref()?)?;
            let first = &usage["input_token_details"];
            let second = &usage["input_tokens_details"];
            if !first.is_null() && !second.is_null() && first != second {
                return None;
            }
            let details = if first.is_null() { second } else { first };
            let input_cost = if text_rate == audio_rate {
                Decimal::from(input).checked_mul(text_rate)?
            } else if input == 0 {
                Decimal::ZERO
            } else {
                let audio = details["audio_tokens"].as_u64()?;
                let text = details["text_tokens"].as_u64()?;
                if text.checked_add(audio)? != input {
                    return None;
                }
                Decimal::from(text)
                    .checked_mul(text_rate)?
                    .checked_add(Decimal::from(audio).checked_mul(audio_rate)?)?
            };
            input_cost
                .checked_add(
                    Decimal::from(output)
                        .checked_mul(rate(prices.output_text_per_million.as_deref()?)?)?,
                )?
                .checked_div(Decimal::from(1_000_000))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn audio_cost_uses_reported_duration_and_keeps_missing_usage_unknown() {
        let price =
            serde_json::from_value(json!({"unit":"seconds","inputAudioPerSecond":"0.000075"}))
                .unwrap();
        assert_eq!(
            cost(&price, &json!({"type":"duration","seconds":9}))
                .map(|n| n.normalize().to_string()),
            Some("0.000675".into())
        );
        for usage in [
            json!({}),
            json!({"type":"duration","seconds":-1}),
            json!({"type":"tokens","seconds":9}),
        ] {
            assert!(cost(&price, &usage).is_none());
        }
    }
    #[test]
    fn audio_token_cost_needs_the_text_audio_split_when_rates_differ() {
        let price=serde_json::from_value(json!({"unit":"tokens","inputTextPerMillion":"1.25","inputAudioPerMillion":"3","outputTextPerMillion":"5"})).unwrap();
        assert_eq!(cost(&price,&json!({"type":"tokens","input_tokens":100,"output_tokens":4,"input_token_details":{"text_tokens":20,"audio_tokens":80}})).map(|n|n.normalize().to_string()),Some("0.000285".into()));
        assert!(cost(
            &price,
            &json!({"type":"tokens","input_tokens":100,"output_tokens":4})
        )
        .is_none());
        assert!(cost(&price,&json!({"type":"tokens","input_tokens":100,"output_tokens":4,"input_token_details":{"text_tokens":20,"audio_tokens":101}})).is_none());
    }
}
