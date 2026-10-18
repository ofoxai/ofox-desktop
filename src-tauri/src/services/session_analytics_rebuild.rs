//! One CAS publication for the entire Codex source generation, never per-file commits.
use super::*;
pub(super) const GENERATION_KEY: &str = "analytics-codex-generation-v1";
pub(super) struct Generation {
    epoch: Option<String>,
    roots: Vec<PathBuf>,
    roster: BTreeSet<PathBuf>,
    pub pending: Vec<(String, Option<String>, Entry)>,
    stamps: Vec<(SourceSpec, Value)>,
    pub control: Option<Arc<super::super::usage_rebuild::Control>>,
    marker_token: Option<String>,
    pub enabled: bool,
    marker_current: bool,
    require_route_match: bool,
}
impl Generation {
    pub fn new(
        db: &Database,
        rows: &BTreeMap<String, Row>,
        specs: &[SourceSpec],
        roots: &[(String, PathBuf)],
        require_route_match: bool,
        control: Option<Arc<super::super::usage_rebuild::Control>>,
    ) -> Result<Self, AppError> {
        let roots: Vec<_> = roots
            .iter()
            .filter(|(p, _)| p == "codex")
            .map(|(_, r)| r.clone())
            .collect();
        let roster = specs
            .iter()
            .filter(|s| s.provider == "codex")
            .map(|s| s.path.clone())
            .collect::<BTreeSet<_>>();
        let epoch = {
            let conn = crate::database::lock_conn!(db.conn);
            source_cache::ledger_epoch_on(&conn)?
        };
        let marker_current = rows
            .get(GENERATION_KEY)
            .and_then(|r| r.entry.as_ref().ok())
            .and_then(Option::as_ref)
            .is_some_and(|e| generation_is_current(e, &epoch));
        Ok(Self {
            epoch,
            marker_current,
            require_route_match,
            enabled: !roots.is_empty() || !roster.is_empty() || rows.contains_key(GENERATION_KEY),
            roots,
            roster,
            pending: vec![],
            stamps: vec![],
            control,
            marker_token: rows.get(GENERATION_KEY).and_then(|r| r.token.clone()),
        })
    }
    pub fn check(&self) -> Result<(), AppError> {
        self.control.as_ref().map_or(Ok(()), |c| c.check())
    }
    pub fn needs_publication(&self) -> bool {
        !self.marker_current
    }
    pub fn observed(&mut self, spec: &SourceSpec, stamp: Option<Value>) -> Result<(), AppError> {
        self.check()?;
        if let Some(stamp) = stamp {
            self.stamps.push((spec.clone(), stamp));
        }
        if let Some(control) = &self.control {
            control.advance();
        }
        Ok(())
    }
    pub fn commit(&mut self, db: &Database) -> Result<(), AppError> {
        if !self.enabled {
            return Ok(());
        }
        let result = self.commit_inner(db);
        if let Some(control) = &self.control {
            control.finish(&result);
        }
        result
    }
    fn commit_inner(&mut self, db: &Database) -> Result<(), AppError> {
        self.check()?;
        if !self.marker_current
            && !self.roster.is_empty()
            && !self.pending.iter().any(|(_, _, entry)| {
                entry.native_method && entry.codex_native.is_some() && entry.status != "failed"
            })
        {
            return Err(invalid("statisticsRebuildRequired"));
        }
        let _route_barrier = self.require_route_match.then(crate::config_io::lock);
        if self.require_route_match
            && self.roots.as_slice() != [crate::codex_config::get_codex_config_dir()]
        {
            return Err(invalid("sourceRouteChangedRetry"));
        }
        // Verify every observed source again, including warm entries. No query
        // can receive a combination of generations when append/restore races us.
        for (spec, stamp) in &self.stamps {
            if source_revision(spec)? != *stamp {
                return Err(invalid("sourceChangedRetry"));
            }
        }
        if !self.roots.is_empty() {
            let roots: Vec<_> = self.roots.iter().map(|r| ("codex", r.clone())).collect();
            let after = super::super::session_analytics_discovery::discover_in(&roots);
            if !after.failures.is_empty()
                || after
                    .specs
                    .iter()
                    .map(|s| s.path.clone())
                    .collect::<BTreeSet<_>>()
                    != self.roster
            {
                return Err(invalid("sourceChangedRetry"));
            }
        }
        let mut conn = crate::database::lock_conn!(db.conn);
        if !self.marker_current
            && !self.pending.iter().any(|(_, _, entry)| {
                entry.native_method && entry.codex_native.is_some() && entry.status != "failed"
            })
            && legacy_history_on(&conn)?
        {
            return Err(invalid("statisticsRebuildRequired"));
        }
        let tx = conn.transaction()?;
        if source_cache::ledger_epoch_on(&tx)? != self.epoch {
            return Err(invalid("ledgerChangedRetry"));
        }
        if self.pending.is_empty() && self.marker_current {
            return Ok(());
        }
        for (path, token, entry) in &self.pending {
            self.check()?;
            source_cache::merge_on(
                &tx,
                path,
                "codex",
                None,
                Part::Analytics,
                token,
                Some(serde_json::to_value(entry).map_err(|_| invalid("invalidCacheValue"))?),
            )?;
        }
        let now = chrono::Utc::now().timestamp_millis();
        let mut marker = empty_entry(now);
        marker.native_method = true;
        marker.status = "ok".into();
        marker.succeeded_at_ms = Some(now);
        marker.stamp = Some(
            json!({"methodVersion":super::super::codex_native::VERSION,"epoch":self.epoch,"sources":self.roster.len()}),
        );
        source_cache::merge_on(
            &tx,
            GENERATION_KEY,
            "codex",
            None,
            Part::Analytics,
            &self.marker_token,
            Some(serde_json::to_value(marker).map_err(|_| invalid("invalidCacheValue"))?),
        )?;
        if let Some(control) = &self.control {
            control.publish(|| {
                tx.commit()?;
                Ok(())
            })
        } else {
            tx.commit()?;
            Ok(())
        }
    }
}
impl Drop for Generation {
    fn drop(&mut self) {
        if let Some(control) = &self.control {
            control.finish(&Err(invalid("rebuildInterrupted")));
        }
    }
}
