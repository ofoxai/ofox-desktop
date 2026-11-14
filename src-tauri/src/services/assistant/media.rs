//! Image requests stay in Rust; a submitted request is never replayed after
//! an ambiguous response. Only a refused key permits one deliberate retry.
use super::catalog::ImageCapabilities;
use base64::Engine;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;

pub(super) const INVALID: &str = "assistant_media_invalid_params";
pub(super) const UNKNOWN: &str = "assistant_media_outcome_unknown";
pub(super) const SAFETY: &str = "assistant_media_safety_rejected";
pub(super) const CREDITS: &str = "assistant_media_insufficient_credits";
pub(super) const MAX_IMAGE_BYTES: usize = 30 * 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageRequest {
    pub size: Option<String>,
    pub quality: Option<String>,
    pub n: Option<u8>,
    pub input_attachment_id: Option<String>,
}

impl ImageRequest {
    pub fn count(&self) -> usize {
        self.n.unwrap_or(1) as usize
    }
    pub fn validate(&self, caps: &ImageCapabilities) -> Result<(), String> {
        if !(1..=10).contains(&self.count())
            || (self.n.is_some() && !caps.n)
            || self.size.as_ref().is_some_and(|s| !caps.sizes.contains(s))
            || self
                .quality
                .as_ref()
                .is_some_and(|q| !caps.qualities.contains(q))
            || if self.input_attachment_id.is_some() {
                !caps.edits
            } else {
                !caps.generations
            }
        {
            return Err(INVALID.into());
        }
        Ok(())
    }
}

pub(super) fn cost(
    caps: &ImageCapabilities,
    usage: &Value,
    count: usize,
    edit: bool,
) -> Option<Decimal> {
    let price = |s: &str| Decimal::from_str(s).ok().filter(|n| *n >= Decimal::ZERO);
    if let Some(per_image) = &caps.per_image_price {
        return price(per_image)?.checked_mul(Decimal::from(count as u64));
    }
    let rates = caps.token_pricing.as_ref()?;
    let input = usage["input_tokens"].as_u64()?;
    let output = usage["output_tokens"].as_u64()?;
    let prompt_rate = price(rates.input.as_deref()?)?;
    let image_tokens = usage["input_tokens_details"]["image_tokens"].as_u64();
    let input_cost = match image_tokens {
        Some(images) if images <= input => {
            let image_rate = if images == 0 {
                Decimal::ZERO
            } else {
                price(rates.input_image.as_deref()?)?
            };
            Decimal::from(input - images)
                .checked_mul(prompt_rate)?
                .checked_add(Decimal::from(images).checked_mul(image_rate)?)?
        }
        Some(_) => return None,
        None if edit => Decimal::from(input)
            .checked_mul(prompt_rate.max(price(rates.input_image.as_deref()?)?))?,
        None => Decimal::from(input).checked_mul(prompt_rate)?,
    };
    input_cost
        .checked_add(Decimal::from(output).checked_mul(price(&rates.output_image)?)?)?
        .checked_div(Decimal::from(1_000_000))
}

/// The most one generation can cost on a token-priced model, so a budget can
/// be held before it is sent: the prompt's UTF-8 bytes as input tokens (a
/// prompt never has more tokens than bytes) and one and a half times the
/// published GPT Image output tokens for each image's size and quality. No
/// quality or `auto` is priced as high, and an unknown size as the largest.
/// `None` for a per-image price (that price is used as it is), an edit, or a
/// model without token prices.
pub(super) fn token_upper_bound(
    caps: &ImageCapabilities,
    prompt: &str,
    request: &ImageRequest,
) -> Option<Decimal> {
    if caps.per_image_price.is_some() || request.input_attachment_id.is_some() {
        return None;
    }
    let rates = caps.token_pricing.as_ref()?;
    let rate = |s: &str| Decimal::from_str(s).ok().filter(|n| *n >= Decimal::ZERO);
    let column = match request.quality.as_deref() {
        Some("low") => 0,
        Some("medium") => 1,
        _ => 2,
    };
    let tokens: u64 = if request.size.as_deref() == Some("1024x1024") {
        [272, 1056, 4160][column]
    } else {
        [408, 1584, 6240][column]
    };
    let output = Decimal::from(tokens * 3 / 2 * request.count() as u64)
        .checked_mul(rate(&rates.output_image)?)?;
    let input = Decimal::from(prompt.len() as u64).checked_mul(rate(rates.input.as_deref()?)?)?;
    input
        .checked_add(output)?
        .checked_div(Decimal::from(1_000_000))
}

pub(super) fn decode(value: &Value, expected: usize) -> Result<Vec<(String, Vec<u8>)>, String> {
    let data = value["data"]
        .as_array()
        .filter(|a| a.len() == expected && (1..=10).contains(&expected))
        .ok_or(UNKNOWN)?;
    data.iter()
        .map(|item| {
            let encoded = item["b64_json"]
                .as_str()
                .filter(|s| s.len() <= MAX_IMAGE_BYTES.div_ceil(3) * 4)
                .ok_or(UNKNOWN)?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| UNKNOWN)?;
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(UNKNOWN.into());
            }
            let ext = image_type(&bytes).ok_or(UNKNOWN)?;
            Ok((ext.into(), bytes))
        })
        .collect()
}

pub(super) fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::catalog::ImageTokenPricing;
    use super::*;
    use base64::Engine;
    use serde_json::json;
    use std::str::FromStr;

    #[test]
    fn a_token_priced_generation_has_an_upper_bound_from_its_size_and_quality() {
        let request = |size: Option<&str>, quality: Option<&str>| ImageRequest {
            size: size.map(Into::into),
            quality: quality.map(Into::into),
            ..ImageRequest::default()
        };
        let bound =
            |size, quality| token_upper_bound(&caps(), "ab", &request(size, quality)).unwrap();
        // 2 prompt bytes at $5/M plus 1.5 x the published output tokens at $30/M.
        assert_eq!(
            bound(Some("1024x1024"), Some("low")),
            Decimal::from_str("0.01225").unwrap()
        );
        assert_eq!(
            bound(Some("1536x1024"), Some("high")),
            Decimal::from_str("0.28081").unwrap()
        );
        // Unknown or automatic quality is priced as high; an unknown size as the largest.
        assert_eq!(
            bound(Some("1536x1024"), None),
            bound(Some("1536x1024"), Some("high"))
        );
        assert_eq!(
            bound(None, Some("auto")),
            bound(Some("1024x1536"), Some("high"))
        );
        // A per-image price, an edit or a model without token prices gives no bound here.
        let mut fixed = caps();
        fixed.per_image_price = Some("0.02".into());
        assert!(token_upper_bound(&fixed, "ab", &request(None, None)).is_none());
        let edit = ImageRequest {
            input_attachment_id: Some("a".into()),
            ..ImageRequest::default()
        };
        assert!(token_upper_bound(&caps(), "ab", &edit).is_none());
        let mut unpriced = caps();
        unpriced.token_pricing = None;
        assert!(token_upper_bound(&unpriced, "ab", &request(None, None)).is_none());
    }

    fn caps() -> ImageCapabilities {
        ImageCapabilities {
            generations: true,
            edits: true,
            n: true,
            sizes: vec!["1024x1024".into()],
            qualities: vec!["low".into()],
            per_image_price: None,
            token_pricing: Some(ImageTokenPricing {
                input: Some("5".into()),
                input_image: Some("10".into()),
                output_image: "30".into(),
            }),
        }
    }

    #[test]
    fn media_contract_survives_session_and_ledger_serialization() {
        use super::super::types::{Attachment, Message, Operation};
        let media = json!({"kind":"image","model":"test/image","params":{},"inputIds":[],"status":"completed","outputs":["image-1"],"cost":"0.02"});
        let message: Message = serde_json::from_value(json!({"id":"message-1","role":"media","content":"blue square","createdAt":1,"reasoning":null,"status":"completed","revisionOf":null,"nativeEntryId":null,"media":media})).unwrap();
        assert_eq!(serde_json::to_value(&message).unwrap()["media"], media);
        let operation: Operation = serde_json::from_value(
            json!({"id":"operation-1","runId":"run-1","status":"completed","media":media}),
        )
        .unwrap();
        assert_eq!(serde_json::to_value(&operation).unwrap()["media"], media);
        let file: Attachment = serde_json::from_value(json!({"id":"image-1","name":"image.png","mimeType":"image/png","sizeBytes":12,"status":"ready","origin":"generated","sourceMessageId":"message-1"})).unwrap();
        let file = serde_json::to_value(file).unwrap();
        assert_eq!(file["origin"], "generated");
        assert_eq!(file["sourceMessageId"], "message-1");
    }

    #[test]
    fn accepts_only_advertised_parameters_and_supported_modes() {
        let mut request = ImageRequest::default();
        assert!(request.validate(&caps()).is_ok());
        for n in [0, 11, 255] {
            request.n = Some(n);
            assert_eq!(request.validate(&caps()).unwrap_err(), INVALID);
        }
        request.n = None;
        request.size = Some("arbitrary".into());
        assert_eq!(request.validate(&caps()).unwrap_err(), INVALID);
        request.size = Some("1024x1024".into());
        request.quality = Some("low".into());
        assert!(request.validate(&caps()).is_ok());
        let mut unsupported = caps();
        unsupported.edits = false;
        request.input_attachment_id = Some("input".into());
        assert_eq!(request.validate(&unsupported).unwrap_err(), INVALID);
        request.input_attachment_id = None;
        unsupported.generations = false;
        assert_eq!(request.validate(&unsupported).unwrap_err(), INVALID);
        let mut one = caps();
        one.n = false;
        request.n = Some(2);
        assert_eq!(request.validate(&one).unwrap_err(), INVALID);
    }

    #[test]
    fn charges_per_image_or_separate_text_image_output_tokens_without_guessing() {
        let d = |n| Some(Decimal::from_str(n).unwrap());
        let mut per_image = caps();
        per_image.per_image_price = Some("0.02".into());
        assert_eq!(cost(&per_image, &Value::Null, 3, false), d("0.06"));
        let usage = json!({"input_tokens":120, "input_tokens_details":{"text_tokens":100,"image_tokens":20}, "output_tokens":200});
        assert_eq!(cost(&caps(), &usage, 1, true), d("0.0067"));
        assert_eq!(
            cost(
                &caps(),
                &json!({"input_tokens":100,"output_tokens":200}),
                1,
                false
            ),
            d("0.0065")
        );
        // Edits with no input split take the more expensive input rate.
        assert_eq!(
            cost(
                &caps(),
                &json!({"input_tokens":100,"output_tokens":200}),
                1,
                true
            ),
            d("0.007")
        );
        assert_eq!(cost(&caps(), &Value::Null, 1, false), None);
        assert_eq!(cost(&caps(), &json!({"input_tokens":100}), 1, false), None);
        assert_eq!(
            cost(
                &caps(),
                &json!({"input_tokens":-1,"output_tokens":200}),
                1,
                false
            ),
            None
        );
        let mut unknown = caps();
        unknown.token_pricing.as_mut().unwrap().input_image = None;
        assert_eq!(cost(&unknown, &usage, 1, true), None);
        per_image.per_image_price = Some("-2".into());
        assert_eq!(cost(&per_image, &Value::Null, 3, false), None);
    }

    #[test]
    fn accepts_only_bounded_image_bytes_and_exact_result_count() {
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let png = b"\x89PNG\r\n\x1a\nfixture";
        let valid = json!({"data":[{"b64_json":b64(png)}]});
        assert_eq!(
            decode(&valid, 1).unwrap(),
            vec![("png".into(), png.to_vec())]
        );
        for value in [
            json!({"data":[]}),
            json!({"data":[{"url":"https://untrusted.invalid/image"}]}),
            json!({"data":[{"b64_json":b64(b"not an image")}]}),
            json!({"data":[{"b64_json":"invalid!"}]}),
        ] {
            assert_eq!(decode(&value, 1).unwrap_err(), UNKNOWN);
        }
        assert_eq!(decode(&valid, 2).unwrap_err(), UNKNOWN);
        let huge = vec![0; MAX_IMAGE_BYTES + 1];
        assert_eq!(
            decode(&json!({"data":[{"b64_json":b64(&huge)}]}), 1).unwrap_err(),
            UNKNOWN
        );
    }
    #[test]
    fn generated_files_are_private_referenced_and_roundtrip_in_backup() {
        use super::super::{
            attachments::{self, Source},
            projection::recount_usage,
            store::AssistantStore,
            types::*,
        };
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        std::fs::create_dir(&config).unwrap();
        let store = AssistantStore::for_owner(&config, &"ab".repeat(32)).unwrap();
        let sid = store.create("test/image", false).unwrap().session.id;
        let mid = id();
        let run = id();
        let operation = id();
        let media = MediaRecord {
            kind: "image".into(),
            model: "test/image".into(),
            params: json!({}),
            input_ids: vec![],
            status: "completed".into(),
            outputs: vec![],
            cost: Some("0.02".into()),
            ..Default::default()
        };
        store.mutate(&sid,|record| {
            record.messages.push(serde_json::from_value(json!({"id":mid,"role":"media","content":"blue square","createdAt":1,"status":"streaming","runId":run,"media":media})).unwrap());
            record.operations.push(Operation{media:Some(media.clone()),id:operation,run_id:run,status:OperationStatus::Running,search_price:None}); Ok(())
        }).unwrap();
        let png = b"\x89PNG\r\n\x1a\nfixture".to_vec();
        let files = attachments::generated(
            &store,
            &sid,
            &mid,
            None,
            &[Source::Bytes {
                name: "generated.png".into(),
                bytes: png.clone(),
            }],
            media.clone(),
            |_| Ok(()),
        )
        .unwrap();
        let path = store.attachment_root(&sid).unwrap().join(&files[0].id);
        assert_eq!(std::fs::read(&path).unwrap(), png);
        #[cfg(unix)]
        assert_eq!(crate::config::file_mode(&path), Some(0o600));
        store.cleanup_orphan_attachments(&sid).unwrap();
        assert!(path.is_file());
        store
            .mutate(&sid, |record| {
                record.messages[0].status = "completed".into();
                record.operations[0].status = OperationStatus::Completed;
                recount_usage(record, &[]);
                Ok(())
            })
            .unwrap();
        let archive = root.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let other = root.path().join("restored");
        std::fs::create_dir(&other).unwrap();
        let restored = AssistantStore::for_owner(&other, &"ab".repeat(32)).unwrap();
        restored.restore(&archive, &[], || Ok(())).unwrap();
        let record = restored.load_record(&sid).unwrap();
        assert_eq!(
            record.messages[0].media.as_ref().unwrap().outputs,
            [files[0].id.clone()]
        );
        assert_eq!(record.usage["mediaCostUsd"], "0.02");
        assert_eq!(restored.usage_summary().unwrap()["mediaCostUsd"], "0.02");
        assert_eq!(
            std::fs::read(restored.attachment_root(&sid).unwrap().join(&files[0].id)).unwrap(),
            png
        );
        let before = std::fs::read_dir(store.attachment_root(&sid).unwrap())
            .unwrap()
            .count();
        assert!(attachments::generated(
            &store,
            &sid,
            &mid,
            None,
            &[Source::Bytes {
                name: "refused.png".into(),
                bytes: png
            }],
            media,
            |_| Err("assistant_identity_changed".into())
        )
        .is_err());
        assert_eq!(
            std::fs::read_dir(store.attachment_root(&sid).unwrap())
                .unwrap()
                .count(),
            before
        );
    }

    #[test]
    fn remembered_media_models_do_not_replace_the_legacy_chat_default() {
        use super::super::{catalog::Kind, store::AssistantStore};
        let root = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
        store.remember_model("test/chat").unwrap();
        store
            .remember_model_kind("test/image", Kind::Image)
            .unwrap();
        store
            .remember_model_kind("test/video", Kind::Video)
            .unwrap();
        assert_eq!(
            store.remembered_model().unwrap().as_deref(),
            Some("test/chat")
        );
        assert_eq!(
            store.remembered_models().unwrap(),
            json!({"chat":"test/chat","image":"test/image","video":"test/video", "speech":null, "transcription":null})
        );
    }
    #[test]
    fn image_submission_requires_room_for_worst_case_results() {
        use super::super::{attachments, store::AssistantStore, types::*};
        let root = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
        let sid = store.create("test/image", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        let file:Attachment=serde_json::from_value(json!({"id":id(),"name":"existing.png","mimeType":"image/png","sizeBytes":attachments::MAX_SESSION_BYTES-MAX_IMAGE_BYTES as u64+1,"status":"ready"})).unwrap();
        record.attachments.push(file.clone());
        assert_eq!(
            attachments::image_room(&record, 1).unwrap_err(),
            attachments::TOTAL_TOO_LARGE
        );
        record.attachments = vec![
            Attachment {
                size_bytes: 1,
                ..file
            };
            attachments::MAX_PER_SESSION
        ];
        assert_eq!(
            attachments::image_room(&record, 1).unwrap_err(),
            attachments::TOO_MANY
        );
        record.attachments.clear();
        record.session.active_run = Some(RunRef {
            run_id: id(),
            operation_id: id(),
        });
        assert_eq!(
            attachments::image_room(&record, 1).unwrap_err(),
            "assistant_session_busy"
        );
    }
    #[test]
    fn media_turns_cannot_be_edited_after_switching_to_chat() {
        use super::super::{requests::Branch, store::AssistantStore, types::*};
        let root = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
        let sid = store.create("test/chat", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        let run = id();
        let mid = id();
        record.messages.push(serde_json::from_value(json!({"id":mid,"role":"user","content":"blue square","createdAt":1,"status":"completed","runId":run,"nativeEntryId":"native-user"})).unwrap());
        record.messages.push(serde_json::from_value(json!({"id":id(),"role":"media","content":"","createdAt":2,"status":"completed","runId":run,"media":{"kind":"image","model":"test/image","params":{},"status":"completed","cost":"0.02"}})).unwrap());
        let request:SendRequest=serde_json::from_value(json!({"sessionId":sid,"runId":id(),"operationId":id(),"text":"red square","editMessageId":mid})).unwrap();
        assert_eq!(
            Branch::resolve(&record, &request).err().as_deref(),
            Some(INVALID)
        );
    }
}
