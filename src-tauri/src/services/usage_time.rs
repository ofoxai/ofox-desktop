//! Shared natural-day boundaries for versioned usage and session analytics.
use crate::error::AppError;
use chrono::{Duration, LocalResult, NaiveDate, NaiveTime, TimeZone};

/// Earliest valid instant at or after local midnight, in Unix milliseconds.
/// Time-zone offsets have second precision. Inspect every local second rather
/// than rounding a midnight gap to a 30-minute grid. A skipped natural day may
/// resolve to the following day's first instant. A bounded search fails closed
/// for invalid calendars instead of pretending that local time was UTC.
pub(crate) fn local_day_start_ms<Tz: TimeZone>(tz: &Tz, date: NaiveDate) -> Result<i64, AppError> {
    let mut local = date.and_time(NaiveTime::MIN);
    for _ in 0..=172_800 {
        match tz.from_local_datetime(&local) {
            LocalResult::Single(at) => return Ok(at.timestamp_millis()),
            LocalResult::Ambiguous(a, b) => {
                return Ok(a.timestamp_millis().min(b.timestamp_millis()));
            }
            LocalResult::None => {
                local = local
                    .checked_add_signed(Duration::seconds(1))
                    .ok_or_else(invalid)?;
            }
        }
    }
    Err(invalid())
}

fn invalid() -> AppError {
    AppError::InvalidInput("usage.query.invalidTime".into())
}
