//! Manufacturer API list prices, independent of Ofox availability and region.
//! Like magpie, the refresh reads models.dev's first-party provider entries;
//! relay, cloud-hosted and subscription-plan prices are never eligible.
use super::ofox_pricing::{clean_decimal, parse_rate, ContextTier, ModelRate};
use crate::{database::Database, error::AppError};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
    time::Duration,
};

const KEY: &str = "ofox_official_api_pricing_v1";
const SOURCE: &str = "https://models.dev/api.json";
const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
const MAX_SNAPSHOT: usize = 4 * 1024 * 1024;
const FRESH_MS: i64 = 24 * 60 * 60 * 1000;
const BUNDLED_AT_MS: i64 = 1_792_540_800_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OfficialCatalog {
    version: u8,
    source: String,
    pub content_hash: String,
    pub fetched_at_ms: i64,
    models: BTreeMap<String, ModelRate>,
    aliases: BTreeMap<String, Option<String>>,
}

fn invalid() -> AppError {
    AppError::InvalidInput("usage.pricing.invalidOfficialCatalog".into())
}

fn normalized(id: &str) -> String {
    id.to_ascii_lowercase().replace('.', "-")
}

// These are model manufacturers, not resellers or flat-rate coding plans.
fn manufacturers() -> [(&'static str, &'static str); 15] {
    [
        ("anthropic", "anthropic"),
        ("openai", "openai"),
        ("google", "google"),
        ("deepseek", "deepseek"),
        ("moonshotai", "moonshotai"),
        ("zai", "z-ai"),
        ("minimax", "minimax"),
        ("xai", "x-ai"),
        ("mistral", "mistralai"),
        ("alibaba", "qwen"),
        ("cohere", "cohere"),
        ("perplexity", "perplexity"),
        ("xiaomi", "xiaomi"),
        ("stepfun", "stepfun"),
        ("inception", "inception"),
    ]
}

fn rate(cost: &Value, name: &str) -> Result<Option<String>, AppError> {
    let Some(raw) = cost.get(name).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let text = match raw {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => return Err(invalid()),
    };
    parse_rate(&text)
        .map(clean_decimal)
        .map(Some)
        .map_err(|_| invalid())
}

fn rates(cost: &Value) -> Result<[Option<String>; 4], AppError> {
    Ok([
        rate(cost, "input")?,
        rate(cost, "output")?,
        rate(cost, "cache_read")?,
        rate(cost, "cache_write")?,
    ])
}

impl OfficialCatalog {
    pub(super) fn from_payload(payload: &Value, at_ms: i64) -> Result<Self, AppError> {
        let mut models = BTreeMap::new();
        let mut claims: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (provider, namespace) in manufacturers() {
            let Some(rows) = payload
                .get(provider)
                .and_then(|v| v.get("models"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            for (key, model) in rows {
                let id = model.get("id").and_then(Value::as_str).unwrap_or(key);
                if id.is_empty() || id.len() > 512 || id.chars().any(char::is_whitespace) {
                    continue;
                }
                let bare = if let Some((maker, bare)) = id.split_once('/') {
                    if maker != provider && maker != namespace {
                        continue;
                    }
                    bare
                } else {
                    id
                };
                let Some(cost) = model.get("cost").filter(|v| v.is_object()) else {
                    continue;
                };
                let base = rates(cost)?;
                if base[0].is_none() && base[1].is_none() {
                    continue;
                }
                let mut tiers = Vec::new();
                let listed_tiers = cost
                    .get("tiers")
                    .map(|tiers| tiers.as_array().ok_or_else(invalid))
                    .transpose()?;
                for tier in listed_tiers.into_iter().flatten() {
                    if tier.pointer("/tier/type").and_then(Value::as_str) != Some("context") {
                        return Err(invalid());
                    }
                    let Some(above_tokens) = tier
                        .pointer("/tier/size")
                        .and_then(Value::as_u64)
                        .filter(|n| *n > 0)
                    else {
                        return Err(invalid());
                    };
                    let tier_rates = rates(tier)?;
                    tiers.push(ContextTier {
                        above_tokens,
                        rates: std::array::from_fn(|i| {
                            tier_rates[i].clone().or_else(|| base[i].clone())
                        }),
                    });
                }
                tiers.sort_by_key(|tier| tier.above_tokens);
                if tiers
                    .windows(2)
                    .any(|pair| pair[0].above_tokens == pair[1].above_tokens)
                {
                    return Err(invalid());
                }
                let canonical = format!("{namespace}/{bare}");
                if models
                    .insert(
                        canonical.clone(),
                        ModelRate {
                            rates: base,
                            additional_units: cost
                                .as_object()
                                .into_iter()
                                .flat_map(|cost| cost.keys())
                                .filter(|name| {
                                    !["input", "output", "cache_read", "cache_write", "tiers"]
                                        .contains(&name.as_str())
                                })
                                // Other structured billing dimensions remain unsupported.
                                .map(|name| {
                                    let value = if name == "input_cache_write_1h" {
                                        rate(cost, name)?
                                    } else {
                                        rate(cost, name).unwrap_or(None)
                                    };
                                    Ok((name.clone(), value))
                                })
                                .collect::<Result<_, AppError>>()?,
                            context_tiers: tiers,
                        },
                    )
                    .is_some()
                {
                    return Err(invalid());
                }
                let mut aliases = vec![canonical.clone(), id.to_owned(), bare.to_owned()];
                if let Some(named) = model
                    .get("canonical_model_id")
                    .and_then(Value::as_str)
                    .filter(|id| id.len() <= 512)
                {
                    if let Some((maker, named_bare)) = named.split_once('/') {
                        if maker == namespace || maker == provider {
                            aliases.push(named.to_owned());
                            aliases.push(named_bare.to_owned());
                            aliases.push(format!("{namespace}/{named_bare}"));
                        }
                    }
                }
                for alias in aliases {
                    for alias in [alias.clone(), normalized(&alias)] {
                        claims.entry(alias).or_default().insert(canonical.clone());
                    }
                }
            }
        }
        if models.is_empty() {
            return Err(invalid());
        }
        let aliases = claims
            .into_iter()
            .map(|(alias, owners)| {
                let target = if models.contains_key(&alias) {
                    Some(alias.clone())
                } else if owners.len() == 1 {
                    owners.into_iter().next()
                } else {
                    None
                };
                (alias, target)
            })
            .collect();
        let mut catalog = Self {
            version: 1,
            source: SOURCE.into(),
            content_hash: String::new(),
            fetched_at_ms: at_ms,
            models,
            aliases,
        };
        catalog.content_hash = catalog.hash()?;
        Ok(catalog)
    }

    fn hash(&self) -> Result<String, AppError> {
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(self.version, &self.source, &self.models, &self.aliases))
                    .map_err(|_| invalid())?
            )
        ))
    }

    pub(super) fn resolve(&self, id: &str) -> Option<&ModelRate> {
        self.aliases
            .get(id)
            .or_else(|| self.aliases.get(&normalized(id)))
            .and_then(Option::as_ref)
            .and_then(|key| self.models.get(key))
    }

    pub(super) fn load_on(conn: &Connection) -> Result<Self, AppError> {
        let raw: Option<String> = conn
            .query_row("SELECT value FROM settings WHERE key=?1", [KEY], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(raw) = raw.filter(|raw| raw.len() <= MAX_SNAPSHOT) {
            if let Ok(snapshot) = serde_json::from_str::<Self>(&raw) {
                if snapshot.version == 1
                    && snapshot.source == SOURCE
                    && snapshot.fetched_at_ms >= BUNDLED_AT_MS
                    && snapshot.hash()? == snapshot.content_hash
                {
                    return snapshot.with_history(bundled());
                }
            }
        }
        Ok(bundled().clone())
    }

    fn with_history(mut self, previous: &Self) -> Result<Self, AppError> {
        for (id, rate) in &previous.models {
            self.models
                .entry(id.clone())
                .or_insert_with(|| rate.clone());
        }
        for (alias, target) in &previous.aliases {
            self.aliases
                .entry(alias.clone())
                .or_insert_with(|| target.clone());
        }
        self.content_hash = self.hash()?;
        Ok(self)
    }
}

fn bundled() -> &'static OfficialCatalog {
    static BUNDLED: OnceLock<OfficialCatalog> = OnceLock::new();
    BUNDLED.get_or_init(|| {
        let payload = serde_json::from_str(include_str!(
            "fixtures/manufacturer-api-prices-2026-10-21.json"
        ))
        .expect("bundled manufacturer catalog is valid JSON");
        OfficialCatalog::from_payload(&payload, BUNDLED_AT_MS)
            .expect("bundled manufacturer prices are valid")
    })
}

fn persist_on(conn: &Connection, payload: &Value, now: i64) -> Result<(), AppError> {
    // Validate before publication. Removed models keep their last verified API
    // rate so a manufacturer's retirement does not erase historical estimates.
    let next = OfficialCatalog::from_payload(payload, now)?;
    let previous = OfficialCatalog::load_on(conn)?;
    let next = next.with_history(&previous)?;
    let raw = serde_json::to_string(&next).map_err(|_| invalid())?;
    if raw.len() > MAX_SNAPSHOT {
        return Err(invalid());
    }
    conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![KEY, raw])?;
    Ok(())
}

pub(super) async fn sync_if_needed(db: &Database, force: bool) -> Result<(), AppError> {
    // Serialize downloads so an older response cannot replace newer prices.
    // Wall-clock ordering is unsafe after a system clock correction.
    static SYNC: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _flight = SYNC.lock().await;
    let now = chrono::Utc::now().timestamp_millis();
    if !force {
        let conn = crate::database::lock_conn!(db.conn);
        let current = OfficialCatalog::load_on(&conn)?;
        if (0..FRESH_MS).contains(&now.saturating_sub(current.fetched_at_ms)) {
            return Ok(());
        }
    }
    let mut response = crate::proxy::http_client::get()
        .get(SOURCE)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| invalid())?
        .error_for_status()
        .map_err(|_| invalid())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| invalid())? {
        if bytes.len().saturating_add(chunk.len()) > MAX_PAYLOAD {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk);
    }
    let payload: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let conn = crate::database::lock_conn!(db.conn);
    persist_on(&conn, &payload, chrono::Utc::now().timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn manufacturer_catalog_excludes_resellers_plans_and_cross_vendor_aliases() {
        let catalog = OfficialCatalog::from_payload(&json!({
            "anthropic":{"models":{"same":{"cost":{"input":4,"output":20},"canonical_model_id":"openai/gpt-suspicious"}}},
            "openai":{"models":{"same":{"cost":{"input":2,"output":10}}}},
            "openrouter":{"models":{"relay-only":{"cost":{"input":1,"output":1}}}},
            "zai-coding-plan":{"models":{"plan-only":{"cost":{"input":0,"output":0}}}}
        }), BUNDLED_AT_MS).unwrap();
        assert!(catalog.resolve("same").is_none());
        assert_eq!(
            catalog.resolve("anthropic/same").unwrap().rates[0].as_deref(),
            Some("4")
        );
        for model in ["relay-only", "plan-only", "openai/gpt-suspicious"] {
            assert!(catalog.resolve(model).is_none(), "{model}");
        }
    }

    #[test]
    fn manufacturer_cache_keeps_retired_prices_and_invalid_refresh_keeps_last_good_data() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let first = json!({"openai":{"models":{"test-official":{"cost":{"input":1,"output":2}}}}});
        persist_on(&conn, &first, BUNDLED_AT_MS + 10).unwrap();
        let before = OfficialCatalog::load_on(&conn).unwrap();
        assert!(persist_on(
            &conn,
            &json!({"openrouter":{"models":{}}}),
            BUNDLED_AT_MS + 20
        )
        .is_err());
        assert_eq!(
            OfficialCatalog::load_on(&conn).unwrap().content_hash,
            before.content_hash
        );
        for bad_cost in [
            json!({"input":-1,"output":2}),
            json!({"input":"not-a-rate","output":2}),
            json!({"input":1,"output":false}),
            json!({"input":1,"output":2,"input_cache_write_1h":{}}),
            json!({"input":1,"output":2,"tiers":{}}),
            json!({"input":1,"output":2,"tiers":[{"tier":{"type":"context","size":0},"input":3}]}),
            json!({"input":1,"output":2,"tiers":[{"tier":{"type":"unknown","size":1000},"input":3}]}),
        ] {
            assert!(persist_on(
                &conn,
                &json!({"openai":{"models":{"test-official":{"cost":bad_cost}}}}),
                BUNDLED_AT_MS + 20
            )
            .is_err());
            assert_eq!(
                OfficialCatalog::load_on(&conn).unwrap().content_hash,
                before.content_hash
            );
        }
        persist_on(
            &conn,
            &json!({"openai":{"models":{"new-official":{"cost":{"input":3,"output":4}}}}}),
            BUNDLED_AT_MS + 30,
        )
        .unwrap();
        let after = OfficialCatalog::load_on(&conn).unwrap();
        assert_eq!(
            after.resolve("test-official").unwrap().rates[0].as_deref(),
            Some("1")
        );
        assert!(after.resolve("claude-opus-5.5").is_some());
        assert_ne!(after.content_hash, before.content_hash);
        // A corrected system clock must not permanently block new prices.
        let changed =
            json!({"openai":{"models":{"test-official":{"cost":{"input":7,"output":8}}}}});
        persist_on(&conn, &changed, BUNDLED_AT_MS + 15).unwrap();
        assert_eq!(
            OfficialCatalog::load_on(&conn).unwrap().fetched_at_ms,
            BUNDLED_AT_MS + 15
        );
        assert_eq!(
            OfficialCatalog::load_on(&conn)
                .unwrap()
                .resolve("test-official")
                .unwrap()
                .rates[0]
                .as_deref(),
            Some("7")
        );
    }

    #[test]
    fn corrupt_official_cache_uses_the_verified_bundled_snapshot() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute("INSERT INTO settings(key,value) VALUES(?1,'broken')", [KEY])
            .unwrap();
        assert_eq!(
            OfficialCatalog::load_on(&conn)
                .unwrap()
                .resolve("anthropic/claude-opus-5.5")
                .unwrap()
                .rates[0]
                .as_deref(),
            Some("4")
        );
    }
}
