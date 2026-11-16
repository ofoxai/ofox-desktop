//! Host-owned planned-PPT state. No provider calls, paths or credentials.
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::str::FromStr;

const CATALOG: &str =
    include_str!("../../../../assistant-runtime/skills/ofox-office-pptx/resources/catalog.json");
const INVALID: &str = "assistant_presentation_plan_invalid";
/// Page kinds a plan may name; the list the runtime schema and style mappings use.
const PAGE_KINDS: &[&str] = &[
    "cover",
    "chapter",
    "agenda",
    "statement",
    "quote",
    "list",
    "split-image",
    "cards",
    "stats",
    "comparison",
    "table",
    "chart-bar",
    "chart-line",
    "chart-pie",
    "process",
    "timeline",
    "cycle",
    "hierarchy",
    "diagram",
    "code",
    "team",
    "lesson",
    "pricing",
    "equation",
    "closing",
];
/// The most pictures one deck may generate, and the most it may place
/// (generated now, plus pictures the conversation already holds).
pub(super) const MAX_IMAGES: u8 = 8;
pub(super) const MAX_ASSETS: usize = 12;
const ASSET_INVALID: &str = "assistant_presentation_asset_invalid";
const BUDGET: &str = "assistant_presentation_image_budget";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualPolicy {
    pub mode: String,
    pub max_images: u8,
    pub max_cost_usd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_max_cost_usd: Option<String>,
}
impl Default for VisualPolicy {
    fn default() -> Self {
        Self {
            mode: "off".into(),
            max_images: 0,
            max_cost_usd: "0".into(),
            text_max_cost_usd: None,
        }
    }
}
impl VisualPolicy {
    pub fn validate(&self) -> Result<(), String> {
        self.text_cost_limit()?;
        // `auto`: up to max_images, the model's choice. `exact`: the user
        // asked for that many. Either way max_images caps what is generated.
        if !matches!(
            self.mode.as_str(),
            "off" | "auto" | "illustration" | "exact"
        ) || self.max_images > MAX_IMAGES
            || if self.max_cost_usd == "count-only" {
                self.max_images == 0 || self.mode == "off"
            } else {
                money(&self.max_cost_usd).is_none()
            }
        {
            return Err(BUDGET.into());
        }
        Ok(())
    }

    /// Whether this setting lets the deck include generated pictures at all:
    /// a valid policy that is on, with a count and money (or a count-only cap).
    pub fn allows_images(&self) -> bool {
        self.validate().is_ok()
            && self.mode != "off"
            && self.max_images > 0
            && (self.max_cost_usd == "count-only"
                || money(&self.max_cost_usd).is_some_and(|cost| cost > Decimal::ZERO))
    }

    pub(super) fn text_cost_limit(&self) -> Result<Option<f64>, String> {
        let Some(value) = &self.text_max_cost_usd else {
            return Ok(None);
        };
        let mut parts = value.split('.');
        let whole = parts.next().unwrap_or_default();
        let fraction = parts.next();
        if !matches!(whole, "0" | "1")
            || parts.next().is_some()
            || fraction.is_some_and(|digits| {
                digits.is_empty()
                    || digits.len() > 6
                    || !digits.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err("assistant_invalid_message".into());
        }
        let cost = money(value)
            .filter(|cost| *cost >= Decimal::new(1, 2) && *cost <= Decimal::ONE)
            .ok_or("assistant_invalid_message")?;
        cost.to_string()
            .parse::<f64>()
            .map(Some)
            .map_err(|_| "assistant_invalid_message".into())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct Frozen {
    pub plan_id: String,
    pub revision: u64,
    pub effective_style: String,
    pub decision_summary: String,
    pub catalog_revision: String,
    pub requirements: Value,
    pub slides: Vec<Value>,
    pub request_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
    /// The look every generated image of this deck shares.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_style: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct AssetRequest {
    pub asset_id: String,
    pub kind: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct AssetBinding {
    pub asset_id: String,
    pub attachment_id: String,
    pub mime_type: String,
    pub sha256: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}
#[derive(Clone, Debug)]
struct Reservation {
    asset_id: String,
    cost: Option<Decimal>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct State {
    policy: VisualPolicy,
    frozen: Option<Frozen>,
    assets: BTreeMap<String, AssetBinding>,
    reservations: BTreeMap<String, Reservation>,
    image_attempts: BTreeMap<String, u8>,
    carried_count: usize,
    carried_cost: Decimal,
    unknown_spend: bool,
}
impl State {
    pub fn new(policy: VisualPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }
    /// Whether the user's setting lets this deck generate images of its own.
    pub fn images_on(&self) -> bool {
        self.policy.allows_images()
    }
    /// A plan that asks for generated images while the setting is off could
    /// only fall back: refuse it with what to do instead, so the model mends
    /// the plan in the same reply rather than delivering a deck without the
    /// picture the user asked for.
    pub fn check_unfunded_images(&self, request: &Value) -> Result<(), String> {
        if self.images_on() {
            return Ok(());
        }
        let pages: Vec<String> = request["slides"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(_, slide)| slide["visual"]["kind"] == "generated-image")
            .map(|(index, _)| (index + 1).to_string())
            .collect();
        if pages.is_empty() {
            return Ok(());
        }
        Err(format!("{INVALID}: slide images are off, so page {} cannot plan a generated-image. If the user's message itself asks for that picture, first call generate_image without planId, presentationAssetId or revision, then plan the page as a user-image whose assetId is the new attachment id. To reuse a picture the conversation already holds, plan it as a user-image with its attachment id. Otherwise give the page a code-figure, native or no visual.", pages.join(", ")))
    }
    pub fn current(&self) -> Option<&Frozen> {
        self.frozen.as_ref()
    }
    pub fn effective_style(&self) -> Option<&str> {
        self.current().map(|plan| plan.effective_style.as_str())
    }
    /// Model-visible guidance only. Every image call still checks the current
    /// plan, quote and atomic reservation; this snapshot grants no capability.
    pub fn visual_budget_snapshot(&self) -> Value {
        let count_only = self.policy.max_cost_usd == "count-only";
        let remaining_images = usize::from(self.policy.max_images)
            .saturating_sub(self.carried_count.saturating_add(self.reservations.len()));
        let held_cost = self
            .reservations
            .values()
            .try_fold(self.carried_cost, |sum, item| sum.checked_add(item.cost?));
        let unknown = self.unknown_spend || (!count_only && held_cost.is_none());
        let remaining_cost = if count_only || unknown {
            None
        } else {
            money(&self.policy.max_cost_usd)
                .zip(held_cost)
                .and_then(|(limit, held)| limit.checked_sub(held))
                .map(|cost| cost.max(Decimal::ZERO))
        };
        let available_asset = self.required_assets().iter().any(|asset| {
            asset.kind == "generated-image"
                && !self.assets.contains_key(&asset.asset_id)
                && self
                    .image_attempts
                    .get(&asset.asset_id)
                    .copied()
                    .unwrap_or(0)
                    < 2
                && !self
                    .reservations
                    .values()
                    .any(|held| held.asset_id == asset.asset_id)
        });
        let reason = if self.policy.validate().is_err() {
            "invalid-policy"
        } else if self.policy.mode == "off" {
            "disabled"
        } else if self.frozen.is_none() {
            "no-plan"
        } else if unknown {
            "unknown-spend"
        } else if remaining_images == 0 {
            "image-limit"
        } else if !count_only && remaining_cost.is_none_or(|cost| cost <= Decimal::ZERO) {
            "budget-exhausted"
        } else if !available_asset {
            "no-generated-image-available"
        } else if count_only {
            "authorized-count-only"
        } else {
            "authorized-price-check-required"
        };
        json!({
            "mode":self.policy.mode,
            "maxImages":self.policy.max_images,
            "maxCostUsd":self.policy.max_cost_usd,
            "remainingImages":remaining_images,
            "remainingCostUsd":remaining_cost.map(|cost| cost.normalize().to_string()),
            "canAttempt":matches!(reason,"authorized-count-only"|"authorized-price-check-required"),
            "reason":reason,
            "scope":"freeze-snapshot-not-reservation"
        })
    }
    pub fn freeze(
        &mut self,
        request: &Value,
        manual: Option<&str>,
        python: bool,
    ) -> Result<Frozen, String> {
        self.policy.validate()?;
        fields(
            request,
            &[
                "requirements",
                "styleId",
                "decisionSummary",
                "slides",
                "catalogRevision",
                "candidateIds",
                "source",
                "imageStyle",
            ],
            &[
                "requirements",
                "styleId",
                "decisionSummary",
                "slides",
                "catalogRevision",
                "candidateIds",
            ],
        )?;
        if let Some(style) = request.get("imageStyle") {
            text(style, 300, false)?;
        }
        let requirements = &request["requirements"];
        validate_requirements(requirements)?;
        if let Some(source) = request.get("source") {
            validate_source(source, requirements, python)?;
        }
        validate_slides(&request["slides"], requirements, python)?;
        let style = text(&request["styleId"], 80, false)?;
        if !identifier(style) {
            return Err(INVALID.into());
        }
        let summary = text(&request["decisionSummary"], 1000, false)?;
        let revision = catalog_revision();
        if request["catalogRevision"].as_str() != Some(revision.as_str()) {
            return Err(INVALID.into());
        }
        let eligible = eligible_styles(requirements, manual, python)?;
        if !eligible.iter().any(|candidate| candidate == style)
            || request["candidateIds"] != json!(eligible)
        {
            return Err(INVALID.into());
        }
        let request_hash = format!("{:x}", Sha256::digest(canonical(request).as_bytes()));
        if let Some(frozen) = &self.frozen {
            return if frozen.request_hash == request_hash {
                Ok(frozen.clone())
            } else {
                Err(INVALID.into())
            };
        }
        let frozen = Frozen {
            plan_id: super::types::id(),
            revision: 1,
            effective_style: style.into(),
            decision_summary: summary.into(),
            catalog_revision: revision,
            requirements: requirements.clone(),
            slides: request["slides"].as_array().ok_or(INVALID)?.clone(),
            request_hash,
            source: request.get("source").cloned(),
            image_style: request["imageStyle"].as_str().map(str::to_owned),
        };
        self.frozen = Some(frozen.clone());
        Ok(frozen)
    }
    pub fn validate_document_plan(&self, plan_id: &str, revision: u64) -> Result<(), String> {
        if self
            .frozen
            .as_ref()
            .is_some_and(|plan| plan.plan_id == plan_id && plan.revision == revision)
        {
            Ok(())
        } else {
            Err(INVALID.into())
        }
    }
    pub fn required_assets(&self) -> Vec<AssetRequest> {
        let mut assets = BTreeMap::new();
        if let Some(plan) = &self.frozen {
            for slide in &plan.slides {
                if let Some(asset_id) = slide["visual"]["assetId"].as_str() {
                    assets
                        .entry(asset_id.to_owned())
                        .or_insert_with(|| AssetRequest {
                            asset_id: asset_id.into(),
                            kind: slide["visual"]["kind"].as_str().unwrap_or_default().into(),
                        });
                }
            }
        }
        assets.into_values().collect()
    }
    pub fn assets(&self) -> Vec<AssetBinding> {
        self.assets.values().cloned().collect()
    }
    pub fn carry_spend(&mut self, count: u8, cost: Option<&str>) {
        self.carried_count = self.carried_count.saturating_add(usize::from(count));
        match cost
            .and_then(money)
            .and_then(|cost| self.carried_cost.checked_add(cost))
        {
            Some(total) => self.carried_cost = total,
            None => self.unknown_spend = true,
        }
    }
    pub fn reserve_image(
        &mut self,
        plan_id: &str,
        revision: u64,
        asset_id: &str,
        count: usize,
        per_image_price: Option<&str>,
    ) -> Result<String, String> {
        self.validate_document_plan(plan_id, revision)?;
        self.policy.validate()?;
        if self.policy.mode == "off"
            || self.unknown_spend
            || count != 1
            || self.image_attempts.get(asset_id).copied().unwrap_or(0) >= 2
            || self.assets.contains_key(asset_id)
            || self
                .reservations
                .values()
                .any(|held| held.asset_id == asset_id)
            || !self
                .required_assets()
                .iter()
                .any(|asset| asset.asset_id == asset_id && asset.kind == "generated-image")
        {
            return Err(BUDGET.into());
        }
        // Count-only is a separate, explicitly chosen authorization. An
        // unknown quote remains None; it is never recorded as zero dollars.
        let count_only = self.policy.max_cost_usd == "count-only";
        let cost = per_image_price.and_then(money);
        if !count_only && cost.is_none() {
            return Err(BUDGET.into());
        }
        if self.carried_count.saturating_add(self.reservations.len())
            >= usize::from(self.policy.max_images)
        {
            return Err(BUDGET.into());
        }
        if !count_only {
            let held = self
                .reservations
                .values()
                .try_fold(self.carried_cost, |sum, item| sum.checked_add(item.cost?))
                .ok_or(BUDGET)?;
            let total = held.checked_add(cost.ok_or(BUDGET)?).ok_or(BUDGET)?;
            if total > money(&self.policy.max_cost_usd).ok_or(BUDGET)? {
                return Err(BUDGET.into());
            }
        }
        // A successful preflight can send one provider attempt. Known-unbilled
        // failures release budget, but cannot permit an unbounded retry loop.
        *self.image_attempts.entry(asset_id.into()).or_default() += 1;
        let token = super::types::id();
        self.reservations.insert(
            token.clone(),
            Reservation {
                asset_id: asset_id.into(),
                cost,
            },
        );
        Ok(token)
    }
    pub fn bind_asset(
        &mut self,
        token: &str,
        binding: AssetBinding,
    ) -> Result<AssetBinding, String> {
        let held = self.reservations.get(token).ok_or(ASSET_INVALID)?;
        if held.asset_id != binding.asset_id {
            return Err(ASSET_INVALID.into());
        }
        self.bind(binding)
    }
    pub fn bind_user_asset(&mut self, binding: AssetBinding) -> Result<AssetBinding, String> {
        if !self
            .required_assets()
            .iter()
            .any(|asset| asset.asset_id == binding.asset_id && asset.kind == "user-image")
        {
            return Err(ASSET_INVALID.into());
        }
        self.bind(binding)
    }
    fn bind(&mut self, binding: AssetBinding) -> Result<AssetBinding, String> {
        if !identifier(&binding.asset_id)
            || super::types::validate_id(&binding.attachment_id).is_err()
            || !matches!(binding.mime_type.as_str(), "image/png" | "image/jpeg")
            || binding.sha256.len() != 64
            || !binding
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || binding.size_bytes == 0
            || binding.size_bytes > 10 * 1024 * 1024
            || binding.width == Some(0)
            || binding.height == Some(0)
            || binding
                .width
                .zip(binding.height)
                .is_some_and(|(w, h)| u64::from(w) * u64::from(h) > 40_000_000)
        {
            return Err(ASSET_INVALID.into());
        }
        if let Some(previous) = self.assets.get(&binding.asset_id) {
            return if previous == &binding {
                Ok(previous.clone())
            } else {
                Err(ASSET_INVALID.into())
            };
        }
        let total: u64 = self.assets.values().map(|asset| asset.size_bytes).sum();
        if self.assets.len() >= 4 || total + binding.size_bytes > 20 * 1024 * 1024 {
            return Err(ASSET_INVALID.into());
        }
        self.assets
            .insert(binding.asset_id.clone(), binding.clone());
        Ok(binding)
    }
    pub fn release_image(&mut self, token: &str, known_unbilled: bool) {
        if known_unbilled
            && self
                .reservations
                .get(token)
                .is_some_and(|held| !self.assets.contains_key(&held.asset_id))
        {
            self.reservations.remove(token);
        }
    }
    pub fn image_guidance(&self, asset_id: &str) -> String {
        let Some(plan) = &self.frozen else {
            return String::new();
        };
        // Each page's purpose with the region the picture fills and the part
        // of the page its title and text need kept clear.
        let purposes: Vec<String> = plan
            .slides
            .iter()
            .filter(|slide| {
                slide["visual"]["assetId"] == asset_id
                    && slide["visual"]["kind"] == "generated-image"
            })
            .filter_map(|slide| {
                let visual = &slide["visual"];
                let purpose = visual["purpose"].as_str()?;
                Some(match (
                    visual["placement"].as_str(),
                    visual["aspect"].as_str(),
                    visual["textZone"].as_str(),
                ) {
                    // Only a background picture lies under the text; any other
                    // placement sits beside it and needs no quiet zone.
                    (Some("background"), Some(aspect), Some(zone)) => format!(
                        "{purpose} (a {aspect} background under the slide's text; keep the {zone} of the picture calm, low-contrast and free of detail)"
                    ),
                    (Some(placement), Some(aspect), Some(_)) => format!(
                        "{purpose} (fills a {aspect} {placement} region beside the text; compose for that ratio)"
                    ),
                    _ => purpose.to_owned(),
                })
            })
            .collect();
        if purposes.is_empty() {
            return String::new();
        }
        let catalog: Value = serde_json::from_str(CATALOG).unwrap_or(Value::Null);
        let style = catalog["styles"].as_array().and_then(|styles| {
            styles
                .iter()
                .find(|style| style["id"] == plan.effective_style)
        });
        let public_style=style.map(|style|json!({"description":style["description"],"typography":style["typography"],"colors":style["theme"]["colors"]})).unwrap_or_else(||json!({"description":"Ofox compatible navy","colors":{"primary":"1E3A5F","background":"FFFFFF","accent":"3498DB"}}));
        let look = plan
            .image_style
            .as_deref()
            .map(|style| format!(" Deck-wide image look, shared by every image: {style}."))
            .unwrap_or_default();
        format!("Presentation illustration guidance: {}.{} Page visual purpose: {}. Use the public palette and visual contrast for artwork only; keep slide text, formulas, charts and exact diagrams as native objects and draw no words, numbers or formulas in the image.",public_style,look,purposes.join("; "))
    }
}
fn money(value: &str) -> Option<Decimal> {
    if value.is_empty()
        || value.len() > 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return None;
    }
    Decimal::from_str(value)
        .ok()
        .filter(|cost| *cost >= Decimal::ZERO && *cost <= Decimal::from(999) && cost.scale() <= 6)
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
fn text(value: &Value, max: usize, empty: bool) -> Result<&str, String> {
    value
        .as_str()
        .filter(|value| {
            value.chars().count() <= max && (empty && value.is_empty() || !value.trim().is_empty())
        })
        .ok_or_else(|| INVALID.into())
}
fn fields(value: &Value, allowed: &[&str], required: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or(INVALID)?;
    if object.keys().any(|key| !allowed.contains(&key.as_str()))
        || required.iter().any(|key| !object.contains_key(*key))
    {
        return Err(INVALID.into());
    }
    Ok(())
}
fn one_of(value: &Value, choices: &[&str]) -> bool {
    value.as_str().is_some_and(|value| choices.contains(&value))
}
fn editing(value: &Value) -> Result<(), String> {
    let roles = ["text", "charts", "formulas", "diagrams"];
    fields(value, &roles, &roles)?;
    if roles.iter().any(|role| !value[role].is_boolean()) {
        return Err(INVALID.into());
    }
    Ok(())
}
fn validate_requirements(value: &Value) -> Result<(), String> {
    fields(
        value,
        &[
            "audience",
            "purpose",
            "scenario",
            "language",
            "slideCount",
            "aspectRatio",
            "density",
            "nativeEditing",
            "visualNeeds",
            "roundTrip",
            "durationMinutes",
            "contentMix",
            "brand",
        ],
        &[
            "audience",
            "purpose",
            "scenario",
            "language",
            "slideCount",
            "aspectRatio",
            "density",
            "nativeEditing",
            "visualNeeds",
            "roundTrip",
        ],
    )?;
    text(&value["audience"], 300, false)?;
    text(&value["purpose"], 300, false)?;
    text(&value["scenario"], 80, false)?;
    if !one_of(&value["language"], &["zh", "en", "ja"])
        || !one_of(&value["aspectRatio"], &["16:9", "4:3"])
        || !one_of(&value["density"], &["low", "medium", "high"])
        || !one_of(
            &value["roundTrip"],
            &["unverifiedAllowed", "verifiedRequired"],
        )
        || !value["slideCount"]
            .as_u64()
            .is_some_and(|count| (1..=30).contains(&count))
    {
        return Err(INVALID.into());
    }
    editing(&value["nativeEditing"])?;
    let visuals = value["visualNeeds"].as_array().ok_or(INVALID)?;
    if visuals.len() > 5
        || visuals
            .iter()
            .any(|visual| !one_of(visual, &["none", "photo", "concept", "data", "process"]))
    {
        return Err(INVALID.into());
    }
    if let Some(duration) = value
        .get("durationMinutes")
        .filter(|duration| !duration.is_null())
    {
        if !duration
            .as_u64()
            .is_some_and(|duration| (1..=240).contains(&duration))
        {
            return Err(INVALID.into());
        }
    }
    if let Some(mix) = value.get("contentMix").filter(|mix| !mix.is_null()) {
        fields(
            mix,
            &["data", "formulas", "images"],
            &["data", "formulas", "images"],
        )?;
        let mut sum = 0.0;
        for key in ["data", "formulas", "images"] {
            let proportion = mix[key]
                .as_f64()
                .filter(|number| (0.0..=1.0).contains(number))
                .ok_or(INVALID)?;
            sum += proportion;
        }
        if sum > 1.0 + 1e-9 {
            return Err(INVALID.into());
        }
    }
    if let Some(brand) = value.get("brand").filter(|brand| !brand.is_null()) {
        fields(
            brand,
            &["description", "colors", "fonts", "strict"],
            &["description", "colors", "fonts", "strict"],
        )?;
        text(&brand["description"], 300, true)?;
        if !brand["strict"].is_boolean() {
            return Err(INVALID.into());
        }
        let colors = brand["colors"].as_array().ok_or(INVALID)?;
        if colors.len() > 8
            || colors.iter().any(|color| {
                color.as_str().is_none_or(|color| {
                    let rgb = color.strip_prefix('#').unwrap_or(color);
                    rgb.len() != 6 || !rgb.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            })
        {
            return Err(INVALID.into());
        }
        let fonts = brand["fonts"].as_array().ok_or(INVALID)?;
        if fonts.len() > 4 || fonts.iter().any(|font| text(font, 100, false).is_err()) {
            return Err(INVALID.into());
        }
    }
    Ok(())
}
/// Catalog styles a user or the model may select, in catalog order.
pub(super) fn enabled_catalog_styles() -> Vec<String> {
    let catalog: Value = serde_json::from_str(CATALOG).unwrap_or(Value::Null);
    catalog["styles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|style| {
            style["status"] == "enabled"
                && style["enabled"] == true
                && style["adapted"] == true
                && style["licenseReviewed"] == true
                && style["theme"].is_object()
        })
        .filter_map(|style| style["id"].as_str().map(str::to_owned))
        .collect()
}
fn eligible_styles(
    requirements: &Value,
    manual: Option<&str>,
    python: bool,
) -> Result<Vec<String>, String> {
    let mut enabled = vec!["navy".to_owned()];
    enabled.extend(enabled_catalog_styles());
    let manual = manual.filter(|style| *style != "auto");
    if manual.is_some_and(|manual| !enabled.iter().any(|style| style == manual)) {
        return Err(INVALID.into());
    }
    let brand = &requirements["brand"];
    let strict_brand = brand["strict"] == true
        && (brand["description"]
            .as_str()
            .is_some_and(|description| !description.is_empty())
            || brand["colors"]
                .as_array()
                .is_some_and(|colors| !colors.is_empty())
            || brand["fonts"]
                .as_array()
                .is_some_and(|fonts| !fonts.is_empty()));
    if requirements["aspectRatio"] != "16:9"
        || requirements["roundTrip"] == "verifiedRequired"
        || (requirements["nativeEditing"]["formulas"] == true
            || requirements["nativeEditing"]["charts"] == true)
            && !python
        || strict_brand
    {
        return Ok(vec![]);
    }
    Ok(enabled
        .into_iter()
        .filter(|style| manual.is_none_or(|manual| style == manual))
        .collect())
}
fn validate_source(value: &Value, requirements: &Value, python: bool) -> Result<(), String> {
    if !python {
        return Err(INVALID.into());
    }
    fields(
        value,
        &["attachmentId", "version", "pageNumbers"],
        &["attachmentId", "version", "pageNumbers"],
    )?;
    if super::types::validate_id(text(&value["attachmentId"], 80, false)?).is_err() {
        return Err(INVALID.into());
    }
    let version = text(&value["version"], 64, false)?;
    if version.len() != 64
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(INVALID.into());
    }
    let pages = value["pageNumbers"]
        .as_array()
        .filter(|pages| !pages.is_empty() && pages.len() <= 30)
        .ok_or(INVALID)?;
    if Some(pages.len() as u64) != requirements["slideCount"].as_u64() {
        return Err(INVALID.into());
    }
    let mut unique = std::collections::BTreeSet::new();
    for page in pages {
        let page = page
            .as_u64()
            .filter(|page| (1..=1000).contains(page))
            .ok_or(INVALID)?;
        if !unique.insert(page) {
            return Err(INVALID.into());
        }
    }
    Ok(())
}
fn validate_slides(value: &Value, requirements: &Value, python: bool) -> Result<(), String> {
    let slides = value
        .as_array()
        .filter(|slides| !slides.is_empty() && slides.len() <= 30)
        .ok_or(INVALID)?;
    if Some(slides.len() as u64) != requirements["slideCount"].as_u64() {
        return Err(INVALID.into());
    }
    let mut ids = BTreeMap::new();
    let mut assets = BTreeMap::new();
    for slide in slides {
        fields(
            slide,
            &[
                "id",
                "claim",
                "evidence",
                "relationship",
                "composition",
                "editing",
                "visual",
                "pageKind",
                "continuity",
                "facts",
            ],
            &[
                "id",
                "claim",
                "evidence",
                "relationship",
                "composition",
                "editing",
                "visual",
            ],
        )?;
        let id = text(&slide["id"], 80, false)?;
        if !identifier(id) || ids.insert(id, ()).is_some() {
            return Err(INVALID.into());
        }
        text(&slide["claim"], 1000, false)?;
        text(&slide["composition"], 300, false)?;
        let evidence = slide["evidence"].as_array().ok_or(INVALID)?;
        if evidence.len() > 12 || evidence.iter().any(|item| text(item, 1000, false).is_err()) {
            return Err(INVALID.into());
        }
        if !one_of(
            &slide["relationship"],
            &[
                "sequence",
                "comparison",
                "hierarchy",
                "part-whole",
                "trend",
                "spatial",
                "single-claim",
            ],
        ) {
            return Err(INVALID.into());
        }
        if let Some(kind) = slide.get("pageKind") {
            if !one_of(kind, PAGE_KINDS) {
                return Err(INVALID.into());
            }
        }
        if let Some(continuity) = slide.get("continuity") {
            text(continuity, 200, true)?;
        }
        if let Some(facts) = slide.get("facts") {
            let facts = facts
                .as_array()
                .filter(|facts| facts.len() <= 12)
                .ok_or(INVALID)?;
            for fact in facts {
                fields(fact, &["text", "source"], &["text", "source"])?;
                text(&fact["text"], 300, false)?;
                if !one_of(
                    &fact["source"],
                    &["user", "attachment", "computed", "illustrative", "proposed"],
                ) {
                    return Err(INVALID.into());
                }
            }
        }
        editing(&slide["editing"])?;
        for role in ["text", "charts", "formulas", "diagrams"] {
            if requirements["nativeEditing"][role] == true && slide["editing"][role] != true {
                return Err(INVALID.into());
            }
        }
        let visual = &slide["visual"];
        fields(
            visual,
            &[
                "kind",
                "purpose",
                "fallback",
                "assetId",
                "placement",
                "textZone",
                "aspect",
            ],
            &["kind", "purpose", "fallback"],
        )?;
        // Image placement is all or nothing, for image visuals only.
        let placed = ["placement", "textZone", "aspect"]
            .iter()
            .filter(|key| visual.get(**key).is_some())
            .count();
        let image = one_of(&visual["kind"], &["user-image", "generated-image"]);
        if placed != 0
            && (placed != 3
                || !image
                || !one_of(
                    &visual["placement"],
                    &["side", "inset", "band", "background"],
                )
                || !one_of(
                    &visual["textZone"],
                    &["left", "right", "top", "bottom", "center", "none"],
                )
                || !one_of(&visual["aspect"], &["16:9", "4:3", "1:1", "3:4", "9:16"])
                || visual["placement"] == "background" && visual["textZone"] == "none")
        {
            return Err(INVALID.into());
        }
        if !one_of(
            &visual["kind"],
            &[
                "none",
                "native-chart",
                "native-diagram",
                "native-math",
                "code-figure",
                "user-image",
                "generated-image",
            ],
        ) || !one_of(&visual["fallback"], &["native-summary", "omit", "stop"])
            || one_of(
                &visual["kind"],
                &["native-math", "native-chart", "code-figure"],
            ) && !python
        {
            return Err(INVALID.into());
        }
        text(&visual["purpose"], 500, false)?;
        let image = one_of(&visual["kind"], &["user-image", "generated-image"]);
        if image != visual.get("assetId").is_some() {
            return Err(INVALID.into());
        }
        if image {
            let asset_id = text(&visual["assetId"], 80, false)?;
            if !identifier(asset_id) {
                return Err(INVALID.into());
            }
            let kind = visual["kind"].as_str().ok_or(INVALID)?;
            if assets
                .insert(asset_id, kind)
                .is_some_and(|previous| previous != kind)
            {
                return Err(INVALID.into());
            }
        }
    }
    if assets.len() > MAX_ASSETS
        || assets
            .values()
            .filter(|kind| **kind == "generated-image")
            .count()
            > usize::from(MAX_IMAGES)
    {
        return Err(INVALID.into());
    }
    // Three consecutive pages of one kind need the third page's stated reason.
    if slides.windows(3).any(|run| {
        run[2]["pageKind"].is_string()
            && run[0]["pageKind"] == run[2]["pageKind"]
            && run[1]["pageKind"] == run[2]["pageKind"]
            && run[2]["continuity"]
                .as_str()
                .is_none_or(|reason| reason.trim().is_empty())
    }) {
        return Err(INVALID.into());
    }
    Ok(())
}
fn catalog_revision() -> String {
    format!("{:x}", Sha256::digest(CATALOG.as_bytes()))
}
fn canonical(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            Value::Object(items) => {
                let ordered: BTreeMap<_, _> = items
                    .iter()
                    .map(|(key, value)| (key.clone(), sorted(value)))
                    .collect();
                Value::Object(ordered.into_iter().collect())
            }
            other => other.clone(),
        }
    }
    sorted(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_style_whitelist_follows_the_bundled_catalog() {
        use super::super::types::validate_presentation_style as validate;
        let enabled = enabled_catalog_styles();
        // The five first adaptations plus every later adapter.
        assert!(enabled.len() > 5, "{enabled:?}");
        for id in enabled.iter().map(String::as_str).chain(["auto", "navy"]) {
            assert!(validate(Some(id)).is_ok(), "{id}");
        }
        for id in ["no-such-style", "../catalog", "", "Studio"] {
            assert!(validate(Some(id)).is_err(), "{id}");
        }
        let catalog: Value = serde_json::from_str(CATALOG).unwrap();
        for style in catalog["styles"].as_array().unwrap() {
            if style["enabled"] != true {
                assert!(validate(style["id"].as_str()).is_err());
            }
        }
    }
    #[test]
    fn presentation_plan_page_kinds_image_placement_and_deck_image_look() {
        let freeze = |edit: &dyn Fn(&mut Value)| {
            let mut value = request();
            edit(&mut value);
            let mut state = State::new(policy());
            state.freeze(&value, None, true).map(|_| state)
        };
        // Placement, text zone and ratio reach the image guidance with the deck look.
        let state = freeze(&|value| {
            value["imageStyle"] = json!("Flat ink line drawings on warm paper");
            value["slides"][0]["pageKind"] = json!("cover");
            value["slides"][0]["facts"] =
                json!([{"text":"24 sample members","source":"illustrative"}]);
            let visual = &mut value["slides"][0]["visual"];
            visual["placement"] = json!("side");
            visual["textZone"] = json!("left");
            visual["aspect"] = json!("4:3");
        })
        .unwrap();
        let guidance = state.image_guidance("first-image");
        assert!(guidance.contains("Flat ink line drawings"), "{guidance}");
        assert!(guidance.contains("4:3 side region"), "{guidance}");
        // A side picture sits beside the text: no quiet zone is asked of it.
        assert!(!guidance.contains("calm"), "{guidance}");
        let background = freeze(&|value| {
            let visual = &mut value["slides"][0]["visual"];
            visual["placement"] = json!("background");
            visual["textZone"] = json!("left");
            visual["aspect"] = json!("16:9");
        })
        .unwrap()
        .image_guidance("first-image");
        assert!(
            background.contains("keep the left of the picture calm"),
            "{background}"
        );
        // Partial, contradictory or misplaced placement and unknown values are refused.
        for edit in [
            (&|value: &mut Value| value["slides"][0]["visual"]["placement"] = json!("side"))
                as &dyn Fn(&mut Value),
            &|value: &mut Value| {
                let visual = &mut value["slides"][0]["visual"];
                visual["placement"] = json!("background");
                visual["textZone"] = json!("none");
                visual["aspect"] = json!("16:9");
            },
            &|value: &mut Value| value["slides"][0]["pageKind"] = json!("poster"),
            &|value: &mut Value| {
                value["slides"][0]["facts"] = json!([{"text":"x","source":"guess"}])
            },
            &|value: &mut Value| value["imageStyle"] = json!(""),
        ] {
            assert!(freeze(edit).is_err());
        }
        // A third page of the same kind needs its stated reason.
        let three = |reason: Option<&str>| {
            let reason = reason.map(str::to_owned);
            freeze(&move |value: &mut Value| {
                let mut third = value["slides"][1].clone();
                third["id"] = json!("third");
                third["visual"] = json!({"kind":"none","purpose":"Text only","fallback":"omit"});
                if let Some(reason) = &reason {
                    third["continuity"] = json!(reason);
                }
                value["slides"].as_array_mut().unwrap().push(third);
                value["requirements"]["slideCount"] = json!(3);
                for slide in value["slides"].as_array_mut().unwrap() {
                    slide["pageKind"] = json!("cards");
                }
            })
        };
        assert!(three(None).is_err());
        assert!(three(Some("One series of three cases")).is_ok());
    }
    fn request() -> Value {
        json!({
            "catalogRevision":format!("{:x}",Sha256::digest(CATALOG.as_bytes())),
            "candidateIds":std::iter::once("navy".to_owned()).chain(enabled_catalog_styles()).collect::<Vec<_>>(),
            "styleId":"cartesian","decisionSummary":"Quiet research hierarchy; duration unknown",
            "requirements":{"audience":"Researchers","purpose":"Explain evidence","scenario":"academic","language":"zh","slideCount":2,"aspectRatio":"16:9","density":"medium","nativeEditing":{"text":true,"charts":false,"formulas":false,"diagrams":true},"visualNeeds":["concept"],"roundTrip":"unverifiedAllowed","durationMinutes":null,"contentMix":null,"brand":null},
            "slides":[
                {"id":"first","claim":"First claim","evidence":["Authored"],"relationship":"single-claim","composition":"Editorial","editing":{"text":true,"charts":false,"formulas":false,"diagrams":true},"visual":{"kind":"generated-image","purpose":"Illustrate the concept","fallback":"native-summary","assetId":"first-image"}},
                {"id":"second","claim":"Second claim","evidence":[],"relationship":"comparison","composition":"Two columns","editing":{"text":true,"charts":false,"formulas":false,"diagrams":true},"visual":{"kind":"generated-image","purpose":"Illustrate another concept","fallback":"omit","assetId":"second-image"}}
            ]
        })
    }
    fn policy() -> VisualPolicy {
        VisualPolicy {
            mode: "auto".into(),
            max_images: 2,
            max_cost_usd: "2".into(),
            text_max_cost_usd: None,
        }
    }
    fn binding(asset: &str) -> AssetBinding {
        AssetBinding {
            asset_id: asset.into(),
            attachment_id: "123e4567-e89b-42d3-a456-426614174000".into(),
            mime_type: "image/png".into(),
            sha256: "a".repeat(64),
            size_bytes: 80,
            width: Some(4),
            height: Some(3),
        }
    }
    #[test]
    fn a_page_may_plan_a_figure_drawn_in_code_on_the_python_route() {
        let mut figure = request();
        figure["slides"][1]["visual"] = json!({"kind":"code-figure","purpose":"Encoder and decoder stacks","fallback":"native-summary"});
        assert!(State::default().freeze(&figure, None, true).is_ok());
        // Drawn by the Python deck code: the deck-object route cannot draw one.
        assert!(State::default().freeze(&figure, None, false).is_err());
        // A drawing is not a paid image: it carries no asset.
        figure["slides"][1]["visual"]["assetId"] = json!("second-image");
        assert!(State::default().freeze(&figure, None, true).is_err());
    }
    #[test]
    fn a_policy_allows_images_only_with_a_count_and_money_to_spend() {
        assert!(policy().allows_images());
        assert!(!VisualPolicy::default().allows_images());
        for policy in [
            VisualPolicy {
                max_images: 0,
                ..policy()
            },
            VisualPolicy {
                max_cost_usd: "0".into(),
                ..policy()
            },
            VisualPolicy {
                mode: "off".into(),
                ..policy()
            },
        ] {
            assert!(!policy.allows_images());
        }
        assert!(VisualPolicy {
            max_cost_usd: "count-only".into(),
            ..policy()
        }
        .allows_images());
    }
    #[test]
    fn presentation_freeze_preserves_auto_choice_and_is_immutable() {
        let mut state = State::default();
        let plan = state.freeze(&request(), None, true).unwrap();
        assert_eq!(plan.effective_style, "cartesian");
        assert_eq!(state.freeze(&request(), None, true).unwrap(), plan);
        let mut other = request();
        other["decisionSummary"] = json!("Different");
        assert!(state.freeze(&other, None, true).is_err());
        assert_eq!(state.current(), Some(&plan));
        assert!(state
            .validate_document_plan(&plan.plan_id, plan.revision + 1)
            .is_err());
    }
    #[test]
    fn presentation_freeze_checks_catalog_manual_and_page_contract() {
        for change in [
            "catalog",
            "manual",
            "blank",
            "duplicate",
            "brand",
            "mix",
            "budget",
        ] {
            let mut value = request();
            let manual = if change == "manual" {
                Some("navy")
            } else {
                None
            };
            match change {
                "catalog" => value["catalogRevision"] = json!("forged"),
                "blank" => value["slides"][0]["claim"] = json!(" "),
                "duplicate" => value["slides"][1]["id"] = json!("first"),
                "brand" => {
                    value["requirements"]["brand"] = json!({"description":"Company","colors":["112233"],"fonts":[],"strict":true})
                }
                "mix" => {
                    value["requirements"]["contentMix"] =
                        json!({"data":0.8,"formulas":0.2,"images":0.3})
                }
                "budget" => value["budget"] = json!({"maxUsd":999}),
                _ => {}
            }
            let mut state = State::default();
            assert!(state.freeze(&value, manual, true).is_err(), "{change}");
            assert!(state.current().is_none());
        }
    }
    #[test]
    fn presentation_policy_is_explicit_bounded_and_not_a_plan_field() {
        assert_eq!(VisualPolicy::default().mode, "off");
        assert!(VisualPolicy::default().validate().is_ok());
        for cost in ["-1", "1000", "1.0000001", "1e2", "NaN"] {
            assert!(
                VisualPolicy {
                    max_cost_usd: cost.into(),
                    ..policy()
                }
                .validate()
                .is_err(),
                "{cost}"
            );
        }
        let mut state = State::default();
        let plan = state.freeze(&request(), None, true).unwrap();
        assert!(state
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, Some("0.01"))
            .is_err());
    }
    #[test]
    fn presentation_budget_snapshot_reports_authorization_without_reserving() {
        let mut state = State::new(VisualPolicy {
            mode: "auto".into(),
            max_images: 1,
            max_cost_usd: "0.03".into(),
            text_max_cost_usd: None,
        });
        let plan = state.freeze(&request(), None, true).unwrap();
        assert_eq!(state.visual_budget_snapshot()["canAttempt"], true);
        assert_eq!(state.visual_budget_snapshot()["remainingCostUsd"], "0.03");
        assert!(state.reservations.is_empty());
        assert!(state.image_attempts.is_empty());
        let token = state
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, Some("0.03"))
            .unwrap();
        let held = state.visual_budget_snapshot();
        assert_eq!(held["canAttempt"], false);
        assert_eq!(held["remainingImages"], 0);
        assert_eq!(held["remainingCostUsd"], "0");
        assert_eq!(held["reason"], "image-limit");
        state.release_image(&token, false);
        assert_eq!(state.visual_budget_snapshot(), held);
        state.release_image(&token, true);
        assert_eq!(state.visual_budget_snapshot()["canAttempt"], true);
        assert!(
            state
                .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, Some("0.04"))
                .is_err(),
            "a snapshot cannot bypass the live quote guard"
        );
    }
    #[test]
    fn presentation_budget_snapshot_keeps_carried_unknown_and_count_only_distinct() {
        let mut carried = State::new(policy());
        carried.carry_spend(1, Some("0.50"));
        carried.freeze(&request(), None, true).unwrap();
        let shown = carried.visual_budget_snapshot();
        assert_eq!(shown["remainingImages"], 1);
        assert_eq!(shown["remainingCostUsd"], "1.5");
        assert_eq!(shown["canAttempt"], true);

        let mut unknown = State::new(policy());
        unknown.carry_spend(1, None);
        unknown.freeze(&request(), None, true).unwrap();
        let shown = unknown.visual_budget_snapshot();
        assert!(shown["remainingCostUsd"].is_null());
        assert_eq!(shown["canAttempt"], false);
        assert_eq!(shown["reason"], "unknown-spend");

        let mut count_only = State::new(VisualPolicy {
            max_cost_usd: "count-only".into(),
            ..policy()
        });
        let plan = count_only.freeze(&request(), None, true).unwrap();
        count_only
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, None)
            .unwrap();
        let shown = count_only.visual_budget_snapshot();
        assert_eq!(shown["remainingImages"], 1);
        assert!(shown["remainingCostUsd"].is_null());
        assert_eq!(shown["canAttempt"], true);
        assert_eq!(shown["reason"], "authorized-count-only");
    }
    #[test]
    fn presentation_budget_snapshot_never_creates_authorization_or_asset_availability() {
        let mut off = State::default();
        off.freeze(&request(), None, true).unwrap();
        assert_eq!(off.visual_budget_snapshot()["reason"], "disabled");
        assert_eq!(off.visual_budget_snapshot()["canAttempt"], false);
        let mut state = State::new(policy());
        let mut without_image = request();
        for slide in without_image["slides"].as_array_mut().unwrap() {
            slide["visual"] = json!({"kind":"none","purpose":"Native text","fallback":"omit"});
        }
        state.freeze(&without_image, None, true).unwrap();
        assert_eq!(state.visual_budget_snapshot()["canAttempt"], false);
        assert_eq!(
            state.visual_budget_snapshot()["reason"],
            "no-generated-image-available"
        );
        let mut zero = State::new(VisualPolicy {
            max_cost_usd: "0".into(),
            ..policy()
        });
        zero.freeze(&request(), None, true).unwrap();
        assert_eq!(zero.visual_budget_snapshot()["canAttempt"], false);
        assert_eq!(zero.visual_budget_snapshot()["reason"], "budget-exhausted");
    }
    #[test]
    fn presentation_budget_reserves_unknown_outcomes_and_releases_only_known_unbilled() {
        let mut state = State::new(policy());
        let plan = state.freeze(&request(), None, true).unwrap();
        assert!(state
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, None)
            .is_err());
        let token = state
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, Some("1.25"))
            .unwrap();
        state.release_image(&token, false);
        assert!(state
            .reserve_image(
                &plan.plan_id,
                plan.revision,
                "second-image",
                1,
                Some("1.25")
            )
            .is_err());
        state.release_image(&token, true);
        assert!(state
            .reserve_image(
                &plan.plan_id,
                plan.revision,
                "second-image",
                1,
                Some("1.25")
            )
            .is_ok());
    }
    #[test]
    fn presentation_asset_binding_is_owned_immutable_and_uses_declared_ids() {
        let mut state = State::new(policy());
        let plan = state.freeze(&request(), None, true).unwrap();
        assert!(state
            .reserve_image(&plan.plan_id, plan.revision, "foreign", 1, Some("0.01"))
            .is_err());
        assert!(state
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 2, Some("0.01"))
            .is_err());
        let token = state
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, Some("0.01"))
            .unwrap();
        assert!(state.bind_asset(&token, binding("second-image")).is_err());
        let bound = state.bind_asset(&token, binding("first-image")).unwrap();
        assert_eq!(state.assets(), vec![bound]);
        assert_eq!(state.required_assets().len(), 2);
        let mut replaced = binding("first-image");
        replaced.sha256 = "b".repeat(64);
        assert!(state.bind_asset(&token, replaced).is_err());
    }
    #[test]
    fn presentation_rejects_mixed_asset_kinds_and_scopes_public_guidance() {
        let mut mixed = request();
        mixed["slides"][1]["visual"]["assetId"] = json!("first-image");
        mixed["slides"][1]["visual"]["kind"] = json!("user-image");
        assert!(State::default().freeze(&mixed, None, true).is_err());
        let mut state = State::default();
        state.freeze(&request(), None, true).unwrap();
        assert!(state.image_guidance("foreign").is_empty());
        let guidance = state.image_guidance("first-image");
        assert!(guidance.contains("Illustrate the concept"));
        assert!(guidance.contains("native objects"));
        assert!(!guidance.contains("another concept"));
    }
    #[test]
    fn presentation_carries_prior_budget_and_unknown_outcomes_across_runs() {
        let mut state = State::new(policy());
        state.carry_spend(1, Some("1.50"));
        let plan = state.freeze(&request(), None, true).unwrap();
        assert!(state
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, Some("0.51"))
            .is_err());
        assert!(state
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, Some("0.50"))
            .is_ok());
        let mut unknown = State::new(policy());
        unknown.carry_spend(1, None);
        let plan = unknown.freeze(&request(), None, true).unwrap();
        assert!(unknown
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, Some("0.01"))
            .is_err());
        let mut full = State::new(policy());
        full.carry_spend(2, Some("0"));
        let plan = full.freeze(&request(), None, true).unwrap();
        assert!(full
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, Some("0"))
            .is_err());
    }
    #[test]
    fn presentation_native_chart_requirements_need_the_python_engine() {
        let mut value = request();
        value["requirements"]["nativeEditing"]["charts"] = json!(true);
        for slide in value["slides"].as_array_mut().unwrap() {
            slide["editing"]["charts"] = json!(true);
        }
        assert!(State::default().freeze(&value, None, false).is_err());
        assert!(State::default().freeze(&value, None, true).is_ok());
    }
    #[test]
    fn with_images_off_a_plan_of_generated_images_is_sent_back_with_what_to_do() {
        let off = State::default();
        let refused = off.check_unfunded_images(&request()).unwrap_err();
        assert!(refused.starts_with(INVALID), "{refused}");
        assert!(refused.contains("page 1, 2") && refused.contains("user-image"));
        assert!(refused.contains("without planId"));
        // Pictures the conversation holds, drawings and nothing are all fine.
        let mut reused = request();
        reused["slides"][0]["visual"]["kind"] = json!("user-image");
        reused["slides"][1]["visual"] = json!({"kind":"none","purpose":"x","fallback":"omit"});
        assert_eq!(off.check_unfunded_images(&reused), Ok(()));
        // With images on, the budget decides.
        assert_eq!(
            State::new(policy()).check_unfunded_images(&request()),
            Ok(())
        );
    }
    #[test]
    fn a_deck_may_generate_eight_images_and_be_asked_for_an_exact_count() {
        let eight = VisualPolicy {
            mode: "exact".into(),
            max_images: 8,
            max_cost_usd: "3.20".into(),
            text_max_cost_usd: None,
        };
        assert!(eight.validate().is_ok() && eight.allows_images());
        assert!(VisualPolicy {
            max_images: 9,
            ..eight.clone()
        }
        .validate()
        .is_err());
        assert!(VisualPolicy {
            mode: "many".into(),
            ..eight
        }
        .validate()
        .is_err());
    }
    #[test]
    fn a_plan_places_up_to_twelve_pictures_of_which_eight_are_generated() {
        let plan = |generated: usize, existing: usize| {
            let mut value = request();
            let slide = value["slides"][0].clone();
            let slides: Vec<Value> = (0..generated + existing)
                .map(|index| {
                    let mut slide = slide.clone();
                    slide["id"] = json!(format!("p{index}"));
                    slide["visual"] = json!({
                        "kind": if index < generated { "generated-image" } else { "user-image" },
                        "purpose": "picture", "fallback": "omit",
                        "assetId": format!("asset-{index}"),
                    });
                    slide
                })
                .collect();
            value["requirements"]["slideCount"] = json!(slides.len());
            value["slides"] = json!(slides);
            State::default().freeze(&value, None, true).map(|_| ())
        };
        assert_eq!(plan(8, 4), Ok(()));
        assert!(plan(9, 0).is_err());
        assert!(plan(8, 5).is_err());
    }
    #[test]
    fn presentation_explicit_count_only_policy_allows_unknown_price_with_a_count_cap() {
        let count_only = VisualPolicy {
            mode: "auto".into(),
            max_images: 1,
            max_cost_usd: "count-only".into(),
            text_max_cost_usd: None,
        };
        assert!(count_only.validate().is_ok());
        let mut state = State::new(count_only);
        let plan = state.freeze(&request(), None, true).unwrap();
        let token = state
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, None)
            .unwrap();
        assert_eq!(state.reservations.get(&token).unwrap().cost, None);
        state.release_image(&token, false);
        assert!(state
            .reserve_image(&plan.plan_id, plan.revision, "second-image", 1, None)
            .is_err());
        state.release_image(&token, true);
        assert!(state
            .reserve_image(&plan.plan_id, plan.revision, "second-image", 1, None)
            .is_ok());
    }
    #[test]
    fn presentation_zero_usd_never_becomes_count_only_implicitly() {
        let invalid = VisualPolicy {
            max_cost_usd: "count-only".into(),
            ..VisualPolicy::default()
        };
        assert!(invalid.validate().is_err());
        let mut state = State::new(VisualPolicy {
            mode: "auto".into(),
            max_images: 1,
            max_cost_usd: "0".into(),
            text_max_cost_usd: None,
        });
        let plan = state.freeze(&request(), None, true).unwrap();
        assert!(state
            .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, None)
            .is_err());
    }
    #[test]
    fn presentation_source_freeze_keeps_version_and_actual_target_pages() {
        let mut value = request();
        value["requirements"]["slideCount"] = json!(1);
        value["slides"].as_array_mut().unwrap().truncate(1);
        value["source"] = json!({"attachmentId":"123e4567-e89b-42d3-a456-426614174000","version":"a".repeat(64),"pageNumbers":[70]});
        let plan = State::default().freeze(&value, None, true).unwrap();
        assert_eq!(
            serde_json::to_value(plan).unwrap()["source"],
            value["source"]
        );
        assert!(State::default().freeze(&value, None, false).is_err());
        value["source"]["pageNumbers"] = json!([70, 70]);
        assert!(State::default().freeze(&value, None, true).is_err());
    }
    #[test]
    fn presentation_off_policy_cannot_authorize_count_only() {
        let value = VisualPolicy {
            mode: "off".into(),
            max_images: 1,
            max_cost_usd: "count-only".into(),
            text_max_cost_usd: None,
        };
        assert!(value.validate().is_err());
    }
    #[test]
    fn presentation_asset_attempts_are_bounded_even_after_unbilled_release() {
        for policy in [
            policy(),
            VisualPolicy {
                mode: "auto".into(),
                max_images: 2,
                max_cost_usd: "count-only".into(),
                text_max_cost_usd: None,
            },
            VisualPolicy {
                mode: "auto".into(),
                max_images: 2,
                max_cost_usd: "0".into(),
                text_max_cost_usd: None,
            },
        ] {
            let count_only = policy.max_cost_usd == "count-only";
            let zero_usd = policy.max_cost_usd == "0";
            let mut state = State::new(policy);
            let plan = state.freeze(&request(), None, true).unwrap();
            // Failures before any provider dispatch must not use an attempt.
            for _ in 0..4 {
                assert!(state
                    .reserve_image(&plan.plan_id, plan.revision, "first-image", 2, Some("0.01"))
                    .is_err());
                if !count_only {
                    assert!(state
                        .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, None)
                        .is_err());
                    assert!(state
                        .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, Some("999"))
                        .is_err());
                }
            }
            let quote = if count_only {
                None
            } else if zero_usd {
                Some("0")
            } else {
                Some("0.01")
            };
            for _ in 0..2 {
                let token = state
                    .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, quote)
                    .unwrap();
                state.release_image(&token, true);
            }
            assert!(state
                .reserve_image(&plan.plan_id, plan.revision, "first-image", 1, quote)
                .is_err());
            assert!(state
                .reserve_image(&plan.plan_id, plan.revision, "second-image", 1, quote)
                .is_ok());
        }
    }
}
