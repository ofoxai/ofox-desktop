//! A request-only, read-only projection over the same frozen rows as paging/CSV.
use super::{invalid, RequestMeasurements, RequestView};
use crate::{
    error::AppError,
    services::{
        ofox_pricing::CostEstimate,
        usage_analytics::UsageTrend,
        usage_data::{request_provider, RequestFilters},
        usage_overview::Granularity,
    },
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    requests: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    total_tokens_including_cache: u64,
    cache_hit_rate: f64,
    estimate: CostEstimate,
    measurements: RequestMeasurements,
}
#[derive(Debug, Clone, Serialize)]
pub struct Point {
    start: i64,
    end: i64,
    totals: Totals,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    id: String,
    label: Option<String>,
    model_id: Option<String>,
    tool_id: Option<String>,
    provider_key: Option<String>,
    provider_label: Option<String>,
    totals: Totals,
    series: Vec<Point>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Dimension {
    groups: Vec<Group>,
    rankings: Vec<Group>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Provider {
    key: String,
    label: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Dashboard {
    granularity: Granularity,
    totals: Totals,
    series: Vec<Point>,
    dimensions: BTreeMap<String, Dimension>,
    providers: Vec<Provider>,
}

#[derive(Default)]
struct Aggregate {
    requests: u64,
    tokens: [u64; 4],
    estimate: CostEstimate,
    status_samples: u64,
    failed_samples: u64,
    latency_samples: u64,
    latency_sum: u128,
    first_samples: u64,
    first_sum: u128,
}
fn overflow() -> AppError {
    invalid("aggregateOverflow")
}
pub(super) fn row_tokens(row: &RequestView) -> Result<u64, AppError> {
    [
        row.record.input_tokens,
        row.record.output_tokens,
        row.record.cache_read_tokens,
        row.record.cache_creation_tokens,
    ]
    .into_iter()
    .try_fold(0_u64, |sum, n| sum.checked_add(n).ok_or_else(overflow))
}
impl Aggregate {
    fn add(&mut self, view: &RequestView) -> Result<(), AppError> {
        self.requests = self.requests.checked_add(1).ok_or_else(overflow)?;
        let row = &view.record;
        for (sum, n) in self.tokens.iter_mut().zip([
            row.input_tokens,
            row.output_tokens,
            row.cache_read_tokens,
            row.cache_creation_tokens,
        ]) {
            *sum = sum.checked_add(n).ok_or_else(overflow)?;
        }
        self.estimate.combine(&view.current_estimate)?;
        if let Some(status) = view.observed_status_code {
            self.status_samples += 1;
            self.failed_samples += u64::from(status >= 400);
        }
        if let Some(latency) = view.observed_latency_ms {
            self.latency_samples += 1;
            self.latency_sum += u128::from(latency);
        }
        if let Some(first) = view.observed_first_token_ms {
            self.first_samples += 1;
            self.first_sum += u128::from(first);
        }
        Ok(())
    }
    fn totals(&self) -> Result<Totals, AppError> {
        let prompt = self.tokens[0]
            .checked_add(self.tokens[2])
            .and_then(|n| n.checked_add(self.tokens[3]))
            .ok_or_else(overflow)?;
        let total = prompt.checked_add(self.tokens[1]).ok_or_else(overflow)?;
        Ok(Totals {
            requests: self.requests,
            input_tokens: self.tokens[0],
            output_tokens: self.tokens[1],
            cache_read_tokens: self.tokens[2],
            cache_write_tokens: self.tokens[3],
            total_tokens_including_cache: total,
            cache_hit_rate: if prompt == 0 {
                0.0
            } else {
                self.tokens[2] as f64 / prompt as f64
            },
            estimate: if self.requests == 0 {
                CostEstimate::empty()
            } else {
                self.estimate.clone()
            },
            measurements: RequestMeasurements {
                status_samples: self.status_samples,
                failed_samples: self.failed_samples,
                failure_rate: (self.status_samples > 0)
                    .then(|| self.failed_samples as f64 / self.status_samples as f64),
                latency_samples: self.latency_samples,
                avg_latency_ms: (self.latency_samples > 0)
                    .then(|| self.latency_sum as f64 / self.latency_samples as f64),
                first_token_samples: self.first_samples,
                avg_first_token_ms: (self.first_samples > 0)
                    .then(|| self.first_sum as f64 / self.first_samples as f64),
                // The existing readers do not prove generation timing. Keep it absent.
                generation_speed_samples: 0,
                avg_generation_tokens_per_second: None,
            },
        })
    }
}

fn identity(view: &RequestView, dimension: &str) -> Group {
    let row = &view.record;
    let (provider_key, provider_label) = request_provider(row);
    let (id, label, model_id, tool_id, provider_key) = match dimension {
        "model" => (
            row.model.clone(),
            Some(row.model.clone()),
            Some(row.model.clone()),
            None,
            None,
        ),
        "agent" => (
            row.app_type.clone(),
            Some(row.app_type.clone()),
            None,
            Some(row.app_type.clone()),
            None,
        ),
        "provider" => (
            provider_key.clone(),
            provider_label.clone(),
            None,
            None,
            Some(provider_key),
        ),
        _ => (
            serde_json::to_string(&(&row.model, &provider_key))
                .expect("string pair serialization is infallible"),
            Some(row.model.clone()),
            Some(row.model.clone()),
            None,
            Some(provider_key),
        ),
    };
    Group {
        id,
        label,
        model_id,
        tool_id,
        provider_key,
        provider_label,
        totals: Aggregate::default().totals().expect("empty aggregate"),
        series: vec![],
    }
}

fn matches(view: &RequestView, filters: &RequestFilters, ignore: Option<&str>) -> bool {
    let row = &view.record;
    let provider = request_provider(row).0;
    filters
        .provider_key
        .as_ref()
        .is_none_or(|key| key == &provider)
        && filters.selection.as_ref().is_none_or(|selection| {
            (matches!(ignore, Some("model" | "modelProvider"))
                || selection
                    .model_id
                    .as_ref()
                    .is_none_or(|id| id == &row.model))
                && (matches!(ignore, Some("agent"))
                    || selection
                        .tool_id
                        .as_ref()
                        .is_none_or(|id| id == &row.app_type))
                && (matches!(ignore, Some("provider" | "modelProvider"))
                    || selection
                        .provider_key
                        .as_ref()
                        .is_none_or(|key| key == &provider))
        })
}

type GroupMap = BTreeMap<String, (Group, Aggregate, BTreeMap<usize, Aggregate>)>;
fn add_group(
    map: &mut GroupMap,
    view: &RequestView,
    dimension: &str,
    bucket: Option<usize>,
) -> Result<(), AppError> {
    let group = identity(view, dimension);
    let entry = map
        .entry(group.id.clone())
        .or_insert_with(|| (group, Aggregate::default(), BTreeMap::new()));
    entry.1.add(view)?;
    if let Some(bucket) = bucket {
        entry.2.entry(bucket).or_default().add(view)?;
    }
    Ok(())
}
fn finish(map: GroupMap, calendar: &[(i64, i64)]) -> Result<Vec<Group>, AppError> {
    map.into_values()
        .map(|(mut group, total, points)| {
            group.totals = total.totals()?;
            group.series = points
                .into_iter()
                .map(|(i, sum)| {
                    Ok(Point {
                        start: calendar[i].0,
                        end: calendar[i].1,
                        totals: sum.totals()?,
                    })
                })
                .collect::<Result<_, AppError>>()?;
            Ok(group)
        })
        .collect()
}

pub(super) fn build(
    rows: &[RequestView],
    trend: &UsageTrend,
    end: i64,
    filters: &RequestFilters,
) -> Result<(Dashboard, Vec<usize>), AppError> {
    let calendar: Vec<_> = trend
        .series
        .iter()
        .enumerate()
        .map(|(i, p)| {
            (
                p.start,
                trend.series.get(i + 1).map_or(end, |next| next.start),
            )
        })
        .collect();
    let day = filters
        .day_start_ms
        .map(|start| {
            if !matches!(trend.granularity, Granularity::Day) {
                return Err(invalid("invalidDay"));
            }
            calendar
                .iter()
                .find(|point| point.0 == start)
                .copied()
                .ok_or_else(|| invalid("invalidDay"))
        })
        .transpose()?;
    let in_day = |view: &RequestView| {
        day.is_none_or(|(from, to)| view.created_at_ms >= from && view.created_at_ms < to)
    };
    let mut total = Aggregate::default();
    let mut selected = Vec::new();
    let mut points: Vec<Aggregate> = calendar.iter().map(|_| Aggregate::default()).collect();
    let mut dimensions = BTreeMap::new();
    let mut providers = BTreeMap::new();
    let mut chart: BTreeMap<&str, GroupMap> = BTreeMap::new();
    let mut ranks: BTreeMap<&str, GroupMap> = BTreeMap::new();
    for (index, view) in rows.iter().enumerate() {
        let (key, label) = request_provider(&view.record);
        providers.entry(key).or_insert(label);
        let bucket = calendar
            .partition_point(|(start, _)| *start <= view.created_at_ms)
            .checked_sub(1)
            .ok_or_else(|| invalid("invalidBucket"))?;
        if view.created_at_ms >= calendar[bucket].1 {
            return Err(invalid("invalidBucket"));
        }
        let focused = matches(view, filters, None);
        if focused {
            points[bucket].add(view)?;
            if in_day(view) {
                selected.push(index);
                total.add(view)?;
            }
        }
        for dimension in ["model", "modelProvider", "provider", "agent"] {
            if focused {
                add_group(
                    chart.entry(dimension).or_default(),
                    view,
                    dimension,
                    Some(bucket),
                )?;
            }
            if in_day(view) && matches(view, filters, Some(dimension)) {
                add_group(ranks.entry(dimension).or_default(), view, dimension, None)?;
            }
        }
    }
    for dimension in ["model", "modelProvider", "provider", "agent"] {
        dimensions.insert(
            dimension.to_owned(),
            Dimension {
                groups: finish(chart.remove(dimension).unwrap_or_default(), &calendar)?,
                rankings: finish(ranks.remove(dimension).unwrap_or_default(), &calendar)?,
            },
        );
    }
    let series = points
        .into_iter()
        .enumerate()
        .map(|(i, sum)| {
            Ok(Point {
                start: calendar[i].0,
                end: calendar[i].1,
                totals: sum.totals()?,
            })
        })
        .collect::<Result<_, AppError>>()?;
    Ok((
        Dashboard {
            granularity: trend.granularity,
            totals: total.totals()?,
            series,
            dimensions,
            providers: providers
                .into_iter()
                .map(|(key, label)| Provider { key, label })
                .collect(),
        },
        selected,
    ))
}
