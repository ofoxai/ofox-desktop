//! Low-balance OS-level alert.
//!
//! Pulls the latest wallet balance from OFox via [`OfoxAuthManager::refresh_user_info`]
//! and fires a system notification when:
//!
//!   1. user has the alert enabled
//!   2. session is `Active`
//!   3. balance < threshold
//!   4. edge-trigger latch is open (we never alerted on this threshold yet,
//!      OR the 24h cooldown has elapsed since the last alert)
//!
//! Triggering writes back the latch (`last_alert_threshold` /
//! `last_alert_at`) so subsequent rounds within 24h stay silent. Threshold
//! changes are handled in [`commands::settings::merge_settings_for_save`]
//! by clearing the latch.
//!
//! Why a 24h cooldown rather than "alert once until balance recovers"?
//! Recovery probes would require remembering "we alerted, now waiting for
//! balance to climb back above threshold" forever, which gets weird if the
//! user later raises the threshold. A blunt 24h timer covers the common
//! case (user tops up, gets reminder a day later if still low) without
//! the state machine.

use std::sync::Arc;

use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::RwLock;

use crate::ofox_auth::OfoxAuthManager;

/// Tauri event emitted when the alert actually fires (in addition to the
/// OS notification). Lets the in-app UI also react if it cares.
pub const LOW_BALANCE_ALERT_EVENT: &str = "ofox-low-balance-alert";

/// 24-hour cooldown between repeated alerts at the same threshold.
const COOLDOWN_MS: i64 = 24 * 60 * 60 * 1000;

/// One full evaluation pass — refresh user info, decide whether to alert,
/// fire the OS notification + emit the in-app event, write the latch back
/// to settings.
///
/// Returns `true` if a notification was attempted (regardless of OS
/// success). Errors from the OFox API are logged and the round is skipped
/// (we'll re-evaluate on the next tick).
pub async fn evaluate_and_notify(
    app: &AppHandle,
    manager_arc: &Arc<RwLock<OfoxAuthManager>>,
    now_ms: i64,
) -> bool {
    // ── Guard 1: user disabled the feature ────────────────────────────
    if !crate::settings::low_balance_alert_enabled() {
        return false;
    }

    // ── Guard 2: must be authenticated ────────────────────────────────
    {
        let manager = manager_arc.read().await;
        if !manager.is_authenticated() {
            log::debug!("[low-balance] skipping: not authenticated");
            return false;
        }
    }

    let threshold = crate::settings::low_balance_threshold_usd();
    if threshold <= 0.0 {
        log::warn!("[low-balance] threshold is <= 0 ({}), skipping", threshold);
        return false;
    }

    // ── Refresh balance ───────────────────────────────────────────────
    let user = {
        let manager = manager_arc.read().await;
        match manager.refresh_user_info().await {
            Ok(u) => u,
            Err(e) => {
                log::warn!("[low-balance] refresh_user_info failed: {}", e);
                return false;
            }
        }
    };

    let balance = match user.balance.as_ref().and_then(|b| b.balance) {
        Some(v) => v,
        None => {
            log::debug!("[low-balance] balance field missing, skipping");
            return false;
        }
    };

    // ── Currency guard ────────────────────────────────────────────────
    // OFox today only quotes USD; if a different currency shows up we
    // can't meaningfully compare to the user's USD-typed threshold.
    if let Some(cur) = user.balance.as_ref().and_then(|b| b.currency.clone()) {
        if !cur.eq_ignore_ascii_case("USD") {
            log::warn!("[low-balance] non-USD currency {:?}, skipping", cur);
            return false;
        }
    }

    if !should_alert(balance, threshold, now_ms) {
        return false;
    }

    // ── Fire ──────────────────────────────────────────────────────────
    let title = "OFox 余额提醒";
    let body = format!("当前余额 ${:.2}，低于阈值 ${:.2}。", balance, threshold);
    if let Err(e) = app.notification().builder().title(title).body(&body).show() {
        log::warn!("[low-balance] notify failed (continuing): {}", e);
    } else {
        log::info!("[low-balance] notification sent: {}", body);
    }

    // Update latch even if the OS notification failed — the user already
    // saw the in-app event (if anything is listening) and we don't want
    // to retry a permission-denied notification every tick.
    if let Err(e) = crate::settings::set_low_balance_latch(threshold, now_ms) {
        log::warn!("[low-balance] persisting latch failed: {}", e);
    }

    let _ = app.emit(
        LOW_BALANCE_ALERT_EVENT,
        serde_json::json!({
            "balance": balance,
            "threshold": threshold,
            "at": now_ms,
        }),
    );

    true
}

/// Pure decision function — easy to test.
///
/// Returns `true` iff:
///   - balance < threshold, AND
///   - latch is open (no prior alert at this threshold, or 24h has passed)
pub fn should_alert(balance: f64, threshold: f64, now_ms: i64) -> bool {
    if balance >= threshold {
        return false;
    }

    let last_threshold = crate::settings::low_balance_last_alert_threshold();
    let last_at = crate::settings::low_balance_last_alert_at();

    match (last_threshold, last_at) {
        // Never alerted before — fire.
        (None, _) | (_, None) => true,
        // Threshold matches a prior alert — must clear cooldown.
        // (`merge_settings_for_save` already clears the latch on threshold
        // change, so when both Some we know last_threshold == threshold.)
        (Some(_lt), Some(lat)) => now_ms - lat >= COOLDOWN_MS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The pure decision logic is tested via a parametric variant that
    // takes the latch state explicitly — production `should_alert` reads
    // global settings, which we don't want to mutate from unit tests.

    /// Mirror of `should_alert` that takes the latch state as args.
    fn decide(
        balance: f64,
        threshold: f64,
        now_ms: i64,
        last_threshold: Option<f64>,
        last_at: Option<i64>,
    ) -> bool {
        if balance >= threshold {
            return false;
        }
        match (last_threshold, last_at) {
            (None, _) | (_, None) => true,
            (Some(_), Some(lat)) => now_ms - lat >= COOLDOWN_MS,
        }
    }

    #[test]
    fn no_alert_when_balance_above_threshold() {
        assert!(!decide(20.0, 10.0, 1_000, None, None));
    }

    #[test]
    fn no_alert_when_balance_equals_threshold() {
        // strict less-than, not <=
        assert!(!decide(10.0, 10.0, 1_000, None, None));
    }

    #[test]
    fn alert_when_below_and_no_prior_alert() {
        assert!(decide(5.0, 10.0, 1_000, None, None));
    }

    #[test]
    fn no_alert_within_cooldown() {
        let last_at = 1_700_000_000_000;
        // 1 hour later — well within 24h cooldown
        let now = last_at + 60 * 60 * 1000;
        assert!(!decide(5.0, 10.0, now, Some(10.0), Some(last_at)));
    }

    #[test]
    fn alert_after_cooldown_expires() {
        let last_at = 1_700_000_000_000;
        // 25 hours later — past the 24h cooldown
        let now = last_at + 25 * 60 * 60 * 1000;
        assert!(decide(5.0, 10.0, now, Some(10.0), Some(last_at)));
    }

    #[test]
    fn alert_at_exact_cooldown_boundary() {
        let last_at = 1_700_000_000_000;
        let now = last_at + COOLDOWN_MS;
        assert!(decide(5.0, 10.0, now, Some(10.0), Some(last_at)));
    }

    #[test]
    fn alert_when_only_one_latch_field_set() {
        // Defensive: corrupted settings (only last_at, no last_threshold).
        // Treat as "no prior alert" and fire.
        assert!(decide(5.0, 10.0, 1_000, None, Some(500)));
        assert!(decide(5.0, 10.0, 1_000, Some(10.0), None));
    }
}
