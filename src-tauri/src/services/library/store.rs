use super::LibrarySnapshot;
use crate::{error::AppError, Database};

pub(crate) fn load(conn: &rusqlite::Connection) -> Result<LibrarySnapshot, AppError> {
    use super::{Definitions, MachineState, DEFINITIONS_KEY, MACHINE_KEY};
    use rusqlite::OptionalExtension;
    let read = |key: &str| -> Result<Option<String>, AppError> {
        conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| AppError::Database(e.to_string()))
    };
    let definitions: Definitions = match read(DEFINITIONS_KEY)? {
        Some(raw) => serde_json::from_str(&raw)
            .map_err(|_| AppError::Config("library.stateUnreadable".into()))?,
        None => Definitions::default(),
    };
    let machine: MachineState = match read(MACHINE_KEY)? {
        Some(raw) => serde_json::from_str(&raw)
            .map_err(|_| AppError::Config("library.stateUnreadable".into()))?,
        None => MachineState::default(),
    };
    let snapshot = LibrarySnapshot {
        definitions,
        machine,
    };
    validate(&snapshot)?;
    Ok(snapshot)
}

fn receipt_key(receipt: &super::Receipt) -> Result<String, AppError> {
    Ok(match receipt.kind {
        super::ResourceKind::Instructions => format!("instructions:{}", receipt.physical_target),
        super::ResourceKind::Skills => format!("skills:{}", receipt.physical_target),
        super::ResourceKind::Mcp => {
            format!("mcp:{}:{}", receipt.physical_target, receipt.resource_id)
        }
    })
}

pub(crate) fn validate(snapshot: &LibrarySnapshot) -> Result<(), AppError> {
    if snapshot.definitions.schema_version != 1 || snapshot.machine.schema_version != 1 {
        return Err(AppError::Config("library.stateVersionUnsupported".into()));
    }
    if snapshot
        .definitions
        .instructions
        .iter()
        .any(|(id, set)| id != &set.id || set.revision == 0)
    {
        return Err(AppError::Config("library.stateUnreadable".into()));
    }
    let unreadable = || AppError::Config("library.stateUnreadable".into());
    let hash_valid =
        |hash: &str| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
    if snapshot
        .definitions
        .mcp
        .iter()
        .chain(snapshot.definitions.skills.iter())
        .any(|(id, meta)| id.is_empty() || meta.revision == 0 || !hash_valid(&meta.hash))
    {
        return Err(unreadable());
    }
    let validate_receipt = |key: &str, receipt: &super::Receipt| -> Result<(), AppError> {
        if receipt.resource_id.is_empty()
            || receipt.resource_revision == 0
            || !hash_valid(&receipt.definition_hash)
            || receipt.owners.is_empty()
            || receipt
                .owners
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != receipt.owners.len()
            || !std::path::Path::new(&receipt.path).is_absolute()
            || !std::path::Path::new(&receipt.physical_target).is_absolute()
            || key != receipt_key(receipt)?
        {
            return Err(unreadable());
        }
        if receipt.kind == super::ResourceKind::Instructions {
            let removed = super::patch_instructions(
                &receipt.owned_segment,
                Some(&receipt.owned_segment),
                None,
            )
            .map_err(|_| unreadable())?;
            if !removed.text.is_empty() {
                return Err(unreadable());
            }
        } else if receipt.kind == super::ResourceKind::Skills {
            super::skill_resource::validate_receipt(receipt)?;
        } else {
            let owned: super::mcp_codec::OwnedFields =
                serde_json::from_str(&receipt.owned_segment).map_err(|_| unreadable())?;
            super::mcp_codec::validate_owned(&owned)?;
            if owned.server_id != receipt.resource_id {
                return Err(unreadable());
            }
        }
        Ok(())
    };
    for (key, receipt) in &snapshot.machine.receipts {
        validate_receipt(key, receipt)?;
    }
    for (key, operation) in &snapshot.machine.operations {
        if let Some(routing) = &operation.routing {
            if routing.version != 1
                || !hash_valid(&routing.environment_hash)
                || routing.paths.iter().any(|(path, physical)| {
                    !std::path::Path::new(path).is_absolute()
                        || !std::path::Path::new(physical).is_absolute()
                })
            {
                return Err(unreadable());
            }
        }
        if key != &operation.id
            || operation.resource_revision == 0
            || !hash_valid(&operation.resource_hash)
            || operation.resource_id.is_empty()
            || ![
                "previewed",
                "prepared",
                "partial",
                "applied",
                "needsReview",
                "cancelled",
            ]
            .contains(&operation.status.as_str())
        {
            return Err(unreadable());
        }
        for target in &operation.targets {
            for (path, source) in &target.source_fingerprints {
                if !std::path::Path::new(path).is_absolute()
                    || !std::path::Path::new(&source.physical_target).is_absolute()
                    || (!source.hash.is_empty()
                        && source.hash != "missing"
                        && !hash_valid(&source.hash))
                {
                    return Err(unreadable());
                }
            }
            if !std::path::Path::new(&target.path).is_absolute()
                || !std::path::Path::new(&target.physical_target).is_absolute()
                || target.receipt_key
                    != match operation.kind {
                        super::ResourceKind::Instructions => {
                            format!("instructions:{}", target.physical_target)
                        }
                        super::ResourceKind::Skills => format!("skills:{}", target.physical_target),
                        super::ResourceKind::Mcp => {
                            format!("mcp:{}:{}", target.physical_target, operation.resource_id)
                        }
                    }
                || !["pending", "applied", "conflict", "failed", "needsReview"]
                    .contains(&target.status.as_str())
            {
                return Err(unreadable());
            }
            for receipt in [&target.previous_receipt, &target.receipt]
                .into_iter()
                .flatten()
            {
                validate_receipt(&target.receipt_key, receipt)?;
                if receipt.physical_target != target.physical_target
                    || receipt.kind != operation.kind
                {
                    return Err(unreadable());
                }
            }
            if let Some(receipt) = &target.receipt {
                if receipt.path != target.path
                    || receipt.resource_id != operation.resource_id
                    || receipt.resource_revision != operation.resource_revision
                    || receipt.definition_hash != operation.resource_hash
                    || receipt.owners != target.tools
                {
                    return Err(unreadable());
                }
            }
            if operation.kind == super::ResourceKind::Skills {
                super::skill_resource::validate_target(target, operation)?;
                continue;
            }
            if target.directory.is_some() || target.transfer.is_some() {
                return Err(unreadable());
            }
            if !target.after_hash.is_empty() {
                let compacted = target.status == "applied"
                    && target.after_text.is_empty()
                    && target.receipt.is_none()
                    && target.previous_receipt.is_none();
                if !hash_valid(&target.after_hash)
                    || (target.before_hash != "missing" && !hash_valid(&target.before_hash))
                    || (!compacted
                        && target.after_hash
                            != super::coordinator::fingerprint(Some(target.after_text.as_bytes())))
                {
                    return Err(unreadable());
                }
            } else if target.status == "pending" || target.status == "applied" {
                return Err(unreadable());
            }
            if !target.after_hash.is_empty() {
                if let Some(receipt) = &target.receipt {
                    if receipt.kind == super::ResourceKind::Instructions {
                        if target.after_text.matches(&receipt.owned_segment).count() != 1 {
                            return Err(unreadable());
                        }
                    } else {
                        let owned: super::mcp_codec::OwnedFields =
                            serde_json::from_str(&receipt.owned_segment)
                                .map_err(|_| unreadable())?;
                        super::mcp_codec::verify_owned_text(&target.after_text, &owned)
                            .map_err(|_| unreadable())?;
                    }
                }
            }
        }
    }
    Ok(())
}

impl Database {
    /// Batch parent, non-executable children and standard receipts have one CAS
    /// boundary. No native write may be authorized by a detached parent record.
    pub(crate) fn library_commit_batch(
        &self,
        expected: &LibrarySnapshot,
        next: &LibrarySnapshot,
        expected_batches: &super::batch::BatchStore,
        next_batches: &super::batch::BatchStore,
    ) -> Result<(LibrarySnapshot, super::batch::BatchStore), AppError> {
        validate(next)?;
        super::batch::validate_store(next_batches, next)?;
        let mut conn = crate::database::lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Database(e.to_string()))?;
        let current = load(&tx)?;
        if &current != expected || &super::batch::load_on(&tx, &current)? != expected_batches {
            return Err(AppError::Config("library.revisionChanged".into()));
        }
        if next.machine.epoch < expected.machine.epoch {
            return Err(AppError::Config("library.epochChanged".into()));
        }
        let increment = |revision: u64| {
            revision
                .checked_add(1)
                .ok_or_else(|| AppError::Config("library.revisionOverflow".into()))
        };
        let mut saved = next.clone();
        saved.definitions.revision = increment(expected.definitions.revision)?;
        saved.machine.revision = increment(expected.machine.revision)?;
        let mut batches = next_batches.clone();
        batches.revision = increment(expected_batches.revision)?;
        let values = [
            (
                super::DEFINITIONS_KEY,
                serde_json::to_string(&saved.definitions),
            ),
            (super::MACHINE_KEY, serde_json::to_string(&saved.machine)),
            (super::batch::SETTING_KEY, serde_json::to_string(&batches)),
        ];
        for (key, value) in values {
            let value = value.map_err(|_| AppError::Config("library.stateUnreadable".into()))?;
            if key == super::batch::SETTING_KEY && value.len() > super::batch::MAX_STORE_BYTES {
                return Err(AppError::Config("library.batchTooLarge".into()));
            }
            tx.execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
                rusqlite::params![key, value],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }
        tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
        Ok((saved, batches))
    }
    pub fn library_snapshot(&self) -> Result<LibrarySnapshot, AppError> {
        let conn = crate::database::lock_conn!(self.conn);
        load(&conn)
    }

    /// Both formal JSON contracts change in one SQLite transaction, with CAS.
    pub fn library_commit(
        &self,
        expected: &LibrarySnapshot,
        next: &LibrarySnapshot,
    ) -> Result<LibrarySnapshot, AppError> {
        self.library_commit_mcp(expected, next, None)
    }

    pub(crate) fn library_commit_mcp(
        &self,
        expected: &LibrarySnapshot,
        next: &LibrarySnapshot,
        server: Option<&crate::app_config::McpServer>,
    ) -> Result<LibrarySnapshot, AppError> {
        self.library_commit_resources(expected, next, server, None)
    }

    pub(crate) fn library_commit_resources(
        &self,
        expected: &LibrarySnapshot,
        next: &LibrarySnapshot,
        server: Option<&crate::app_config::McpServer>,
        delete_id: Option<&str>,
    ) -> Result<LibrarySnapshot, AppError> {
        self.library_commit_all(expected, next, server, delete_id, None, None)
    }
    pub(crate) fn library_commit_skill(
        &self,
        expected: &LibrarySnapshot,
        next: &LibrarySnapshot,
        skill: Option<&crate::app_config::InstalledSkill>,
        delete_id: Option<&str>,
    ) -> Result<LibrarySnapshot, AppError> {
        self.library_commit_all(expected, next, None, None, skill, delete_id)
    }
    fn library_commit_all(
        &self,
        expected: &LibrarySnapshot,
        next: &LibrarySnapshot,
        server: Option<&crate::app_config::McpServer>,
        delete_id: Option<&str>,
        skill: Option<&crate::app_config::InstalledSkill>,
        delete_skill_id: Option<&str>,
    ) -> Result<LibrarySnapshot, AppError> {
        use super::{DEFINITIONS_KEY, MACHINE_KEY};
        validate(next)?;
        let mut conn = crate::database::lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Database(e.to_string()))?;
        if &load(&tx)? != expected {
            return Err(AppError::Config("library.revisionChanged".into()));
        }
        if next.machine.epoch < expected.machine.epoch {
            return Err(AppError::Config("library.epochChanged".into()));
        }
        let mut saved = next.clone();
        saved.definitions.revision = expected
            .definitions
            .revision
            .checked_add(1)
            .ok_or_else(|| AppError::Config("library.revisionOverflow".into()))?;
        saved.machine.revision = expected
            .machine
            .revision
            .checked_add(1)
            .ok_or_else(|| AppError::Config("library.revisionOverflow".into()))?;
        for (key, value) in [
            (DEFINITIONS_KEY, serde_json::to_string(&saved.definitions)),
            (MACHINE_KEY, serde_json::to_string(&saved.machine)),
        ] {
            let value = value.map_err(|e| AppError::Config(e.to_string()))?;
            tx.execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES (?1,?2)",
                rusqlite::params![key, value],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }
        if let Some(server) = server {
            let expected_meta = expected.definitions.mcp.get(&server.id);
            if let Some(meta) = expected_meta {
                let stored: String = tx
                    .query_row(
                        "SELECT server_config FROM mcp_servers WHERE id=?1",
                        [&server.id],
                        |row| row.get(0),
                    )
                    .map_err(|_| AppError::Config("library.revisionChanged".into()))?;
                let value: serde_json::Value = serde_json::from_str(&stored)
                    .map_err(|_| AppError::Config("library.stateUnreadable".into()))?;
                if super::mcp_resource::spec_hash(&value)? != meta.hash {
                    return Err(AppError::Config("library.revisionChanged".into()));
                }
            }
            tx.execute("INSERT OR REPLACE INTO mcp_servers (id,name,server_config,description,homepage,docs,tags,enabled_claude,enabled_codex,enabled_gemini,enabled_opencode,enabled_hermes) VALUES (?1,?2,?3,?4,?5,?6,?7,0,0,0,0,0)",rusqlite::params![server.id,server.name,serde_json::to_string(&server.server).map_err(|_|AppError::Config("library.invalidMcp".into()))?,server.description,server.homepage,server.docs,serde_json::to_string(&server.tags).map_err(|_|AppError::Config("library.invalidMcp".into()))?]).map_err(|e|AppError::Database(e.to_string()))?;
        }
        if let Some(id) = delete_id {
            tx.execute("DELETE FROM mcp_servers WHERE id=?1", [id])
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        if let Some(skill) = skill {
            super::skill_resource::validate_skill(skill)?;
            if let Some(meta) = expected.definitions.skills.get(&skill.id) {
                let stored = Database::skill_from_connection(&tx, &skill.id)?
                    .ok_or_else(|| AppError::Config("library.revisionChanged".into()))?;
                super::skill_resource::validate_skill(&stored)?;
                if super::skill_resource::spec_hash(&stored)? != meta.hash {
                    return Err(AppError::Config("library.revisionChanged".into()));
                }
            }
            tx.execute("INSERT OR REPLACE INTO skills (id,name,description,directory,repo_owner,repo_name,repo_branch,readme_url,enabled_claude,enabled_codex,enabled_gemini,enabled_opencode,enabled_hermes,installed_at,content_hash,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0,0,0,0,0,?9,?10,?11)", rusqlite::params![skill.id,skill.name,skill.description,skill.directory,skill.repo_owner,skill.repo_name,skill.repo_branch,skill.readme_url,skill.installed_at,skill.content_hash,skill.updated_at]).map_err(|e| AppError::Database(e.to_string()))?;
        }
        if let Some(id) = delete_skill_id {
            tx.execute("DELETE FROM skills WHERE id=?1", [id])
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
        Ok(saved)
    }
}

/// Restoration carries definitions, never another machine's ownership proof.
pub(crate) fn restore_machine_state(
    source: &rusqlite::Connection,
    target: &rusqlite::Connection,
) -> Result<(), AppError> {
    let mut machine = load(source)?.machine;
    machine.epoch = machine
        .epoch
        .checked_add(1)
        .ok_or_else(|| AppError::Config("library.revisionOverflow".into()))?;
    machine.revision = machine
        .revision
        .checked_add(1)
        .ok_or_else(|| AppError::Config("library.revisionOverflow".into()))?;
    for operation in machine.operations.values_mut() {
        if operation.status == "previewed"
            || operation.status == "prepared"
            || operation.status == "partial"
        {
            operation.status = "needsReview".into();
            for step in &mut operation.targets {
                if step.status != "applied" {
                    step.status = "needsReview".into();
                    step.reason = Some("library.epochChanged".into());
                }
            }
        }
    }
    let raw = serde_json::to_string(&machine).map_err(|e| AppError::Database(e.to_string()))?;
    let tx = target
        .unchecked_transaction()
        .map_err(|e| AppError::Database(e.to_string()))?;
    tx.execute(
        "INSERT OR REPLACE INTO settings(key,value) VALUES (?1,?2)",
        rusqlite::params![super::MACHINE_KEY, raw],
    )
    .map_err(|e| AppError::Database(e.to_string()))?;
    super::batch::restore_local(source, &tx, machine.epoch)?;
    load(&tx)?;
    tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::library::{InstructionSet, DEFINITIONS_KEY, MACHINE_KEY};

    #[test]
    fn cas_is_atomic_and_survives_a_new_snapshot() {
        let db = Database::memory().unwrap();
        let initial = db.library_snapshot().unwrap();
        let mut next = initial.clone();
        next.definitions.instructions.insert(
            "fixture".into(),
            InstructionSet {
                id: "fixture".into(),
                name: "Fixture".into(),
                content: "shared".into(),
                revision: 1,
            },
        );
        let saved = db.library_commit(&initial, &next).unwrap();
        assert_eq!(saved, db.library_snapshot().unwrap());
        assert!(db.library_commit(&initial, &next).is_err());
        assert_eq!(db.library_snapshot().unwrap(), saved);
        assert_eq!(
            db.library_snapshot().unwrap().definitions.instructions["fixture"].content,
            "shared"
        );
    }

    #[test]
    fn corrupt_or_unknown_schema_is_evidence_not_an_empty_default() {
        for key in [DEFINITIONS_KEY, MACHINE_KEY] {
            let db = Database::memory().unwrap();
            db.set_setting(key, "broken").unwrap();
            assert!(db.library_snapshot().is_err());
            let mut snapshot = LibrarySnapshot::default();
            snapshot.definitions.schema_version = 999;
            db.set_setting(key, &serde_json::to_string(&snapshot.definitions).unwrap())
                .unwrap();
            assert!(db.library_snapshot().is_err());
            assert!(db
                .library_commit(&LibrarySnapshot::default(), &LibrarySnapshot::default())
                .is_err());
        }
    }
}
