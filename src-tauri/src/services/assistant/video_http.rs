//! Bounded video HTTP requests. Credentials only accompany gateway requests.
use super::provenance::{Receipt, Response};
use super::{
    media,
    media_http::Failure,
    video::{self, VideoRequest},
};
use serde_json::{json, Value};

pub(super) async fn provider_prices(apex: &str, model: &str) -> Result<Value, String> {
    let url = crate::ofox_apex::model_pricing_url_for(apex, model).ok_or(media::INVALID)?;
    let client = crate::proxy::http_client::assistant_media_client()?;
    pricing_at(&client, &url).await
}
async fn pricing_at(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let mut response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| media::INVALID)?;
    const CAP: usize = 4 * 1024 * 1024;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > CAP as u64)
    {
        return Err(media::INVALID.into());
    }
    let mut bytes = vec![];
    while let Some(chunk) = response.chunk().await.map_err(|_| media::INVALID)? {
        if chunk.len() > CAP - bytes.len() {
            return Err(media::INVALID.into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| media::INVALID)?;
    value
        .is_object()
        .then_some(value)
        .ok_or_else(|| media::INVALID.into())
}

fn failure(code: &str) -> Failure {
    Failure {
        receipt: None,
        code: code.into(),
        unbilled: false,
        detail: None,
    }
}

/// One attempt only. A caller may refresh a refused key; it must never
/// replay a request after an ambiguous result.
#[allow(clippy::too_many_arguments)]
pub(super) async fn create(
    client: &reqwest::Client,
    gateway: &str,
    key: &str,
    model: &str,
    prompt: &str,
    params: &VideoRequest,
    frames: &[(&str, String)],
    references: &[(&str, String)],
) -> Result<Response, Failure> {
    let mut body = json!({"model":model,"prompt":prompt,"duration":params.duration,
        "resolution":params.resolution,"generate_audio":params.generate_audio});
    if let Some(aspect) = &params.aspect_ratio {
        body["aspect_ratio"] = json!(aspect);
    }
    if !frames.is_empty() {
        body["frame_images"] = json!(frames
            .iter()
            .map(|(kind, data)| json!({
                "type":"image_url","image_url":{"url":data},"frame_type":kind
            }))
            .collect::<Vec<_>>());
    }
    // `frame_images` and `input_references` exclude each other (POST
    // /v1/videos); the caller's plan never asks for both.
    if !references.is_empty() {
        if !frames.is_empty() {
            return Err(Failure {
                unbilled: true,
                ..failure(media::INVALID)
            });
        }
        body["input_references"] = json!(references
            .iter()
            .map(|(kind, data)| json!({"type":kind, *kind:{"url":data}}))
            .collect::<Vec<_>>());
    }
    let response = client
        .post(format!("{}/v1/videos", gateway.trim_end_matches('/')))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|_| failure(media::UNKNOWN))?;
    let value = read_response(response, key, false).await?;
    video::job_id(&value).map_err(|_| Failure {
        receipt: Some(value.receipt.clone()),
        ..failure(media::UNKNOWN)
    })?;
    Ok(value)
}
pub(super) async fn query(
    client: &reqwest::Client,
    gateway: &str,
    key: &str,
    job: &str,
) -> Result<Response, Failure> {
    super::types::validate_id(job).map_err(|_| failure(media::INVALID))?;
    let response = client
        .get(format!("{}/v1/videos/{job}", gateway.trim_end_matches('/')))
        .bearer_auth(key)
        .send()
        .await
        .map_err(|_| failure("assistant_network_error"))?;
    read_response(response, key, false).await
}
pub(super) async fn cancel(
    client: &reqwest::Client,
    gateway: &str,
    key: &str,
    job: &str,
) -> Result<(), Failure> {
    super::types::validate_id(job).map_err(|_| failure(media::INVALID))?;
    let response = client
        .delete(format!("{}/v1/videos/{job}", gateway.trim_end_matches('/')))
        .bearer_auth(key)
        .send()
        .await
        .map_err(|_| failure("assistant_network_error"))?;
    read_response(response, key, true).await.map(|_| ())
}
async fn read_response(
    mut response: reqwest::Response,
    key: &str,
    cancelling: bool,
) -> Result<Response, Failure> {
    let status = response.status();
    let receipt = Receipt::from_response(&response, key);
    let failure = |code: &str| Failure {
        receipt: Some(receipt.clone()),
        ..failure(code)
    };
    if cancelling && status == reqwest::StatusCode::NO_CONTENT {
        return Ok(Response {
            value: Value::Null,
            receipt,
        });
    }
    let cap = 1024 * 1024;
    if response
        .content_length()
        .is_some_and(|len| len > cap as u64)
    {
        return Err(failure(media::UNKNOWN));
    }
    let mut bytes = vec![];
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| failure(media::UNKNOWN))?
    {
        if chunk.len() > cap - bytes.len() {
            return Err(failure(media::UNKNOWN));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if status.is_success() && !cancelling {
        return value
            .is_object()
            .then_some(Response {
                value,
                receipt: receipt.clone(),
            })
            .ok_or_else(|| failure(media::UNKNOWN));
    }
    let upstream = value["error"]["code"].as_str().unwrap_or("");
    let code = match status.as_u16() {
        401 => "assistant_key_rejected",
        402 => media::CREDITS,
        429 => "assistant_rate_limited",
        404 => "assistant_media_job_not_found",
        400 if upstream == "cancel_not_supported" => "assistant_media_cancel_not_supported",
        400..=499 if cancelling => "assistant_media_cancel_failed",
        400..=499 => media::INVALID,
        _ => media::UNKNOWN,
    };
    Err(Failure {
        receipt: Some(receipt),
        code: code.into(),
        unbilled: status.is_client_error(),
        detail: value["error"]["message"].as_str().map(|text| {
            super::host::scrub_detail(text, &[key.to_owned()])
                .chars()
                .take(2000)
                .collect()
        }),
    })
}
fn safe_download_url(value: &str) -> bool {
    super::download::source_url(value).is_ok()
}
pub(super) fn download_url(value: &Value) -> Result<String, String> {
    for key in ["mirror_urls", "unsigned_urls"] {
        if let Some(url) = value[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find(|value| safe_download_url(value))
        {
            return Ok(url.to_owned());
        }
    }
    Err("assistant_media_download_failed".into())
}
pub(super) async fn download(
    store: super::store::AssistantStore,
    client: &reqwest::Client,
    value: &Value,
    proxy: Option<super::types::RunProxy>,
    secrets: &[String],
) -> Result<super::download::Downloaded, String> {
    super::download::get(store, &download_url(value)?, proxy, client, secrets).await
}

#[cfg(test)]
mod tests {
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
    fn params() -> VideoRequest {
        serde_json::from_value(
            json!({"duration":4,"resolution":"480p","aspectRatio":"16:9","generateAudio":false}),
        )
        .unwrap()
    }
    #[tokio::test]
    async fn video_provider_prices_are_keyless_bounded_and_no_redirect() {
        let server = MockServer::start().await;
        let data = json!({"provider_cards":[]});
        Mock::given(path("/price"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&data))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            pricing_at(&client(), &format!("{}/price", server.uri()))
                .await
                .unwrap(),
            data
        );
        assert!(!server.received_requests().await.unwrap()[0]
            .headers
            .contains_key("authorization"));
        Mock::given(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/price", server.uri())),
            )
            .mount(&server)
            .await;
        assert!(pricing_at(&client(), &format!("{}/redirect", server.uri()))
            .await
            .is_err());
        Mock::given(path("/large"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(vec![b' '; 4 * 1024 * 1024 + 1]),
            )
            .mount(&server)
            .await;
        assert!(pricing_at(&client(), &format!("{}/large", server.uri()))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn video_create_sends_exact_params_and_frame_bytes_once() {
        let server = MockServer::start().await;
        let job = super::super::types::id();
        let response =
            json!({"id":job,"status":"pending","polling_url":"https://unrelated.invalid/"});
        Mock::given(method("POST")).and(path("/v1/videos"))
            .and(header("Authorization","Bearer synthetic-key"))
            .and(body_json(json!({"model":"test/video","prompt":"a square","duration":4,"resolution":"480p","aspect_ratio":"16:9","generate_audio":false,
                "frame_images":[{"type":"image_url","image_url":{"url":"data:image/png;base64,fixture"},"frame_type":"first_frame"}]})))
            .respond_with(ResponseTemplate::new(202).set_body_json(&response)).expect(1).mount(&server).await;
        assert_eq!(
            create(
                &client(),
                &server.uri(),
                "synthetic-key",
                "test/video",
                "a square",
                &params(),
                &[("first_frame", "data:image/png;base64,fixture".into())],
                &[]
            )
            .await
            .unwrap(),
            response
        );
    }
    #[tokio::test]
    async fn video_create_sends_references_by_kind_and_never_with_frames() {
        let server = MockServer::start().await;
        let job = super::super::types::id();
        let response = json!({"id":job,"status":"pending"});
        Mock::given(method("POST")).and(path("/v1/videos"))
            .and(body_json(json!({"model":"test/video","prompt":"a square","duration":4,"resolution":"480p","aspect_ratio":"16:9","generate_audio":false,
                "input_references":[{"type":"image_url","image_url":{"url":"data:image/png;base64,a"}},{"type":"audio_url","audio_url":{"url":"data:audio/mpeg;base64,b"}},{"type":"video_url","video_url":{"url":"data:video/mp4;base64,c"}}]})))
            .respond_with(ResponseTemplate::new(202).set_body_json(&response)).expect(1).mount(&server).await;
        let references = [
            ("image_url", "data:image/png;base64,a".to_string()),
            ("audio_url", "data:audio/mpeg;base64,b".to_string()),
            ("video_url", "data:video/mp4;base64,c".to_string()),
        ];
        let created = create(
            &client(),
            &server.uri(),
            "synthetic-key",
            "test/video",
            "a square",
            &params(),
            &[],
            &references,
        )
        .await
        .unwrap();
        assert_eq!(created, response);
        let both = create(
            &client(),
            &server.uri(),
            "synthetic-key",
            "test/video",
            "a square",
            &params(),
            &[("first_frame", "data:image/png;base64,a".into())],
            &references,
        )
        .await
        .unwrap_err();
        assert!(both.unbilled && both.code == media::INVALID);
    }
    #[tokio::test]
    async fn video_submit_never_replays_a_refusal_or_ambiguous_response() {
        for (status, expected) in [
            (401, "assistant_key_rejected"),
            (402, media::CREDITS),
            (500, media::UNKNOWN),
            (202, media::UNKNOWN),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("x-request-id", "req_submit")
                        .set_body_string(if status == 202 { "{}" } else { "invalid-json" }),
                )
                .expect(1)
                .mount(&server)
                .await;
            let failure = create(
                &client(),
                &server.uri(),
                "synthetic-key",
                "test/video",
                "a square",
                &params(),
                &[],
                &[],
            )
            .await
            .unwrap_err();
            assert_eq!(failure.code, expected);
            assert_eq!(
                failure.receipt.as_ref().unwrap().request_id.as_deref(),
                Some("req_submit")
            );
            assert_eq!(failure.receipt.as_ref().unwrap().http_status, status);
        }
    }
    #[tokio::test]
    async fn video_query_and_cancel_use_validated_gateway_paths_and_report_real_cancellation() {
        let server = MockServer::start().await;
        let job = super::super::types::id();
        let route = format!("/v1/videos/{job}");
        Mock::given(method("GET"))
            .and(path(&route))
            .and(header("Authorization", "Bearer synthetic-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id":job,"status":"in_progress"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(&route))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            query(&client(), &server.uri(), "synthetic-key", &job)
                .await
                .unwrap()["status"],
            "in_progress"
        );
        assert!(cancel(&client(), &server.uri(), "synthetic-key", &job)
            .await
            .is_ok());
        assert!(query(&client(), &server.uri(), "synthetic-key", "../other")
            .await
            .is_err());
        let refused = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"error":{"code":"cancel_not_supported"}})),
            )
            .expect(1)
            .mount(&refused)
            .await;
        assert_eq!(
            cancel(&client(), &refused.uri(), "synthetic-key", &job)
                .await
                .unwrap_err()
                .code,
            "assistant_media_cancel_not_supported"
        );
    }
    #[test]
    fn video_download_prefers_https_mirrors_without_credentials_or_local_urls() {
        assert_eq!(download_url(&json!({"mirror_urls":["https://cdn.example/video.mp4"],"unsigned_urls":["https://upstream.example/video.mp4"]})).unwrap(),"https://cdn.example/video.mp4");
        assert_eq!(
            download_url(&json!({"unsigned_urls":["https://upstream.example/video.mp4"]})).unwrap(),
            "https://upstream.example/video.mp4"
        );
        for url in [
            "http://cdn.example/video.mp4",
            "file:///tmp/video.mp4",
            "https://user:secret@cdn.example/video.mp4",
            "https://127.0.0.1/video.mp4",
        ] {
            assert!(download_url(&json!({"mirror_urls":[url]})).is_err());
        }
    }
    #[test]
    fn video_download_rejects_reserved_and_transition_addresses_before_any_connection() {
        for host in [
            "100.64.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "0.1.2.3",
            "[2001:db8::1]",
            "[2002:7f00:1::1]",
            "[64:ff9b::7f00:1]",
        ] {
            assert!(
                !safe_download_url(&format!("https://{host}/video.mp4")),
                "{host} is not a public download target"
            );
        }
        for host in ["1.1.1.1", "8.8.8.8", "[2606:4700:4700::1111]"] {
            assert!(safe_download_url(&format!("https://{host}/video.mp4")));
        }
    }
}
