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
pub fn parse_models(value: Value) -> Vec<Model> {
    value["data"].as_array().into_iter().flatten().filter_map(|item| {
        let id = item["id"].as_str()?;
        let context = item["context_length"].as_u64().filter(|v| *v >= 1024)?;
        let endpoints: Vec<String> = item["supported_endpoints"].as_array()?.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        if !endpoints.iter().any(|e| e.contains("chat/completions") || e.contains("responses") || e.contains("messages")) { return None; }
        let inputs: Vec<String> = item["architecture"]["input_modalities"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        Some(Model { id: id.into(), name: item["name"].as_str().unwrap_or(id).into(), context_window: context,
            max_output_tokens: item["top_provider"]["max_completion_tokens"].as_u64().filter(|v| *v > 0).unwrap_or(8192).min(context / 2),
            supports_images: inputs.iter().any(|v| v == "image"), input_modalities: inputs,
            supported_endpoints: endpoints, input_price_per_million: million(&item["pricing"]["prompt"]), output_price_per_million: million(&item["pricing"]["completion"]), pricing: item["pricing"].clone(), cost: json!({"input":million(&item["pricing"]["prompt"]).and_then(|v|v.parse::<f64>().ok()),"output":million(&item["pricing"]["completion"]).and_then(|v|v.parse::<f64>().ok()),"cacheRead":million(&item["pricing"]["input_cache_read"]).and_then(|v|v.parse::<f64>().ok()),"cacheWrite":million(item["pricing"].get("input_cache_write_5m").unwrap_or(&item["pricing"]["input_cache_write"])).and_then(|v|v.parse::<f64>().ok())}),pricing_known:million(&item["pricing"]["prompt"]).is_some() && million(&item["pricing"]["completion"]).is_some() })
    }).collect()
}

pub async fn fetch_models() -> Result<Vec<Model>, String> {
    let response = crate::proxy::http_client::get()
        .get(format!(
            "{}/v1/models",
            crate::ofox_apex::models_catalog_base()
        ))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| "assistant_catalog_unavailable")?;
    if !response.status().is_success() {
        return Err("assistant_catalog_unavailable".into());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "assistant_catalog_unavailable")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("assistant_catalog_too_large".into());
    }
    let models =
        parse_models(serde_json::from_slice(&bytes).map_err(|_| "assistant_catalog_invalid")?);
    if models.is_empty() {
        return Err("assistant_no_compatible_models".into());
    }
    Ok(models)
}

pub async fn capabilities(token: &str) -> Value {
    capabilities_at(token, crate::ofox_apex::current_apex()).await
}
pub async fn capabilities_at(token: &str, apex: &str) -> Value {
    let fallback =
        || json!({"schemaVersion":1,"search":{"available":false,"reason":"service_unavailable"}});
    let response = match crate::proxy::http_client::get()
        .get(format!(
            "{}/openapi/assistant/capabilities",
            crate::ofox_apex::auth_api_base_for(apex)
        ))
        .bearer_auth(token)
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return fallback(),
    };
    if !response.status().is_success() {
        return fallback();
    }
    let value: Value = match response.json().await {
        Ok(v) => v,
        Err(_) => return fallback(),
    };
    validate_capabilities(value)
}

fn validate_capabilities(mut value: Value) -> Value {
    let search = &value["search"];
    let valid = value["schemaVersion"] == 1
        && search["available"] == true
        && search["model"] == "ofox/web-search"
        && search["endpoint"] == "/v1/search"
        && search["currency"] == "USD"
        && search["unit"] == "request"
        && search["priceVersion"]
            .as_str()
            .is_some_and(|v| v.starts_with("sha256:") && v.len() == 71)
        && search["unitPrice"]
            .as_str()
            .and_then(|v| v.parse::<rust_decimal::Decimal>().ok())
            .is_some_and(|v| v > rust_decimal::Decimal::ZERO);
    if !valid {
        value =
            json!({"schemaVersion":1,"search":{"available":false,"reason":"pricing_unavailable"}});
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capabilities_require_supported_schema_and_a_valid_display_price() {
        let valid = json!({"schemaVersion":1,"search":{"available":true,"model":"ofox/web-search","endpoint":"/v1/search","unitPrice":"0.01","currency":"USD","unit":"request","priceVersion":format!("sha256:{}","a".repeat(64))}});
        assert_eq!(
            validate_capabilities(valid.clone())["search"]["available"],
            true
        );
        for price in ["0", "-1", "n/a", ""] {
            let mut v = valid.clone();
            v["search"]["unitPrice"] = json!(price);
            assert_eq!(validate_capabilities(v)["search"]["available"], false);
        }
        let mut old = valid.clone();
        old["schemaVersion"] = json!(2);
        assert_eq!(validate_capabilities(old)["search"]["available"], false);
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
}
