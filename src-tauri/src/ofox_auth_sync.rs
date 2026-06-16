//! Cross-module helper: keep the bound tools' provider rows in sync with the
//! current OFox access_token.
//!
//! Lives outside `ofox_auth.rs` on purpose — that module knows about HTTP and
//! disk, not databases. This module knows about the database and the
//! `ofox-*` provider row layout. They're glued together by the
//! `ofox-auth-token-refreshed` Tauri event (see `lib.rs`).

use serde_json::{json, Value};

use crate::database::Database;

/// The static map of "which ofox-* provider belongs to which app + where the
/// token lives in its settings_config".
///
/// Must stay in sync with:
///   - `database/dao/providers_seed.rs::OFOX_SEEDS`
///   - `commands/ofox_auth.rs::ofox_provider_for`
///
/// Gemini intentionally omitted — its OfoxAI seed has no token field today
/// (Gemini CLI uses Google OAuth, not an OFox bearer).
const OFOX_TOKEN_TARGETS: &[(&str, &str, &[&str])] = &[
    // (app_type, provider_id, path-in-settings_config)
    ("claude", "ofox-claude", &["env", "ANTHROPIC_AUTH_TOKEN"]),
    ("codex", "ofox-codex", &["auth", "OPENAI_API_KEY"]),
];

/// Overwrite the leaf at `path` inside `settings` with `token`, creating any
/// missing intermediate objects.
fn write_token_at_path(
    settings: &mut Value,
    path: &[&str],
    token: &str,
) -> Result<(), String> {
    if path.is_empty() {
        return Err("token path is empty".into());
    }
    if !settings.is_object() {
        *settings = json!({});
    }
    let (last, ancestors) = path.split_last().expect("non-empty per check above");
    let mut cursor = settings;
    for key in ancestors {
        let obj = cursor
            .as_object_mut()
            .ok_or_else(|| format!("settings_config 路径 {key} 处不是对象"))?;
        if !obj.get(*key).map(|v| v.is_object()).unwrap_or(false) {
            obj.insert((*key).to_string(), json!({}));
        }
        cursor = obj.get_mut(*key).expect("just inserted");
    }
    let leaf = cursor
        .as_object_mut()
        .ok_or_else(|| format!("settings_config 路径 {last} 的父级不是对象"))?;
    leaf.insert((*last).to_string(), json!(token));
    Ok(())
}

/// Refresh the cached OFox access_token in every `ofox-*` provider row so
/// the proxy forwards subsequent requests with the new bearer.
///
/// No-op for providers that don't exist (e.g. user hasn't bound that app
/// yet). Errors on individual providers are logged and skipped — we want
/// the partial-success case (e.g. db blip on one row) to still update the
/// others so the user isn't left with all tools broken.
pub fn sync_token_to_ofox_providers(db: &Database, token: &str) -> Result<(), String> {
    let mut updated = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for (app_type, provider_id, path) in OFOX_TOKEN_TARGETS {
        match db.get_provider_by_id(provider_id, app_type) {
            Ok(Some(mut provider)) => {
                if let Err(e) = write_token_at_path(&mut provider.settings_config, path, token)
                {
                    errors.push(format!("{provider_id}: {e}"));
                    continue;
                }
                match db.update_provider_settings_config(
                    app_type,
                    provider_id,
                    &provider.settings_config,
                ) {
                    Ok(()) => {
                        updated += 1;
                    }
                    Err(e) => errors.push(format!("{provider_id}: 持久化失败: {e}")),
                }
            }
            Ok(None) => {
                // Provider row not in DB — could happen on a very fresh
                // install before seeds run, or if the user deleted the seed.
                // Either way, nothing to update; not an error.
            }
            Err(e) => errors.push(format!("{provider_id}: 读取失败: {e}")),
        }
    }

    log::info!(
        "[OfoxAuth] token-sync: updated {} ofox-* provider rows ({} errors)",
        updated,
        errors.len()
    );

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}
