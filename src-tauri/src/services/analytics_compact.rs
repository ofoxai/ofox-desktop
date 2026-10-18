//! Lossless compact forms for the analytics cache's largest arrays.
//!
//! A listing reads every cached source in one query under a byte budget, and
//! per-event arrays that repeat 64-character keys on every line outgrew it: a
//! 460 MB Codex rollout kept 18 MB of activity alone. The same values are
//! written shorter: SHA-256 digests as one base64 run of their bytes, and
//! activity as runs of consecutive events that share a session, stream and
//! main flag, with times as deltas. Order and every value are kept, so a reader
//! sees exactly what the plain form held. The plain form is still read, and is
//! still written for anything the compact form cannot hold exactly.
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::session_native_activity::NativeActivityEvent;
use crate::error::AppError;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD_NO_PAD;
const HEX: &[u8; 16] = b"0123456789abcdef";

/// Appends the bytes of a lowercase hex SHA-256 digest; false for any other text.
fn digest_bytes(hex: &str, out: &mut Vec<u8>) -> bool {
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    if hex.len() != 64 {
        return false;
    }
    for pair in hex.as_bytes().chunks(2) {
        match (nibble(pair[0]), nibble(pair[1])) {
            (Some(high), Some(low)) => out.push(high << 4 | low),
            _ => return false,
        }
    }
    true
}

/// Digests as one base64 string of their bytes; none when one is not a
/// lowercase hex SHA-256 digest, so the values are kept as written.
fn pack<'a>(digests: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut bytes = Vec::new();
    for digest in digests {
        if !digest_bytes(digest, &mut bytes) {
            return None;
        }
    }
    Some(B64.encode(bytes))
}

fn unpack(packed: &str) -> Option<Vec<String>> {
    let bytes = B64.decode(packed).ok()?;
    if bytes.len() % 32 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks(32)
            .map(|digest| {
                let mut hex = String::with_capacity(64);
                for byte in digest {
                    hex.push(HEX[usize::from(byte >> 4)] as char);
                    hex.push(HEX[usize::from(byte & 15)] as char);
                }
                hex
            })
            .collect(),
    )
}

/// `#[serde(with)]` for a list of SHA-256 hex digests.
pub(crate) mod digests {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(values: &[String], s: S) -> Result<S::Ok, S::Error> {
        match pack(values.iter().map(String::as_str)) {
            Some(packed) => s.serialize_str(&packed),
            None => values.serialize(s),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form {
            Packed(String),
            Plain(Vec<String>),
        }
        match Form::deserialize(d)? {
            Form::Packed(packed) => {
                unpack(&packed).ok_or_else(|| serde::de::Error::custom("invalid packed digests"))
            }
            Form::Plain(values) => Ok(values),
        }
    }
}

/// Consecutive activity events of one session, stream and main flag.
#[derive(Serialize, Deserialize)]
struct Run {
    session: String,
    stream: String,
    main: bool,
    /// The events' keys, packed.
    events: String,
    /// The first event's time, then each one's change from the one before.
    at: Vec<i64>,
}

#[derive(Serialize, Deserialize)]
struct Runs {
    runs: Vec<Run>,
}

/// The events as runs; none when a key is not a digest or a delta overflows.
fn runs(events: &[NativeActivityEvent]) -> Option<Runs> {
    let mut runs: Vec<Run> = Vec::new();
    let mut keys: Vec<&str> = Vec::new();
    let mut previous = 0i64;
    for event in events {
        let continues = runs.last().is_some_and(|run| {
            run.session == event.session_key
                && run.stream == event.stream_key
                && run.main == event.main
        });
        if !continues {
            if let Some(run) = runs.last_mut() {
                run.events = pack(keys.drain(..))?;
            }
            runs.push(Run {
                session: event.session_key.clone(),
                stream: event.stream_key.clone(),
                main: event.main,
                events: String::new(),
                at: Vec::new(),
            });
            previous = 0;
        }
        let run = runs.last_mut()?;
        run.at.push(event.at_ms.checked_sub(previous)?);
        previous = event.at_ms;
        keys.push(&event.event_key);
    }
    if let Some(run) = runs.last_mut() {
        run.events = pack(keys.drain(..))?;
    }
    Some(Runs { runs })
}

fn events(runs: Runs) -> Option<Vec<NativeActivityEvent>> {
    let mut events = Vec::new();
    for run in runs.runs {
        let keys = unpack(&run.events)?;
        if keys.len() != run.at.len() {
            return None;
        }
        let mut at = 0i64;
        for (event_key, delta) in keys.into_iter().zip(run.at) {
            at = at.checked_add(delta)?;
            events.push(NativeActivityEvent {
                session_key: run.session.clone(),
                event_key,
                at_ms: at,
                main: run.main,
                stream_key: run.stream.clone(),
            });
        }
    }
    Some(events)
}

/// `#[serde(with)]` for a source's native activity events.
pub(crate) mod activity {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(
        events: &[NativeActivityEvent],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match runs(events) {
            Some(runs) => runs.serialize(s),
            None => events.serialize(s),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Vec<NativeActivityEvent>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form {
            Runs(Runs),
            Plain(Vec<NativeActivityEvent>),
        }
        match Form::deserialize(d)? {
            Form::Runs(runs) => {
                events(runs).ok_or_else(|| serde::de::Error::custom("invalid activity runs"))
            }
            Form::Plain(events) => Ok(events),
        }
    }
}

/// Writes an analytics part's plain arrays in the compact form; true when
/// anything changed. Everything else in the value is left as it was.
fn compact_value(value: &mut Value) -> bool {
    let mut changed = false;
    if let Some(slot) = value.get_mut("nativeActivity").filter(|v| v.is_array()) {
        // read in place: a value that cannot be taken apart stays as it was
        let compact = Vec::<NativeActivityEvent>::deserialize(&*slot)
            .ok()
            .and_then(|events| runs(&events))
            .and_then(|runs| serde_json::to_value(runs).ok());
        if let Some(compact) = compact {
            *slot = compact;
            changed = true;
        }
    }
    if let Some(receipt) = value.get_mut("codexNative") {
        for key in ["recordHashes", "eventSequence"] {
            let Some(slot) = receipt.get_mut(key) else {
                continue;
            };
            let packed = slot
                .as_array()
                .and_then(|items| items.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
                .and_then(pack);
            if let Some(packed) = packed {
                *slot = Value::String(packed);
                changed = true;
            }
        }
    }
    changed
}

/// Rewrites, one row at a time, the analytics parts an older version wrote in
/// the plain form, keeping each part's revision. A cache that outgrew the byte
/// budget of the one-query read is read again after this pass, without
/// parsing any source again. A row changed meanwhile is left for the next pass.
pub(crate) fn compact_legacy_on(conn: &Connection) -> Result<usize, AppError> {
    let paths = {
        let mut statement = conn.prepare(
            "SELECT source_path FROM session_meta_cache WHERE instr(meta_json, '\"nativeActivity\":[') > 0
             OR instr(meta_json, '\"recordHashes\":[') > 0 OR instr(meta_json, '\"eventSequence\":[') > 0",
        )?;
        let paths = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        paths
    };
    let mut rewritten = 0;
    for path in paths {
        let Some(raw) = conn
            .query_row(
                "SELECT meta_json FROM session_meta_cache WHERE source_path = ?1",
                [&path],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
        else {
            continue;
        };
        let Ok(mut root) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let Some(value) = root.pointer_mut("/parts/analytics/value") else {
            continue;
        };
        if !compact_value(value) {
            continue;
        }
        let next = serde_json::to_string(&root)
            .map_err(|e| AppError::Database(format!("analytics compaction: {e}")))?;
        rewritten += conn.execute(
            "UPDATE session_meta_cache SET meta_json = ?1 WHERE source_path = ?2 AND meta_json = ?3",
            params![next, path, raw],
        )?;
    }
    Ok(rewritten)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn digest(n: u8) -> String {
        format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest([n]))
    }

    fn event(session: &str, stream: &str, n: u8, at_ms: i64, main: bool) -> NativeActivityEvent {
        NativeActivityEvent {
            session_key: session.into(),
            event_key: digest(n),
            at_ms,
            main,
            stream_key: stream.into(),
        }
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Holder {
        #[serde(with = "activity")]
        activity: Vec<NativeActivityEvent>,
        #[serde(with = "digests")]
        hashes: Vec<String>,
    }

    #[test]
    fn digests_pack_to_one_string_and_read_back_exactly() {
        let hashes: Vec<_> = (0..5).map(digest).collect();
        let holder = Holder {
            activity: vec![],
            hashes: hashes.clone(),
        };
        let json = serde_json::to_value(&holder).unwrap();
        assert!(json["hashes"].is_string());
        assert_eq!(serde_json::from_value::<Holder>(json).unwrap(), holder);
        // the plain form is still read
        let plain = json!({"activity": [], "hashes": hashes});
        assert_eq!(serde_json::from_value::<Holder>(plain).unwrap(), holder);
    }

    #[test]
    fn a_value_that_is_no_digest_keeps_the_plain_form() {
        for odd in ["ABC", "not-a-digest", &digest(1).to_uppercase()] {
            let holder = Holder {
                activity: vec![],
                hashes: vec![digest(0), odd.to_string()],
            };
            let json = serde_json::to_value(&holder).unwrap();
            assert!(json["hashes"].is_array(), "{odd}");
            assert_eq!(serde_json::from_value::<Holder>(json).unwrap(), holder);
        }
    }

    #[test]
    fn activity_runs_keep_every_event_in_order() {
        let activity = vec![
            event("s1", "a:0", 1, 1_000, true),
            event("s1", "a:0", 2, 1_250, true),
            // a clock that went back
            event("s1", "a:0", 3, 900, true),
            event("s1", "a:1", 4, 2_000, true),
            event("s1", "a:1", 5, 2_100, false),
            event("s2", "", 6, 3_000, true),
            event("s1", "a:0", 7, 4_000, true),
        ];
        let holder = Holder {
            activity: activity.clone(),
            hashes: vec![],
        };
        let json = serde_json::to_value(&holder).unwrap();
        assert_eq!(json["activity"]["runs"].as_array().unwrap().len(), 5);
        assert_eq!(json["activity"]["runs"][0]["at"], json!([1_000, 250, -350]));
        assert_eq!(serde_json::from_value::<Holder>(json).unwrap(), holder);
        let plain = json!({"activity": activity, "hashes": []});
        assert_eq!(serde_json::from_value::<Holder>(plain).unwrap(), holder);
    }

    #[test]
    fn activity_with_a_key_that_is_no_digest_keeps_the_plain_form() {
        let mut odd = event("s1", "a:0", 1, 1_000, true);
        odd.event_key = "legacy-key".into();
        let holder = Holder {
            activity: vec![event("s1", "a:0", 2, 900, true), odd],
            hashes: vec![],
        };
        let json = serde_json::to_value(&holder).unwrap();
        assert!(json["activity"].is_array());
        assert_eq!(serde_json::from_value::<Holder>(json).unwrap(), holder);
    }

    #[test]
    fn the_compact_form_is_a_fraction_of_the_plain_one() {
        let session = digest(200);
        let stream = format!("{}:0", digest(201));
        let activity: Vec<_> = (0..1_000)
            .map(|n| NativeActivityEvent {
                session_key: session.clone(),
                event_key: digest((n % 250) as u8),
                at_ms: 1_791_983_792_038 + n * 7,
                main: true,
                stream_key: stream.clone(),
            })
            .collect();
        let plain = serde_json::to_string(&activity).unwrap().len();
        let compact = serde_json::to_string(&Holder {
            activity,
            hashes: vec![],
        })
        .unwrap()
        .len();
        assert!(compact * 5 < plain, "{compact} vs {plain}");
    }

    fn cache(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE session_meta_cache (source_path TEXT PRIMARY KEY, provider_id TEXT,
             session_id TEXT, stamp TEXT, parser_version INTEGER, meta_json TEXT, updated_at INTEGER);",
        )
        .unwrap();
    }

    #[test]
    fn a_plain_row_is_rewritten_compact_keeping_its_revision_and_other_fields() {
        let conn = Connection::open_in_memory().unwrap();
        cache(&conn);
        let activity = vec![
            event("s1", "a:0", 1, 1_000, true),
            event("s1", "a:0", 2, 1_100, true),
        ];
        let hashes: Vec<_> = (0..3).map(digest).collect();
        let row = json!({"formatVersion": 1, "parts": {
            "metadata": {"revision": "m1", "value": {"stamp": "x"}},
            "analytics": {"revision": "r1", "value": {
                "version": 1, "nativeActivity": activity, "extra": {"kept": true},
                "codexNative": {"recordHashes": hashes, "eventSequence": [digest(9)],
                    "legacyEvents": ["kept"]}}}}});
        conn.execute(
            "INSERT INTO session_meta_cache(source_path, provider_id, meta_json) VALUES ('a', 'codex', ?1)",
            [row.to_string()],
        )
        .unwrap();

        assert_eq!(compact_legacy_on(&conn).unwrap(), 1);
        let raw: String = conn
            .query_row("SELECT meta_json FROM session_meta_cache", [], |r| r.get(0))
            .unwrap();
        assert!(raw.len() < row.to_string().len());
        let saved: Value = serde_json::from_str(&raw).unwrap();
        let analytics = &saved["parts"]["analytics"];
        assert_eq!(analytics["revision"], "r1");
        assert_eq!(analytics["value"]["extra"], json!({"kept": true}));
        assert_eq!(
            analytics["value"]["codexNative"]["legacyEvents"],
            json!(["kept"])
        );
        assert_eq!(saved["parts"]["metadata"], row["parts"]["metadata"]);
        let read: Vec<NativeActivityEvent> =
            activity::deserialize(analytics["value"]["nativeActivity"].clone()).unwrap();
        assert_eq!(read, activity);
        let read: Vec<String> =
            digests::deserialize(analytics["value"]["codexNative"]["recordHashes"].clone())
                .unwrap();
        assert_eq!(read, hashes);

        // nothing is left to do
        assert_eq!(compact_legacy_on(&conn).unwrap(), 0);
    }
}
