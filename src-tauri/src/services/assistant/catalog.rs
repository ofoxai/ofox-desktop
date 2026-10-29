//! The Ofox model catalog as the assistant sees it: `/v1/models` merged with
//! `/anthropic/v1/models`, every row classified by kind, with the capacity,
//! prices and capabilities the runtime and the model picker need.
//! Conversations use the chat models only (see [`Catalog::conversation_models`]).
use super::thinking::{self, AnthropicCompat, LevelMap};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

/// What a model makes. Rows of any other kind (embeddings, decisions) are
/// dropped when the catalog is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Text replies: the only kind offered for conversations so far. A model
    /// saved before kinds existed reads as chat.
    #[default]
    Chat,
    Image,
    Video,
    Speech,
    Transcription,
}

/// The API a chat model runs with, named as the runtime (Pi) names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Protocol {
    #[serde(rename = "openai-completions")]
    OpenAiCompletions,
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
    #[serde(rename = "anthropic-messages")]
    AnthropicMessages,
}

/// The two catalogs the assistant reads. Both are public, keyless and
/// always online (see [`crate::ofox_apex::models_catalog_base_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// `/v1/models`: every vendor and kind except Claude.
    V1,
    /// `/anthropic/v1/models`: Messages-API models, the only home of Claude.
    /// Only its Claude rows are used (see [`parse_models`]).
    Anthropic,
}

impl Source {
    fn path(self) -> &'static str {
        match self {
            Self::V1 => "/v1/models",
            Self::Anthropic => "/anthropic/v1/models",
        }
    }
}

/// Where an `/anthropic/v1/models` row is served; its rows list no endpoints.
const ANTHROPIC_MESSAGES_ENDPOINT: &str = "/anthropic/v1/messages";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    /// The catalog's full name (an Anthropic row's `display_name`).
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
    #[serde(default)]
    pub kind: Kind,
    /// The id's prefix (`openai`, `anthropic`, …); empty when it has none.
    #[serde(default)]
    pub vendor: String,
    /// `name` without a leading "Vendor: ", for a list grouped by vendor.
    #[serde(default)]
    pub short_name: String,
    /// The API a chat model runs with; `None` for the other kinds.
    #[serde(default)]
    pub protocol: Option<Protocol>,
    /// The catalog lists a `reasoning` parameter.
    #[serde(default)]
    pub reasoning: bool,
    /// The model takes tools; assumed when the catalog lists no parameters.
    #[serde(default = "assumed_tools")]
    pub tools: bool,
    /// The thinking levels a conversation may choose, in Pi's order; empty
    /// when the model does not reason. No level (the default) sends nothing.
    #[serde(default)]
    pub thinking_levels: Vec<String>,
    /// Pi's map from a level to what the provider is sent, when Pi has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level_map: Option<LevelMap>,
    /// Pi's thinking compat for a Claude model run with Messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_compat: Option<AnthropicCompat>,
    /// When the model was added, in Unix seconds.
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoCapabilities>,
}

fn assumed_tools() -> bool {
    true
}

/// What an image model accepts, from its endpoints and `image_attributes`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCapabilities {
    /// `/v1/images/generations`: draws from a prompt.
    pub generations: bool,
    /// `/v1/images/edits`: changes an input image.
    pub edits: bool,
    /// Takes `n`, several images per request.
    pub n: bool,
    /// The sizes the catalog lists; empty when it lists none.
    pub sizes: Vec<String>,
    /// The qualities the catalog lists; empty when it lists none.
    pub qualities: Vec<String>,
    /// USD per image (`pricing.output_image_per_num`).
    pub per_image_price: Option<String>,
    /// USD per million tokens, for a model billed by image tokens.
    pub token_pricing: Option<ImageTokenPricing>,
}

/// An image model's token prices, USD per million tokens.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageTokenPricing {
    /// Prompt text (`pricing.prompt`).
    pub input: Option<String>,
    /// Input images (`pricing.image`).
    pub input_image: Option<String>,
    /// Generated images (`pricing.output_image`).
    pub output_image: String,
}

/// What a video model accepts, from its `video_attributes`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCapabilities {
    /// `t2v`, `i2v`, `v2v`.
    pub modes: Vec<String>,
    pub resolutions: Vec<String>,
    pub default_resolution: Option<String>,
    /// Seconds.
    pub min_duration: Option<u64>,
    /// Seconds.
    pub max_duration: Option<u64>,
    pub supports_audio: bool,
    pub aspect_ratios: Vec<String>,
    /// USD per second at the catalog's listed rate; tiers may cost more.
    pub from_price_per_second: Option<String>,
}

/// A model as a run receives it: the fields the runtime read before the
/// catalog learned kinds, so a run at the defaults gets them byte for byte.
/// The fields after `api` appear only when they change the run: thinking
/// only for a run at a level (`reasoning` switches Pi's thinking parameters
/// on), `tools` only for a model that takes none.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeModel<'a> {
    id: &'a str,
    name: &'a str,
    context_window: u64,
    max_output_tokens: u64,
    supports_images: bool,
    input_modalities: &'a [String],
    supported_endpoints: &'a [String],
    input_price_per_million: &'a Option<String>,
    output_price_per_million: &'a Option<String>,
    pricing: &'a Value,
    cost: &'a Value,
    pricing_known: bool,
    /// The protocol, only when it is not the first one the endpoints name:
    /// the runtime picks that one by itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    api: Option<Protocol>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_level_map: Option<&'a LevelMap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    anthropic_compat: Option<&'a AnthropicCompat>,
    /// `false` only: the runtime then registers no tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<bool>,
}

impl Model {
    /// Whether a conversation can use this model: chat or supported images.
    pub fn for_conversations(&self) -> bool {
        self.kind == Kind::Chat
            || (self.kind == Kind::Image
                && self
                    .image
                    .as_ref()
                    .is_some_and(|i| i.generations || i.edits))
    }

    /// Whether a conversation may run this model at `level`.
    pub fn offers_thinking_level(&self, level: &str) -> bool {
        self.thinking_levels.iter().any(|offered| offered == level)
    }

    /// The model for a run; `thinks` when the run has a thinking level (one
    /// of [`Self::thinking_levels`]).
    pub fn for_runtime(&self, thinks: bool) -> RuntimeModel<'_> {
        let first = self
            .supported_endpoints
            .iter()
            .find_map(|e| endpoint_protocol(e));
        RuntimeModel {
            id: &self.id,
            name: &self.name,
            context_window: self.context_window,
            max_output_tokens: self.max_output_tokens,
            supports_images: self.supports_images,
            input_modalities: &self.input_modalities,
            supported_endpoints: &self.supported_endpoints,
            input_price_per_million: &self.input_price_per_million,
            output_price_per_million: &self.output_price_per_million,
            pricing: &self.pricing,
            cost: &self.cost,
            pricing_known: self.pricing_known,
            api: self.protocol.filter(|protocol| Some(*protocol) != first),
            reasoning: thinks.then_some(true),
            thinking_level_map: self.thinking_level_map.as_ref().filter(|_| thinks),
            anthropic_compat: self.anthropic_compat.as_ref().filter(|_| thinks),
            tools: (!self.tools).then_some(false),
        }
    }
}

/// Defaults tried in order when nothing remembered is still offered. Every
/// entry was a chat model in the live catalogs of both apexes when chosen; one
/// that disappears is skipped, never offered. The last is the cheapest Claude,
/// for when `/v1` cannot be read: the first Claude row would otherwise be the
/// default, and it is the priciest.
pub const PREFERRED_MODEL_IDS: &[&str] = &[
    "openai/gpt-6.1-sol",
    "openai/gpt-6-sol",
    "openai/gpt-5.4-mini",
    "anthropic/claude-haiku-5.5",
];

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

/// A catalog price as it is (USD per image, per second), normalised.
fn unit_price(value: &Value) -> Option<String> {
    price(value).map(|price| price.normalize().to_string())
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

/// An endpoint as a path without its leading `/` and `v1/`:
/// `/v1/chat/completions` → `chat/completions`.
fn endpoint_path(endpoint: &str) -> &str {
    let path = endpoint.trim_start_matches('/');
    path.strip_prefix("v1/").unwrap_or(path)
}

/// The protocol an endpoint speaks, if the runtime can drive it. The same
/// table as the runtime's `modelForOfox` aliases, restricted to catalog paths.
fn endpoint_protocol(endpoint: &str) -> Option<Protocol> {
    match endpoint_path(endpoint) {
        "chat/completions" => Some(Protocol::OpenAiCompletions),
        "responses" => Some(Protocol::OpenAiResponses),
        "messages" | "anthropic/v1/messages" => Some(Protocol::AnthropicMessages),
        _ => None,
    }
}

/// The runtime runs a model with the first protocol its endpoints name, which
/// is the choice every `/v1` model keeps. Claude runs with Messages wherever
/// it is listed.
fn chat_protocol(vendor: &str, endpoints: &[String]) -> Option<Protocol> {
    let mut advertised = endpoints.iter().filter_map(|e| endpoint_protocol(e));
    if vendor == "anthropic"
        && advertised
            .clone()
            .any(|protocol| protocol == Protocol::AnthropicMessages)
    {
        return Some(Protocol::AnthropicMessages);
    }
    advertised.next()
}

/// A modality list the catalog states must include `modality`; a missing list
/// is no evidence either way (older rows and fixtures omit it).
fn allows(modalities: &Option<Vec<String>>, modality: &str) -> bool {
    modalities
        .as_ref()
        .is_none_or(|list| list.iter().any(|m| m == modality))
}

/// A chat model needs at least this much context; media rows may report 0.
const MIN_CHAT_CONTEXT: u64 = 1024;

/// The kind of a row, first match wins: video, image (this catches the Gemini
/// image models that also list chat), speech, transcription, then chat (a
/// protocol the runtime speaks, text in, text out, a usable context window).
/// Anything else is no kind the assistant uses.
fn kind_of(
    endpoints: &[String],
    inputs: &Option<Vec<String>>,
    outputs: &Option<Vec<String>>,
    context: u64,
) -> Option<Kind> {
    let serves = |path: &str| endpoints.iter().any(|e| endpoint_path(e) == path);
    if serves("videos") && allows(outputs, "video") {
        Some(Kind::Video)
    } else if (serves("images/generations") || serves("images/edits")) && allows(outputs, "image") {
        Some(Kind::Image)
    } else if serves("audio/speech") {
        Some(Kind::Speech)
    } else if serves("audio/transcriptions") {
        Some(Kind::Transcription)
    } else if endpoints.iter().any(|e| endpoint_protocol(e).is_some())
        && allows(inputs, "text")
        && allows(outputs, "text")
        && context >= MIN_CHAT_CONTEXT
    {
        Some(Kind::Chat)
    } else {
        None
    }
}

/// `name` without a leading one-word vendor and colon ("OpenAI: GPT-5.4" →
/// "GPT-5.4"); unchanged when it has none.
fn short_name(name: &str) -> &str {
    match name.split_once(": ") {
        Some((vendor, rest))
            if !vendor.is_empty()
                && !vendor.contains(char::is_whitespace)
                && !rest.trim().is_empty() =>
        {
            rest.trim()
        }
        _ => name,
    }
}

/// `created` (Unix seconds) on `/v1` rows, `created_at` (RFC 3339) on
/// Anthropic rows.
fn created(item: &Value) -> Option<i64> {
    item["created"].as_i64().or_else(|| {
        chrono::DateTime::parse_from_rfc3339(item["created_at"].as_str()?)
            .ok()
            .map(|at| at.timestamp())
    })
}

fn image_capabilities(item: &Value, endpoints: &[String]) -> ImageCapabilities {
    let serves = |path: &str| endpoints.iter().any(|e| endpoint_path(e) == path);
    let attributes = &item["image_attributes"];
    let params = strings(&attributes["supported_params"])
        .or_else(|| strings(&item["supported_parameters"]))
        .unwrap_or_default();
    let pricing = &item["pricing"];
    ImageCapabilities {
        generations: serves("images/generations"),
        edits: serves("images/edits"),
        n: params.iter().any(|p| p == "n"),
        sizes: strings(&attributes["sizes"]).unwrap_or_default(),
        qualities: strings(&attributes["qualities"]).unwrap_or_default(),
        per_image_price: unit_price(&pricing["output_image_per_num"]),
        token_pricing: million(&pricing["output_image"]).map(|output_image| ImageTokenPricing {
            input: million(&pricing["prompt"]),
            input_image: million(&pricing["image"]),
            output_image,
        }),
    }
}

fn video_capabilities(item: &Value) -> VideoCapabilities {
    let attributes = &item["video_attributes"];
    VideoCapabilities {
        modes: strings(&attributes["modes"]).unwrap_or_default(),
        resolutions: strings(&attributes["resolutions"]).unwrap_or_default(),
        default_resolution: attributes["default_resolution"].as_str().map(str::to_owned),
        min_duration: attributes["min_duration_seconds"].as_u64(),
        max_duration: attributes["max_duration_seconds"].as_u64(),
        supports_audio: attributes["supports_audio"].as_bool().unwrap_or(false),
        aspect_ratios: strings(&attributes["aspect_ratios"]).unwrap_or_default(),
        from_price_per_second: unit_price(&item["pricing"]["output_video_per_second"]),
    }
}

/// The rows of one catalog the assistant can use, each with its kind. The
/// Anthropic catalog also lists other vendors' models, which `/v1` serves with
/// their own protocol: only its Claude rows are taken, so no model changes
/// protocol when `/v1` cannot be read.
pub fn parse_models(value: Value, source: Source) -> Vec<Model> {
    let Some(items) = value["data"].as_array() else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|item| parse_model(item, source))
        .collect()
}

fn parse_model(item: &Value, source: Source) -> Option<Model> {
    let id = item["id"].as_str()?;
    let endpoints = match source {
        Source::V1 => strings(&item["supported_endpoints"])?,
        // The catalog that lists the row is the Messages API's.
        Source::Anthropic => vec![ANTHROPIC_MESSAGES_ENDPOINT.to_owned()],
    };
    let context = item["context_length"].as_u64().unwrap_or(0);
    let inputs = strings(&item["architecture"]["input_modalities"]);
    let outputs = strings(&item["architecture"]["output_modalities"]);
    let vendor = id.split_once('/').map_or("", |(vendor, _)| vendor);
    if source == Source::Anthropic && vendor != "anthropic" {
        return None;
    }
    let kind = kind_of(&endpoints, &inputs, &outputs, context)?;
    let name = item["name"]
        .as_str()
        .or_else(|| item["display_name"].as_str())
        .unwrap_or(id);
    let parameters = strings(&item["supported_parameters"]);
    let lists = |parameter: &str| {
        parameters
            .as_ref()
            .map(|list| list.iter().any(|p| p == parameter))
    };
    let inputs = inputs.unwrap_or_default();
    let reasoning = lists("reasoning").unwrap_or(false);
    let protocol = match kind {
        Kind::Chat => chat_protocol(vendor, &endpoints),
        _ => None,
    };
    let thinking = match kind {
        Kind::Chat => thinking::for_model(id, reasoning, protocol),
        _ => thinking::Thinking::default(),
    };
    let pricing = &item["pricing"];
    let cache_write = pricing
        .get("input_cache_write_5m")
        .unwrap_or(&pricing["input_cache_write"]);
    Some(Model {
        id: id.into(),
        name: name.into(),
        context_window: context,
        max_output_tokens: item["top_provider"]["max_completion_tokens"]
            .as_u64()
            .filter(|v| *v > 0)
            .unwrap_or(8192)
            .min(context / 2),
        supports_images: inputs.iter().any(|v| v == "image"),
        input_modalities: inputs,
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
        kind,
        vendor: vendor.into(),
        short_name: short_name(name).into(),
        protocol,
        reasoning,
        tools: lists("tools").unwrap_or(true),
        thinking_levels: thinking.levels,
        thinking_level_map: thinking.thinking_level_map,
        anthropic_compat: thinking.anthropic_compat,
        created: created(item),
        image: (kind == Kind::Image).then(|| image_capabilities(item, &endpoints)),
        video: (kind == Kind::Video).then(|| video_capabilities(item)),
        supported_endpoints: endpoints,
    })
}

/// Both catalogs of an apex as one list, and the sources that could not be
/// read (shown as "some models are unavailable").
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    /// `/v1` rows in catalog order, then the Claude rows `/v1` lacks.
    pub models: Vec<Model>,
    pub partial: Vec<Source>,
}

impl Catalog {
    /// The models a conversation can use (see [`Model::for_conversations`]).
    pub fn conversation_models(&self) -> Vec<Model> {
        self.models
            .iter()
            .filter(|m| m.for_conversations())
            .cloned()
            .collect()
    }

    /// The conversation model `id`, if the catalog offers it.
    pub fn conversation_model(self, id: &str) -> Option<Model> {
        self.models
            .into_iter()
            .find(|m| m.id == id && m.for_conversations())
    }
}

/// One catalog from both sources. A row whose id `/v1` also lists is the
/// `/v1` row. A source that failed is named in `partial`; with both failed
/// the `/v1` error stands, and a catalog without any conversation model is
/// `assistant_no_compatible_models`.
pub fn merge(
    v1: Result<Vec<Model>, String>,
    anthropic: Result<Vec<Model>, String>,
) -> Result<Catalog, String> {
    let (v1, anthropic) = match (v1, anthropic) {
        (Err(error), Err(_)) => return Err(error),
        pair => pair,
    };
    let mut partial = vec![];
    let mut models = v1.unwrap_or_else(|_| {
        partial.push(Source::V1);
        vec![]
    });
    let listed: HashSet<String> = models.iter().map(|m| m.id.clone()).collect();
    match anthropic {
        Ok(rows) => models.extend(rows.into_iter().filter(|m| !listed.contains(&m.id))),
        Err(_) => partial.push(Source::Anthropic),
    }
    if !models.iter().any(|m| m.kind == Kind::Chat) {
        return Err(NO_COMPATIBLE_MODELS.into());
    }
    Ok(Catalog { models, partial })
}

const NO_COMPATIBLE_MODELS: &str = "assistant_no_compatible_models";

/// The model a new session starts with: the remembered one if the catalog still
/// offers it for conversations, else the first offered entry of
/// [`PREFERRED_MODEL_IDS`], else the first chat model of `models`. Never a
/// model `models` does not contain, never one of another kind; `None` only
/// when there is no chat model.
pub fn default_model_id(models: &[Model], remembered: Option<&str>) -> Option<String> {
    let offered = |id: &str| models.iter().any(|m| m.id == id && m.kind == Kind::Chat);
    remembered
        .filter(|id| offered(id))
        .or_else(|| PREFERRED_MODEL_IDS.iter().copied().find(|id| offered(id)))
        .or_else(|| {
            models
                .iter()
                .find(|m| m.kind == Kind::Chat)
                .map(|m| m.id.as_str())
        })
        .map(str::to_owned)
}

/// The live catalog is ~150 KiB; anything near this is not a catalog.
const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;

/// The usable rows of one of `apex`'s public catalogs. An apex that is not a
/// known Ofox apex is refused before any request is made.
pub async fn fetch_models(apex: &str, source: Source) -> Result<Vec<Model>, String> {
    let base = crate::ofox_apex::models_catalog_base_for(apex).ok_or("assistant_unknown_apex")?;
    fetch_models_at(&format!("{base}{}", source.path()), source).await
}

/// [`fetch_models`] against a given URL, so tests can point it at a local server.
async fn fetch_models_at(url: &str, source: Source) -> Result<Vec<Model>, String> {
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
    let catalog: Value = serde_json::from_slice(&bytes).map_err(|_| "assistant_catalog_invalid")?;
    if catalog["has_more"] == true {
        log::warn!("[Assistant] {source:?} model catalog has more pages than the first");
    }
    let models = parse_models(catalog, source);
    // An answer without a conversation model (empty, or media only) is no
    // catalog: it fails, so it never replaces one read before.
    if !models.iter().any(|m| m.kind == Kind::Chat) {
        return Err(NO_COMPATIBLE_MODELS.into());
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

    fn v1(value: Value) -> Vec<Model> {
        parse_models(value, Source::V1)
    }

    fn ids(models: &[Model]) -> Vec<&str> {
        models.iter().map(|m| m.id.as_str()).collect()
    }

    fn kinds(models: &[Model]) -> Vec<(&str, Kind)> {
        models.iter().map(|m| (m.id.as_str(), m.kind)).collect()
    }

    fn catalog(ids: &[&str]) -> Vec<Model> {
        v1(json!({"data": ids
            .iter()
            .map(|id| entry(id, &["/v1/chat/completions"], &["text"], &["text"]))
            .collect::<Vec<_>>()}))
    }

    /// Rows trimmed from the live `/v1/models` (public, no key needed).
    fn v1_fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/assistant-catalog/v1-models.json"
        ))
        .unwrap()
    }

    /// Rows trimmed from the live `/anthropic/v1/models`: Claude, which only
    /// this catalog lists, and two other vendors' ids `/v1` lists too.
    fn anthropic_fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/assistant-catalog/anthropic-models.json"
        ))
        .unwrap()
    }

    fn real_catalog() -> Catalog {
        merge(
            Ok(v1(v1_fixture())),
            Ok(parse_models(anthropic_fixture(), Source::Anthropic)),
        )
        .unwrap()
    }

    const CLAUDE_IDS: &[&str] = &[
        "anthropic/claude-haiku-5.5",
        "anthropic/claude-sonnet-5.5",
        "anthropic/claude-opus-5.5",
        "anthropic/claude-fable-5.1",
    ];

    #[test]
    fn runtime_fixture_projection_matches_the_sidecar_contract() {
        assert!(million(&json!("-0.01")).is_none());
        assert!(million(&json!("9999999999999999999999999999")).is_none());
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../assistant-runtime/test/fixtures/catalog-model.json"
        ))
        .unwrap();
        let model =
            serde_json::to_value(v1(fixture["catalog"].clone())[0].for_runtime(false)).unwrap();
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
    fn a_claude_row_projects_to_the_runtime_fixture() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../assistant-runtime/test/fixtures/catalog-claude-model.json"
        ))
        .unwrap();
        let models = parse_models(fixture["catalog"].clone(), Source::Anthropic);
        assert_eq!(
            serde_json::to_value(models[0].for_runtime(false)).unwrap(),
            fixture["runtime"]
        );
    }

    #[test]
    fn model_projection_preserves_capacity_modalities_and_both_prices() {
        let models = v1(
            json!({"data":[{"id":"test/model","context_length":128000,"top_provider":{"max_completion_tokens":16000},"supported_endpoints":["/v1/responses"],"architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0.000002","completion":"0.00001"}}]}),
        );
        assert_eq!(models[0].context_window, 128000);
        assert_eq!(models[0].max_output_tokens, 16000);
        assert!(models[0].supports_images);
        assert_eq!(models[0].input_price_per_million.as_deref(), Some("2"));
        assert_eq!(models[0].output_price_per_million.as_deref(), Some("10"));
        assert!(v1(json!({"data":[{"id":"unknown"}]})).is_empty());
    }

    #[test]
    fn each_row_gets_its_kind_and_unusable_rows_are_dropped() {
        let models = v1(json!({"data": [
            entry("chat/completions", &["/v1/chat/completions"], &["text", "image"], &["text"]),
            entry("chat/responses", &["/v1/responses"], &["text"], &["text"]),
            entry("chat/messages", &["/v1/messages"], &["text"], &["text"]),
            // A chat endpoint alone does not make a chat model.
            entry("image/out", &["/v1/chat/completions", "/v1/images/generations"], &["text", "image"], &["image"]),
            entry("audio/in", &["/v1/chat/completions"], &["audio"], &["text"]),
            entry("embed/text", &["/v1/embeddings"], &["text"], &["text"]),
            entry("video/out", &["/v1/videos"], &["text"], &["video"]),
            entry("speech/out", &["/v1/audio/speech"], &["text"], &["audio"]),
            entry("transcribe/in", &["/v1/audio/transcriptions"], &["audio"], &["text"]),
            entry("decide/text", &["/v1/decisions"], &["text"], &["text"]),
            entry("lookalike/path", &["/v1/realtime/responses-preview"], &["text"], &["text"]),
        ]}));
        assert_eq!(
            kinds(&models),
            [
                ("chat/completions", Kind::Chat),
                ("chat/responses", Kind::Chat),
                ("chat/messages", Kind::Chat),
                ("image/out", Kind::Image),
                ("video/out", Kind::Video),
                ("speech/out", Kind::Speech),
                ("transcribe/in", Kind::Transcription),
            ]
        );
    }

    #[test]
    fn only_chat_rows_need_a_usable_context_window() {
        let mut small = entry(
            "chat/small",
            &["/v1/chat/completions"],
            &["text"],
            &["text"],
        );
        small["context_length"] = json!(1000);
        let mut video = entry("video/zero", &["/v1/videos"], &["text"], &["video"]);
        video["context_length"] = json!(0);
        let mut image = entry(
            "image/small",
            &["/v1/images/generations"],
            &["text"],
            &["image"],
        );
        image["context_length"] = json!(0);
        assert_eq!(
            ids(&v1(json!({"data": [small, video, image]}))),
            ["video/zero", "image/small"]
        );
    }

    #[test]
    fn every_row_of_the_real_catalogs_gets_its_kind() {
        let mut expected: Vec<(&str, Kind)> = vec![];
        let chat = |ids: &'static [&'static str]| ids.iter().map(|id| (*id, Kind::Chat));
        expected.extend(chat(TODAY_CHAT_IDS));
        expected.extend([
            ("google/gemini-3.1-flash-image", Kind::Image),
            ("openai/gpt-image-2", Kind::Image),
            ("volcengine/doubao-seedream-5.0-flash", Kind::Image),
            ("qwen/qwen-image-3.0", Kind::Image),
            ("microsoft/mai-image-2.5", Kind::Image),
            ("bytedance/seedance-2.5", Kind::Video),
            ("alibaba/wan-3.0", Kind::Video),
            ("google/gemini-2.5-flash-tts", Kind::Speech),
            ("elevenlabs/eleven_v4", Kind::Speech),
            ("openai/gpt-4o-mini-transcribe", Kind::Transcription),
            // openai/text-embedding-3-small and typesafe/jev-latest (decisions)
            // are dropped.
        ]);
        expected.extend(chat(CLAUDE_IDS));
        assert_eq!(kinds(&real_catalog().models), expected);
    }

    #[test]
    fn claude_comes_from_the_anthropic_catalog_and_runs_with_messages() {
        let models = parse_models(anthropic_fixture(), Source::Anthropic);
        // The DeepSeek and OpenAI rows of that catalog are left to `/v1`.
        assert_eq!(ids(&models), CLAUDE_IDS);
        let haiku = &models[0];
        assert_eq!(haiku.id, "anthropic/claude-haiku-5.5");
        assert_eq!(haiku.name, "Anthropic: Claude Haiku 5.5");
        assert_eq!(haiku.short_name, "Claude Haiku 5.5");
        assert_eq!(haiku.vendor, "anthropic");
        assert_eq!(haiku.kind, Kind::Chat);
        assert_eq!(haiku.protocol, Some(Protocol::AnthropicMessages));
        assert_eq!(haiku.supported_endpoints, ["/anthropic/v1/messages"]);
        assert!(haiku.reasoning && haiku.tools && haiku.supports_images);
        // 2026-10-09T04:54:25Z
        assert_eq!(haiku.created, Some(1_791_521_665));
        let conversation = real_catalog().conversation_models();
        for id in CLAUDE_IDS {
            let model = conversation.iter().find(|m| m.id == *id).unwrap();
            assert_eq!(model.protocol, Some(Protocol::AnthropicMessages), "{id}");
        }
    }

    #[test]
    fn other_vendors_never_run_with_messages_when_v1_is_down() {
        // The Anthropic catalog lists them too; `/v1` alone offers them.
        let only_anthropic = merge(
            Err("assistant_catalog_unavailable".into()),
            Ok(parse_models(anthropic_fixture(), Source::Anthropic)),
        )
        .unwrap();
        assert_eq!(ids(&only_anthropic.models), CLAUDE_IDS);
        assert_eq!(
            default_model_id(&only_anthropic.models, None).as_deref(),
            Some(CLAUDE_IDS[0])
        );
        for id in ["deepseek/deepseek-v4-flash-0423", "openai/gpt-5.4-mini"] {
            assert!(
                only_anthropic.clone().conversation_model(id).is_none(),
                "{id}"
            );
        }
        assert!(only_anthropic
            .conversation_model(CLAUDE_IDS[0])
            .is_some_and(|m| m.protocol == Some(Protocol::AnthropicMessages)));
    }

    #[test]
    fn an_id_both_catalogs_list_is_the_v1_row() {
        let both = ["/v1/chat/completions", "/v1/responses"];
        let listed = v1(json!({"data": [
            entry("anthropic/claude-both", &both, &["text"], &["text"]),
        ]}));
        let claude = parse_models(
            json!({"data": [
                {"id": "anthropic/claude-both", "display_name": "Anthropic: Claude Both",
                 "context_length": 200000, "pricing": {}},
                {"id": "anthropic/claude-only", "display_name": "Anthropic: Claude Only",
                 "context_length": 200000, "pricing": {}},
            ]}),
            Source::Anthropic,
        );
        let merged = merge(Ok(listed), Ok(claude)).unwrap();
        assert_eq!(
            ids(&merged.models),
            ["anthropic/claude-both", "anthropic/claude-only"]
        );
        assert_eq!(merged.models[0].supported_endpoints, both);
        assert_eq!(
            merged.models[0].name, "anthropic/claude-both",
            "the /v1 row"
        );

        let merged = real_catalog();
        let mut unique = ids(&merged.models);
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), merged.models.len(), "no id twice");
        for id in ["deepseek/deepseek-v4-flash-0423", "openai/gpt-5.4-mini"] {
            let model = merged.models.iter().find(|m| m.id == id).unwrap();
            assert_eq!(
                model.supported_endpoints,
                ["/v1/chat/completions", "/v1/responses"],
                "{id}"
            );
            assert_eq!(model.protocol, Some(Protocol::OpenAiCompletions), "{id}");
        }
        // The /v1 row's own timestamp, not the Anthropic `created_at`.
        let deepseek = merged
            .models
            .iter()
            .find(|m| m.id == "deepseek/deepseek-v4-flash-0423")
            .unwrap();
        assert_eq!(deepseek.created, Some(1_776_902_400));
        assert!(merged.partial.is_empty());
    }

    #[test]
    fn a_source_that_could_not_be_read_is_named_and_the_other_still_counts() {
        let rows = || Ok(v1(v1_fixture()));
        let claude = || Ok(parse_models(anthropic_fixture(), Source::Anthropic));
        let down = || Err::<Vec<Model>, _>("assistant_catalog_unavailable".to_string());

        let without_claude = merge(rows(), down()).unwrap();
        assert_eq!(without_claude.partial, [Source::Anthropic]);
        assert_eq!(
            ids(&without_claude
                .conversation_models()
                .into_iter()
                .filter(|m| m.kind == Kind::Chat)
                .collect::<Vec<_>>()),
            TODAY_CHAT_IDS
        );
        assert_eq!(
            without_claude
                .conversation_models()
                .iter()
                .filter(|m| m.kind == Kind::Image)
                .count(),
            5
        );

        let only_anthropic = merge(down(), claude()).unwrap();
        assert_eq!(only_anthropic.partial, [Source::V1]);
        assert_eq!(ids(&only_anthropic.models), CLAUDE_IDS);
        assert_eq!(
            serde_json::to_value(&only_anthropic.partial).unwrap(),
            json!(["v1"])
        );

        // Both down: the `/v1` error, as before the second catalog.
        assert_eq!(
            merge(Err("assistant_catalog_invalid".into()), down()).unwrap_err(),
            "assistant_catalog_invalid"
        );
        // Media alone is no catalog a conversation can use.
        let only_images = v1(json!({"data": [
            entry("image/model", &["/v1/images/generations"], &["text"], &["image"]),
        ]}));
        assert_eq!(
            merge(Ok(only_images), down()).unwrap_err(),
            "assistant_no_compatible_models"
        );
    }

    #[test]
    fn image_and_video_capabilities_come_from_the_catalog() {
        let merged = real_catalog();
        let find = |id: &str| merged.models.iter().find(|m| m.id == id).unwrap();
        assert_eq!(
            find("openai/gpt-image-2").image,
            Some(ImageCapabilities {
                generations: true,
                edits: true,
                n: true,
                sizes: vec![],
                qualities: vec![],
                per_image_price: None,
                token_pricing: Some(ImageTokenPricing {
                    input: Some("5".into()),
                    input_image: Some("8".into()),
                    output_image: "30".into(),
                }),
            })
        );
        assert_eq!(
            find("volcengine/doubao-seedream-5.0-flash").image,
            Some(ImageCapabilities {
                generations: true,
                edits: false,
                n: false,
                sizes: vec!["1K".into(), "1.5K".into(), "2K".into()],
                qualities: vec![],
                per_image_price: Some("0.02".into()),
                token_pricing: None,
            })
        );
        let gemini_image = find("google/gemini-3.1-flash-image");
        assert!(gemini_image.image.as_ref().is_some_and(|i| i.edits));
        assert_eq!(gemini_image.protocol, None, "never run as a chat model");
        let seedance = find("bytedance/seedance-2.5");
        assert_eq!(seedance.context_window, 0);
        assert_eq!(
            seedance.video,
            Some(VideoCapabilities {
                modes: vec!["t2v".into(), "i2v".into(), "v2v".into()],
                resolutions: vec!["480p".into(), "720p".into(), "1080p".into()],
                default_resolution: Some("720p".into()),
                min_duration: Some(4),
                max_duration: Some(30),
                supports_audio: true,
                aspect_ratios: ["21:9", "16:9", "4:3", "1:1", "3:4", "9:16", "adaptive"]
                    .map(String::from)
                    .to_vec(),
                from_price_per_second: Some("0.11".into()),
            })
        );
        assert!(seedance.image.is_none() && !seedance.tools);
        // Chat rows carry neither, and say so by leaving the fields out.
        let chat = serde_json::to_value(find("openai/gpt-6.1-sol")).unwrap();
        assert!(chat.get("image").is_none() && chat.get("video").is_none());
        assert_eq!(chat["kind"], "chat");
        assert_eq!(serde_json::to_value(seedance).unwrap()["kind"], "video");
    }

    #[test]
    fn reasoning_and_tools_follow_the_listed_parameters() {
        let with = |parameters: Option<&[&str]>| {
            let mut row = entry(
                "test/model",
                &["/v1/chat/completions"],
                &["text"],
                &["text"],
            );
            if let Some(parameters) = parameters {
                row["supported_parameters"] = json!(parameters);
            }
            let model = v1(json!({"data": [row]})).remove(0);
            (model.reasoning, model.tools)
        };
        assert_eq!(with(None), (false, true), "no list: tools assumed");
        assert_eq!(with(Some(&["tools", "reasoning"])), (true, true));
        assert_eq!(with(Some(&["temperature"])), (false, false));
    }

    #[test]
    fn a_leading_vendor_is_dropped_from_the_short_name_only() {
        for (name, short) in [
            ("OpenAI: GPT-6.1 Sol", "GPT-6.1 Sol"),
            (
                "Google: Nano Banana 2 (Gemini 3.1 Flash Image)",
                "Nano Banana 2 (Gemini 3.1 Flash Image)",
            ),
            ("Z.ai: GLM-5", "GLM-5"),
            ("GPT-5 Nano", "GPT-5 Nano"),
            ("Doubao Seed 2.1 Pro: Turbo", "Doubao Seed 2.1 Pro: Turbo"),
            ("Vendor: ", "Vendor: "),
        ] {
            assert_eq!(short_name(name), short, "{name}");
        }
        let gpt = real_catalog().models.remove(0);
        assert_eq!(
            (gpt.name.as_str(), gpt.short_name.as_str()),
            ("OpenAI: GPT-6.1 Sol", "GPT-6.1 Sol")
        );
    }

    #[test]
    fn claude_listed_by_v1_would_still_run_with_messages() {
        let both = ["/v1/chat/completions", "/v1/messages"];
        let models = v1(json!({"data": [
            entry("anthropic/claude-test", &both, &["text"], &["text"]),
            entry("other/model", &both, &["text"], &["text"]),
        ]}));
        assert_eq!(models[0].protocol, Some(Protocol::AnthropicMessages));
        assert_eq!(models[1].protocol, Some(Protocol::OpenAiCompletions));
        // The runtime picks the first protocol by itself; it is told only
        // when the catalog's choice differs.
        let runtime = |model: &Model| serde_json::to_value(model.for_runtime(false)).unwrap();
        assert_eq!(runtime(&models[0])["api"], "anthropic-messages");
        assert!(runtime(&models[1]).get("api").is_none());
    }

    #[test]
    fn a_model_saved_before_kinds_reads_as_a_chat_model() {
        let model: Model = serde_json::from_str(TODAY_RUNTIME_MODEL).unwrap();
        assert_eq!(model.kind, Kind::Chat);
        assert!(model.tools && !model.reasoning);
        assert_eq!(model.protocol, None);
        assert!(model.image.is_none() && model.video.is_none());
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

    #[test]
    fn the_default_is_never_a_media_model() {
        let mut media = v1(json!({"data": [
            entry("image/first", &["/v1/images/generations"], &["text"], &["image"]),
            entry("video/second", &["/v1/videos"], &["text"], &["video"]),
        ]}));
        assert_eq!(default_model_id(&media, Some("image/first")), None);
        media.extend(catalog(&["chat/third"]));
        for remembered in [None, Some("image/first"), Some("video/second")] {
            assert_eq!(
                default_model_id(&media, remembered).as_deref(),
                Some("chat/third"),
                "{remembered:?}"
            );
        }
        // A preferred id listed as a media model is passed over too.
        let mut preferred = entry(
            PREFERRED_MODEL_IDS[0],
            &["/v1/videos"],
            &["text"],
            &["video"],
        );
        preferred["context_length"] = json!(0);
        media.insert(0, v1(json!({"data": [preferred]})).remove(0));
        assert_eq!(
            default_model_id(&media, None).as_deref(),
            Some("chat/third")
        );
        assert_eq!(
            default_model_id(&real_catalog().models, None).as_deref(),
            Some(PREFERRED_MODEL_IDS[0])
        );
    }

    #[test]
    fn the_preferred_models_are_chat_models_of_the_real_catalog() {
        let merged = real_catalog();
        for id in PREFERRED_MODEL_IDS {
            let model = merged.models.iter().find(|m| m.id == *id).unwrap();
            assert_eq!(model.kind, Kind::Chat, "{id}");
        }
        assert_eq!(
            merged
                .models
                .iter()
                .find(|m| m.id == SEARCH_MODEL)
                .map(|m| m.kind),
            Some(Kind::Chat)
        );
    }

    async fn serve(path: &str, response: wiremock::ResponseTemplate) -> wiremock::MockServer {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    async fn fetch_from(server: &wiremock::MockServer) -> Result<Vec<Model>, String> {
        fetch_models_at(&format!("{}/v1/models", server.uri()), Source::V1).await
    }

    #[tokio::test]
    async fn fetch_returns_every_usable_row_with_its_kind() {
        let body = json!({"data": [
            entry("chat/model", &["/v1/chat/completions"], &["text"], &["text"]),
            entry("image/model", &["/v1/images/generations"], &["text"], &["image"]),
            entry("embed/model", &["/v1/embeddings"], &["text"], &["text"]),
        ]});
        let server = serve(
            "/v1/models",
            wiremock::ResponseTemplate::new(200).set_body_json(body),
        )
        .await;
        assert_eq!(
            kinds(&fetch_from(&server).await.unwrap()),
            [("chat/model", Kind::Chat), ("image/model", Kind::Image)]
        );
    }

    #[tokio::test]
    async fn fetch_reads_the_anthropic_catalog_as_messages_models() {
        assert_eq!(Source::V1.path(), "/v1/models");
        assert_eq!(Source::Anthropic.path(), "/anthropic/v1/models");
        let server = serve(
            "/anthropic/v1/models",
            wiremock::ResponseTemplate::new(200).set_body_json(anthropic_fixture()),
        )
        .await;
        let models = fetch_models_at(
            &format!("{}/anthropic/v1/models", server.uri()),
            Source::Anthropic,
        )
        .await
        .unwrap();
        assert_eq!(ids(&models), CLAUDE_IDS);
        assert!(models
            .iter()
            .all(|m| m.protocol == Some(Protocol::AnthropicMessages)));
    }

    #[tokio::test]
    async fn fetch_reports_each_failure_with_its_own_code() {
        use wiremock::ResponseTemplate;
        let only_embeddings = json!({"data": [
            entry("embed/model", &["/v1/embeddings"], &["text"], &["text"]),
        ]});
        // Media alone fails too, so it never replaces a copy with chat models.
        let only_media = json!({"data": [
            entry("image/model", &["/v1/images/generations"], &["text"], &["image"]),
            entry("video/model", &["/v1/videos"], &["text"], &["video"]),
        ]});
        for (response, code) in [
            (ResponseTemplate::new(503), "assistant_catalog_unavailable"),
            (
                ResponseTemplate::new(200).set_body_string("not json"),
                "assistant_catalog_invalid",
            ),
            (
                ResponseTemplate::new(200).set_body_json(only_embeddings),
                "assistant_no_compatible_models",
            ),
            (
                ResponseTemplate::new(200).set_body_json(only_media),
                "assistant_no_compatible_models",
            ),
        ] {
            let server = serve("/v1/models", response).await;
            assert_eq!(fetch_from(&server).await.unwrap_err(), code);
        }
    }

    #[tokio::test]
    async fn fetch_refuses_a_catalog_over_the_size_limit() {
        let body = vec![b' '; MAX_CATALOG_BYTES + 1];
        let server = serve(
            "/v1/models",
            wiremock::ResponseTemplate::new(200).set_body_bytes(body),
        )
        .await;
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
        let result = fetch_models_at(&format!("http://{address}/v1/models"), Source::V1).await;
        server.abort();
        assert_eq!(result.unwrap_err(), "assistant_catalog_too_large");
    }

    #[tokio::test]
    async fn fetch_never_builds_a_host_from_an_unknown_apex() {
        for source in [Source::V1, Source::Anthropic] {
            assert_eq!(
                fetch_models("evil.example", source).await.unwrap_err(),
                "assistant_unknown_apex"
            );
        }
    }

    /// Both live catalogs of both apexes, read and merged as the host does.
    /// Free and keyless, but online: run with
    /// `cargo test --lib assistant::catalog::tests::live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "reads the live catalogs"]
    async fn live_catalogs_of_both_apexes_merge() {
        for apex in ["ofox.ai", "ofox.io"] {
            let (rows, claude) = tokio::join!(
                fetch_models(apex, Source::V1),
                fetch_models(apex, Source::Anthropic)
            );
            let (v1_rows, anthropic_rows) = (
                rows.as_ref().map_or(0, Vec::len),
                claude.as_ref().map_or(0, Vec::len),
            );
            let merged = merge(rows, claude).unwrap();
            let count = |kind: Kind| merged.models.iter().filter(|m| m.kind == kind).count();
            println!(
                "{apex}: /v1 {v1_rows} usable, /anthropic {anthropic_rows}, merged {} \
                 (chat {}, image {}, video {}, speech {}, transcription {}), \
                 claude {}, partial {:?}, default {:?}",
                merged.models.len(),
                count(Kind::Chat),
                count(Kind::Image),
                count(Kind::Video),
                count(Kind::Speech),
                count(Kind::Transcription),
                merged
                    .conversation_models()
                    .iter()
                    .filter(|m| m.vendor == "anthropic")
                    .count(),
                merged.partial,
                default_model_id(&merged.models, None),
            );
            assert!(merged.partial.is_empty(), "{apex}");
            assert!(merged
                .conversation_models()
                .iter()
                .any(|m| m.protocol == Some(Protocol::AnthropicMessages)));
            // Every live chat model runs with the protocol the runtime picks
            // by itself: no run is told one.
            for model in merged.conversation_models() {
                assert!(model.for_runtime(false).api.is_none(), "{}", model.id);
            }
        }
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
        search_price(&v1(
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

    /// The chat models the assistant offered from [`v1_fixture`] before the
    /// catalog learned kinds. Every one must still be offered, in this order.
    const TODAY_CHAT_IDS: &[&str] = &[
        "openai/gpt-6.1-sol",
        "openai/gpt-6-sol",
        "openai/gpt-5.4-mini",
        "openai/gpt-5-nano",
        "openai/gpt-5.4-pro",
        "google/gemini-3.1-flash-lite",
        "deepseek/deepseek-v4-flash-0423",
        "qwen/qwen3.8-flash",
        "z-ai/glm-5",
        "x-ai/grok-4.7",
        "moonshotai/kimi-k3",
        "volcengine/doubao-seed-2.1-pro",
        "minimax/minimax-m3",
    ];

    #[test]
    fn todays_chat_models_of_the_real_catalog_are_pinned() {
        let offered: Vec<_> = real_catalog()
            .conversation_models()
            .into_iter()
            .filter(|m| m.kind == Kind::Chat)
            .collect();
        let offered = ids(&offered);
        let (today, added) = offered.split_at(TODAY_CHAT_IDS.len());
        assert_eq!(today, TODAY_CHAT_IDS);
        assert_eq!(added, CLAUDE_IDS, "Claude is added after them");
    }

    /// What the runtime received for one model before the catalog learned
    /// kinds, byte for byte.
    const TODAY_RUNTIME_MODEL: &str = concat!(
        r#"{"id":"openai/gpt-6.1-sol","name":"OpenAI: GPT-6.1 Sol","contextWindow":1050000,"#,
        r#""maxOutputTokens":128000,"supportsImages":true,"inputModalities":["text","image"],"#,
        r#""supportedEndpoints":["/v1/chat/completions","/v1/responses"],"#,
        r#""inputPricePerMillion":"2","outputPricePerMillion":"10","#,
        r#""pricing":{"prompt":"0.000002","completion":"0.00001","input_cache_read":"0.0000001","#,
        r#""input_cache_write":"0.0000025"},"#,
        r#""cost":{"input":2.0,"output":10.0,"cacheRead":0.1,"cacheWrite":2.5},"pricingKnown":true}"#,
    );

    #[test]
    fn todays_runtime_model_bytes_are_pinned() {
        let offered = real_catalog().conversation_models();
        let model = offered
            .iter()
            .find(|m| m.id == "openai/gpt-6.1-sol")
            .unwrap();
        assert_eq!(
            serde_json::to_string(&model.for_runtime(false)).unwrap(),
            TODAY_RUNTIME_MODEL
        );
        // No model of today's catalog hands the runtime a field it did not
        // get before.
        let today: Value = serde_json::from_str(TODAY_RUNTIME_MODEL).unwrap();
        let fields: Vec<&String> = today.as_object().unwrap().keys().collect();
        for model in offered
            .iter()
            .filter(|m| TODAY_CHAT_IDS.contains(&m.id.as_str()))
        {
            let runtime = serde_json::to_value(model.for_runtime(false)).unwrap();
            assert_eq!(
                runtime.as_object().unwrap().keys().collect::<Vec<_>>(),
                fields,
                "{}",
                model.id
            );
        }
    }

    fn levels(model: &Model) -> Vec<&str> {
        model.thinking_levels.iter().map(String::as_str).collect()
    }

    #[test]
    fn chat_models_that_reason_offer_thinking_levels() {
        let merged = real_catalog();
        let find = |id: &str| merged.models.iter().find(|m| m.id == id).unwrap();
        let haiku = find("anthropic/claude-haiku-5.5");
        assert_eq!(
            levels(haiku),
            ["off", "low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(
            haiku.anthropic_compat,
            Some(thinking::AnthropicCompat {
                force_adaptive_thinking: Some(true),
                supports_temperature: Some(false),
            })
        );
        assert_eq!(
            levels(find("openai/gpt-6.1-sol")),
            ["low", "medium", "high", "xhigh", "max"]
        );
        let sol = find("openai/gpt-6-sol");
        assert_eq!(levels(sol)[0], "off");
        assert_eq!(
            sol.thinking_level_map.as_ref().unwrap()["off"].as_deref(),
            Some("none")
        );
        assert!(sol.anthropic_compat.is_none());
        assert_eq!(
            levels(find("google/gemini-3.1-flash-lite")),
            ["minimal", "low", "medium", "high"]
        );
        // Over Chat Completions Pi sends nothing for an unmapped "off".
        assert_eq!(
            levels(find("minimax/minimax-m3")),
            ["minimal", "low", "medium", "high"]
        );
        // Pi has no entry for it: the plain three.
        let glm = find("z-ai/glm-5");
        assert_eq!(levels(glm), ["low", "medium", "high"]);
        assert!(glm.thinking_level_map.is_none());
        // No `reasoning` in the catalog, or not a chat model: no level.
        for id in [
            "deepseek/deepseek-v4-flash-0423",
            "openai/gpt-5-nano",
            "openai/gpt-image-2",
        ] {
            let model = find(id);
            assert!(model.thinking_levels.is_empty(), "{id}");
            assert!(model.thinking_level_map.is_none() && model.anthropic_compat.is_none());
        }
    }

    #[test]
    fn a_run_at_a_level_also_gets_reasoning_the_map_and_claudes_compat() {
        let merged = real_catalog();
        let find = |id: &str| merged.models.iter().find(|m| m.id == id).unwrap();
        let runtime =
            |model: &Model, thinks: bool| serde_json::to_value(model.for_runtime(thinks)).unwrap();
        let haiku = find("anthropic/claude-haiku-5.5");
        let plain = runtime(haiku, false);
        let thinking = runtime(haiku, true);
        let mut keys: Vec<&String> = thinking.as_object().unwrap().keys().collect();
        let added = keys.split_off(plain.as_object().unwrap().len());
        assert_eq!(
            keys,
            plain.as_object().unwrap().keys().collect::<Vec<_>>(),
            "the new fields come after today's"
        );
        assert_eq!(added, ["reasoning", "thinkingLevelMap", "anthropicCompat"]);
        assert_eq!(thinking["reasoning"], true);
        assert_eq!(thinking["thinkingLevelMap"]["minimal"], Value::Null);
        assert_eq!(
            thinking["anthropicCompat"],
            json!({"forceAdaptiveThinking": true, "supportsTemperature": false})
        );
        let sol = runtime(find("openai/gpt-6-sol"), true);
        assert_eq!(sol["thinkingLevelMap"]["off"], "none");
        assert!(sol.get("anthropicCompat").is_none());
        // A model Pi does not know runs its level unmapped.
        let glm = runtime(find("z-ai/glm-5"), true);
        assert_eq!(glm["reasoning"], true);
        assert!(glm.get("thinkingLevelMap").is_none());
    }

    #[test]
    fn a_model_without_tools_says_so_to_the_runtime() {
        let mut row = entry(
            "plain/chat",
            &["/v1/chat/completions"],
            &["text"],
            &["text"],
        );
        row["supported_parameters"] = json!(["max_tokens"]);
        let models = v1(json!({"data": [row]}));
        assert!(!models[0].tools);
        let runtime = serde_json::to_value(models[0].for_runtime(false)).unwrap();
        assert_eq!(runtime["tools"], false);
        assert!(runtime.get("reasoning").is_none());
    }

    #[test]
    fn todays_endpoints_of_the_protocol_fixture_are_pinned() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../assistant-runtime/test/fixtures/catalog-protocols.json"
        ))
        .unwrap();
        let merged = real_catalog();
        for pin in fixture["models"].as_array().unwrap() {
            let model = merged.models.iter().find(|m| m.id == pin["id"]).unwrap();
            assert_eq!(
                json!(model.supported_endpoints),
                pin["supportedEndpoints"],
                "{}",
                model.id
            );
            assert_eq!(json!(model.protocol), pin["api"], "{}", model.id);
        }
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
