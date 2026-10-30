//! Bounded binary speech response; a successful body is never a JSON receipt.
use super::*;
pub(in crate::services::assistant) struct Response {
    pub bytes: Vec<u8>,
    pub info: OutputInfo,
    pub request_id: Option<String>,
    pub request_id_source: Option<String>,
}
#[derive(Debug)]
pub(in crate::services::assistant) struct Failure {
    pub code: &'static str,
    pub unbilled: bool,
    pub request_id: Option<String>,
    pub request_id_source: Option<String>,
}
pub(in crate::services::assistant) async fn synthesize(
    client: &reqwest::Client,
    gateway: &str,
    key: &str,
    model: &Model,
    request: &SpeechRequest,
) -> Result<Response, Failure> {
    let unknown = || Failure {
        code: audio::UNKNOWN,
        unbilled: false,
        request_id: None,
        request_id_source: None,
    };
    let invalid = || Failure {
        code: audio::INVALID,
        unbilled: true,
        request_id: None,
        request_id_source: None,
    };
    // The permit stays with blocking validation after cancellation, bounding
    // outstanding decoder memory even if its async caller goes away.
    static RESPONSE_SLOTS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    let permit = RESPONSE_SLOTS
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| unknown())?;
    let mut body = request.body(model).map_err(|_| invalid())?;
    super::super::host::scrub_secrets_in(&mut body, &[key.to_owned()]);
    let mut response = client
        .post(format!("{}/v1/audio/speech", gateway.trim_end_matches('/')))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|error| {
            if error.is_builder() {
                invalid()
            } else {
                unknown()
            }
        })?;
    let status = response.status();
    let mut request_id = None;
    let mut request_id_source = None;
    for name in ["x-ofox-request-id", "x-request-id", "request-id"] {
        if let Some(value) = response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 128
                    && !v.contains(key)
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-:".contains(&b))
            })
        {
            request_id = Some(value.to_owned());
            request_id_source = Some(name.to_owned());
            break;
        }
    }
    let fail = |code, unbilled| Failure {
        code,
        unbilled,
        request_id: request_id.clone(),
        request_id_source: request_id_source.clone(),
    };
    if status != reqwest::StatusCode::OK {
        let unbilled = status.is_client_error() && status.as_u16() != 408;
        let code = match status.as_u16() {
            401 => "assistant_key_rejected",
            402 => "assistant_insufficient_balance",
            429 => "assistant_rate_limited",
            400..=499 if unbilled => audio::INVALID,
            _ => audio::UNKNOWN,
        };
        // Never copy or log arbitrary provider error bodies. No retry, even 401.
        return Err(fail(code, unbilled));
    }
    let mime = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(str::trim)
        .unwrap_or("");
    let expected = match request.response_format.as_str() {
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "opus" => "audio/ogg",
        _ => return Err(invalid()),
    };
    if mime != expected
        || response
            .content_length()
            .is_some_and(|n| n > MAX_OUTPUT_BYTES as u64)
    {
        return Err(fail(audio::UNKNOWN, false));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| fail(audio::UNKNOWN, false))?
    {
        if chunk.len() > MAX_OUTPUT_BYTES.saturating_sub(bytes.len()) {
            return Err(fail(audio::UNKNOWN, false));
        }
        bytes.extend_from_slice(&chunk);
    }
    if !key.is_empty()
        && bytes
            .windows(key.len())
            .any(|window| window == key.as_bytes())
    {
        return Err(fail(audio::UNKNOWN, false));
    }
    let format = request.response_format.clone();
    let (bytes, info) = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let info = inspect(&format, &bytes)?;
        Ok::<_, String>((bytes, info))
    })
    .await
    .map_err(|_| fail(audio::UNKNOWN, false))?
    .map_err(|_| fail(audio::UNKNOWN, false))?;
    Ok(Response {
        bytes,
        info,
        request_id,
        request_id_source,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{method, path},
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
    async fn speech_http_sends_only_explicit_supported_fields_and_preserves_binary_provenance() {
        let server = MockServer::start().await;
        let bytes = super::super::tests::wav(1);
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "audio/wav")
                    .insert_header("x-ofox-request-id", "req_speech_1")
                    .set_body_bytes(bytes.clone()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let r = super::super::tests::request("google/gemini-3.8-flash-tts", "wav", "Kore");
        let output = synthesize(
            &client(),
            &server.uri(),
            "synthetic-speech-key",
            &super::super::tests::model(&r.model_id),
            &r,
        )
        .await
        .unwrap();
        assert_eq!(output.bytes, bytes);
        assert_eq!(output.info.mime_type, "audio/wav");
        assert_eq!(output.request_id.as_deref(), Some("req_speech_1"));
        let received = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&received[0].body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"model":r.model_id,"voice":"Kore","input":r.input,"response_format":"wav","language_code":"cmn-CN"})
        );
    }
    #[tokio::test]
    async fn speech_http_refuses_json_mismatched_and_incomplete_success_bodies_without_retrying() {
        for (status, mime, bytes) in [
            (
                200,
                "application/json",
                br#"{"error":{"message":"private-secret"}}"#.to_vec(),
            ),
            (200, "audio/mpeg", super::super::tests::wav(1)),
            (206, "audio/wav", super::super::tests::wav(1)),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("Content-Type", mime)
                        .set_body_bytes(bytes),
                )
                .expect(1)
                .mount(&server)
                .await;
            let r = super::super::tests::request("google/gemini-3.8-flash-tts", "wav", "Kore");
            let error = synthesize(
                &client(),
                &server.uri(),
                "private-secret",
                &super::super::tests::model(&r.model_id),
                &r,
            )
            .await
            .err()
            .unwrap();
            assert!(!error.unbilled);
            assert_eq!(error.code, audio::UNKNOWN);
            assert!(!format!("{error:?}").contains("private-secret"));
        }
    }
    #[tokio::test]
    async fn speech_http_never_replays_refused_or_ambiguous_responses() {
        for (status, unbilled) in [
            (401, true),
            (402, true),
            (429, true),
            (408, false),
            (500, false),
            (307, false),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("x-request-id", "synthetic-speech-key")
                        .set_body_string("synthetic-speech-key"),
                )
                .expect(1)
                .mount(&server)
                .await;
            let r = super::super::tests::request("google/gemini-3.8-flash-tts", "wav", "Kore");
            let e = synthesize(
                &client(),
                &server.uri(),
                "synthetic-speech-key",
                &super::super::tests::model(&r.model_id),
                &r,
            )
            .await
            .err()
            .unwrap();
            assert_eq!(e.unbilled, unbilled);
            assert!(e.request_id.is_none());
        }
    }
    #[tokio::test]
    async fn speech_http_refuses_declared_oversize_before_reading_a_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "audio/wav")
                    .insert_header("Content-Length", (MAX_OUTPUT_BYTES + 1).to_string())
                    .set_body_bytes(Vec::new()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let r = super::super::tests::request("google/gemini-3.8-flash-tts", "wav", "Kore");
        let error = synthesize(
            &client(),
            &server.uri(),
            "key",
            &super::super::tests::model(&r.model_id),
            &r,
        )
        .await
        .err()
        .unwrap();
        assert!(!error.unbilled);
    }
}
