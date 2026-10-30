//! One gateway image attempt. Authentication retry belongs to the host.
use super::media::{ImageRequest, CREDITS, INVALID, MAX_IMAGE_BYTES, SAFETY, UNKNOWN};
use super::provenance::{Receipt, Response};
use serde_json::{json, Value};

pub(super) struct InputImage {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct Failure {
    pub receipt: Option<Receipt>,
    pub code: String,
    pub unbilled: bool,
    pub detail: Option<String>,
}

pub(super) async fn request(
    client: &reqwest::Client,
    gateway: &str,
    key: &str,
    model: &str,
    prompt: &str,
    params: &ImageRequest,
    input: Option<&InputImage>,
) -> Result<Response, Failure> {
    let unknown = || Failure {
        receipt: None,
        detail: None,
        code: UNKNOWN.into(),
        unbilled: false,
    };
    let mut fields = json!({"model":model,"prompt":prompt});
    if let Some(n) = params.n {
        fields["n"] = json!(n);
    }
    if let Some(size) = &params.size {
        fields["size"] = json!(size);
    }
    if let Some(quality) = &params.quality {
        fields["quality"] = json!(quality);
    }
    let endpoint = if input.is_some() {
        "edits"
    } else {
        "generations"
    };
    // Only a host-resolved base is accepted by this private function. Never
    // use a provider-supplied URL for an authenticated image request.
    let mut request = client
        .post(format!(
            "{}/v1/images/{endpoint}",
            gateway.trim_end_matches('/')
        ))
        .bearer_auth(key);
    if let Some(input) = input {
        let mut form = reqwest::multipart::Form::new();
        for (key, value) in fields.as_object().expect("object") {
            form = form.text(
                key.clone(),
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string()),
            );
        }
        let part = reqwest::multipart::Part::bytes(input.bytes.clone())
            .file_name(input.name.clone())
            .mime_str(&input.mime)
            .map_err(|_| Failure {
                receipt: None,
                detail: None,
                code: INVALID.into(),
                unbilled: true,
            })?;
        request = request.multipart(form.part("image", part));
    } else {
        request = request.json(&fields);
    }
    let mut response = request.send().await.map_err(|_| unknown())?;
    let status = response.status();
    let receipt = Receipt::from_response(&response, key);
    let unknown = || Failure {
        receipt: Some(receipt.clone()),
        ..unknown()
    };
    // Bound encoded JSON before allocating decoded images. Error bodies
    // have a separate small cap and are never returned verbatim.
    let cap = if status.is_success() {
        params.count() * MAX_IMAGE_BYTES.div_ceil(3) * 4 + 1024 * 1024
    } else {
        64 * 1024
    };
    if response.content_length().is_some_and(|n| n > cap as u64) {
        return Err(unknown());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unknown())? {
        if chunk.len() > cap.saturating_sub(bytes.len()) {
            return Err(unknown());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if status.is_success() {
        return value
            .is_object()
            .then_some(Response {
                value,
                receipt: receipt.clone(),
            })
            .ok_or_else(unknown);
    }
    let upstream = value["error"]["code"]
        .as_str()
        .or(value["error"]["type"].as_str())
        .unwrap_or("");
    let code = match status.as_u16() {
        401 => "assistant_key_rejected",
        402 => CREDITS,
        429 => "assistant_rate_limited",
        400..=499
            if upstream.contains("moderation") || upstream == "image_generation_user_error" =>
        {
            SAFETY
        }
        400..=499 => INVALID,
        _ => UNKNOWN,
    };
    Err(Failure {
        receipt: Some(receipt),
        detail: value["error"]["message"].as_str().map(|message| {
            super::host::scrub_detail(message, &[key.to_owned()])
                .chars()
                .take(2000)
                .collect()
        }),
        code: code.into(),
        unbilled: status.is_client_error(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::media::{CREDITS, INVALID, SAFETY};
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{body_json, header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn response_receipt_keeps_header_identity_even_when_body_is_invalid_or_refused() {
        let server = MockServer::start().await;
        for (status, body) in [(200, "broken"), (429, "{}"), (200, "{}")] {
            server.reset().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("x-ofox-request-id", "req_image_1")
                        .set_body_string(body),
                )
                .mount(&server)
                .await;
            let result = request(
                &client(),
                &server.uri(),
                "synthetic-key",
                "test/image",
                "square",
                &ImageRequest::default(),
                None,
            )
            .await;
            let receipt = match &result {
                Ok(value) => &value.receipt,
                Err(error) => error
                    .receipt
                    .as_ref()
                    .expect("received headers survive a failed body"),
            };
            assert_eq!(receipt.request_id.as_deref(), Some("req_image_1"));
            assert_eq!(
                receipt.request_id_source.as_deref(),
                Some("x-ofox-request-id")
            );
            assert_eq!(receipt.http_status, status);
        }
    }

    #[tokio::test]
    async fn generation_sends_only_explicit_parameters_and_gateway_auth() {
        let server = MockServer::start().await;
        let result = json!({"data":[{"b64_json":"fixture"}],"usage":{"output_tokens":250}});
        Mock::given(method("POST")).and(path("/v1/images/generations"))
            .and(header("Authorization", "Bearer synthetic-test-key"))
            .and(body_json(json!({"model":"test/image","prompt":"blue square","size":"1024x1024","quality":"low","n":2})))
            .respond_with(ResponseTemplate::new(200).set_body_json(&result)).expect(1).mount(&server).await;
        let params = ImageRequest {
            size: Some("1024x1024".into()),
            quality: Some("low".into()),
            n: Some(2),
            ..Default::default()
        };
        assert_eq!(
            request(
                &client(),
                &server.uri(),
                "synthetic-test-key",
                "test/image",
                "blue square",
                &params,
                None
            )
            .await
            .unwrap(),
            result
        );
        let requests = server.received_requests().await.unwrap();
        assert!(!String::from_utf8_lossy(&requests[0].body).contains("synthetic-test-key"));
    }

    #[tokio::test]
    async fn edits_upload_bytes_as_multipart_without_local_paths() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
            .expect(1)
            .mount(&server)
            .await;
        let input = InputImage {
            name: "input.png".into(),
            mime: "image/png".into(),
            bytes: b"\x89PNG\r\n\x1a\nfixture".to_vec(),
        };
        request(
            &client(),
            &server.uri(),
            "synthetic-test-key",
            "test/image",
            "green square",
            &ImageRequest::default(),
            Some(&input),
        )
        .await
        .unwrap();
        let calls = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&calls[0].body);
        for part in [
            "name=\"model\"",
            "test/image",
            "name=\"prompt\"",
            "green square",
            "name=\"image\"; filename=\"input.png\"",
            "Content-Type: image/png",
            "fixture",
        ] {
            assert!(body.contains(part), "missing multipart field {part}");
        }
        assert!(!body.contains("name=\"n\""));
    }

    #[tokio::test]
    async fn refusal_codes_never_replay_and_ambiguous_success_is_unknown() {
        for (status, upstream, code, unbilled) in [
            (401, "invalid_api_key", "assistant_key_rejected", true),
            (402, "insufficient_quota", CREDITS, true),
            (400, "moderation_blocked", SAFETY, true),
            (400, "image_generation_user_error", SAFETY, true),
            (400, "endpoint_not_supported", INVALID, true),
            (500, "server_error", UNKNOWN, false),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status).set_body_json(
                    json!({"error":{"code":upstream,"message":"provider refusal: synthetic-test-key"}}),
                ))
                .expect(1)
                .mount(&server)
                .await;
            let error = request(
                &client(),
                &server.uri(),
                "synthetic-test-key",
                "test/image",
                "blue square",
                &ImageRequest::default(),
                None,
            )
            .await
            .unwrap_err();
            assert_eq!((error.code.as_str(), error.unbilled), (code, unbilled));
            let detail = error.detail.as_deref().expect("safe provider diagnostic");
            assert!(detail.starts_with("provider refusal:"));
            assert!(!detail.contains("synthetic-test-key"));
        }
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("broken JSON"))
            .expect(1)
            .mount(&server)
            .await;
        let error = request(
            &client(),
            &server.uri(),
            "synthetic-test-key",
            "test/image",
            "blue square",
            &ImageRequest::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, UNKNOWN);
        assert!(!error.unbilled);
    }

    #[tokio::test]
    async fn redirect_is_not_followed_or_retried() {
        let origin = MockServer::start().await;
        let target = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(307).insert_header("Location", target.uri()))
            .expect(1)
            .mount(&origin)
            .await;
        let error = request(
            &crate::proxy::http_client::assistant_media_client().unwrap(),
            &origin.uri(),
            "synthetic-test-key",
            "test/image",
            "blue square",
            &ImageRequest::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, UNKNOWN);
        assert_eq!(target.received_requests().await.unwrap().len(), 0);
    }
    /// Explicit opt-in only: one low-quality image and one edit, never video.
    /// Inject the test key with the approved vault command substitution.
    #[tokio::test]
    #[ignore = "paid image smoke test; requires explicit opt-in and a test key"]
    async fn live_image_generation_and_edit() {
        use super::super::{catalog, media};
        use rust_decimal::Decimal;
        assert_eq!(
            std::env::var("OFOX_ASSISTANT_LIVE_IMAGES").as_deref(),
            Ok("1")
        );
        let key = std::env::var("OFOX_ASSISTANT_LIVE_KEY").expect("test key injection required");
        let apex = std::env::var("OFOX_ASSISTANT_LIVE_APEX").unwrap_or_else(|_| "ofox.ai".into());
        let gateway = crate::ofox_apex::gateway_base_for(&apex).expect("known apex");
        assert!(gateway.starts_with("https://"));
        let output = std::path::PathBuf::from(
            std::env::var("OFOX_ASSISTANT_LIVE_OUTPUT").expect("private output directory required"),
        );
        crate::config::create_private_dir_all(&output).unwrap();
        let client = crate::proxy::http_client::assistant_media_client().unwrap();
        let models = catalog::fetch_models(&apex, catalog::Source::V1)
            .await
            .unwrap();
        let model = models
            .iter()
            .find(|m| m.id == "openai/gpt-image-2")
            .expect("live model listed");
        let caps = model.image.as_ref().expect("image capabilities");
        async fn spend(client: &reqwest::Client, gateway: &str, key: &str) -> Option<Decimal> {
            let response = client
                .get(format!("{gateway}/v1/user/balance"))
                .bearer_auth(key)
                .send()
                .await
                .ok()?;
            if !response.status().is_success() {
                return None;
            }
            let value: Value = response.json().await.ok()?;
            value["used"].as_number()?.to_string().parse().ok()
        }
        let params = ImageRequest {
            size: Some("1024x1024".into()),
            quality: Some("low".into()),
            ..Default::default()
        };
        let mut input = None;
        let mut reports = vec![];
        for (label, prompt) in [
            (
                "generate",
                "A plain blue square centered on a white background, no text.",
            ),
            (
                "edit",
                "Change the blue square to red, keeping everything else unchanged.",
            ),
        ] {
            let before = spend(&client, &gateway, &key).await;
            let result = request(
                &client,
                &gateway,
                &key,
                &model.id,
                prompt,
                &params,
                input.as_ref(),
            )
            .await
            .expect("image request failed; no automatic replay");
            let images = media::decode(&result, 1).unwrap();
            let estimate = media::cost(caps, &result["usage"], images.len(), input.is_some());
            let after = spend(&client, &gateway, &key).await;
            let observed = after.zip(before).and_then(|(a, b)| a.checked_sub(b));
            let (ext, bytes) = images.into_iter().next().unwrap();
            crate::config::atomic_write_with(
                &output.join(format!("{label}.{ext}")),
                &bytes,
                crate::config::AtomicOpts { mode: Some(0o600) },
            )
            .unwrap();
            let report = json!({"step":label,"apex":apex,"model":model.id,"imageBytes":bytes.len(),"inputTokens":result["usage"]["input_tokens"].as_u64(),"imageInputTokens":result["usage"]["input_tokens_details"]["image_tokens"].as_u64(),"outputTokens":result["usage"]["output_tokens"].as_u64(),"computedCostUsd":estimate.map(|n|n.normalize().to_string()),"observedSpendDeltaUsd":observed.map(|n|n.normalize().to_string())});
            println!("{report}");
            reports.push(report);
            input = Some(InputImage {
                name: format!("input.{ext}"),
                mime: super::super::attachments::mime_type(&ext).unwrap().into(),
                bytes,
            });
        }
        crate::config::atomic_write_with(
            &output.join("report.json"),
            &serde_json::to_vec_pretty(&reports).unwrap(),
            crate::config::AtomicOpts { mode: Some(0o600) },
        )
        .unwrap();
    }
}
