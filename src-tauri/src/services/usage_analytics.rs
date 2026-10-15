//! Filtered overview, calculated from one immutable set of live rows and daily rollups.
use super::{
    ofox_pricing::{CostEstimate, PricingCoverage},
    usage_data::{self, Atom, FilterOptions},
    usage_overview::{
        Granularity, ModelUsage, ToolUsage, UsageOverview, UsagePeriod, UsageSeriesPoint,
        UsageTotals,
    },
    usage_query::{Coverage, QueryReceipt, SourceHealth, UsageQueryV1},
};
use crate::{database::Database, error::AppError};
use chrono::{Datelike, TimeZone};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewAnalytics {
    #[serde(flatten)]
    pub receipt: QueryReceipt,
    pub coverage: Coverage,
    pub pricing_coverage: PricingCoverage,
    pub options: FilterOptions,
    pub sources: Vec<SourceHealth>,
}

#[derive(Default)]
pub(crate) struct Sum {
    requests: u64,
    tokens: [u64; 4],
    saved_cost: Decimal,
    estimate: CostEstimate,
}
fn overflow() -> AppError {
    AppError::Database("usage.query.aggregateOverflow".into())
}
impl Sum {
    pub(crate) fn add(
        &mut self,
        atom: &Atom,
        catalog: &super::ofox_pricing::PriceCatalog,
    ) -> Result<(), AppError> {
        self.requests = self
            .requests
            .checked_add(atom.requests)
            .ok_or_else(overflow)?;
        for (sum, value) in self.tokens.iter_mut().zip(atom.tokens) {
            *sum = sum.checked_add(value).ok_or_else(overflow)?;
        }
        self.saved_cost = self
            .saved_cost
            .checked_add(atom.saved_cost)
            .ok_or_else(overflow)?;
        self.estimate.combine(&catalog.estimate(
            &atom.model,
            atom.tokens,
            (!atom.archived).then_some(atom.requests),
        ));
        Ok(())
    }
    pub(crate) fn totals(&self) -> Result<UsageTotals, AppError> {
        let total_tokens = self.tokens[0]
            .checked_add(self.tokens[1])
            .ok_or_else(overflow)?;
        let prompt = self.tokens[0]
            .checked_add(self.tokens[2])
            .and_then(|n| n.checked_add(self.tokens[3]))
            .ok_or_else(overflow)?;
        Ok(UsageTotals {
            requests: self.requests,
            input_tokens: self.tokens[0],
            output_tokens: self.tokens[1],
            cache_read_tokens: self.tokens[2],
            cache_write_tokens: self.tokens[3],
            total_tokens,
            cache_hit_rate: if prompt == 0 {
                0.0
            } else {
                self.tokens[2] as f64 / prompt as f64
            },
            cost_usd: self
                .saved_cost
                .to_string()
                .parse()
                .map_err(|_| overflow())?,
            estimate: Some(if self.requests == 0 {
                CostEstimate::empty()
            } else {
                self.estimate.clone()
            }),
        })
    }
}

fn boundary<Tz: TimeZone>(tz: &Tz, date: chrono::NaiveDate) -> Result<i64, AppError> {
    let mut at = date.and_hms_opt(0, 0, 0).ok_or_else(overflow)?;
    for _ in 0..48 {
        match tz.from_local_datetime(&at) {
            chrono::LocalResult::Single(v) => return Ok(v.timestamp_millis()),
            chrono::LocalResult::Ambiguous(a, b) => {
                return Ok(a.timestamp_millis().min(b.timestamp_millis()))
            }
            chrono::LocalResult::None => at += chrono::Duration::minutes(30),
        }
    }
    Err(overflow())
}

pub fn build_query_overview<Tz: TimeZone>(
    db: &Database,
    period: UsagePeriod,
    query: UsageQueryV1,
    apex: &str,
    now_ms: i64,
    tz: &Tz,
    last_synced_at: Option<i64>,
) -> Result<UsageOverview, AppError> {
    let data = usage_data::read_dataset(db, query, None, apex, now_ms, tz)?;
    let first = data
        .query
        .start_ms
        .or_else(|| data.atoms.iter().map(|a| a.at_ms).min())
        .unwrap_or(data.query.end_ms);
    let span = data.query.end_ms.saturating_sub(first);
    let granularity = if span <= 36 * 3_600_000 {
        Granularity::Hour
    } else if span > 180 * 86_400_000 {
        Granularity::Week
    } else {
        Granularity::Day
    };
    let day_of = |at| {
        tz.timestamp_millis_opt(at)
            .single()
            .map(|v| v.date_naive())
            .ok_or_else(overflow)
    };
    let bucket = |at: i64| -> Result<i64, AppError> {
        match granularity {
            Granularity::Hour => {
                let start = boundary(tz, day_of(at)?)?;
                Ok(start + (at - start).div_euclid(3_600_000) * 3_600_000)
            }
            Granularity::Day => boundary(tz, day_of(at)?),
            Granularity::Week => {
                let day = day_of(at)?;
                boundary(
                    tz,
                    day - chrono::Duration::days(day.weekday().num_days_from_monday() as i64),
                )
            }
        }
    };
    let mut series: BTreeMap<i64, Sum> = BTreeMap::new();
    let mut at = bucket(first)?;
    while at < data.query.end_ms {
        if series.len() >= 10_000 {
            return Err(AppError::InvalidInput("usage.query.tooManyBuckets".into()));
        }
        series.insert(at, Sum::default());
        at = match granularity {
            Granularity::Hour => at.checked_add(3_600_000).ok_or_else(overflow)?,
            Granularity::Day => boundary(tz, day_of(at)?.succ_opt().ok_or_else(overflow)?)?,
            Granularity::Week => boundary(tz, day_of(at)? + chrono::Duration::days(7))?,
        };
    }
    let mut total = Sum::default();
    let mut tools: BTreeMap<String, Sum> = BTreeMap::new();
    let mut models: BTreeMap<String, (Sum, BTreeMap<String, u64>)> = BTreeMap::new();
    for atom in &data.atoms {
        total.add(atom, &data.catalog)?;
        tools
            .entry(atom.tool.clone())
            .or_default()
            .add(atom, &data.catalog)?;
        let (sum, apps) = models.entry(atom.model.clone()).or_default();
        sum.add(atom, &data.catalog)?;
        let weight = apps.entry(atom.tool.clone()).or_default();
        *weight = weight
            .checked_add(atom.tokens[0])
            .and_then(|v| v.checked_add(atom.tokens[1]))
            .ok_or_else(overflow)?;
        series
            .entry(bucket(atom.at_ms)?)
            .or_default()
            .add(atom, &data.catalog)?;
    }
    let totals = total.totals()?;
    let mut by_tool = tools
        .into_iter()
        .map(|(app_type, sum)| {
            Ok(ToolUsage {
                app_type,
                totals: sum.totals()?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    by_tool.sort_by(|a, b| {
        b.totals
            .total_tokens
            .cmp(&a.totals.total_tokens)
            .then_with(|| a.app_type.cmp(&b.app_type))
    });
    let mut by_model = models
        .into_iter()
        .map(|(model, (sum, apps))| {
            let mut apps = apps.into_iter().collect::<Vec<_>>();
            apps.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let totals = sum.totals()?;
            Ok(ModelUsage {
                model,
                app_types: apps.into_iter().map(|(a, _)| a).collect(),
                priced: totals
                    .estimate
                    .as_ref()
                    .is_some_and(|e| e.known_cost_usd.is_some()),
                totals,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    by_model.sort_by(|a, b| {
        b.totals
            .total_tokens
            .cmp(&a.totals.total_tokens)
            .then_with(|| a.model.cmp(&b.model))
    });
    let series = series
        .into_iter()
        .map(|(start, sum)| {
            let totals = sum.totals()?;
            Ok(UsageSeriesPoint {
                start,
                requests: totals.requests,
                input_tokens: totals.input_tokens,
                output_tokens: totals.output_tokens,
                cache_read_tokens: totals.cache_read_tokens,
                cache_write_tokens: totals.cache_write_tokens,
                cost_usd: totals.cost_usd,
                estimate: totals.estimate,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let analytics = OverviewAnalytics {
        receipt: QueryReceipt {
            query_fingerprint: data.query.fingerprint()?,
            applied_query: data.query.clone(),
            snapshot_version: data.snapshot_version,
        },
        coverage: data.coverage,
        pricing_coverage: data
            .catalog
            .coverage_for(totals.estimate.as_ref().ok_or_else(overflow)?),
        options: data.options,
        sources: data.sources,
    };
    Ok(UsageOverview {
        period,
        start: data.query.start_ms,
        end: data.query.end_ms,
        granularity,
        totals,
        series,
        by_tool,
        by_model,
        last_synced_at,
        unsupported_tools: super::usage_query::source_capabilities()
            .into_iter()
            .filter(|c| !c.usage_supported)
            .map(|c| c.tool_id)
            .collect(),
        analytics: Some(analytics),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use serde_json::json;
    fn query() -> UsageQueryV1 {
        UsageQueryV1 {
            version: 1,
            start_ms: Some(1_760_054_400_000),
            end_ms: 1_760_227_200_000,
            tool_ids: vec!["claude".into()],
            model_ids: vec!["free".into()],
            project_ids: vec![],
            source_ids: vec![],
        }
    }
    #[test]
    fn analytics_query_unions_only_live_and_rollups_and_echoes_normalized_query() {
        let db = Database::memory().unwrap();
        let q = query();
        {
            let conn = db.conn.lock().unwrap();
            super::super::ofox_pricing::PriceCatalog::ingest_on(
                &conn,
                "ofox.ai",
                &json!({"data":[{"id":"free","pricing":{"prompt":"0","completion":"0"}}]}),
                q.end_ms,
            )
            .unwrap();
            for (id, tool, model, at) in [
                ("a", "claude", "free", q.start_ms.unwrap() / 1000),
                ("b", "claude", "free", q.end_ms / 1000),
                ("c", "codex", "free", q.start_ms.unwrap() / 1000),
                ("d", "claude", "free-extra", q.start_ms.unwrap() / 1000),
            ] {
                conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,created_at,total_cost_usd) VALUES(?1,'p',?2,?3,10,1,0,200,?4,'7')",params![id,tool,model,at]).unwrap();
            }
            let date = chrono::DateTime::from_timestamp_millis(q.start_ms.unwrap())
                .unwrap()
                .date_naive()
                .to_string();
            conn.execute("INSERT INTO usage_daily_rollups(date,app_type,provider_id,model,request_count,input_tokens,output_tokens,total_cost_usd) VALUES(?1,'claude','p','free',2,20,2,'9')",[date]).unwrap();
            conn.execute("INSERT INTO session_usage_archive(app_type,session_id,model,input_tokens,output_tokens,request_count,cost_usd) VALUES('claude','s','free',9999,9999,999,'999')",[]).unwrap();
        }
        let result = build_query_overview(
            &db,
            UsagePeriod::All,
            q.clone(),
            "ofox.ai",
            q.end_ms,
            &chrono::Utc,
            None,
        )
        .unwrap();
        assert_eq!(result.totals.requests, 3);
        assert_eq!(result.totals.total_tokens, 33);
        assert_eq!(result.totals.cost_usd, 16.0);
        assert_eq!(
            result.totals.estimate.as_ref().unwrap().availability,
            "free"
        );
        assert_eq!(result.series.iter().map(|p| p.requests).sum::<u64>(), 3);
        let receipt = &result.analytics.as_ref().unwrap().receipt;
        assert_eq!(receipt.applied_query, q);
        assert_eq!(
            result.analytics.unwrap().options.model_ids,
            ["free", "free-extra"]
        );
        let saved: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT total_cost_usd FROM proxy_request_logs WHERE request_id='a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(saved, "7");
    }
}
