//! Native-thread projection shared by analysis, list amounts and effective detail.
use super::*;
use crate::services::codex_native;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeReceipt {
    pub version: u8,
    pub native_thread_id: String,
    pub billing_session_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub source_digest: String,
    #[serde(with = "crate::services::analytics_compact::digests")]
    pub event_sequence: Vec<String>,
    pub strong_events: bool,
    #[serde(default, with = "crate::services::analytics_compact::digests")]
    pub record_hashes: Vec<String>,
    #[serde(default)]
    pub prefix_proven: bool,
    #[serde(default)]
    pub legacy_events: Vec<String>,
    #[serde(default)]
    pub legacy_intervals: BTreeMap<String, String>,
}

pub(super) fn parse(spec: &SourceSpec, now: i64) -> Result<Entry, AppError> {
    let native = codex_native::read(&spec.path)?;
    let mut entry = empty_entry(now);
    entry.native_method = true;
    entry.priority = 2;
    entry.reasons = native.reasons.iter().cloned().collect();
    let Some(sid) = &native.native_thread_id else {
        entry.status = "partial".into();
        entry.succeeded_at_ms = Some(now);
        return Ok(entry);
    };
    let session_key = key("codex", sid);
    let project_path = native.project_dir.clone();
    let project = project_path
        .as_ref()
        .map_or(Project::Unparsed, |p| Project::Known(project_key(p)));
    let meta = crate::session_manager::SessionMeta {
        provider_id: "codex".into(),
        session_id: sid.clone(),
        source_path: Some(spec.path.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let title = native
        .title_hint
        .clone()
        .or_else(|| {
            project_path
                .as_ref()
                .and_then(|p| Path::new(p).file_name())
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| sid.clone());
    let message_reasons: Vec<_> = native
        .reasons
        .iter()
        .filter(|r| {
            matches!(
                r.as_str(),
                "unprovenInheritedBoundary"
                    | "malformedNativeRecord"
                    | "missingTimestamp"
                    | "sourceLineBudgetExceeded"
                    | "projectionBudgetExceeded"
                    | "activityTimeUnavailable"
                    | "activityIdentityUnavailable"
            )
        })
        .cloned()
        .collect();
    entry.sessions.push(SessionDescriptor {
        identity: SessionIdentity {
            key: session_key.clone(),
            tool_id: "codex".into(),
            project,
            verified_root_key: None,
        },
        native_session_id: sid.clone(),
        title: crate::redact::scrub(&title).into_owned(),
        project_label: project_path
            .as_ref()
            .and_then(|p| Path::new(p).file_name())
            .map(|p| p.to_string_lossy().into_owned()),
        project_path,
        manager_key: Some(crate::session_manager::managed::key(&meta)),
        source_available: true,
        messages_supported: true,
        message_reasons,
    });
    let mut models: BTreeMap<String, codex_thread_usage::Model> = BTreeMap::new();
    let mut event_sequence = Vec::new();
    for event in &native.events {
        let Some(at_ms) = event.at_ms else { continue };
        if event.native_thread_id != *sid {
            continue;
        }
        let event_key = key("codex", &event.key);
        event_sequence.push(event_key.clone());
        entry.facts.push(UsageFact {
            event_key,
            session_key: Some(session_key.clone()),
            tool_id: "codex".into(),
            source_id: "codex_session".into(),
            model: event.model.clone(),
            at_ms,
            buckets: event.buckets,
            complete: event.complete,
            cache_write_1h: 0,
        });
        let m = models
            .entry(event.model.clone())
            .or_insert_with(|| codex_thread_usage::Model {
                model: event.model.clone(),
                buckets: [0; 4],
                records: 0,
            });
        for (sum, value) in m.buckets.iter_mut().zip(event.buckets) {
            *sum = sum
                .checked_add(value)
                .ok_or_else(|| invalid("nativeTokenOverflow"))?;
        }
        m.records = m
            .records
            .checked_add(1)
            .ok_or_else(|| invalid("nativeTokenOverflow"))?;
    }
    entry.messages = native
        .messages
        .iter()
        .filter_map(|m| {
            Some(MessageEvent {
                session_key: session_key.clone(),
                native_id: key("codex", &format!("message:{sid}:{}", m.key)),
                at_ms: m.at_ms?,
                role: if m.role == "user" {
                    Role::User
                } else {
                    Role::Assistant
                },
                main: true,
            })
        })
        .collect();
    entry.native_activity = native
        .activity
        .iter()
        .map(
            |e| super::super::session_native_activity::NativeActivityEvent {
                session_key: session_key.clone(),
                event_key: key("codex", &format!("activity:{sid}:{}", e.key)),
                at_ms: e.at_ms,
                main: true,
                stream_key: format!("{}:{}", native.source_digest, e.stream_segment),
            },
        )
        .collect();
    let facts_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&entry.facts).map_err(|_| invalid("invalidCacheValue"))?)
    );
    let mut projection_reasons = entry.reasons.clone();
    if models.is_empty() {
        projection_reasons.push("nativeUsageUnavailable".into());
    }
    entry.thread_usage = Some(codex_thread_usage::Projection {
        version: 1,
        incomplete_models: vec![],
        native_thread_id: Some(sid.clone()),
        billing_session_id: native.billing_session_id.clone(),
        parent_session_id: native.parent_thread_id.clone(),
        state: if models.is_empty() {
            "unavailable"
        } else {
            "available"
        }
        .into(),
        reasons: projection_reasons,
        models: models.into_values().collect(),
        facts_hash: Some(facts_hash),
        source_digest: Some(native.source_digest.clone()),
    });
    entry.codex_native = Some(NativeReceipt {
        version: codex_native::VERSION,
        native_thread_id: sid.clone(),
        billing_session_id: native.billing_session_id,
        parent_thread_id: native.parent_thread_id,
        source_digest: native.source_digest,
        event_sequence,
        strong_events: native.events.iter().all(|e| e.response_id.is_some()),
        record_hashes: native.record_hashes,
        prefix_proven: native.complete,
        legacy_events: native
            .events
            .iter()
            .filter(|e| e.response_id.is_none())
            .map(|e| key("codex", &e.key))
            .collect(),
        legacy_intervals: native
            .events
            .iter()
            .filter_map(|event| Some((key("codex", &event.key), event.interval_identity.clone()?)))
            .collect(),
    });
    entry.status = if entry.reasons.is_empty() {
        "ok"
    } else {
        "partial"
    }
    .into();
    entry.succeeded_at_ms = Some(now);
    Ok(entry)
}

/// Cross-file legacy equality is not a response identity. Only complete native
/// record prefixes or byte-identical replicas establish a shared history.
/// Work is O(total cached facts + sorting sources), rather than source pairs.
pub(super) fn reconcile(
    entries: &mut [(String, Entry)],
    coverage: &mut BTreeMap<String, super::super::session_analytics_listing::ThreadUsageCoverage>,
) {
    fn compatible(a: &NativeReceipt, b: &NativeReceipt) -> bool {
        a.source_digest == b.source_digest
            || a.prefix_proven
                && b.prefix_proven
                && !a.record_hashes.is_empty()
                && !b.record_hashes.is_empty()
                && (a.record_hashes.starts_with(&b.record_hashes)
                    || b.record_hashes.starts_with(&a.record_hashes))
    }
    fn shared_prefix(entries: &[(String, Entry)], ids: &BTreeSet<usize>) -> bool {
        let Some(&first) = ids.iter().min_by_key(|i| {
            entries[**i]
                .1
                .codex_native
                .as_ref()
                .unwrap()
                .record_hashes
                .len()
        }) else {
            return true;
        };
        let shortest = entries[first].1.codex_native.as_ref().unwrap();
        ids.iter()
            .all(|i| compatible(shortest, entries[*i].1.codex_native.as_ref().unwrap()))
    }
    let mut threads = BTreeMap::<String, Vec<usize>>::new();
    for (i, (provider, entry)) in entries.iter().enumerate() {
        if provider == "codex" && entry.available && entry.status != "failed" {
            if let Some(receipt) = &entry.codex_native {
                threads
                    .entry(receipt.native_thread_id.clone())
                    .or_default()
                    .push(i);
            }
        }
    }
    for ids in threads.values().filter(|ids| ids.len() > 1) {
        let mut semantic = BTreeMap::<(i64, String, [u64; 4]), BTreeSet<usize>>::new();
        let mut activity = BTreeMap::<String, BTreeSet<usize>>::new();
        let mut intervals = BTreeMap::<String, BTreeSet<(String, [u64; 4])>>::new();
        for &i in ids {
            for fact in &entries[i].1.facts {
                semantic
                    .entry((fact.at_ms, fact.model.clone(), fact.buckets))
                    .or_default()
                    .insert(i);
            }
            for event in &entries[i].1.native_activity {
                activity
                    .entry(event.event_key.clone())
                    .or_default()
                    .insert(i);
            }
            let receipt = entries[i].1.codex_native.as_ref().unwrap();
            for fact in &entries[i].1.facts {
                if let Some(interval) = receipt.legacy_intervals.get(&fact.event_key) {
                    intervals
                        .entry(interval.clone())
                        .or_default()
                        .insert((fact.model.clone(), fact.buckets));
                }
            }
        }
        let conflicting_intervals: BTreeSet<_> = intervals
            .into_iter()
            .filter(|(_, values)| values.len() > 1)
            .map(|(interval, _)| interval)
            .collect();
        let ambiguous: BTreeSet<_> = semantic
            .iter()
            .filter(|(_, origins)| origins.len() > 1 && !shared_prefix(entries, origins))
            .map(|(semantic, _)| semantic.clone())
            .collect();
        let ambiguous_activity: BTreeSet<_> = activity
            .iter()
            .filter(|(_, origins)| origins.len() > 1 && !shared_prefix(entries, origins))
            .map(|(id, _)| id.clone())
            .collect();
        for &i in ids {
            let entry = &mut entries[i].1;
            let legacy: BTreeSet<_> = entry
                .codex_native
                .as_ref()
                .unwrap()
                .legacy_events
                .iter()
                .cloned()
                .collect();
            let intervals = &entry.codex_native.as_ref().unwrap().legacy_intervals;
            let before = entry.facts.len();
            entry.facts.retain(|fact| {
                let keep = !intervals
                    .get(&fact.event_key)
                    .is_some_and(|interval| conflicting_intervals.contains(interval));
                if !keep {
                    note_thread_usage_gap(coverage, fact, "conflictingUsageReplica");
                }
                keep
            });
            if entry.facts.len() != before {
                entry.reasons.push("conflictingUsageReplica".into());
                entry.status = "partial".into();
            }
            let before = entry.facts.len();
            entry.facts.retain(|fact| {
                let keep = !legacy.contains(&fact.event_key)
                    || !ambiguous.contains(&(fact.at_ms, fact.model.clone(), fact.buckets));
                if !keep {
                    note_thread_usage_gap(coverage, fact, "unprovenLegacyReplica");
                }
                keep
            });
            if entry.facts.len() != before {
                entry.reasons.push("unprovenLegacyReplica".into());
                entry.status = "partial".into();
            }
            if !ambiguous_activity.is_empty() {
                let mut segment = 0usize;
                let mut removed = false;
                entry.native_activity.retain_mut(|e| {
                    if ambiguous_activity.contains(&e.event_key) {
                        segment += 1;
                        removed = true;
                        return false;
                    }
                    e.stream_key = format!("{}:verified:{segment}", e.stream_key);
                    true
                });
                if removed {
                    entry.reasons.push("activityIdentityUnavailable".into());
                    entry.status = "partial".into();
                    for descriptor in &mut entry.sessions {
                        descriptor
                            .message_reasons
                            .push("activityIdentityUnavailable".into());
                    }
                }
            }
        }
    }
}
