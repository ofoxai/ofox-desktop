//! Frozen, fictional-source parity against Magpie bfaedf18. No paid calls.
use super::*;
use crate::services::{
    ofox_pricing::PriceCatalog,
    session_analytics_core::{Activity, DayBoundary, Window},
    session_native_activity::{activity, NativeActivityEvent},
    usage_analytics,
    usage_data::RequestFilters,
    usage_health,
    usage_overview::UsagePeriod,
    usage_query::UsageQueryV1,
    usage_requests, usage_time,
};
use chrono::{
    DateTime, Datelike, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone,
    Timelike, Utc,
};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

const REFERENCE: &str = "bfaedf18301d02a423bd5422d22e41432d231ab8";
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scripts/usage-parity/fixtures"
);

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn at(text: &str) -> i64 {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .timestamp_millis()
}
fn tokens(buckets: [u64; 4]) -> Value {
    json!({"input":buckets[0],"output":buckets[1],"cacheRead":buckets[2],"cacheWrite":buckets[3],"main":buckets[0].checked_add(buckets[1]).unwrap()})
}
fn usage_tokens(totals: &crate::services::usage_overview::UsageTotals) -> Value {
    tokens([
        totals.input_tokens,
        totals.output_tokens,
        totals.cache_read_tokens,
        totals.cache_write_tokens,
    ])
}
fn analysis_tokens(totals: &crate::services::session_analytics::TokenTotals) -> Value {
    tokens([
        totals.input_tokens,
        totals.output_tokens,
        totals.cache_read_tokens,
        totals.cache_write_tokens,
    ])
}
fn check(errors: &mut Vec<String>, label: &str, observed: Value, expected: &Value) {
    if &observed != expected {
        errors.push(format!("{label}: expected {expected}, observed {observed}"));
    }
}
fn money(errors: &mut Vec<String>, label: &str, observed: Option<&str>, expected: &Value) {
    let observed = observed.map(|v| v.parse::<Decimal>().unwrap().normalize().to_string());
    let expected = expected
        .as_str()
        .map(|v| v.parse::<Decimal>().unwrap().normalize().to_string());
    check(errors, label, json!(observed), &json!(expected));
}

#[derive(Default)]
struct Counter {
    buckets: [u64; 4],
    cost: Decimal,
    unknown_cost: bool,
    records: u64,
}
impl Counter {
    fn add(&mut self, buckets: [u64; 4], cost: Option<&str>) {
        for (sum, value) in self.buckets.iter_mut().zip(buckets) {
            *sum = sum.checked_add(value).unwrap();
        }
        if let Some(cost) = cost {
            self.cost = self.cost.checked_add(cost.parse().unwrap()).unwrap();
        } else {
            self.unknown_cost = true;
        }
        self.records += 1;
    }
    fn cost(&self) -> Option<String> {
        (!self.unknown_cost).then(|| self.cost.normalize().to_string())
    }
}
fn check_counter(errors: &mut Vec<String>, label: &str, total: &Counter, expected: &Value) {
    check(
        errors,
        &format!("{label}.tokens"),
        tokens(total.buckets),
        &expected["tokens"],
    );
    money(
        errors,
        &format!("{label}.costUsd"),
        total.cost().as_deref(),
        &expected["costUsd"],
    );
}

fn day_boundaries<Tz: TimeZone>(start: i64, end: i64, tz: &Tz) -> Vec<DayBoundary> {
    let mut date = tz
        .timestamp_millis_opt(start)
        .single()
        .unwrap()
        .date_naive();
    let last = tz
        .timestamp_millis_opt(end - 1)
        .single()
        .unwrap()
        .date_naive();
    let mut days = vec![];
    while date <= last {
        let next = date.succ_opt().unwrap();
        let start_ms = usage_time::local_day_start_ms(tz, date).unwrap();
        let end_ms = usage_time::local_day_start_ms(tz, next).unwrap();
        if start_ms < end_ms {
            days.push(DayBoundary {
                date: date.to_string(),
                start_ms,
                end_ms,
            });
        }
        date = next;
    }
    days
}
fn hour_allocations<Tz: TimeZone>(
    projection: &Activity,
    tz: &Tz,
) -> BTreeMap<String, BTreeMap<String, u64>> {
    let mut hours = BTreeMap::<String, BTreeMap<String, u64>>::new();
    for &(later, duration) in &projection.native_allocations {
        let later = tz.timestamp_millis_opt(later).single().unwrap();
        let sum = hours
            .entry(later.date_naive().to_string())
            .or_default()
            .entry(later.hour().to_string())
            .or_default();
        *sum = sum.checked_add(duration).unwrap();
    }
    hours
}

/// The same bidirectional gap/fold fixture as calendar_tests, restricted to the
/// manifest's 2026 New York case. This is not a product timezone database.
#[derive(Clone, Copy)]
struct NewYork2026;
impl NewYork2026 {
    fn east_at(utc_ms: i64) -> i32 {
        if (at("2026-03-08T07:00:00Z")..at("2026-11-01T06:00:00Z")).contains(&utc_ms) {
            -4 * 3600
        } else {
            -5 * 3600
        }
    }
}
impl TimeZone for NewYork2026 {
    type Offset = FixedOffset;

    fn from_offset(_: &Self::Offset) -> Self {
        Self
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<Self::Offset> {
        self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<Self::Offset> {
        let wall_ms = local.and_utc().timestamp_millis();
        let daylight = -4 * 3600;
        let standard = -5 * 3600;
        let holds = |east: i32| Self::east_at(wall_ms - i64::from(east) * 1000) == east;
        let offset = |east| FixedOffset::east_opt(east).unwrap();
        match (holds(daylight), holds(standard)) {
            (true, true) => LocalResult::Ambiguous(offset(daylight), offset(standard)),
            (true, false) => LocalResult::Single(offset(daylight)),
            (false, true) => LocalResult::Single(offset(standard)),
            (false, false) => LocalResult::None,
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> Self::Offset {
        self.offset_from_utc_datetime(&utc.and_time(NaiveTime::MIN))
    }

    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> Self::Offset {
        FixedOffset::east_opt(Self::east_at(utc.and_utc().timestamp_millis())).unwrap()
    }
}

fn activity_case<Tz: TimeZone>(case: &Value, tz: &Tz) -> Value {
    let id = case["id"].as_str().unwrap();
    let events = case["timestamps"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(n, timestamp)| NativeActivityEvent {
            session_key: id.into(),
            event_key: format!("{id}:{n}"),
            at_ms: at(timestamp.as_str().unwrap()),
            main: case["main"].as_bool().unwrap(),
            stream_key: id.into(),
        })
        .collect::<Vec<_>>();
    let first = events.iter().map(|e| e.at_ms).min().unwrap();
    let last = events.iter().map(|e| e.at_ms).max().unwrap();
    let first_date = tz
        .timestamp_millis_opt(first)
        .single()
        .unwrap()
        .date_naive();
    let last_date = tz.timestamp_millis_opt(last).single().unwrap().date_naive();
    let start = usage_time::local_day_start_ms(tz, first_date).unwrap();
    let end = usage_time::local_day_start_ms(tz, last_date.succ_opt().unwrap()).unwrap();
    let projection = activity(
        &events,
        &Window {
            start_ms: Some(start),
            end_ms: end,
        },
        &day_boundaries(start, end, tz),
    )
    .unwrap();
    json!({"id":id,"timezone":case["timezone"],"activeMs":projection.active_ms,"dayActiveMs":projection.daily_active_ms,"hourActiveMs":hour_allocations(&projection,tz)})
}
fn activity_cases(manifest: &Value) -> Vec<Value> {
    manifest["activityCases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| match case["timezone"].as_str().unwrap() {
            "UTC" => activity_case(case, &Utc),
            "America/New_York" => activity_case(case, &NewYork2026),
            other => panic!("unregistered frozen activity timezone: {other}"),
        })
        .collect()
}

// These values are the pinned Go run's provenance and direct state.saw cases,
// not product data or guessed target totals. Token/day/thread expectations live
// in the independently calculated expected.json beside the exact source bytes.
fn reference_hashes() -> Value {
    json!({
        "rollout-2026-10-02T00-00-00-parity-a.jsonl":"92ef480d3cdbbe0fe983e42521a6a842755821e18121c6c8d39c6a78c50fda8a",
        "rollout-2026-10-02T00-00-00-parity-b.jsonl":"ad6352ba8f151be7b69e71951e48972160b3ac47ae0e483517610b10c9500068",
        "rollout-2026-10-02T00-00-00-parity-c.jsonl":"6e33ec58a853e7da607929be7494acfd7df1796f894b592fb051074113f0ac0c",
        "rollout-2026-10-02T00-00-00-parity-empty.jsonl":"67035b5a54151b949a67f597c0f46ea93a556441ecf5a3687093c47c93f03860"
    })
}
fn reference_activity_cases() -> Value {
    json!([
        {"id":"strict-gap-and-clock-rollback","timezone":"UTC","activeMs":300000,"dayActiveMs":{"2026-10-02":300000},"hourActiveMs":{"2026-10-02":{"0":300000}}},
        {"id":"midnight-later-event","timezone":"UTC","activeMs":1700,"dayActiveMs":{"2026-10-02":1700},"hourActiveMs":{"2026-10-02":{"0":1700}}},
        {"id":"dst-fall-back-later-local-hour","timezone":"America/New_York","activeMs":4000,"dayActiveMs":{"2026-11-01":4000},"hourActiveMs":{"2026-11-01":{"1":4000}}},
        {"id":"subagent-does-not-add-runtime","timezone":"UTC","activeMs":0,"dayActiveMs":{},"hourActiveMs":{}}
    ])
}
fn normalized_report(mut value: Value) -> Value {
    fn normalize(value: &mut Value) {
        match value {
            Value::Object(fields) => {
                if let Some(amount) = fields.get_mut("costUsd") {
                    if let Some(text) = amount.as_str() {
                        *amount = json!(text.parse::<Decimal>().unwrap().normalize().to_string());
                    }
                }
                for value in fields.values_mut() {
                    normalize(value);
                }
            }
            Value::Array(rows) => {
                for row in rows.iter_mut() {
                    normalize(row);
                }
                if rows.iter().all(|row| row.get("id").is_some()) {
                    rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
                } else if rows.iter().all(|row| row.get("date").is_some()) {
                    rows.sort_by(|a, b| a["date"].as_str().cmp(&b["date"].as_str()));
                }
            }
            _ => {}
        }
    }
    normalize(&mut value);
    value
}

#[test]
#[serial_test::serial]
fn native_parity_frozen_four_sources_match_magpie_and_all_consumers() {
    let sandbox = crate::test_sandbox::sandbox();
    let fixture_dir = Path::new(FIXTURES);
    let manifest = read_json(&fixture_dir.join("manifest.json"));
    let mut expected = read_json(&fixture_dir.join("expected.json"));
    assert_eq!(manifest["synthetic"], true);
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["referenceCommit"], REFERENCE);
    assert_eq!(expected["referenceCommit"], REFERENCE);
    assert_eq!(manifest["timezone"], "UTC");
    assert_eq!(manifest["files"].as_array().unwrap().len(), 4);
    assert_eq!(
        manifest["pricesUsdPerMillion"],
        json!({"parity-priced":{"input":2,"output":8,"cacheRead":0.25,"cacheWrite":3}})
    );

    let native_dir = sandbox.home().join(".codex/sessions/usage-parity");
    std::fs::create_dir_all(&native_dir).unwrap();
    let mut sources = vec![];
    let mut listing = vec![];
    let mut source_hashes = BTreeMap::<String, String>::new();
    let mut original = vec![];
    for file in manifest["files"].as_array().unwrap() {
        let name = file["path"].as_str().unwrap();
        assert_eq!(Path::new(name).file_name().unwrap(), name);
        assert!(reference_hashes().get(name).is_some());
        let source = fixture_dir.join(name);
        let metadata = std::fs::symlink_metadata(&source).unwrap();
        assert!(metadata.is_file() && !metadata.file_type().is_symlink());
        let bytes = std::fs::read(&source).unwrap();
        source_hashes.insert(name.into(), format!("{:x}", Sha256::digest(&bytes)));
        let path = native_dir.join(name);
        std::fs::write(&path, &bytes).unwrap();
        original.push((path.clone(), bytes));
        listing.push(crate::session_manager::SessionMeta {
            provider_id: "codex".into(),
            session_id: file["threadId"].as_str().unwrap().into(),
            source_path: Some(path.to_string_lossy().into_owned()),
            ..Default::default()
        });
        sources.push(SourceSpec {
            provider: "codex".into(),
            path,
            subagent: false,
            project_file: None,
            title_file: None,
        });
    }
    assert_eq!(
        json!(source_hashes),
        reference_hashes(),
        "frozen inputs changed since the pinned Go run"
    );
    let db = Database::memory().unwrap();
    let refresh = refresh_sources(&db, &sources).unwrap();
    assert_eq!(refresh.failed, 0);
    assert_eq!(refresh.parsed, 4);
    let now = at(manifest["now"].as_str().unwrap());
    let query = UsageQueryV1 {
        version: 1,
        start_ms: Some(at("2026-10-01T00:00:00Z")),
        end_ms: at("2026-10-03T00:00:00Z"),
        tool_ids: vec!["codex".into()],
        // The isolated database contains only these Codex sources. A source
        // attribution filter intentionally makes runtime unavailable in the
        // product; the common-format baseline compares the whole native tool.
        source_ids: vec![],
        model_ids: vec![],
        project_ids: vec![],
    };
    let price = json!({"data":[{"id":"parity-priced","pricing":{"prompt":"0.000002","completion":"0.000008","input_cache_read":"0.00000025","input_cache_write":"0.000003"}}]});
    let (input, frozen_threads) = {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for apex in ["ofox.ai", "ofox.io"] {
            PriceCatalog::ingest_on(&tx, apex, &price, now).unwrap();
        }
        let checked = projection_cache().read_on(&tx).unwrap();
        assert!(
            checked.codex_native_ready,
            "only the current-epoch canonical generation may be compared"
        );
        let observed = checked
            .source_observations
            .iter()
            .filter(|source| source.tool_id == "codex" && source.source_id == "codex_session")
            .collect::<Vec<_>>();
        assert_eq!(observed.len(), sources.len());
        assert!(
            observed.iter().all(|source| source.status == "ok"),
            "the frozen-source run cannot certify a failed or partial read"
        );
        // refresh_sources only publishes the sandbox's decoded projection.
        // Production's refresh coordinator also records both health channels;
        // certify precisely this successful, fully enumerated fixture run.
        // This does not infer success from a cached value or touch live health.
        for channel in [
            usage_health::Channel::Ledger,
            usage_health::Channel::Analytics,
        ] {
            let attempt =
                usage_health::begin_on(&tx, "codex", "codex_session", channel, now).unwrap();
            assert!(
                usage_health::finish_on(&tx, &attempt, usage_health::Outcome::Ok, now).unwrap()
            );
        }
        let input = projection_cache().read_on(&tx).unwrap();
        let projections = listing_projections_on(&tx, &listing).unwrap().0;
        tx.commit().unwrap();
        (input, projections)
    };

    let overview = usage_analytics::build_query_overview(
        &db,
        UsagePeriod::All,
        query.clone(),
        "ofox.ai",
        now,
        &Utc,
        None,
    )
    .unwrap();
    let requests = usage_requests::request_page_in(
        &db,
        query.clone(),
        RequestFilters::default(),
        usage_requests::Pagination {
            page: 0,
            page_size: 100,
        },
        None,
        "ofox.ai",
        (&Utc, now),
    )
    .unwrap();
    let analysis =
        crate::services::session_analytics_api::query(&db, query.clone(), "ofox.ai", now, &Utc)
            .unwrap();
    let (_, listed, _) = crate::session_manager::usage::load_for_listing(&db, &listing).unwrap();
    let native = activity(
        &input.native_activity,
        &Window {
            start_ms: query.start_ms,
            end_ms: query.end_ms,
        },
        &day_boundaries(query.start_ms.unwrap(), query.end_ms, &Utc),
    )
    .unwrap();
    let hours = hour_allocations(&native, &Utc);
    let mut errors = vec![];
    let expected_hour_cells = native.native_allocations.iter().fold(
        BTreeMap::<(u32, u32), u64>::new(),
        |mut cells, (later, duration)| {
            let later = Utc.timestamp_millis_opt(*later).single().unwrap();
            *cells
                .entry((later.weekday().num_days_from_monday(), later.hour()))
                .or_default() += duration;
            cells
        },
    );
    let actual_hour_cells = analysis
        .activity_by_hour
        .cells
        .iter()
        .filter(|cell| cell.active_ms > 0)
        .map(|cell| ((cell.weekday, cell.hour), cell.active_ms))
        .collect::<BTreeMap<_, _>>();
    if actual_hour_cells != expected_hour_cells {
        errors.push(format!(
            "analysis.hourActivity: expected {expected_hour_cells:?}, observed {actual_hour_cells:?}"
        ));
    }
    let want_tokens = &expected["tokens"];
    check(
        &mut errors,
        "overview.tokens",
        usage_tokens(&overview.totals),
        want_tokens,
    );
    check(
        &mut errors,
        "requests.tokens",
        usage_tokens(&requests.summary),
        want_tokens,
    );
    money(
        &mut errors,
        "overview.costUsd",
        overview
            .totals
            .estimate
            .as_ref()
            .and_then(|e| e.known_cost_usd.as_deref()),
        &expected["costUsd"],
    );
    money(
        &mut errors,
        "requests.costUsd",
        requests
            .summary
            .estimate
            .as_ref()
            .and_then(|e| e.known_cost_usd.as_deref()),
        &expected["costUsd"],
    );
    check(
        &mut errors,
        "overview.threads",
        json!(
            overview
                .analytics
                .as_ref()
                .unwrap()
                .attribution
                .session_count
        ),
        &expected["threadCount"],
    );
    check(
        &mut errors,
        "requests.count",
        json!(requests.total),
        &json!(8),
    );
    check(
        &mut errors,
        "requests.pageCount",
        json!(requests.data.len()),
        &json!(8),
    );
    assert_eq!(analysis.method_version, "magpie-native-v1");
    assert_eq!(
        overview.analytics.as_ref().unwrap().method_version,
        "magpie-native-v1"
    );
    assert_eq!(requests.method_version, "magpie-native-v1");
    assert_eq!(analysis.receipt.applied_query, query);
    assert_eq!(requests.receipt.applied_query, query);
    assert_eq!(
        overview.analytics.as_ref().unwrap().receipt.applied_query,
        query
    );
    assert!(input
        .facts
        .iter()
        .all(|fact| fact.at_ms >= query.start_ms.unwrap()
            && fact.at_ms < query.end_ms
            && fact.tool_id == "codex"));
    for version in [
        &analysis.input_snapshot_version,
        &overview.analytics.as_ref().unwrap().input_snapshot_version,
        &requests.input_snapshot_version,
    ] {
        assert_eq!(version, &input.snapshot_version);
    }
    for projection in frozen_threads.values().filter(|p| p.state == "available") {
        assert_eq!(
            projection.facts_hash.as_deref(),
            Some(input.snapshot_version.as_str())
        );
    }

    let mut request_days = BTreeMap::<String, Counter>::new();
    let mut request_threads = BTreeMap::<String, Counter>::new();
    let event_threads = input
        .facts
        .iter()
        .map(|fact| {
            (
                format!("codex_native_v1:{}", fact.event_key),
                fact.session_key.as_deref().unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let identities = input
        .sessions
        .iter()
        .map(|session| {
            (
                session.identity.key.as_str(),
                session.native_session_id.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut request_ids = BTreeSet::new();
    for view in &requests.data {
        assert!(
            request_ids.insert(&view.record.request_id),
            "canonical identity duplicated in detail"
        );
        assert!(view.recorded_cost_usd.is_none());
        assert!(view.observed_status_code.is_none());
        assert!(view.observed_latency_ms.is_none());
        assert!(view.observed_first_token_ms.is_none());
        assert!(view.record.duration_ms.is_none());
        assert_eq!(view.current_estimate.availability, "known");
        let buckets = [
            view.record.input_tokens,
            view.record.output_tokens,
            view.record.cache_read_tokens,
            view.record.cache_creation_tokens,
        ];
        let date = Utc
            .timestamp_millis_opt(view.created_at_ms)
            .single()
            .unwrap()
            .date_naive()
            .to_string();
        request_days
            .entry(date)
            .or_default()
            .add(buckets, view.current_estimate.known_cost_usd.as_deref());
        let session = event_threads
            .get(&view.record.request_id)
            .and_then(|key| identities.get(key))
            .copied()
            .unwrap_or("unassociated");
        request_threads
            .entry(session.into())
            .or_default()
            .add(buckets, view.current_estimate.known_cost_usd.as_deref());
        let detail = usage_requests::request_detail(&view.record.request_id, &requests.snapshot_id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.current_estimate, view.current_estimate);
        assert_eq!(detail.created_at_ms, view.created_at_ms);
    }
    let mut overview_days = BTreeMap::new();
    for day in &overview.series {
        let date = Utc
            .timestamp_millis_opt(day.start)
            .single()
            .unwrap()
            .date_naive()
            .to_string();
        overview_days.insert(date,json!({"tokens":tokens([day.input_tokens,day.output_tokens,day.cache_read_tokens,day.cache_write_tokens]),"costUsd":day.estimate.as_ref().and_then(|e|e.known_cost_usd.as_deref())}));
    }
    for day in expected["days"].as_array().unwrap() {
        let date = day["date"].as_str().unwrap();
        check_counter(
            &mut errors,
            &format!("requests.days[{date}]"),
            request_days.get(date).unwrap_or(&Counter::default()),
            day,
        );
        let overview_day = overview_days.get(date).cloned().unwrap_or(Value::Null);
        check(
            &mut errors,
            &format!("overview.days[{date}].tokens"),
            overview_day["tokens"].clone(),
            &day["tokens"],
        );
        money(
            &mut errors,
            &format!("overview.days[{date}].costUsd"),
            overview_day["costUsd"].as_str(),
            &day["costUsd"],
        );
    }
    check(
        &mut errors,
        "requests.dayCount",
        json!(request_days.len()),
        &json!(2),
    );
    check(
        &mut errors,
        "overview.dayCount",
        json!(overview_days.len()),
        &json!(2),
    );

    let mut list_total = Counter::default();
    for thread in expected["threads"].as_array().unwrap() {
        let sid = thread["id"].as_str().unwrap();
        check_counter(
            &mut errors,
            &format!("requests.threads[{sid}]"),
            request_threads.get(sid).unwrap_or(&Counter::default()),
            thread,
        );
        let (usage, display) = listed
            .get(sid)
            .expect("every sandbox list row receives a display receipt");
        if let Some(usage) = usage {
            let buckets = [
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens,
            ];
            check(
                &mut errors,
                &format!("list.threads[{sid}].tokens"),
                tokens(buckets),
                &thread["tokens"],
            );
            let amount = display
                .estimate
                .as_ref()
                .and_then(|e| e.known_cost_usd.as_deref());
            money(
                &mut errors,
                &format!("list.threads[{sid}].costUsd"),
                amount,
                &thread["costUsd"],
            );
            list_total.add(buckets, amount);
            assert_eq!(
                display.billing_session_id.as_deref(),
                Some("parity-shared-billing")
            );
            assert_eq!(display.scope, "nativeThread");
        } else {
            errors.push(format!("list.threads[{sid}]: known own usage unavailable"));
        }
    }
    check_counter(&mut errors, "list.total", &list_total, &expected);
    check(
        &mut errors,
        "requests.threadCount",
        json!(request_threads.len()),
        &expected["threadCount"],
    );
    assert!(
        listed["parity-empty"].0.is_none(),
        "metadata-only source is not zero-cost activity"
    );

    let csv = usage_requests::export_csv(&requests.snapshot_id, None).unwrap();
    assert_eq!(csv.record_count, requests.total);
    let mut csv_total = Counter::default();
    for row in csv.csv.lines().skip(1) {
        // All values in this fixed, fictional fixture are comma/quote-free.
        let cells = row
            .split(',')
            .map(|s| s.trim_matches('"'))
            .collect::<Vec<_>>();
        assert_eq!(cells.len(), 15);
        assert_eq!((cells[9], cells[13], cells[14]), ("", "", ""));
        let parsed = std::array::from_fn(|index| cells[index + 5].parse::<u64>().unwrap());
        csv_total.add(parsed, (!cells[10].is_empty()).then_some(cells[10]));
    }
    check_counter(&mut errors, "csv.total", &csv_total, &expected);
    check(
        &mut errors,
        "csv.count",
        json!(csv_total.records),
        &json!(8),
    );

    let days = analysis
        .series
        .iter()
        .map(|day| {
            json!({
                "date":day.date,"tokens":analysis_tokens(&day.tokens),"activeMs":day.active_ms,
                "referenceActiveSeconds":day.active_ms.map(|ms|ms/1000),
                "hourActiveMs":hours.get(&day.date).cloned().unwrap_or_default(),
                "costUsd":day.estimate.known_cost_usd
            })
        })
        .collect::<Vec<_>>();
    let threads=analysis.top_sessions.iter().map(|session|json!({
        "id":session.native_session_id,"tokens":session.tokens.as_ref().map(analysis_tokens),
        "activeMs":session.active_ms,"referenceActiveSeconds":session.active_ms.map(|ms|ms/1000),
        "costUsd":session.estimate.known_cost_usd
    })).collect::<Vec<_>>();
    let report = json!({
        "version":1,"referenceCommit":REFERENCE,"timezone":"UTC",
        "threadCount":analysis.totals.session_count,"tokens":analysis_tokens(&analysis.totals.tokens),
        "quantiles":{"median":analysis.totals.median_tokens.as_deref().map(|v|v.parse::<u64>().unwrap()),"p90":analysis.totals.p90_tokens},
        "costUsd":analysis.totals.estimate.known_cost_usd,"activeMs":analysis.totals.active_ms,
        "referenceActiveSeconds":analysis.totals.active_ms.map(|ms|ms/1000),
        "days":days,"threads":threads,"activityCases":activity_cases(&manifest),
        "callCount":requests.total,"callTokens":usage_tokens(&requests.summary),"sourceSha256":source_hashes
    });
    expected["activityCases"] = reference_activity_cases();
    expected["sourceSha256"] = reference_hashes();
    expected["callCount"] = json!(8);
    expected["callTokens"] = expected["tokens"].clone();
    let report = normalized_report(report);
    check(
        &mut errors,
        "reference.normalizedReport",
        report.clone(),
        &normalized_report(expected),
    );

    if let Some(destination) = std::env::var_os("OFOX_USAGE_PARITY_OUTPUT") {
        let path = std::path::PathBuf::from(destination);
        assert!(
            path.is_absolute(),
            "explicit report destination must be absolute"
        );
        let missing = std::fs::symlink_metadata(&path)
            .expect_err("refuse to replace an existing file or symlink with a test report");
        assert_eq!(
            missing.kind(),
            std::io::ErrorKind::NotFound,
            "an unreadable destination is not a new report path"
        );
        let mut bytes = serde_json::to_vec_pretty(&report).unwrap();
        bytes.push(b'\n');
        crate::config::atomic_write(&path, &bytes).unwrap();
    }
    assert_eq!(
        refresh_sources(&db, &sources).unwrap().parsed,
        0,
        "the accepted frozen sources stay warm"
    );
    for (path, bytes) in original {
        assert_eq!(
            std::fs::read(path).unwrap(),
            bytes,
            "read-only consumers changed a source"
        );
    }
    assert_eq!(
        db.conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "parity rebuilding must not manufacture saved historical charges"
    );
    assert!(
        errors.is_empty(),
        "frozen usage parity differences:\n{}",
        errors.join("\n")
    );
}
