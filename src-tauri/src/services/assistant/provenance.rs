//! Correlation facts for generated requests. Gateway headers are not invoices.
use super::{
    host::{blocking, scrub_secrets_in, AssistantHost, RunControl},
    store::SessionRecord,
    types::*,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub http_status: u16,
    pub request_id: Option<String>,
    pub request_id_source: Option<String>,
}
impl Receipt {
    pub(super) fn from_response(response: &reqwest::Response, key: &str) -> Self {
        let mut result = Self {
            http_status: response.status().as_u16(),
            ..Self::default()
        };
        for name in ["x-ofox-request-id", "x-request-id", "request-id"] {
            if let Some(value) = response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .filter(|v| {
                    !v.is_empty()
                        && v.len() <= 200
                        && (key.is_empty() || !v.contains(key))
                        && v.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
                })
            {
                result.request_id = Some(value.to_owned());
                result.request_id_source = Some(name.to_owned());
                break;
            }
        }
        result
    }
}
#[derive(Debug)]
pub(super) struct Response {
    pub value: Value,
    pub receipt: Receipt,
}
impl std::ops::Deref for Response {
    type Target = Value;
    fn deref(&self) -> &Value {
        &self.value
    }
}
impl PartialEq<Value> for Response {
    fn eq(&self, other: &Value) -> bool {
        &self.value == other
    }
}

/// A missing, empty or oversized usage payload is unknown. This guard also
/// applies on restored history and the final download publication path.
fn bounded_usage(value: &Value) -> Option<Value> {
    value
        .as_object()
        .filter(|v| !v.is_empty())
        .filter(|_| serde_json::to_vec(value).is_ok_and(|v| v.len() <= 64 * 1024))
        .map(|_| value.clone())
}

pub(super) fn video_facts(media: &mut MediaRecord, usage: &Value, cost: Option<String>) {
    if let Some(usage) = bounded_usage(usage) {
        media.response_usage = Some(usage);
    }
    if cost.is_some() {
        media.cost = cost;
    }
}

impl AssistantHost {
    pub(super) async fn record_video_usage(
        &self,
        run: &Arc<RunControl>,
        job: &str,
        usage: &Value,
    ) -> Result<(), String> {
        let Some(mut usage) = bounded_usage(usage) else {
            return Ok(());
        };
        let store = self.store(&run.identity.owner)?;
        let sid = run.session_id.clone();
        let mid = run.message_id.clone();
        let job = job.to_owned();
        scrub_secrets_in(&mut usage, &self.known_keys());
        blocking(move || {
            store.mutate(&sid, |record| {
                let Some(message) = record.messages.iter_mut().find(|m| m.id == mid) else {
                    return Ok(());
                };
                let Some(media) = message
                    .media
                    .as_mut()
                    .filter(|m| m.job_id.as_ref() == Some(&job))
                else {
                    return Ok(());
                };
                media.response_usage = Some(usage);
                let media = media.clone();
                let rid = message.run_id.clone();
                for op in record
                    .operations
                    .iter_mut()
                    .filter(|op| Some(&op.run_id) == rid.as_ref())
                {
                    op.media = Some(media.clone());
                }
                Ok(())
            })
        })
        .await
    }

    /// Save to the originating account even if its authority was revoked while
    /// receiving headers/body. Never emit here or create a replacement session.
    pub(super) async fn record_media_attempt(
        &self,
        run: &Arc<RunControl>,
        receipt: Option<&Receipt>,
        usage: Option<&Value>,
    ) -> Result<MediaRecord, String> {
        let store = self.store(&run.identity.owner)?;
        let sid = run.session_id.clone();
        let mid = run.message_id.clone();
        let receipt = receipt.cloned().unwrap_or_default();
        let mut usage = usage.and_then(bounded_usage);
        if let Some(usage) = &mut usage {
            scrub_secrets_in(usage, &self.known_keys());
        }
        let keys = self.known_keys();
        blocking(move || {
            store.mutate(&sid, |record| {
                let message = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == mid)
                    .ok_or("assistant_message_not_found")?;
                let media = message.media.as_mut().ok_or(super::media::INVALID)?;
                // A submission has at most one refused-key refresh. Polls never
                // replace or append generation identifiers.
                if media.request_attempts.len() >= 2 {
                    return Err(super::host::PROTOCOL.into());
                }
                let mut value =
                    serde_json::to_value(&receipt).map_err(|_| super::host::PROTOCOL)?;
                scrub_secrets_in(&mut value, &keys);
                media
                    .request_attempts
                    .push(serde_json::from_value(value).map_err(|_| super::host::PROTOCOL)?);
                if usage.is_some() {
                    media.response_usage = usage;
                }
                let saved = media.clone();
                let rid = message.run_id.clone();
                for op in record
                    .operations
                    .iter_mut()
                    .filter(|op| Some(&op.run_id) == rid.as_ref())
                {
                    op.media = Some(saved.clone());
                }
                Ok(saved)
            })
        })
        .await
    }
}

/// Project receipt facts without adding a second charge. Unknown/legacy
/// histories remain readable, and missing correlation identifiers stay null.
pub(super) fn with_media(mut usage: Value, record: &SessionRecord) -> Value {
    let mut requests = usage
        .get("requests")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for op in &record.operations {
        let Some(media) = &op.media else {
            continue;
        };
        if media.kind == "transcription" || media.kind == "speech" {
            if let Some(audio) = &media.audio {
                requests.push(json!({"kind":media.kind,"modelId":media.model,"apex":media.apex,
                    "runId":op.run_id,"operationId":op.id,"status":media.status,
                    "requestId":audio.request_id,"requestIdSource":audio.request_id_source,
                    "usage":bounded_usage(&audio.usage),"priceSnapshot":audio.pricing,"estimatedCostUsd":media.cost.as_ref().or(audio.estimate.as_ref()),"billedCostUsd":null}));
            }
        } else {
            for (index, attempt) in media.request_attempts.iter().enumerate() {
                let last = index + 1 == media.request_attempts.len();
                requests.push(json!({"kind":media.kind,"modelId":media.model,"apex":media.apex,
                    "runId":op.run_id,"operationId":op.id,"status":if (400..500).contains(&attempt.http_status) {"rejected"} else {media.status.as_str()},
                    "httpStatus":(attempt.http_status != 0).then_some(attempt.http_status),
                    "requestId":attempt.request_id,"requestIdSource":attempt.request_id_source,
                    "usage":if last {media.response_usage.as_ref().and_then(bounded_usage).unwrap_or(Value::Null)} else {Value::Null},
                    "priceSnapshot":media.price_snapshot,"estimatedCostUsd":if last && media.kind == "image" {media.cost.clone()} else {None},
                    "reportedCostUsd":if last && media.kind == "video" {media.cost.clone()} else {None},
                    "jobId":media.job_id,"billedCostUsd":null}));
            }
        }
    }
    if !requests.is_empty() {
        super::projection::ensure_object(&mut usage);
        usage["requests"] = json!(requests);
    }
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};
    #[test]
    fn provenance_restored_usage_is_bounded_before_it_reaches_the_renderer() {
        let root = tempfile::tempdir().unwrap();
        let store =
            super::super::store::AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
        let sid = store.create("test/video", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        record.operations.push(serde_json::from_value(json!({"id":id(),"runId":id(),"status":"completed","media":{"kind":"video","model":"test/video","params":{},"status":"completed","responseUsage":{"unexpected":"x".repeat(64*1024)},"requestAttempts":[{"httpStatus":202,"requestId":"req_bounded","requestIdSource":"x-request-id"}]}})).unwrap());
        let usage = with_media(json!({}), &record);
        assert!(
            usage["requests"][0]["usage"].is_null(),
            "an oversized usage object is unknown, not copied to the renderer"
        );
        assert_eq!(usage["requests"][0]["requestId"], "req_bounded");
    }

    #[test]
    fn provenance_video_download_keeps_previous_facts_when_a_later_poll_omits_usage() {
        let mut media = MediaRecord {
            response_usage: Some(json!({"video_cost":"0.08"})),
            cost: Some("0.08".into()),
            ..Default::default()
        };
        for value in [
            Value::Null,
            json!({}),
            json!({"unexpected":"x".repeat(64*1024)}),
        ] {
            video_facts(&mut media, &value, None);
            assert_eq!(media.cost.as_deref(), Some("0.08"));
            assert_eq!(media.response_usage.as_ref().unwrap()["video_cost"], "0.08");
        }
        video_facts(
            &mut media,
            &json!({"video_cost":"0.09"}),
            Some("0.09".into()),
        );
        assert_eq!(media.cost.as_deref(), Some("0.09"));
        assert_eq!(media.response_usage.as_ref().unwrap()["video_cost"], "0.09");
    }

    #[tokio::test]
    async fn provenance_identifier_prefers_gateway_header_and_rejects_secrets_and_unbounded_values()
    {
        let server = MockServer::start().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for header in [
            "req_valid".to_owned(),
            "synthetic-key".to_owned(),
            "bad value".to_owned(),
            "x".repeat(201),
        ] {
            server.reset().await;
            Mock::given(method("GET"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("x-ofox-request-id", header.as_str())
                        .insert_header("x-request-id", "req_fallback"),
                )
                .mount(&server)
                .await;
            let response = client.get(server.uri()).send().await.unwrap();
            let receipt = Receipt::from_response(&response, "synthetic-key");
            assert_eq!(
                receipt.request_id.as_deref(),
                Some(if header == "req_valid" {
                    "req_valid"
                } else {
                    "req_fallback"
                })
            );
            assert_eq!(
                receipt.request_id_source.as_deref(),
                Some(if header == "req_valid" {
                    "x-ofox-request-id"
                } else {
                    "x-request-id"
                })
            );
        }
    }
}
