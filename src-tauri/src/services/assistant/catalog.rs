//! The Ofox model catalog as the assistant sees it: chat-capable models only,
//! with the capacity and prices the runtime needs.
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub name: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    pub supports_images: bool,
    pub input_modalities: Vec<String>,
    pub supported_endpoints: Vec<String>,
    pub input_price_per_million: Option<String>,
    pub output_price_per_million: Option<String>,
    pub pricing: Value,
    pub cost: Value,
    pub pricing_known: bool,
}

/// Defaults tried in order when nothing remembered is still offered. Every
/// entry was in the live `/v1/models` catalog of both apexes when chosen; one
/// that disappears is skipped, never offered.
pub const PREFERRED_MODEL_IDS: &[&str] = &[
    "openai/gpt-6.1-sol",
    "openai/gpt-6-sol",
    "openai/gpt-5.4-mini",
];

/// Protocols the runtime can drive, as catalog paths without `/v1/`.
const CHAT_ENDPOINTS: &[&str] = &["chat/completions", "responses", "messages"];

/// The model the bundled search server searches with (search-mcp's
/// `DEFAULT_SEARCH_MODEL`). The host prices every search at its catalog row.
pub const SEARCH_MODEL: &str = "openai/gpt-5-nano";

/// What web searches cost, from the catalog row of [`SEARCH_MODEL`]: USD per
/// search (`pricing.web_search`) and per input and output token. A run keeps
/// the price it started with; it prices only searches made with `model`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPrice {
    /// The model the price is for. Absent: no search matches it.
    #[serde(default)]
    pub model: String,
    pub per_search: Decimal,
    pub per_input_token: Decimal,
    pub per_output_token: Decimal,
}

impl SearchPrice {
    /// `searches` paid searches that used these tokens; `None` on overflow.
    pub fn cost(&self, searches: u64, input: u64, output: u64) -> Option<Decimal> {
        let part = |price: Decimal, count: u64| price.checked_mul(Decimal::from(count));
        part(self.per_search, searches)?
            .checked_add(part(self.per_input_token, input)?)?
            .checked_add(part(self.per_output_token, output)?)
    }
}

/// A catalog price: a non-negative decimal string.
fn price(value: &Value) -> Option<Decimal> {
    value
        .as_str()?
        .parse::<Decimal>()
        .ok()
        .filter(|price| !price.is_sign_negative())
}

/// The search price `models` gives, or `None` when the catalog lacks the
/// search model or any of its three prices: search is then unavailable.
pub fn search_price(models: &[Model]) -> Option<SearchPrice> {
    let pricing = &models.iter().find(|m| m.id == SEARCH_MODEL)?.pricing;
    Some(SearchPrice {
        model: SEARCH_MODEL.into(),
        per_search: price(&pricing["web_search"])?,
        per_input_token: price(&pricing["prompt"])?,
        per_output_token: price(&pricing["completion"])?,
    })
}

fn million(value: &Value) -> Option<String> {
    let value = value.as_str()?.parse::<rust_decimal::Decimal>().ok()?;
    if value.is_sign_negative() {
        return None;
    }
    Some(
        (value.checked_mul(rust_decimal::Decimal::from(1_000_000))?)
            .normalize()
            .to_string(),
    )
}

fn per_million_f64(value: &Value) -> Option<f64> {
    million(value).and_then(|v| v.parse().ok())
}

fn strings(value: &Value) -> Option<Vec<String>> {
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
    )
}

fn is_chat_endpoint(endpoint: &str) -> bool {
    let path = endpoint.trim_start_matches('/');
    let path = path.strip_prefix("v1/").unwrap_or(path);
    CHAT_ENDPOINTS.contains(&path)
}

/// A modality list the catalog states must include text; a missing list is no
/// evidence either way (older rows and fixtures omit it).
fn allows_text(modalities: &Option<Vec<String>>) -> bool {
    modalities
        .as_ref()
        .is_none_or(|list| list.iter().any(|m| m == "text"))
}

/// Chat-capable means: a protocol the runtime speaks, text in, text out, and a
/// usable context window. Image, audio, video and embedding models are dropped.
pub fn parse_models(value: Value) -> Vec<Model> {
    let Some(items) = value["data"].as_array() else {
        return vec![];
    };
    items.iter().filter_map(parse_model).collect()
}

fn parse_model(item: &Value) -> Option<Model> {
    let id = item["id"].as_str()?;
    let context = item["context_length"].as_u64().filter(|v| *v >= 1024)?;
    let endpoints = strings(&item["supported_endpoints"])?;
    if !endpoints.iter().any(|e| is_chat_endpoint(e)) {
        return None;
    }
    let inputs = strings(&item["architecture"]["input_modalities"]);
    let outputs = strings(&item["architecture"]["output_modalities"]);
    if !allows_text(&inputs) || !allows_text(&outputs) {
        return None;
    }
    let inputs = inputs.unwrap_or_default();
    let pricing = &item["pricing"];
    let cache_write = pricing
        .get("input_cache_write_5m")
        .unwrap_or(&pricing["input_cache_write"]);
    Some(Model {
        id: id.into(),
        name: item["name"].as_str().unwrap_or(id).into(),
        context_window: context,
        max_output_tokens: item["top_provider"]["max_completion_tokens"]
            .as_u64()
            .filter(|v| *v > 0)
            .unwrap_or(8192)
            .min(context / 2),
        supports_images: inputs.iter().any(|v| v == "image"),
        input_modalities: inputs,
        supported_endpoints: endpoints,
        input_price_per_million: million(&pricing["prompt"]),
        output_price_per_million: million(&pricing["completion"]),
        pricing: pricing.clone(),
        cost: json!({
            "input": per_million_f64(&pricing["prompt"]),
            "output": per_million_f64(&pricing["completion"]),
            "cacheRead": per_million_f64(&pricing["input_cache_read"]),
            "cacheWrite": per_million_f64(cache_write),
        }),
        pricing_known: million(&pricing["prompt"]).is_some()
            && million(&pricing["completion"]).is_some(),
    })
}

/// The model a new session starts with: the remembered one if the catalog still
/// offers it, else the first offered entry of [`PREFERRED_MODEL_IDS`], else the
/// first model of `models` (all chat-capable, see [`parse_models`]). Never a
/// model `models` does not contain; `None` only for an empty catalog.
pub fn default_model_id(models: &[Model], remembered: Option<&str>) -> Option<String> {
    let offered = |id: &str| models.iter().any(|m| m.id == id);
    remembered
        .filter(|id| offered(id))
        .or_else(|| PREFERRED_MODEL_IDS.iter().copied().find(|id| offered(id)))
        .or_else(|| models.first().map(|m| m.id.as_str()))
        .map(str::to_owned)
}

/// The live catalog is ~150 KiB; anything near this is not a catalog.
const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;

/// The chat models of `apex`'s public catalog. An apex that is not a known Ofox
/// apex is refused before any request is made.
pub async fn fetch_models(apex: &str) -> Result<Vec<Model>, String> {
    let base = crate::ofox_apex::models_catalog_base_for(apex).ok_or("assistant_unknown_apex")?;
    fetch_models_at(&format!("{base}/v1/models")).await
}

/// [`fetch_models`] against a given URL, so tests can point it at a local server.
async fn fetch_models_at(url: &str) -> Result<Vec<Model>, String> {
    let unavailable = |_| "assistant_catalog_unavailable".to_string();
    let mut response = crate::proxy::http_client::get()
        .get(url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(unavailable)?;
    if !response.status().is_success() {
        return Err("assistant_catalog_unavailable".into());
    }
    let too_large = || "assistant_catalog_too_large".to_string();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_CATALOG_BYTES as u64)
    {
        return Err(too_large());
    }
    // Count while reading: a missing or false length cannot make us buffer more.
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(unavailable)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_CATALOG_BYTES {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    let catalog = serde_json::from_slice(&bytes).map_err(|_| "assistant_catalog_invalid")?;
    let models = parse_models(catalog);
    if models.is_empty() {
        return Err("assistant_no_compatible_models".into());
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A catalog entry shaped like the live `/v1/models` rows.
    fn entry(id: &str, endpoints: &[&str], inputs: &[&str], outputs: &[&str]) -> Value {
        json!({
            "id": id,
            "context_length": 128000,
            "top_provider": {"max_completion_tokens": 16000},
            "supported_endpoints": endpoints,
            "architecture": {"input_modalities": inputs, "output_modalities": outputs},
            "pricing": {"prompt": "0.000002", "completion": "0.00001"}
        })
    }

    fn ids(models: &[Model]) -> Vec<&str> {
        models.iter().map(|m| m.id.as_str()).collect()
    }

    fn catalog(ids: &[&str]) -> Vec<Model> {
        parse_models(json!({"data": ids
            .iter()
            .map(|id| entry(id, &["/v1/chat/completions"], &["text"], &["text"]))
            .collect::<Vec<_>>()}))
    }

    #[test]
    fn runtime_fixture_projection_matches_the_sidecar_contract() {
        assert!(million(&json!("-0.01")).is_none());
        assert!(million(&json!("9999999999999999999999999999")).is_none());
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../assistant-runtime/test/fixtures/catalog-model.json"
        ))
        .unwrap();
        let model = serde_json::to_value(&parse_models(fixture["catalog"].clone())[0]).unwrap();
        for field in [
            "id",
            "contextWindow",
            "maxOutputTokens",
            "supportsImages",
            "supportedEndpoints",
        ] {
            assert_eq!(model[field], fixture["runtime"][field]);
        }
        assert_eq!(model["cost"]["input"], 2.0);
        assert_eq!(model["cost"]["cacheRead"], 0.5);
    }

    #[test]
    fn model_projection_preserves_capacity_modalities_and_both_prices() {
        let models = parse_models(
            json!({"data":[{"id":"test/model","context_length":128000,"top_provider":{"max_completion_tokens":16000},"supported_endpoints":["/v1/responses"],"architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0.000002","completion":"0.00001"}}]}),
        );
        assert_eq!(models[0].context_window, 128000);
        assert_eq!(models[0].max_output_tokens, 16000);
        assert!(models[0].supports_images);
        assert_eq!(models[0].input_price_per_million.as_deref(), Some("2"));
        assert_eq!(models[0].output_price_per_million.as_deref(), Some("10"));
        assert!(parse_models(json!({"data":[{"id":"unknown"}]})).is_empty());
    }

    #[test]
    fn only_text_chat_models_are_offered() {
        let models = parse_models(json!({"data": [
            entry("chat/completions", &["/v1/chat/completions"], &["text", "image"], &["text"]),
            entry("chat/responses", &["/v1/responses"], &["text"], &["text"]),
            entry("chat/messages", &["/v1/messages"], &["text"], &["text"]),
            // Live catalog rows that a chat endpoint alone would let through.
            entry("image/out", &["/v1/chat/completions", "/v1/images/generations"], &["text", "image"], &["image"]),
            entry("audio/in", &["/v1/chat/completions"], &["audio"], &["text"]),
            entry("embed/text", &["/v1/embeddings"], &["text"], &["text"]),
            entry("video/out", &["/v1/videos"], &["text"], &["video"]),
            entry("decide/text", &["/v1/decisions"], &["text"], &["text"]),
            entry("lookalike/path", &["/v1/realtime/responses-preview"], &["text"], &["text"]),
        ]}));
        assert_eq!(
            ids(&models),
            ["chat/completions", "chat/responses", "chat/messages"]
        );
    }

    #[test]
    fn a_remembered_model_still_in_the_catalog_wins() {
        let models = catalog(&["z/first", PREFERRED_MODEL_IDS[0], "a/remembered"]);
        assert_eq!(
            default_model_id(&models, Some("a/remembered")).as_deref(),
            Some("a/remembered")
        );
    }

    #[test]
    fn a_remembered_model_gone_from_the_catalog_falls_back_to_the_preference_list() {
        let preferred = PREFERRED_MODEL_IDS[PREFERRED_MODEL_IDS.len() - 1];
        let models = catalog(&["z/first", preferred]);
        assert_eq!(
            default_model_id(&models, Some("gone/model")).as_deref(),
            Some(preferred)
        );
    }

    #[test]
    fn the_first_available_preference_is_picked_in_list_order() {
        let mut offered = PREFERRED_MODEL_IDS.to_vec();
        offered.reverse();
        offered.push("z/other");
        let models = catalog(&offered);
        assert_eq!(
            default_model_id(&models, None).as_deref(),
            Some(PREFERRED_MODEL_IDS[0])
        );
    }

    #[test]
    fn without_any_preference_the_first_chat_model_is_the_default() {
        let models = catalog(&["z/first", "a/second"]);
        assert_eq!(default_model_id(&models, None).as_deref(), Some("z/first"));
    }

    #[test]
    fn an_empty_catalog_has_no_default() {
        assert_eq!(default_model_id(&[], Some(PREFERRED_MODEL_IDS[0])), None);
    }

    async fn serve(response: wiremock::ResponseTemplate) -> wiremock::MockServer {
        use wiremock::matchers::{method, path};
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    async fn fetch_from(server: &wiremock::MockServer) -> Result<Vec<Model>, String> {
        fetch_models_at(&format!("{}/v1/models", server.uri())).await
    }

    #[tokio::test]
    async fn fetch_returns_the_chat_models_of_the_catalog() {
        let body = json!({"data": [
            entry("chat/model", &["/v1/chat/completions"], &["text"], &["text"]),
            entry("image/model", &["/v1/images/generations"], &["text"], &["image"]),
        ]});
        let server = serve(wiremock::ResponseTemplate::new(200).set_body_json(body)).await;
        assert_eq!(ids(&fetch_from(&server).await.unwrap()), ["chat/model"]);
    }

    #[tokio::test]
    async fn fetch_reports_each_failure_with_its_own_code() {
        use wiremock::ResponseTemplate;
        let only_images = json!({"data": [
            entry("image/model", &["/v1/images/generations"], &["text"], &["image"]),
        ]});
        for (response, code) in [
            (ResponseTemplate::new(503), "assistant_catalog_unavailable"),
            (
                ResponseTemplate::new(200).set_body_string("not json"),
                "assistant_catalog_invalid",
            ),
            (
                ResponseTemplate::new(200).set_body_json(only_images),
                "assistant_no_compatible_models",
            ),
        ] {
            let server = serve(response).await;
            assert_eq!(fetch_from(&server).await.unwrap_err(), code);
        }
    }

    #[tokio::test]
    async fn fetch_refuses_a_catalog_over_the_size_limit() {
        let body = vec![b' '; MAX_CATALOG_BYTES + 1];
        let server = serve(wiremock::ResponseTemplate::new(200).set_body_bytes(body)).await;
        assert_eq!(
            fetch_from(&server).await.unwrap_err(),
            "assistant_catalog_too_large"
        );
    }

    #[tokio::test]
    async fn fetch_stops_reading_an_unsized_catalog_at_the_limit() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            // Chunked, no length header: only the running count can stop this.
            let head = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n";
            if socket.write_all(head).await.is_err() {
                return;
            }
            let chunk = vec![b' '; 1024 * 1024];
            for _ in 0..16 {
                let size = format!("{:x}\r\n", chunk.len());
                for part in [size.as_bytes(), &chunk, b"\r\n"] {
                    if socket.write_all(part).await.is_err() {
                        return;
                    }
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        });
        let result = fetch_models_at(&format!("http://{address}/v1/models")).await;
        server.abort();
        assert_eq!(result.unwrap_err(), "assistant_catalog_too_large");
    }

    #[tokio::test]
    async fn fetch_never_builds_a_host_from_an_unknown_apex() {
        assert_eq!(
            fetch_models("evil.example").await.unwrap_err(),
            "assistant_unknown_apex"
        );
    }

    /// The live row of the search model (prices as `/v1/models` gave them).
    fn search_row(pricing: Value) -> Value {
        json!({
            "id": SEARCH_MODEL,
            "context_length": 128000,
            "top_provider": {"max_completion_tokens": 16384},
            "supported_endpoints": ["/v1/chat/completions", "/v1/responses"],
            "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]},
            "pricing": pricing,
        })
    }

    fn priced(pricing: Value) -> Option<SearchPrice> {
        search_price(&parse_models(
            json!({"data": [entry("chat/model", &["/v1/chat/completions"], &["text"], &["text"]), search_row(pricing)]}),
        ))
    }

    #[test]
    fn a_search_costs_its_web_search_price_plus_the_search_models_tokens() {
        let price = priced(json!({
            "prompt": "0.00000005", "completion": "0.0000004",
            "input_cache_read": "0.00000001", "web_search": "0.014",
        }))
        .unwrap();
        assert_eq!(price.per_search.to_string(), "0.014");
        // Two searches, 1000 tokens in and 500 out.
        assert_eq!(
            price.cost(2, 1000, 500).unwrap().normalize().to_string(),
            "0.02825"
        );
        assert_eq!(price.cost(0, 0, 0).unwrap().normalize().to_string(), "0");
        // Saved with a run as decimal strings, read back the same.
        let saved = serde_json::to_value(&price).unwrap();
        assert_eq!(
            saved,
            json!({"model": SEARCH_MODEL, "perSearch": "0.014",
                   "perInputToken": "0.00000005", "perOutputToken": "0.0000004"})
        );
        assert_eq!(serde_json::from_value::<SearchPrice>(saved).unwrap(), price);
    }

    #[test]
    fn search_is_unpriced_without_the_search_model_or_any_of_its_prices() {
        let full =
            json!({"prompt": "0.00000005", "completion": "0.0000004", "web_search": "0.014"});
        assert!(priced(full.clone()).is_some());
        for missing in ["web_search", "prompt", "completion"] {
            let mut pricing = full.clone();
            pricing.as_object_mut().unwrap().remove(missing);
            assert!(priced(pricing).is_none(), "{missing}");
        }
        for bad in [json!("-0.014"), json!("free"), json!(0.014), Value::Null] {
            let mut pricing = full.clone();
            pricing["web_search"] = bad.clone();
            assert!(priced(pricing).is_none(), "{bad}");
        }
        assert!(search_price(&catalog(&["chat/model"])).is_none());
    }

    #[test]
    fn the_preference_list_is_non_empty_and_has_no_duplicates() {
        assert!(!PREFERRED_MODEL_IDS.is_empty());
        let mut unique = PREFERRED_MODEL_IDS.to_vec();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), PREFERRED_MODEL_IDS.len());
    }
}
