//! Bounded audio gateway transport.
use super::super::{audio_input::Input, catalog::Model};
use super::*;
pub(super) struct Response {
    pub text: String,
    pub usage: Value,
    pub request_id: Option<String>,
    pub request_id_source: Option<String>,
}
#[derive(Debug)]
pub(super) struct Failure {
    pub code: &'static str,
    pub unbilled: bool,
    pub request_id: Option<String>,
    pub request_id_source: Option<String>,
}
pub(super) async fn transcribe(
    client: &reqwest::Client,
    gateway: &str,
    key: &str,
    model: &Model,
    input: &Input,
    language: Option<&str>,
) -> Result<Response, Failure> {
    let unknown = || Failure {
        code: UNKNOWN,
        unbilled: false,
        request_id: None,
        request_id_source: None,
    };
    let invalid = || Failure {
        code: INVALID,
        unbilled: true,
        request_id: None,
        request_id_source: None,
    };
    let mut form = reqwest::multipart::Form::new()
        .text("model", model.id.clone())
        .text("response_format", "json");
    if let Some(language) = language {
        form = form.text("language", language.to_owned());
    }
    if language == Some("zh") && model.audio.as_ref().is_some_and(|audio| audio.prompt) {
        form = form.text("prompt", "以下是普通话的句子，请使用简体中文。");
    }
    let part = reqwest::multipart::Part::bytes(input.bytes.clone())
        .file_name(input.name.clone())
        .mime_str(&input.info.mime_type)
        .map_err(|_| invalid())?;
    let mut response = client
        .post(format!(
            "{}/v1/audio/transcriptions",
            gateway.trim_end_matches('/')
        ))
        .bearer_auth(key)
        .multipart(form.part("file", part))
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
            .and_then(|v| identifier(v, key))
        {
            request_id = Some(value);
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
    let cap: usize = if status.is_success() {
        2 * 1024 * 1024
    } else {
        64 * 1024
    };
    if response.content_length().is_some_and(|n| n > cap as u64) {
        return Err(fail(UNKNOWN, false));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| fail(UNKNOWN, false))? {
        if chunk.len() > cap.saturating_sub(bytes.len()) {
            return Err(fail(UNKNOWN, false));
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let code = match status.as_u16() {
            401 => "assistant_key_rejected",
            402 => "assistant_insufficient_balance",
            429 => "assistant_rate_limited",
            400..=499 => INVALID,
            _ => UNKNOWN,
        };
        return Err(fail(code, status.is_client_error()));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| fail(UNKNOWN, false))?;
    let text = value["text"]
        .as_str()
        .filter(|v| v.len() <= MAX_TEXT_BYTES)
        .ok_or_else(|| fail(UNKNOWN, false))?;
    let mut usage = value["usage"].clone();
    if !usage.is_object() || serde_json::to_vec(&usage).map_or(true, |v| v.len() > 64 * 1024) {
        usage = Value::Null;
    }
    let secrets = [key.to_owned()];
    super::super::host::scrub_secrets_in(&mut usage, &secrets);
    Ok(Response {
        text: super::super::host::scrub_secrets(text, &secrets),
        usage,
        request_id,
        request_id_source,
    })
}
fn identifier(value: &str, key: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 128
        && !value.contains(key)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-:".contains(&b)))
    .then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };
    fn model(id: &str) -> Model {
        super::super::super::catalog::parse_models(json!({"data":[{"id":id,"supported_endpoints":["audio/transcriptions"],"pricing":{"input_audio_per_second":"0.000075"}}]}),super::super::super::catalog::Source::V1).remove(0)
    }
    fn input() -> Input {
        Input::from_bytes(
            "../../speech.txt",
            include_bytes!("../../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3").to_vec(),
        )
        .unwrap()
    }
    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }
    #[tokio::test]
    async fn audio_transport_posts_only_documented_fields_and_returns_bounded_provenance() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .and(header("authorization", "Bearer test-audio-secret"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-request-id", "req_audio_1")
                    .set_body_json(
                        json!({"text":"下午三点开会。","usage":{"type":"duration","seconds":9}}),
                    ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = transcribe(
            &client(),
            &server.uri(),
            "test-audio-secret",
            &model("openai/gpt-transcribe"),
            &input(),
            Some("zh"),
        )
        .await
        .unwrap();
        assert_eq!(result.text, "下午三点开会。");
        assert_eq!(result.request_id.as_deref(), Some("req_audio_1"));
        assert_eq!(result.usage["seconds"], 9);
        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        for field in [
            "name=\"model\"",
            "openai/gpt-transcribe",
            "name=\"language\"",
            "name=\"prompt\"",
            "请使用简体中文",
            "name=\"file\"",
            "filename=\"speech.mp3\"",
            "audio/mpeg",
        ] {
            assert!(body.contains(field), "missing {field}");
        }
        for forbidden in ["test-audio-secret", "system", "instructions", "../"] {
            assert!(!body.contains(forbidden));
        }
    }
    #[tokio::test]
    async fn audio_transport_omits_unsupported_prompt_and_automatic_language() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text":""})))
            .mount(&server)
            .await;
        let result = transcribe(
            &client(),
            &server.uri(),
            "key",
            &model("elevenlabs/scribe_v2"),
            &input(),
            None,
        )
        .await
        .unwrap();
        assert!(result.text.is_empty());
        assert!(result.usage.is_null());
        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(!body.contains("name=\"prompt\""));
        assert!(!body.contains("name=\"language\""));
    }
    #[tokio::test]
    async fn audio_transport_never_replays_ambiguous_or_refused_responses() {
        for (status, unbilled) in [
            (400, true),
            (401, true),
            (402, true),
            (429, true),
            (500, false),
            (307, false),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("x-request-id", "do-not-leak-this-key")
                        .set_body_json(json!({"error":{"message":"do-not-leak-this-key"}})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let error = transcribe(
                &client(),
                &server.uri(),
                "do-not-leak-this-key",
                &model("openai/gpt-transcribe"),
                &input(),
                None,
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error.unbilled, unbilled);
            assert!(error.request_id.is_none());
            assert!(!format!("{error:?}").contains("do-not-leak-this-key"));
        }
    }
}
