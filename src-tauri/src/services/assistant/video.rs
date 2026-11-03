//! Video parameters, quotes and persisted job-state rules. HTTP stays in Rust.
use super::{catalog::VideoCapabilities, media, types::validate_id};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(super) const QUOTE_EXPIRED: &str = "assistant_media_quote_expired";
pub(super) const MAX_VIDEO_BYTES: u64 = 200 * 1024 * 1024;
pub(super) const QUOTE_TTL_MS: i64 = 10 * 60 * 1000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoRequest {
    pub duration: u64,
    pub resolution: String,
    pub aspect_ratio: Option<String>,
    #[serde(default)]
    pub generate_audio: bool,
    pub first_frame_id: Option<String>,
    pub last_frame_id: Option<String>,
}
impl VideoRequest {
    pub fn validate(&self, caps: &VideoCapabilities) -> Result<(), String> {
        let mode = if self.first_frame_id.is_some() {
            "i2v"
        } else {
            "t2v"
        };
        if !caps.modes.iter().any(|value| value == mode)
            || self.duration == 0
            || caps.min_duration.is_none_or(|min| self.duration < min)
            || caps.max_duration.is_none_or(|max| self.duration > max)
            || !caps.resolutions.contains(&self.resolution)
            || self
                .aspect_ratio
                .as_ref()
                .is_some_and(|aspect| !caps.aspect_ratios.contains(aspect))
            || (self.generate_audio && !caps.supports_audio)
            || (self.last_frame_id.is_some() && self.first_frame_id.is_none())
        {
            return Err(media::INVALID.into());
        }
        for id in self.first_frame_id.iter().chain(self.last_frame_id.iter()) {
            validate_id(id).map_err(|_| media::INVALID)?;
        }
        Ok(())
    }
}
/// Do not use the catalog's generic "from" price for a resolution-specific
/// confirmation. Image-to-video uses the text-to-video tier by contract.
pub(super) fn estimate(catalog: &Value, params: &VideoRequest) -> Option<Decimal> {
    use std::str::FromStr;
    let mut highest = None;
    for provider in catalog["provider_cards"].as_array()? {
        let pricing = &provider["pricing"]["video_pricing"];
        if pricing["unit"]
            .as_str()
            .is_some_and(|unit| unit != "per_second")
        {
            return None;
        }
        for tier in pricing["tiers"].as_array()? {
            if tier["resolution"].as_str() == Some(&params.resolution)
                && tier["input_type"] == "t2v"
            {
                let rate = Decimal::from_str(tier["price"].as_str()?).ok()?;
                if rate < Decimal::ZERO {
                    return None;
                }
                highest = Some(highest.map_or(rate, |old: Decimal| old.max(rate)));
            }
        }
    }
    highest?.checked_mul(Decimal::from(params.duration))
}
pub(super) fn validate_quote(
    quoted: &VideoRequest,
    submitted: &VideoRequest,
    old_price: Option<Decimal>,
    new_price: Option<Decimal>,
    issued_at: i64,
    now: i64,
) -> Result<(), String> {
    if quoted != submitted
        || old_price != new_price
        || now < issued_at
        || now.saturating_sub(issued_at) >= QUOTE_TTL_MS
    {
        return Err(QUOTE_EXPIRED.into());
    }
    Ok(())
}
pub(super) fn job_id(response: &Value) -> Result<String, String> {
    let id = response["id"].as_str().ok_or(media::UNKNOWN)?;
    validate_id(id).map_err(|_| media::UNKNOWN)?;
    Ok(id.to_owned())
}
pub(super) fn active_status(status: &str) -> bool {
    matches!(
        status,
        "submitting"
            | "queued"
            | "in_progress"
            | "downloading"
            | "paused"
            | "unknown"
            | "query_expired"
            | "download_failed"
    )
}

/// Only unresolved remote generation reserves a slot. Local download recovery
/// remains mutable without blocking another generation or permanent deletion.
pub(super) fn occupies_generation_slot(status: &str) -> bool {
    active_status(status) && !matches!(status, "downloading" | "download_failed")
}

/// The videos of `record` still being generated: direct media turns and
/// generation tool calls alike.
pub(super) fn occupied_slots(record: &super::store::SessionRecord) -> usize {
    record
        .messages
        .iter()
        .flat_map(|message| message.all_media())
        .filter(|media| media.kind == "video" && occupies_generation_slot(&media.status))
        .count()
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoActionRequest {
    #[serde(flatten)]
    pub authority: super::types::DeleteRequest,
    pub message_id: String,
    /// A generation tool's video; `None` for a direct media turn.
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::str::FromStr;
    fn params() -> VideoRequest {
        serde_json::from_value(json!({"duration":4,"resolution":"480p","generateAudio":false}))
            .unwrap()
    }
    fn caps() -> VideoCapabilities {
        VideoCapabilities {
            modes: vec!["t2v".into(), "i2v".into()],
            resolutions: vec!["480p".into(), "720p".into()],
            default_resolution: Some("720p".into()),
            min_duration: Some(4),
            max_duration: Some(15),
            supports_audio: false,
            aspect_ratios: vec!["16:9".into()],
            from_price_per_second: Some("0.02".into()),
        }
    }
    #[test]
    fn video_job_metadata_survives_roundtrip_without_persisting_download_urls() {
        use super::super::types::MediaRecord;
        let value = json!({"kind":"video","model":"test/video","params":{"duration":4},"inputIds":[],"outputs":[],"status":"queued","cost":null,
            "jobId":super::super::types::id(),"apex":"ofox.io","submittedAt":100,"pollStartedAt":200});
        let media: MediaRecord = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(media).unwrap(), value);
    }
    #[test]
    fn a_persisted_background_video_is_not_an_interrupted_chat_run() {
        use super::super::{
            projection,
            store::AssistantStore,
            types::{Message, Operation},
        };
        let root = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(root.path(), &"ef".repeat(32)).unwrap();
        let sid = store.create("test/video", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        let media = json!({"kind":"video","model":"test/video","params":{},"status":"queued","jobId":super::super::types::id(),"apex":"ofox.io","submittedAt":100,"pollStartedAt":100});
        record.operations.push(
            serde_json::from_value::<Operation>(
                json!({"id":"op","runId":"run","status":"pendingMedia","media":media}),
            )
            .unwrap(),
        );
        record.messages.push(serde_json::from_value::<Message>(json!({"id":"reply","role":"media","content":"","createdAt":1,"status":"queued","runId":"run","media":media})).unwrap());
        assert!(!projection::is_dangling(&record));
        assert_eq!(
            serde_json::to_value(&record.operations[0]).unwrap()["media"]["jobId"],
            media["jobId"]
        );
    }

    #[test]
    fn video_params_require_advertised_modes_limits_and_valid_frame_ids() {
        let mut p = params();
        let caps = caps();
        assert!(p.validate(&caps).is_ok());
        p.duration = 3;
        assert!(p.validate(&caps).is_err());
        p.duration = 4;
        p.resolution = "1080p".into();
        assert!(p.validate(&caps).is_err());
        p.resolution = "480p".into();
        p.generate_audio = true;
        assert!(p.validate(&caps).is_err());
        p.generate_audio = false;
        p.aspect_ratio = Some("9:16".into());
        assert!(p.validate(&caps).is_err());
        p.aspect_ratio = None;
        p.last_frame_id = Some(super::super::types::id());
        assert!(p.validate(&caps).is_err());
        p.first_frame_id = Some(super::super::types::id());
        assert!(p.validate(&caps).is_ok());
        p.first_frame_id = Some("../outside".into());
        assert!(p.validate(&caps).is_err());
        p.first_frame_id = Some(super::super::types::id());
        let mut text_only = caps.clone();
        text_only.modes = vec!["t2v".into()];
        assert!(p.validate(&text_only).is_err());
    }
    #[test]
    fn video_quote_uses_highest_matching_resolution_t2v_rate_including_image_input() {
        let catalog = json!({"provider_cards":[
            {"pricing":{"video_pricing":{"tiers":[
                {"resolution":"480p","input_type":"t2v","price":"0.02"},
                {"resolution":"720p","input_type":"t2v","price":"0.9"},
                {"resolution":"480p","input_type":"v2v","price":"0.8"},{"price":"1"}]}}},
            {"pricing":{"video_pricing":{"tiers":[{"resolution":"480p","input_type":"t2v","price":"0.03"}]}}}
        ]});
        let mut p = params();
        assert_eq!(
            estimate(&catalog, &p),
            Some(Decimal::from_str("0.12").unwrap())
        );
        p.first_frame_id = Some(super::super::types::id());
        assert_eq!(
            estimate(&catalog, &p),
            Some(Decimal::from_str("0.12").unwrap())
        );
        p.resolution = "1080p".into();
        assert_eq!(estimate(&catalog, &p), None);
        assert_eq!(
            estimate(
                &json!({"pricing":{"output_video_per_second":"0.01"}}),
                &params()
            ),
            None
        );
        assert_eq!(
            estimate(
                &json!({"provider_cards":[{"pricing":{"video_pricing":{"tiers":[{"resolution":"480p","input_type":"t2v","price":"-1"}]}}}]}),
                &params()
            ),
            None
        );
    }
    #[test]
    fn video_quote_expires_and_refuses_parameter_or_price_changes() {
        let p = params();
        let price = Some(Decimal::ONE);
        assert!(validate_quote(&p, &p, price, price, 100, 599_999).is_ok());
        assert!(validate_quote(&p, &p, price, price, 100, 600_100).is_err());
        assert!(validate_quote(&p, &p, price, price, 100, 99).is_err());
        let mut changed = p.clone();
        changed.duration = 5;
        assert!(validate_quote(&p, &changed, price, price, 100, 200).is_err());
        assert!(validate_quote(&p, &p, price, Some(Decimal::from(2)), 100, 200).is_err());
        assert!(validate_quote(&p, &p, None, None, 100, 200).is_ok());
        assert!(validate_quote(&p, &p, price, None, 100, 200).is_err());
    }
    #[test]
    fn accepted_video_requires_a_job_id_and_never_trusts_polling_url() {
        let id = super::super::types::id();
        assert_eq!(
            job_id(
                &json!({"id":id,"status":"pending","polling_url":"https://outside.invalid/steal"})
            )
            .unwrap(),
            id
        );
        for value in [
            json!({}),
            json!({"id":"../../outside"}),
            json!({"id":"https://outside.invalid"}),
        ] {
            assert_eq!(job_id(&value).unwrap_err(), media::UNKNOWN);
        }
    }
    #[test]
    fn video_recovery_accepts_download_failures_without_reserving_generation_slots() {
        for status in [
            "submitting",
            "queued",
            "in_progress",
            "downloading",
            "paused",
            "unknown",
            "query_expired",
            "download_failed",
        ] {
            assert!(active_status(status), "{status}");
            assert_eq!(
                occupies_generation_slot(status),
                !matches!(status, "downloading" | "download_failed")
            );
        }
        for status in ["completed", "failed", "cancelled", "expired"] {
            assert!(!active_status(status));
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuoteMediaRequest {
    #[serde(flatten)]
    pub authority: super::types::DeleteRequest,
    pub params: VideoRequest,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaQuote {
    pub id: String,
    pub estimated_cost_usd: Option<String>,
    pub expires_at: i64,
}
