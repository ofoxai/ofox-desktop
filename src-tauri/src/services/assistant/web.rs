use super::{
    catalog,
    host::{self, RunControl},
    store, types,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tauri::Manager;

async fn decode_search_response(mut response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "assistant_search_outcome_unknown")?
    {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("assistant_search_outcome_unknown".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let decoded = serde_json::from_slice::<Value>(&bytes);
    // A provider/gateway failure can arrive after billing. Never allow a new
    // tool-call ID to replay it, and never expose its untrusted response body.
    if status.is_server_error()
        || decoded
            .as_ref()
            .is_ok_and(|body| body["error"]["type"] == "search_outcome_unknown")
    {
        return Err("assistant_search_outcome_unknown".into());
    }
    // Explicit client refusals are known rejections even with a non-JSON body.
    if !status.is_success() {
        return Err(format!("assistant_search_http_{}", status.as_u16()));
    }
    let body = decoded.map_err(|_| "assistant_search_outcome_unknown".to_string())?;
    let results = body
        .get("results")
        .and_then(Value::as_array)
        .ok_or("assistant_search_outcome_unknown")?;
    // Valid JSON alone does not establish a completed search. Preserve optional
    // metadata from older responses, but reject a broken result or supplied
    // usage rather than inventing an empty success or a new retry opportunity.
    if results
        .iter()
        .any(|item| item.get("url").and_then(Value::as_str).is_none())
        || body
            .get("usage")
            .is_some_and(|usage| usage.get("search_count").and_then(Value::as_u64) != Some(1))
    {
        return Err("assistant_search_outcome_unknown".into());
    }
    Ok(body)
}

fn after_paid_search<T>(persist: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    // HTTP success may already be charged, even if local bookkeeping or its
    // subsequent read fails. Storage diagnostics are not safe retry signals.
    persist().map_err(|_| "assistant_search_outcome_unknown".to_string())
}

pub async fn search(
    app: &tauri::AppHandle,
    run: &Arc<RunControl>,
    args: Value,
) -> Result<Value, String> {
    let state = app.state::<crate::store::AppState>();
    let detail = store::get(&state.db, &run.identity.owner, &run.session_id)?;
    if !detail.session.web_search {
        return Err("assistant_search_disabled".into());
    }
    if host::identity(app).await? != run.identity {
        return Err("assistant_identity_changed".into());
    }
    app.state::<host::AssistantHost>().validate_run(run)?;
    let token = host::token(app).await?;
    let capabilities = catalog::capabilities_at(&token, &run.identity.apex).await;
    if capabilities["search"]["available"] != true {
        return Err("assistant_search_unavailable".into());
    }
    if capabilities["search"]["priceVersion"] != run.search_capabilities["search"]["priceVersion"] {
        return Err("assistant_search_price_changed".into());
    }
    let query = args["query"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 4000)
        .ok_or("assistant_invalid_search")?;
    let query = crate::redact::scrub(query).replace(&run.secret, crate::redact::REDACTED);
    let max = args["maxResults"].as_u64().unwrap_or(5).clamp(1, 10);
    let mut cancel = run.cancel.subscribe();
    if host::identity(app).await? != run.identity {
        return Err("assistant_identity_changed".into());
    }
    app.state::<host::AssistantHost>().validate_run(run)?;
    let url = format!(
        "{}/v1/search",
        crate::ofox_apex::gateway_base_for(&run.identity.apex)
    );
    let mut result: Value = tokio::select! {
        biased;
        _=cancel.changed()=>return Err("assistant_search_outcome_unknown".into()),
        result=async {
            let response=crate::proxy::http_client::get().post(url).bearer_auth(&token).json(&json!({"query":query,"max_results":max})).timeout(Duration::from_secs(45)).send().await.map_err(|_|"assistant_search_outcome_unknown".to_string())?;
            decode_search_response(response).await
        }=>result?,
    };
    store::scrub_value(&mut result, &[&run.secret, &token]);
    let search_id = types::id();
    let sources:Vec<Value>=result["results"].as_array().into_iter().flatten().enumerate().filter_map(|(i,item)| {
        let url=item["url"].as_str()?;
        let parsed=url::Url::parse(url).ok()?;
        if parsed.scheme()!="https" || parsed.username()!="" || parsed.password().is_some() {return None;}
        Some(json!({"id":format!("{}-{}-{}",run.run_id,search_id,i+1),"title":item["title"].as_str().unwrap_or(url),"url":url,"excerpt":item["content"].as_str().unwrap_or("")}))
    }).collect();
    let all = after_paid_search(|| {
        store::mutate(&state.db, &run.identity.owner, &run.session_id, |detail| {
            if let Some(message) = detail.messages.iter_mut().find(|m| m.id == run.message_id) {
                message.sources.extend(sources.clone());
            }
            let count = detail.usage["searchCount"].as_u64().unwrap_or(0)
                + result["usage"]["search_count"].as_u64().unwrap_or(1);
            if !detail.usage.is_object() {
                detail.usage = json!({});
            }
            detail.usage["searchCount"] = json!(count);
            let price = run.search_capabilities["search"]["unitPrice"]
                .as_str()
                .and_then(|v| v.parse::<rust_decimal::Decimal>().ok());
            if let Some(price) = price {
                let old = detail.usage["searchEstimatedCostUsd"]
                    .as_str()
                    .and_then(|v| v.parse::<rust_decimal::Decimal>().ok())
                    .unwrap_or_default();
                detail.usage["searchEstimatedCostUsd"] = json!((old + price).to_string());
            }
            detail.session.updated_at = types::now();
            Ok(())
        })?;
        Ok(store::get(&state.db, &run.identity.owner, &run.session_id)?
            .messages
            .into_iter()
            .find(|m| m.id == run.message_id)
            .map(|m| m.sources)
            .unwrap_or(sources))
    })?;
    host::emit(app, run, "sources", json!({"sources":all}));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use serial_test::serial;
    use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

    async fn fixture_response(status: u16, body: impl Into<Vec<u8>>) -> Result<Value, String> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_bytes(body))
            .expect(1)
            .mount(&server)
            .await;
        let response = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
            .post(server.uri())
            .send()
            .await
            .unwrap();
        let result = decode_search_response(response).await;
        server.verify().await;
        result
    }

    #[tokio::test]
    async fn backend_search_unknown_outcome_is_not_an_ordinary_http_error() {
        for status in [502, 200, 400] {
            let error = fixture_response(
                status,
                br#"{"error":{"type":"search_outcome_unknown","message":"opaque-test-secret"}}"#
                    .to_vec(),
            )
            .await
            .unwrap_err();
            assert_eq!(error, "assistant_search_outcome_unknown", "HTTP {status}");
            assert!(!error.contains("opaque-test-secret"));
        }
    }

    #[tokio::test]
    async fn server_errors_do_not_allow_automatic_paid_search_retries() {
        for status in [500, 502, 503, 504] {
            let error =
                fixture_response(status, br#"{"error":{"type":"upstream_error"}}"#.to_vec())
                    .await
                    .unwrap_err();
            assert_eq!(error, "assistant_search_outcome_unknown", "HTTP {status}");
        }
    }

    #[tokio::test]
    async fn explicit_client_rejections_remain_ordinary_http_errors() {
        for status in [400, 401, 403, 404, 429] {
            for body in [
                br#"{"error":{"type":"invalid_request","message":"opaque-test-secret"}}"#.to_vec(),
                b"opaque-test-secret client refusal".to_vec(),
            ] {
                let error = fixture_response(status, body).await.unwrap_err();
                assert_eq!(error, format!("assistant_search_http_{status}"));
                assert!(!error.contains("opaque-test-secret"));
            }
        }
    }

    #[tokio::test]
    async fn unreadable_or_oversized_search_results_remain_unknown() {
        for (status, body) in [
            (200, b"opaque-test-secret".to_vec()),
            (502, b"opaque-test-secret".to_vec()),
            (200, vec![b'x'; 2 * 1024 * 1024 + 1]),
        ] {
            let error = fixture_response(status, body).await.unwrap_err();
            assert_eq!(error, "assistant_search_outcome_unknown", "HTTP {status}");
            assert!(!error.contains("opaque-test-secret"));
        }
    }

    #[tokio::test]
    async fn semantically_damaged_successful_search_results_remain_unknown() {
        // Gateway success is an object with a results array and one search.
        // An empty results array is valid; a damaged response is not no-results.
        for body in [
            json!(null),
            json!([]),
            json!({}),
            json!({"results": null, "usage": {"search_count": 1}}),
            json!({"results": "opaque-test-secret", "usage": {"search_count": 1}}),
            json!({"results": [], "usage": {"search_count": "one"}}),
            json!({"results": [], "usage": {"search_count": 0}}),
            json!({"results": [null], "usage": {"search_count": 1}}),
            json!({"results": [{"url": 42, "title": "Example"}], "usage": {"search_count": 1}}),
        ] {
            let error = fixture_response(200, body.to_string().into_bytes())
                .await
                .expect_err("semantically damaged paid responses must not permit retries");
            assert_eq!(error, "assistant_search_outcome_unknown");
            assert!(!error.contains("opaque-test-secret"));
        }
    }

    #[tokio::test]
    #[serial]
    async fn paid_search_write_failure_after_http_success_is_unknown() {
        let _sandbox = crate::test_sandbox::sandbox();
        let result = fixture_response(
            200,
            br#"{"results":[],"usage":{"search_count":1}}"#.to_vec(),
        )
        .await
        .unwrap();
        let db = Database::memory().unwrap();
        let owner = store::namespace("search-user", "org").unwrap();
        let detail =
            store::create(&db, &owner, types::AssistantKind::General, "test", true).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch("PRAGMA query_only=ON")
            .unwrap();
        let error = after_paid_search(|| {
            store::mutate(&db, &owner, &detail.session.id, |detail| {
                detail.usage["searchCount"] = result["usage"]["search_count"].clone();
                Ok(())
            })
        })
        .unwrap_err();
        assert_eq!(error, "assistant_search_outcome_unknown");
        assert_eq!(
            store::get(&db, &owner, &detail.session.id).unwrap().usage,
            json!({})
        );
    }

    #[tokio::test]
    #[serial]
    async fn paid_search_read_failure_after_committed_bookkeeping_is_unknown() {
        let _sandbox = crate::test_sandbox::sandbox();
        fixture_response(
            200,
            br#"{"results":[],"usage":{"search_count":1}}"#.to_vec(),
        )
        .await
        .unwrap();
        let db = Database::memory().unwrap();
        let owner = store::namespace("search-user", "org").unwrap();
        let detail =
            store::create(&db, &owner, types::AssistantKind::General, "test", true).unwrap();
        let error = after_paid_search(|| {
            store::mutate(&db, &owner, &detail.session.id, |detail| {
                detail.usage["searchCount"] = json!(1);
                Ok(())
            })?;
            assert_eq!(
                store::get(&db, &owner, &detail.session.id)?.usage["searchCount"],
                1
            );
            db.conn
                .lock()
                .unwrap()
                .execute("DELETE FROM settings WHERE key LIKE 'assistant_v1:%'", [])
                .unwrap();
            store::get(&db, &owner, &detail.session.id)
        })
        .unwrap_err();
        assert_eq!(error, "assistant_search_outcome_unknown");
    }

    #[tokio::test]
    async fn successful_search_and_bookkeeping_keep_the_result() {
        for expected in [
            json!({"results":[{"url":"https://example.com","title":"Example"}],"usage":{"search_count":1}}),
            json!({"results":[],"usage":{"search_count":1}}),
            json!({"results":[]}),
            json!({"results":[{"url":"https://example.com"}]}),
        ] {
            let result = fixture_response(200, expected.to_string().into_bytes())
                .await
                .unwrap();
            assert_eq!(after_paid_search(|| Ok(result)).unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn interrupted_search_body_is_unknown_without_exposing_partial_content() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0u8; 1024];
            assert!(socket.read(&mut request).unwrap() > 0);
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 500\r\nConnection: close\r\n\r\nopaque-test-secret").unwrap();
        });
        let response = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap();
        let error = decode_search_response(response).await.unwrap_err();
        server.join().unwrap();
        assert_eq!(error, "assistant_search_outcome_unknown");
        assert!(!error.contains("opaque-test-secret"));
    }
}
