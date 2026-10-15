//! Current OFox base-token estimates, independent of saved request amounts.
//! The old model_pricing table mixes seeds, custom edits and downloaded prices;
//! it is deliberately not used as evidence for this catalog.
use crate::{database::Database, error::AppError};
use rusqlite::{params, Connection, OptionalExtension};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const POLICY: &str = "current-ofox-base-v1";
const MAX_CATALOG_BYTES: usize = 4 * 1024 * 1024;
const FRESH_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostEstimate {
    pub known_cost_usd: Option<String>,
    pub availability: String,
    pub unpriced_model_ids: Vec<String>,
    pub unpriced_record_count: Option<u64>,
    pub additional_units_unsupported: bool,
}
impl Default for CostEstimate {
    fn default() -> Self {
        Self {
            known_cost_usd: None,
            availability: "unknown".into(),
            unpriced_model_ids: vec![],
            unpriced_record_count: Some(0),
            additional_units_unsupported: false,
        }
    }
}
impl CostEstimate {
    pub fn empty() -> Self {
        Self {
            known_cost_usd: Some("0".into()),
            availability: "known".into(),
            ..Self::default()
        }
    }
    pub fn combine(&mut self, other: &Self) -> Result<(), AppError> {
        let before_has_unknown = self.availability == "partial"
            || (self.availability == "unknown" && !self.unpriced_model_ids.is_empty());
        let other_has_unknown = other.availability == "partial"
            || (other.availability == "unknown" && !other.unpriced_model_ids.is_empty());
        let a = self
            .known_cost_usd
            .as_deref()
            .map(parse_amount)
            .transpose()?;
        let b = other
            .known_cost_usd
            .as_deref()
            .map(parse_amount)
            .transpose()?;
        let cost =
            match (a, b) {
                (Some(a), Some(b)) => Some(clean_decimal(a.checked_add(b).ok_or_else(|| {
                    AppError::Database("usage.pricing.aggregateOverflow".into())
                })?)),
                (Some(a), None) => Some(clean_decimal(a)),
                (None, Some(b)) => Some(clean_decimal(b)),
                _ => None,
            };
        let count = self
            .unpriced_record_count
            .zip(other.unpriced_record_count)
            .map(|(a, b)| {
                a.checked_add(b)
                    .ok_or_else(|| AppError::Database("usage.pricing.aggregateOverflow".into()))
            })
            .transpose()?;
        self.known_cost_usd = cost;
        self.unpriced_model_ids
            .extend(other.unpriced_model_ids.iter().cloned());
        self.unpriced_model_ids.sort();
        self.unpriced_model_ids.dedup();
        self.unpriced_record_count = count;
        self.additional_units_unsupported |= other.additional_units_unsupported;
        self.availability = match self.known_cost_usd.as_deref() {
            None => "unknown",
            Some(_) if before_has_unknown || other_has_unknown => "partial",
            Some("0") if !self.additional_units_unsupported => "free",
            Some("0") => "partial",
            _ => "known",
        }
        .into();
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingCoverage {
    pub policy: String,
    pub apex: String,
    pub catalog_version: Option<String>,
    pub catalog_updated_at_ms: Option<i64>,
    pub catalog_state: String,
    pub error_code: Option<String>,
    pub stale: bool,
    pub unpriced_model_ids: Vec<String>,
    pub unpriced_record_count: Option<u64>,
    pub additional_units_unsupported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelRate {
    rates: [Option<String>; 4],
    additional_units: BTreeMap<String, Option<String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogSnapshot {
    version: u8,
    policy: String,
    apex: String,
    content_hash: String,
    fetched_at_ms: i64,
    models: BTreeMap<String, ModelRate>,
    aliases: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone)]
pub struct PriceCatalog {
    snapshot: Option<CatalogSnapshot>,
    pub coverage: PricingCoverage,
}

pub fn clean_decimal(value: Decimal) -> String {
    value.normalize().to_string()
}

/// Exact decimal including scientific notation; unknown is never synthesized as 0.
pub fn parse_amount(value: &str) -> Result<Decimal, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::InvalidInput("usage.pricing.emptyRate".into()));
    }
    let invalid = || AppError::InvalidInput("usage.pricing.invalidAmount".into());
    if let Some((base, exponent)) = value.split_once(['e', 'E']) {
        // from_scientific internally uses the rounding parser for its mantissa.
        // Validate and normalize that mantissa with the exact parser first.
        let base = Decimal::from_str_exact(base).map_err(|_| invalid())?;
        Decimal::from_scientific(&format!("{}e{exponent}", base.normalize())).map_err(|_| invalid())
    } else {
        Decimal::from_str_exact(value).map_err(|_| invalid())
    }
}

pub fn parse_rate(value: &str) -> Result<Decimal, AppError> {
    let result = parse_amount(value)?;
    if result.is_sign_negative() {
        return Err(AppError::InvalidInput("usage.pricing.negativeRate".into()));
    }
    Ok(result)
}

fn key(apex: &str) -> String {
    format!("ofox_usage_pricing_catalog_v1:{apex}")
}

fn catalog_hash(
    apex: &str,
    models: &BTreeMap<String, ModelRate>,
    aliases: &BTreeMap<String, Option<String>>,
) -> Result<String, AppError> {
    let canonical = serde_json::to_vec(&(POLICY, apex, models, aliases))
        .map_err(|_| AppError::Database("usage.pricing.serializeCatalog".into()))?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

impl PriceCatalog {
    pub fn load_on(conn: &Connection, apex: &str, now_ms: i64) -> Result<Self, AppError> {
        if !crate::ofox_apex::is_known_apex(apex) {
            return Err(AppError::InvalidInput("usage.pricing.invalidApex".into()));
        }
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [key(apex)],
                |row| row.get(0),
            )
            .optional()?;
        let (snapshot, catalog_state, error_code) = match raw {
            None => (None, "missing", None),
            Some(raw) => {
                if raw.len() > MAX_CATALOG_BYTES {
                    (None, "invalid", Some("usage.pricing.catalogTooLarge"))
                } else {
                    match serde_json::from_str::<CatalogSnapshot>(&raw) {
                        Err(_) => (None, "invalid", Some("usage.pricing.invalidCatalog")),
                        Ok(s) if s.version != 1 || s.apex != apex || s.policy != POLICY => (
                            None,
                            "unsupported",
                            Some("usage.pricing.unsupportedCatalog"),
                        ),
                        Ok(s) if catalog_hash(apex, &s.models, &s.aliases)? != s.content_hash => {
                            (None, "invalid", Some("usage.pricing.contentMismatch"))
                        }
                        Ok(s) => (Some(s), "ready", None),
                    }
                }
            }
        };
        let coverage = PricingCoverage {
            policy: POLICY.into(),
            apex: apex.into(),
            catalog_version: snapshot.as_ref().map(|s| s.content_hash.clone()),
            catalog_updated_at_ms: snapshot.as_ref().map(|s| s.fetched_at_ms),
            catalog_state: catalog_state.into(),
            error_code: error_code.map(str::to_owned),
            stale: snapshot.as_ref().is_none_or(|s| {
                now_ms.saturating_sub(s.fetched_at_ms) >= FRESH_MS || s.fetched_at_ms > now_ms
            }),
            unpriced_model_ids: vec![],
            unpriced_record_count: Some(0),
            additional_units_unsupported: false,
        };
        Ok(Self { snapshot, coverage })
    }

    pub fn ingest_on(
        conn: &Connection,
        apex: &str,
        payload: &Value,
        fetched_at_ms: i64,
    ) -> Result<Self, AppError> {
        if !crate::ofox_apex::is_known_apex(apex) {
            return Err(AppError::InvalidInput("usage.pricing.invalidApex".into()));
        }
        let rows = payload
            .get("data")
            .and_then(Value::as_array)
            .filter(|rows| !rows.is_empty())
            .ok_or_else(|| AppError::InvalidInput("usage.pricing.emptyCatalog".into()))?;
        if rows.len() > 20_000 {
            return Err(AppError::InvalidInput(
                "usage.pricing.catalogTooLarge".into(),
            ));
        }
        let mut models = BTreeMap::new();
        let mut claims: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for row in rows {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 512)
                .ok_or_else(|| AppError::InvalidInput("usage.pricing.invalidModel".into()))?;
            if models.contains_key(id) {
                return Err(AppError::InvalidInput(
                    "usage.pricing.duplicateModel".into(),
                ));
            }
            let pricing = row.get("pricing").and_then(Value::as_object);
            let rate = |name: &str| {
                pricing
                    .and_then(|p| p.get(name))
                    .and_then(Value::as_str)
                    .and_then(|v| parse_rate(v).ok())
                    .and_then(|v| v.checked_mul(Decimal::from(1_000_000)))
                    .map(clean_decimal)
            };
            let write_name = if pricing.is_some_and(|p| p.contains_key("input_cache_write_5m")) {
                "input_cache_write_5m"
            } else {
                "input_cache_write"
            };
            let rates = [
                rate("prompt"),
                rate("completion"),
                rate("input_cache_read"),
                rate(write_name),
            ];
            let additional_units = pricing
                .into_iter()
                .flat_map(|p| p.iter())
                .filter_map(|(name, value)| {
                    if ["prompt", "completion", "input_cache_read", write_name]
                        .contains(&name.as_str())
                    {
                        return None;
                    }
                    let normalized = value
                        .as_str()
                        .and_then(|v| parse_rate(v).ok())
                        .map(clean_decimal);
                    Some((name.clone(), normalized))
                })
                .collect();
            models.insert(
                id.to_owned(),
                ModelRate {
                    rates,
                    additional_units,
                },
            );
            let mut keys = vec![id.to_owned()];
            if let Some((_, bare)) = id.split_once('/') {
                if !bare.is_empty() {
                    keys.push(bare.into());
                }
            }
            if let Some(aliases) = row.get("aliases").and_then(Value::as_array) {
                for alias in aliases {
                    if let Some(alias) = alias.as_str().filter(|s| !s.is_empty() && s.len() <= 512)
                    {
                        keys.push(alias.into());
                        if !alias.contains('/') {
                            if let Some((vendor, _)) = id.split_once('/') {
                                keys.push(format!("{vendor}/{alias}"));
                            }
                        }
                    }
                }
            }
            for alias in keys {
                claims.entry(alias).or_default().insert(id.to_owned());
            }
        }
        let aliases = claims
            .into_iter()
            .map(|(alias, owners)| {
                let canonical = if models.contains_key(&alias) {
                    Some(alias.clone())
                } else if owners.len() == 1 {
                    owners.into_iter().next()
                } else {
                    None
                };
                (alias, canonical)
            })
            .collect::<BTreeMap<_, _>>();
        let snapshot = CatalogSnapshot {
            version: 1,
            policy: POLICY.into(),
            apex: apex.into(),
            content_hash: catalog_hash(apex, &models, &aliases)?,
            fetched_at_ms,
            models,
            aliases,
        };
        let raw = serde_json::to_string(&snapshot)
            .map_err(|_| AppError::Database("usage.pricing.serializeCatalog".into()))?;
        if raw.len() > MAX_CATALOG_BYTES {
            return Err(AppError::InvalidInput(
                "usage.pricing.catalogTooLarge".into(),
            ));
        }
        conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key(apex), raw])?;
        Self::load_on(conn, apex, fetched_at_ms)
    }

    pub fn estimate(&self, model: &str, buckets: [u64; 4], records: Option<u64>) -> CostEstimate {
        let resolved = self.snapshot.as_ref().and_then(|s| {
            s.aliases
                .get(model)
                .and_then(Option::as_ref)
                .and_then(|id| s.models.get(id))
        });
        let mut known = false;
        let mut unknown = false;
        let mut amount = Decimal::ZERO;
        for (index, count) in buckets.into_iter().enumerate() {
            if count == 0 {
                continue;
            }
            match resolved
                .and_then(|rate| rate.rates[index].as_deref())
                .and_then(|v| parse_rate(v).ok())
                .and_then(|rate| rate.checked_mul(Decimal::from(count)))
                .and_then(|cost| cost.checked_div(Decimal::from(1_000_000)))
                .and_then(|cost| amount.checked_add(cost))
            {
                Some(total) => {
                    amount = total;
                    known = true;
                }
                None => unknown = true,
            }
        }
        if buckets == [0; 4] {
            known = resolved.is_some_and(|rate| rate.rates.iter().all(Option::is_some));
            unknown = !known;
        }
        let additional = resolved.is_some_and(|rate| {
            rate.additional_units.values().any(|value| {
                value
                    .as_deref()
                    .and_then(|v| parse_rate(v).ok())
                    .is_none_or(|v| !v.is_zero())
            })
        });
        let availability = if !known {
            "unknown"
        } else if unknown || (amount.is_zero() && additional) {
            "partial"
        } else if amount.is_zero() {
            "free"
        } else {
            "known"
        };
        CostEstimate {
            known_cost_usd: known.then(|| clean_decimal(amount)),
            availability: availability.into(),
            unpriced_model_ids: if unknown { vec![model.into()] } else { vec![] },
            unpriced_record_count: if unknown { records } else { Some(0) },
            additional_units_unsupported: additional,
        }
    }
    pub fn coverage_for(&self, estimate: &CostEstimate) -> PricingCoverage {
        PricingCoverage {
            unpriced_model_ids: estimate.unpriced_model_ids.clone(),
            unpriced_record_count: estimate.unpriced_record_count,
            additional_units_unsupported: estimate.additional_units_unsupported,
            ..self.coverage.clone()
        }
    }
}

/// One anonymous public catalog request; callers freeze apex before awaiting it.
pub async fn fetch_payload(apex: &str) -> Result<Value, AppError> {
    if !crate::ofox_apex::is_known_apex(apex) {
        return Err(AppError::InvalidInput("usage.pricing.invalidApex".into()));
    }
    let url = format!("https://api.{apex}/v1/models");
    let mut response = crate::proxy::http_client::get()
        .get(url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| AppError::Message("usage.pricing.fetchFailed".into()))?;
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "usage.pricing.http{}",
            response.status().as_u16()
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::Message("usage.pricing.fetchFailed".into()))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_CATALOG_BYTES {
            return Err(AppError::Message("usage.pricing.catalogTooLarge".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Message("usage.pricing.invalidCatalog".into()))
}

pub fn persist_payload(
    db: &Database,
    apex: &str,
    payload: &Value,
    at_ms: i64,
) -> Result<PriceCatalog, AppError> {
    let conn = crate::database::lock_conn!(db.conn);
    PriceCatalog::ingest_on(&conn, apex, payload, at_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload() -> Value {
        json!({"data":[
            {"id":"a/free","aliases":["free"],"pricing":{"prompt":"0","completion":"0"}},
            {"id":"a/small","aliases":null,"pricing":{"prompt":"1e-9","completion":"2e-9"}},
            {"id":"a/video","pricing":{"prompt":"0","completion":"0","output_video_per_second":"0.13"}},
            {"id":"a/shared","aliases":["ambiguous"],"pricing":{"prompt":"1e-6"}},
            {"id":"b/shared","aliases":["ambiguous"],"pricing":{"prompt":"2e-6"}}
        ]})
    }

    #[test]
    fn analytics_current_cost_overflow_never_discards_the_previous_sum() {
        let mut total = CostEstimate {
            known_cost_usd: Some(Decimal::MAX.to_string()),
            availability: "known".into(),
            ..CostEstimate::default()
        };
        let increment = CostEstimate {
            known_cost_usd: Some("1".into()),
            availability: "known".into(),
            ..CostEstimate::default()
        };
        assert!(total.combine(&increment).is_err());
        assert!(total.combine(&increment).is_err());
        assert_eq!(
            total.known_cost_usd.as_deref(),
            Some(Decimal::MAX.to_string().as_str()),
            "overflow dropped the earlier sum and a later combine could restart it"
        );
    }

    #[test]
    fn analytics_catalog_distinguishes_free_missing_partial_and_additional_units() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let catalog = PriceCatalog::ingest_on(&conn, "ofox.ai", &payload(), 10).unwrap();
        assert_eq!(
            catalog
                .estimate("free", [100, 10, 0, 0], Some(1))
                .availability,
            "free"
        );
        assert_eq!(
            catalog
                .estimate("missing", [100, 10, 0, 0], Some(1))
                .known_cost_usd,
            None
        );
        let partial = catalog.estimate("free", [100, 10, 5, 0], Some(1));
        assert_eq!(partial.availability, "partial");
        assert_eq!(partial.unpriced_record_count, Some(1));
        assert_eq!(
            catalog
                .estimate("a/small", [1, 0, 0, 0], Some(1))
                .known_cost_usd
                .as_deref(),
            Some("0.000000001")
        );
        assert_eq!(
            catalog
                .estimate("a/video", [1, 0, 0, 0], Some(1))
                .availability,
            "partial"
        );
        assert!(
            catalog
                .estimate("a/video", [1, 0, 0, 0], Some(1))
                .additional_units_unsupported
        );
        assert_eq!(
            catalog
                .estimate("ambiguous", [1, 0, 0, 0], Some(1))
                .availability,
            "unknown"
        );
        assert_eq!(
            catalog
                .estimate("shared", [1, 0, 0, 0], Some(1))
                .availability,
            "unknown"
        );
        assert_eq!(
            catalog
                .estimate("a/shared", [1, 0, 0, 0], Some(1))
                .availability,
            "known"
        );
    }

    #[test]
    fn analytics_rate_underflow_is_unknown_and_not_an_explicit_free_price() {
        assert!(parse_rate("0.00000000000000000000000000001").is_err());
        assert!(parse_rate("0.00000000000000000000000000001e0").is_err());
        assert!(parse_rate("1e-29").is_err());
        assert_eq!(parse_rate("1e-9").unwrap().to_string(), "0.000000001");
    }

    #[test]
    fn analytics_zero_token_record_does_not_make_a_missing_price_free() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let catalog = PriceCatalog::ingest_on(&conn, "ofox.ai", &serde_json::json!({"data":[
            {"id":"missing","pricing":{}},
            {"id":"free","pricing":{"prompt":"0","completion":"0","input_cache_read":"0","input_cache_write":"0"}}
        ]}), 10).unwrap();
        let unknown = catalog.estimate("missing", [0; 4], Some(1));
        assert_eq!(unknown.availability, "unknown");
        assert_eq!(unknown.known_cost_usd, None);
        assert_eq!(unknown.unpriced_record_count, Some(1));
        assert_eq!(
            catalog.estimate("free", [0; 4], Some(1)).availability,
            "free"
        );
    }

    #[test]
    fn analytics_catalog_unknown_cache_does_not_certify_free_or_reuse_a_forged_version() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings(key,value) VALUES(?1,'invalid JSON')",
            [key("ofox.ai")],
        )
        .unwrap();
        let missing = PriceCatalog::load_on(&conn, "ofox.ai", 20).unwrap();
        assert_eq!(
            missing.estimate("free", [1, 0, 0, 0], Some(1)).availability,
            "unknown"
        );
        assert_eq!(missing.coverage.catalog_version, None);
        let valid = PriceCatalog::ingest_on(&conn, "ofox.ai", &payload(), 20).unwrap();
        let mut saved: Value = serde_json::from_str(
            &conn
                .query_row::<String, _, _>(
                    "SELECT value FROM settings WHERE key=?1",
                    [key("ofox.ai")],
                    |r| r.get(0),
                )
                .unwrap(),
        )
        .unwrap();
        saved["models"]["a/free"]["rates"][0] = json!("99");
        conn.execute(
            "UPDATE settings SET value=?1 WHERE key=?2",
            params![saved.to_string(), key("ofox.ai")],
        )
        .unwrap();
        let corrupt = PriceCatalog::load_on(&conn, "ofox.ai", 30).unwrap();
        assert_eq!(corrupt.coverage.catalog_version, None);
        assert_eq!(
            corrupt.estimate("free", [1, 0, 0, 0], Some(1)).availability,
            "unknown"
        );
        assert!(valid.coverage.catalog_version.is_some());
    }
    #[test]
    fn analytics_catalog_version_does_not_depend_on_extra_field_order() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let one:Value=serde_json::from_str(r#"{"data":[{"id":"m","pricing":{"prompt":"1e-6","output_audio":"1","web_search":"2"}}]}"#).unwrap();
        let two:Value=serde_json::from_str(r#"{"data":[{"id":"m","pricing":{"web_search":"2","output_audio":"1","prompt":"1e-6"}}]}"#).unwrap();
        let a = PriceCatalog::ingest_on(&conn, "ofox.ai", &one, 1).unwrap();
        let b = PriceCatalog::ingest_on(&conn, "ofox.ai", &two, 2).unwrap();
        assert_eq!(a.coverage.catalog_version, b.coverage.catalog_version);
    }

    #[test]
    fn analytics_catalog_version_changes_when_an_unsupported_unit_price_changes() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut payload = serde_json::json!({"data":[{"id":"video","pricing":{"prompt":"0","completion":"0","output_video_per_second":"0.13"}}]});
        let a = PriceCatalog::ingest_on(&conn, "ofox.ai", &payload, 10).unwrap();
        payload["data"][0]["pricing"]["output_video_per_second"] = serde_json::json!("0.14");
        let b = PriceCatalog::ingest_on(&conn, "ofox.ai", &payload, 20).unwrap();
        assert_ne!(a.coverage.catalog_version, b.coverage.catalog_version);
        let estimate = b.estimate("video", [1, 0, 0, 0], Some(1));
        assert!(estimate.additional_units_unsupported);
        assert_eq!(estimate.availability, "partial");
    }

    #[test]
    fn analytics_catalog_versions_are_content_based_apex_scoped_and_leave_saved_amounts_alone() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,latency_ms,status_code,created_at,total_cost_usd) VALUES('saved','p','claude','free',0,200,1,'7.123456789')",[]).unwrap();
        let a = PriceCatalog::ingest_on(&conn, "ofox.ai", &payload(), 10).unwrap();
        let b = PriceCatalog::ingest_on(&conn, "ofox.ai", &payload(), 20).unwrap();
        assert_eq!(a.coverage.catalog_version, b.coverage.catalog_version);
        assert_ne!(
            a.coverage.catalog_updated_at_ms,
            b.coverage.catalog_updated_at_ms
        );
        let c = PriceCatalog::ingest_on(&conn, "ofox.io", &payload(), 20).unwrap();
        assert_ne!(b.coverage.catalog_version, c.coverage.catalog_version);
        let saved: String = conn
            .query_row("SELECT total_cost_usd FROM proxy_request_logs", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(saved, "7.123456789");
        assert!(PriceCatalog::ingest_on(&conn, "ofox.ai", &json!({"data":[]}), 30).is_err());
        assert_eq!(
            PriceCatalog::load_on(&conn, "ofox.ai", 30)
                .unwrap()
                .coverage
                .catalog_version,
            b.coverage.catalog_version
        );
    }
}
