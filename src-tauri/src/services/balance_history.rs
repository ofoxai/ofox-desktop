//! The OFox balance over time, and when it runs out at the pace it is spent.
//!
//! Ported from Magpie's `provider/balance_history.go`. Each balance the app reads
//! anyway (the user-info refreshes and the low-balance loop) is kept as a point of
//! `balance_samples` (org, when, amount). Points come at least five minutes apart:
//! a nearer one moves the last point while the balance is still being spent, and a
//! run of the same amount keeps only its first and last point, so an idle balance
//! costs two rows however long it lasts. A top-up, or the first spending after
//! one, keeps its own point. Points older than 45 days go, and an org keeps 4000.
//!
//! [`trend`] draws the last 14 days and fits a least-squares line through the
//! points since the last top-up, within the last 7 days; that line's slope is the
//! burn rate, and where it meets zero is when the balance runs out. All times are
//! epoch milliseconds.

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::ofox_auth::OfoxUserInfo;
use rusqlite::params;
use serde::Serialize;

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;

/// Points older than this are dropped.
const KEPT_MS: i64 = 45 * DAY_MS;
/// An org keeps at most this many points.
const MAX_POINTS: i64 = 4000;
/// Points come at least this far apart while the balance is being spent.
const GAP_MS: i64 = 5 * MINUTE_MS;
/// The points a trend is drawn from.
const SHOW_SPAN_MS: i64 = 14 * DAY_MS;
/// The points a trend is fitted over.
const FIT_SPAN_MS: i64 = 7 * DAY_MS;
/// A fit needs this many points, this far apart from first to last.
const FIT_POINTS: usize = 3;
const FIT_MIN_MS: i64 = HOUR_MS;
/// A run-out further away than this is not said.
const RUNS_OUT_MAX_MS: i64 = 365 * DAY_MS;

/// One reading of the balance: `amount` USD was left at `at`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalancePoint {
    pub at: i64,
    pub amount: f64,
}

/// The balance's recent points, oldest first, and what the fit through the latest
/// of them says. Fewer than two points in the last 14 days leave `points` empty.
/// With no fit (under 3 points, under an hour of them, or a top-up just now) the
/// other fields are `None`; a balance that is not being spent has `per_day` 0 and
/// no run-out.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BalanceTrend {
    pub points: Vec<BalancePoint>,
    /// USD spent a day.
    pub per_day: Option<f64>,
    /// When the balance reaches zero at that pace; only for a balance that is
    /// being spent and runs out within a year.
    pub runs_out_at: Option<i64>,
    /// `runs_out_at` as days from now.
    pub days_left: Option<f64>,
}

/// What a new point does to the points before it.
#[derive(Debug, PartialEq)]
enum Step {
    /// Not after the last point: ignored.
    Skip,
    /// Moves the last point (near it while still spending, or a flat run).
    ReplaceLast,
    Append,
}

/// `tail` is the last two points kept, oldest first.
fn step(tail: &[BalancePoint], n: BalancePoint) -> Step {
    if tail.last().is_some_and(|last| n.at <= last.at) {
        return Step::Skip;
    }
    if let [p1, p2] = tail {
        let flat = n.amount == p2.amount && p2.amount == p1.amount;
        // a top-up, or the first spending after one, keeps its point
        let still_spending =
            n.at - p1.at < GAP_MS && n.amount <= p2.amount && p2.amount <= p1.amount;
        if flat || still_spending {
            return Step::ReplaceLast;
        }
    }
    Step::Append
}

/// Keep the balance `amount` read at `now_ms` for `org_id`.
pub fn note(db: &Database, org_id: &str, amount: f64, now_ms: i64) -> Result<(), AppError> {
    if !amount.is_finite() {
        return Ok(());
    }
    let mut conn = lock_conn!(db.conn);
    let tx = conn.transaction()?;

    let mut tail: Vec<BalancePoint> = {
        let mut stmt = tx.prepare(
            "SELECT at_ms, amount FROM balance_samples
             WHERE org_id = ?1 ORDER BY at_ms DESC LIMIT 2",
        )?;
        let rows = stmt.query_map([org_id], |row| {
            Ok(BalancePoint {
                at: row.get(0)?,
                amount: row.get(1)?,
            })
        })?;
        rows.collect::<Result<_, _>>()?
    };
    tail.reverse();

    let point = BalancePoint { at: now_ms, amount };
    match step(&tail, point) {
        Step::Skip => return Ok(()),
        Step::ReplaceLast => {
            if let Some(last) = tail.last() {
                tx.execute(
                    "DELETE FROM balance_samples WHERE org_id = ?1 AND at_ms = ?2",
                    params![org_id, last.at],
                )?;
            }
        }
        Step::Append => {}
    }
    tx.execute(
        "INSERT INTO balance_samples (org_id, at_ms, amount) VALUES (?1, ?2, ?3)",
        params![org_id, point.at, point.amount],
    )?;

    tx.execute(
        "DELETE FROM balance_samples WHERE org_id = ?1 AND at_ms < ?2",
        params![org_id, now_ms - KEPT_MS],
    )?;
    tx.execute(
        "DELETE FROM balance_samples WHERE org_id = ?1 AND at_ms <=
           (SELECT at_ms FROM balance_samples WHERE org_id = ?1
            ORDER BY at_ms DESC LIMIT 1 OFFSET ?2)",
        params![org_id, MAX_POINTS],
    )?;
    tx.commit()?;
    Ok(())
}

/// Keep the balance a refreshed user info tells, under its billing org. Nothing
/// is kept for a user without an org or a balance, or for a balance that is not
/// in USD (OFox quotes USD; the trend is drawn and fitted in dollars).
pub fn note_user(db: &Database, user: &OfoxUserInfo, now_ms: i64) -> Result<(), AppError> {
    let Some(balance) = user.balance.as_ref() else {
        return Ok(());
    };
    if balance
        .currency
        .as_deref()
        .is_some_and(|c| !c.eq_ignore_ascii_case("USD"))
    {
        return Ok(());
    }
    let (Some(org_id), Some(amount)) = (user.key_owner().org_id, balance.balance) else {
        return Ok(());
    };
    note(db, &org_id, amount, now_ms)
}

/// [`note_user`] at `now_ms`; a failure is logged, because a missing point must
/// not fail the refresh that told the balance.
pub fn record(db: &Database, user: &OfoxUserInfo, now_ms: i64) {
    if let Err(e) = note_user(db, user, now_ms) {
        log::warn!("[balance-history] could not keep the balance: {e}");
    }
}

/// The trend of `org_id`'s kept points at `now_ms`.
pub fn trend_for(db: &Database, org_id: &str, now_ms: i64) -> Result<BalanceTrend, AppError> {
    let conn = lock_conn!(db.conn);
    let mut stmt = conn.prepare(
        "SELECT at_ms, amount FROM balance_samples
         WHERE org_id = ?1 AND at_ms >= ?2 AND at_ms <= ?3 ORDER BY at_ms",
    )?;
    let rows = stmt.query_map(params![org_id, now_ms - SHOW_SPAN_MS, now_ms], |row| {
        Ok(BalancePoint {
            at: row.get(0)?,
            amount: row.get(1)?,
        })
    })?;
    let points: Vec<BalancePoint> = rows.collect::<Result<_, _>>()?;
    Ok(trend(&points, now_ms))
}

fn round4(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

/// `points` (oldest first) as a trend at `now_ms`.
pub fn trend(points: &[BalancePoint], now_ms: i64) -> BalanceTrend {
    let first_shown = points.partition_point(|p| p.at < now_ms - SHOW_SPAN_MS);
    let shown = &points[first_shown..];
    if shown.len() < 2 {
        return BalanceTrend::default();
    }
    let mut trend = BalanceTrend {
        points: shown.to_vec(),
        ..BalanceTrend::default()
    };

    // the points since the last top-up, within the fit span
    let mut j = shown.len() - 1;
    while j > 0 && shown[j - 1].amount >= shown[j].amount && now_ms - shown[j - 1].at <= FIT_SPAN_MS
    {
        j -= 1;
    }
    let fit = &shown[j..];
    let spanned = fit[fit.len() - 1].at - fit[0].at;
    if fit.len() < FIT_POINTS || spanned < FIT_MIN_MS {
        return trend;
    }

    // least squares, time in days from the fit's first point
    let t0 = fit[0].at;
    let days_since = |at: i64| (at - t0) as f64 / DAY_MS as f64;
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
    for p in fit {
        let x = days_since(p.at);
        sx += x;
        sy += p.amount;
        sxx += x * x;
        sxy += x * p.amount;
    }
    let n = fit.len() as f64;
    let den = n * sxx - sx * sx;
    if den <= 0.0 {
        return trend;
    }
    let slope = (n * sxy - sx * sy) / den;
    let intercept = (sy - slope * sx) / n;

    // the points never rise within a fit, so the slope is at most 0
    let per_day = round4(-slope);
    trend.per_day = Some(if per_day == 0.0 { 0.0 } else { per_day });
    if slope < 0.0 {
        // A tiny burn can put the zero crossing beyond i64 milliseconds. Check
        // the remaining days before converting, including non-finite fits.
        let days_left = -intercept / slope - days_since(now_ms);
        if days_left.is_finite() && days_left <= RUNS_OUT_MAX_MS as f64 / DAY_MS as f64 {
            let remaining_ms = (days_left.max(0.0) * DAY_MS as f64) as i64;
            if let Some(runs_out_at) = now_ms.checked_add(remaining_ms) {
                trend.runs_out_at = Some(runs_out_at);
                trend.days_left = Some(round4(remaining_ms as f64 / DAY_MS as f64));
            }
        }
    }
    trend
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000_000;
    const NOW: i64 = T0 + 30 * DAY_MS;

    fn db() -> Database {
        Database::memory().expect("in-memory database")
    }

    fn at_min(m: i64) -> i64 {
        T0 + m * MINUTE_MS
    }

    fn hours_ago(h: f64) -> i64 {
        NOW - (h * HOUR_MS as f64) as i64
    }

    fn point(at: i64, amount: f64) -> BalancePoint {
        BalancePoint { at, amount }
    }

    fn kept(db: &Database, org_id: &str) -> Vec<(i64, f64)> {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT at_ms, amount FROM balance_samples WHERE org_id = ?1 ORDER BY at_ms")
            .unwrap();
        let rows = stmt
            .query_map([org_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        rows.map(Result::unwrap).collect()
    }

    // ── note ──────────────────────────────────────────────────────────────

    /// Readings within five minutes are one point while spending; a run of the
    /// same amount is its first and last point.
    #[test]
    fn note_keeps_points_apart_and_collapses_flat_runs() {
        let db = db();
        note(&db, "org", 100.0, at_min(0)).unwrap();
        note(&db, "org", 99.9, at_min(1)).unwrap();
        note(&db, "org", 99.8, at_min(2)).unwrap(); // within 5 min, still spending: moves it
        note(&db, "org", 99.0, at_min(10)).unwrap();
        note(&db, "org", 99.0, at_min(20)).unwrap();
        note(&db, "org", 99.0, at_min(30)).unwrap(); // the same: the run's end moves

        assert_eq!(
            kept(&db, "org"),
            [
                (at_min(0), 100.0),
                (at_min(2), 99.8),
                (at_min(10), 99.0),
                (at_min(30), 99.0)
            ]
        );
    }

    #[test]
    fn note_keeps_a_top_up_as_its_own_point() {
        let db = db();
        note(&db, "org", 10.0, at_min(0)).unwrap();
        note(&db, "org", 9.5, at_min(10)).unwrap();
        note(&db, "org", 60.0, at_min(11)).unwrap(); // within 5 min of 9.5, but a top-up

        assert_eq!(
            kept(&db, "org"),
            [(at_min(0), 10.0), (at_min(10), 9.5), (at_min(11), 60.0)]
        );
    }

    #[test]
    fn note_ignores_a_reading_that_is_not_after_the_last_point() {
        let db = db();
        note(&db, "org", 10.0, at_min(10)).unwrap();
        note(&db, "org", 9.0, at_min(10)).unwrap();
        note(&db, "org", 8.0, at_min(5)).unwrap();

        assert_eq!(kept(&db, "org"), [(at_min(10), 10.0)]);
    }

    #[test]
    fn note_ignores_an_amount_that_is_not_a_number() {
        let db = db();
        note(&db, "org", f64::NAN, at_min(0)).unwrap();
        note(&db, "org", f64::INFINITY, at_min(1)).unwrap();

        assert_eq!(kept(&db, "org"), []);
    }

    #[test]
    fn note_keeps_each_org_on_its_own() {
        let db = db();
        note(&db, "a", 10.0, at_min(0)).unwrap();
        note(&db, "b", 20.0, at_min(0)).unwrap();
        note(&db, "a", 9.0, at_min(30)).unwrap();

        assert_eq!(kept(&db, "a"), [(at_min(0), 10.0), (at_min(30), 9.0)]);
        assert_eq!(kept(&db, "b"), [(at_min(0), 20.0)]);
    }

    #[test]
    fn note_drops_points_older_than_45_days() {
        let db = db();
        note(&db, "org", 10.0, T0).unwrap();
        note(&db, "org", 9.0, T0 + 44 * DAY_MS).unwrap();
        assert_eq!(kept(&db, "org").len(), 2);

        note(&db, "org", 8.0, T0 + 46 * DAY_MS).unwrap();

        assert_eq!(
            kept(&db, "org"),
            [(T0 + 44 * DAY_MS, 9.0), (T0 + 46 * DAY_MS, 8.0)]
        );
    }

    #[test]
    fn note_keeps_the_newest_4000_points() {
        let db = db();
        // every reading is higher than the last, so none of them is collapsed
        for i in 0..4005 {
            note(&db, "org", i as f64, at_min(i)).unwrap();
        }

        let points = kept(&db, "org");
        assert_eq!(points.len(), 4000);
        assert_eq!(points[0], (at_min(5), 5.0));
        assert_eq!(points[3999], (at_min(4004), 4004.0));
    }

    // ── note_user ─────────────────────────────────────────────────────────

    fn user(json: &str) -> OfoxUserInfo {
        serde_json::from_str(json).expect("user info")
    }

    #[test]
    fn note_user_keeps_the_balance_under_the_balance_org() {
        let db = db();
        let u = user(
            r#"{"id":"u","email":null,"name":null,"org_id":"org_user","avatar_url":null,
                "balance":{"organization_id":"org_wallet","balance":12.5,"currency":"USD"}}"#,
        );
        note_user(&db, &u, at_min(0)).unwrap();

        assert_eq!(kept(&db, "org_wallet"), [(at_min(0), 12.5)]);
        assert_eq!(kept(&db, "org_user"), []);
    }

    #[test]
    fn note_user_falls_back_to_the_user_org() {
        let db = db();
        let u = user(
            r#"{"email":null,"name":null,"org_id":"org_user","avatar_url":null,
                "balance":{"balance":3.0}}"#,
        );
        note_user(&db, &u, at_min(0)).unwrap();

        assert_eq!(kept(&db, "org_user"), [(at_min(0), 3.0)]);
    }

    #[test]
    fn note_user_keeps_nothing_without_an_org_a_balance_or_dollars() {
        let db = db();
        for json in [
            r#"{"email":null,"name":null,"org_id":null,"avatar_url":null,"balance":{"balance":3.0}}"#,
            r#"{"email":null,"name":null,"org_id":"o","avatar_url":null}"#,
            r#"{"email":null,"name":null,"org_id":"o","avatar_url":null,"balance":{}}"#,
            r#"{"email":null,"name":null,"org_id":"o","avatar_url":null,"balance":{"balance":3.0,"currency":"CNY"}}"#,
        ] {
            note_user(&db, &user(json), at_min(0)).unwrap();
        }

        assert_eq!(kept(&db, "o"), []);
    }

    // ── trend ─────────────────────────────────────────────────────────────

    /// Spent 10 a day for two days, with noise, after a top-up to 50. The two
    /// points before the top-up are not part of the fit.
    fn burn_after_top_up() -> Vec<BalancePoint> {
        vec![
            point(hours_ago(96.0), 5.0),
            point(hours_ago(72.0), 3.0),
            point(hours_ago(48.0), 50.0),
            point(hours_ago(36.0), 45.2),
            point(hours_ago(24.0), 39.8),
            point(hours_ago(12.0), 35.1),
            point(hours_ago(0.0), 30.0),
        ]
    }

    #[test]
    fn a_steady_burn_has_a_rate_and_a_run_out() {
        let t = trend(&burn_after_top_up(), NOW);

        assert_eq!(t.points.len(), 7);
        assert!((t.per_day.unwrap() - 10.0).abs() < 0.3, "{:?}", t.per_day);
        let runs_out_in_hours = (t.runs_out_at.unwrap() - NOW) as f64 / HOUR_MS as f64;
        assert!(
            (runs_out_in_hours - 72.0).abs() < 3.0,
            "{runs_out_in_hours}"
        );
        assert!(
            (t.days_left.unwrap() - 3.0).abs() < 0.15,
            "{:?}",
            t.days_left
        );
    }

    #[test]
    fn the_fit_starts_after_the_last_top_up() {
        let mut points = burn_after_top_up();
        let with_older = trend(&points, NOW);
        points.drain(..2);
        let without_older = trend(&points, NOW);

        assert_eq!(with_older.per_day, without_older.per_day);
        assert_eq!(with_older.runs_out_at, without_older.runs_out_at);
    }

    #[test]
    fn a_top_up_just_now_has_no_fit() {
        let points = [
            point(hours_ago(48.0), 20.0),
            point(hours_ago(24.0), 10.0),
            point(hours_ago(0.0), 60.0),
        ];
        let t = trend(&points, NOW);

        assert_eq!(t.points.len(), 3);
        assert_eq!((t.per_day, t.runs_out_at, t.days_left), (None, None, None));
    }

    #[test]
    fn points_within_an_hour_have_no_fit() {
        let points = [
            point(NOW - 20 * MINUTE_MS, 10.0),
            point(NOW - 10 * MINUTE_MS, 9.0),
            point(NOW, 8.0),
        ];
        let t = trend(&points, NOW);

        assert_eq!(t.points.len(), 3);
        assert_eq!((t.per_day, t.runs_out_at, t.days_left), (None, None, None));
    }

    #[test]
    fn two_points_have_no_fit() {
        let points = [point(hours_ago(48.0), 10.0), point(hours_ago(0.0), 5.0)];
        let t = trend(&points, NOW);

        assert_eq!(t.points.len(), 2);
        assert_eq!(t.per_day, None);
    }

    #[test]
    fn a_balance_that_is_not_spent_does_not_run_out() {
        let points = [
            point(hours_ago(48.0), 20.0),
            point(hours_ago(24.0), 20.0),
            point(hours_ago(0.0), 20.0),
        ];
        let t = trend(&points, NOW);

        assert_eq!(t.per_day, Some(0.0));
        assert_eq!((t.runs_out_at, t.days_left), (None, None));
    }

    #[test]
    fn a_run_out_further_than_a_year_is_not_said() {
        let points = [
            point(hours_ago(48.0), 1000.0),
            point(hours_ago(24.0), 999.99),
            point(hours_ago(0.0), 999.98),
        ];
        let t = trend(&points, NOW);

        assert!(t.per_day.unwrap() > 0.0);
        assert_eq!((t.runs_out_at, t.days_left), (None, None));
    }

    #[test]
    fn a_tiny_burn_does_not_overflow_the_run_out_timestamp() {
        let points = [
            point(hours_ago(48.0), 100.0),
            point(hours_ago(24.0), 100.0 - 1e-10),
            point(hours_ago(0.0), 100.0 - 2e-10),
        ];

        let t = trend(&points, NOW);

        assert_eq!(t.per_day, Some(0.0));
        assert_eq!((t.runs_out_at, t.days_left), (None, None));
    }

    #[test]
    fn a_balance_already_at_zero_runs_out_now() {
        let points = [
            point(hours_ago(48.0), 2.0),
            point(hours_ago(24.0), 1.0),
            point(hours_ago(0.0), 0.0),
        ];
        let t = trend(&points, NOW);

        assert_eq!(t.runs_out_at, Some(NOW));
        assert_eq!(t.days_left, Some(0.0));
    }

    #[test]
    fn the_fit_looks_back_seven_days_at_most() {
        // 10 a day for ten days, ending at 0: the burn is steady, so the rate
        // is the same over seven days as over ten, but the fit must not start
        // before the seventh day
        let points: Vec<BalancePoint> = (0..=10)
            .map(|d| point(NOW - (10 - d) * DAY_MS, 100.0 - 10.0 * d as f64))
            .collect();
        let mut recent = points.clone();
        recent.retain(|p| NOW - p.at <= FIT_SPAN_MS);
        // a lump of spending before the seventh day would change the rate if it were fitted
        let mut lumpy = points.clone();
        lumpy[0].amount = 1000.0;
        lumpy[1].amount = 500.0;

        assert_eq!(trend(&lumpy, NOW).per_day, trend(&recent, NOW).per_day);
    }

    #[test]
    fn one_point_within_the_span_shows_nothing() {
        let points = [point(NOW - 20 * DAY_MS, 10.0), point(hours_ago(0.0), 9.0)];

        assert_eq!(trend(&points, NOW), BalanceTrend::default());
    }

    #[test]
    fn points_older_than_14_days_are_not_shown() {
        let points = [
            point(NOW - 20 * DAY_MS, 10.0),
            point(NOW - 3 * DAY_MS, 9.0),
            point(NOW - 2 * DAY_MS, 8.5),
        ];

        assert_eq!(trend(&points, NOW).points, points[1..]);
    }

    #[test]
    fn trend_for_reads_the_kept_points() {
        let db = db();
        for (at, amount) in [
            (hours_ago(48.0), 50.0),
            (hours_ago(36.0), 45.0),
            (hours_ago(24.0), 40.0),
            (hours_ago(12.0), 35.0),
            (hours_ago(0.0), 30.0),
        ] {
            note(&db, "org", amount, at).unwrap();
        }
        note(&db, "other", 1.0, hours_ago(1.0)).unwrap();

        let t = trend_for(&db, "org", NOW).unwrap();

        assert_eq!(t.points.len(), 5);
        assert!((t.per_day.unwrap() - 10.0).abs() < 0.01);
        assert_eq!(
            trend_for(&db, "nobody", NOW).unwrap(),
            BalanceTrend::default()
        );
    }

    #[test]
    fn a_trend_serializes_in_camel_case_with_nulls() {
        let t = trend(&[point(1, 2.0), point(2, 1.0)], 3);

        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            serde_json::json!({
                "points": [{"at": 1, "amount": 2.0}, {"at": 2, "amount": 1.0}],
                "perDay": null,
                "runsOutAt": null,
                "daysLeft": null,
            })
        );
    }
}
