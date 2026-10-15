//! Frozen skill update candidates. Checks never change definitions or native files.
use super::{
    skill_artifact::{self as artifacts, ExtractedDir},
    skill_resource, ResourceRevision,
};
use crate::{
    app_config::InstalledSkill,
    error::AppError,
    services::skill::{SkillRepo, SkillService},
    Database,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
const MAX_PENDING: usize = 8;
const TTL: Duration = Duration::from_secs(600);
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}
#[derive(Clone)]
struct Frozen {
    row: InstalledSkill,
    meta: ResourceRevision,
    epoch: u64,
}
struct Candidate {
    frozen: Frozen,
    downloaded: ExtractedDir,
    source: PathBuf,
    execution: artifacts::ExecutionMap,
    hash: String,
    commit: String,
    next: InstalledSkill,
    created: Instant,
}
#[derive(Default)]
struct Pending {
    active: BTreeMap<String, String>,
    tickets: BTreeMap<String, Candidate>,
}
fn pending() -> &'static Mutex<Pending> {
    static STATE: OnceLock<Mutex<Pending>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(Pending::default()))
}
fn registry() -> Result<std::sync::MutexGuard<'static, Pending>, AppError> {
    pending().lock().map_err(|_| error("skillUpdateBusy"))
}
struct Active {
    id: String,
    token: String,
}
impl Drop for Active {
    fn drop(&mut self) {
        if let Ok(mut state) = pending().lock() {
            if state.active.get(&self.id) == Some(&self.token) {
                state.active.remove(&self.id);
            }
        }
    }
}
fn freeze(db: &Database, id: &str, revision: u64, epoch: u64) -> Result<Frozen, AppError> {
    let _io = crate::config_io::lock();
    let snapshot = db.library_snapshot()?;
    if snapshot.machine.epoch != epoch {
        return Err(error("epochChanged"));
    }
    let meta = snapshot
        .definitions
        .skills
        .get(id)
        .filter(|m| m.revision == revision)
        .ok_or_else(|| error("revisionChanged"))?
        .clone();
    let row = db
        .get_installed_skill(id)?
        .ok_or_else(|| error("resourceMissing"))?;
    if skill_resource::spec_hash(&row)? != meta.hash {
        return Err(error("revisionChanged"));
    }
    Ok(Frozen { row, meta, epoch })
}
fn verify(db: &Database, frozen: &Frozen) -> Result<(), AppError> {
    let current = freeze(db, &frozen.row.id, frozen.meta.revision, frozen.epoch)?;
    if current.meta != frozen.meta {
        return Err(error("revisionChanged"));
    }
    Ok(())
}
fn decode(raw: &str) -> Result<String, AppError> {
    let mut result = vec![];
    let mut bytes = raw.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hex = [
                bytes.next().ok_or_else(|| error("skillSourceAmbiguous"))?,
                bytes.next().ok_or_else(|| error("skillSourceAmbiguous"))?,
            ];
            let text = std::str::from_utf8(&hex).map_err(|_| error("skillSourceAmbiguous"))?;
            result.push(u8::from_str_radix(text, 16).map_err(|_| error("skillSourceAmbiguous"))?);
        } else {
            result.push(b);
        }
    }
    String::from_utf8(result).map_err(|_| error("skillSourceAmbiguous"))
}
fn tracking(row: &InstalledSkill) -> Result<Option<(SkillRepo, PathBuf)>, AppError> {
    let (owner, name) = match (&row.repo_owner, &row.repo_name) {
        (None, None) => return Ok(None),
        (Some(a), Some(b)) => (a, b),
        _ => return Err(error("skillRepositoryInvalid")),
    };
    if [owner, name].iter().any(|s| {
        s.is_empty()
            || s.len() > 100
            || [".", ".."].contains(&s.as_str())
            || !s
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    }) {
        return Err(error("skillRepositoryInvalid"));
    }
    let reference = row
        .repo_branch
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("HEAD");
    if reference.len() > 256 || reference.chars().any(char::is_control) {
        return Err(error("skillRepositoryInvalid"));
    }
    let raw = row
        .readme_url
        .as_deref()
        .ok_or_else(|| error("skillSourceAmbiguous"))?;
    if raw.chars().any(char::is_control) {
        return Err(error("skillSourceAmbiguous"));
    }
    let url = url::Url::parse(raw).map_err(|_| error("skillSourceAmbiguous"))?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(error("skillSourceAmbiguous"));
    }
    let path = decode(url.path())?;
    let tail = path
        .strip_prefix(&format!("/{owner}/{name}/blob/"))
        .or_else(|| path.strip_prefix(&format!("/{owner}/{name}/tree/")))
        .ok_or_else(|| error("skillSourceAmbiguous"))?;
    let (first, rest) = tail
        .split_once('/')
        .ok_or_else(|| error("skillSourceAmbiguous"))?;
    let by_sha =
        (first.len() == 40 && first.bytes().all(|b| b.is_ascii_hexdigit())).then_some(rest);
    let by_ref = tail.strip_prefix(&format!("{reference}/"));
    let doc = match (by_sha, by_ref) {
        (Some(a), Some(b)) if a != b => return Err(error("skillSourceAmbiguous")),
        (Some(a), _) | (_, Some(a)) => a,
        _ => return Err(error("skillSourceAmbiguous")),
    };
    let directory = if doc == "SKILL.md" {
        PathBuf::new()
    } else {
        artifacts::safe_relative(
            doc.strip_suffix("/SKILL.md")
                .ok_or_else(|| error("skillSourceAmbiguous"))?,
        )?
    };
    Ok(Some((
        SkillRepo {
            owner: owner.clone(),
            name: name.clone(),
            branch: reference.into(),
            enabled: true,
        },
        directory,
    )))
}
fn can_copy(hash: &str) -> Result<bool, AppError> {
    let tree = artifacts::path(hash)?;
    let container = tree.parent().ok_or_else(|| error("skillArtifactMissing"))?;
    Ok(match std::fs::symlink_metadata(container) {
        Ok(_) => artifacts::archive_entries(hash).is_ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    })
}
fn base(frozen: &Frozen, state: &str) -> Value {
    json!({"resourceId":frozen.row.id,"revision":frozen.meta.revision,"epoch":frozen.epoch,"state":state,"currentArtifactHash":frozen.row.content_hash,"checkedAtMs":chrono::Utc::now().timestamp_millis()})
}
fn reserve(db: &Database, frozen: &Frozen) -> Result<Active, AppError> {
    let snapshot = db.library_snapshot()?;
    let mut state = registry()?;
    state.tickets.retain(|_, c| {
        c.created.elapsed() < TTL
            && c.frozen.epoch == snapshot.machine.epoch
            && snapshot.definitions.skills.get(&c.frozen.row.id) == Some(&c.frozen.meta)
    });
    if state.active.contains_key(&frozen.row.id) {
        return Err(error("skillUpdateBusy"));
    }
    // A new explicit check replaces this resource's old, unconsumed candidate.
    state
        .tickets
        .retain(|_, c| c.frozen.row.id != frozen.row.id);
    if state.active.len() + state.tickets.len() >= MAX_PENDING {
        return Err(error("skillUpdateBusy"));
    }
    let token = uuid::Uuid::new_v4().to_string();
    state.active.insert(frozen.row.id.clone(), token.clone());
    Ok(Active {
        id: frozen.row.id.clone(),
        token,
    })
}
struct PreparedSource {
    source: PathBuf,
    execution: artifacts::ExecutionMap,
    hash: String,
    next: InstalledSkill,
}
fn prepare_source(
    frozen: &Frozen,
    repo: &SkillRepo,
    relative: &std::path::Path,
    downloaded: &ExtractedDir,
    commit: &str,
) -> Result<Option<PreparedSource>, AppError> {
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error("skillRepositoryInvalid"));
    }
    let source = downloaded.path().join(relative);
    if !source.is_dir() || !source.join("SKILL.md").is_file() {
        return Ok(None);
    }
    let execution = downloaded.execution_for(&source)?;
    let hash = artifacts::source_hash(&source, &execution)?;
    let (name, description) =
        SkillService::read_skill_name_desc(&source.join("SKILL.md"), &frozen.row.directory);
    let mut next = frozen.row.clone();
    next.name = name;
    next.description = description;
    next.updated_at = chrono::Utc::now().timestamp();
    let mut url =
        url::Url::parse("https://github.com/").map_err(|_| error("skillRepositoryInvalid"))?;
    {
        let mut parts = url
            .path_segments_mut()
            .map_err(|_| error("skillRepositoryInvalid"))?;
        parts.extend([repo.owner.as_str(), repo.name.as_str(), "blob", commit]);
        for part in relative.components() {
            parts.push(
                part.as_os_str()
                    .to_str()
                    .ok_or_else(|| error("invalidSkillPath"))?,
            );
        }
        parts.push("SKILL.md");
    }
    next.readme_url = Some(url.into());
    Ok(Some(PreparedSource {
        source,
        execution,
        hash,
        next,
    }))
}
pub(crate) async fn check_with<F, Fut>(
    db: &Database,
    id: &str,
    revision: u64,
    epoch: u64,
    download: F,
) -> Result<Value, AppError>
where
    F: FnOnce(SkillRepo) -> Fut,
    Fut: Future<Output = Result<(ExtractedDir, String), AppError>>,
{
    let (frozen, repo, relative, _active) = {
        let _io = crate::config_io::lock();
        let frozen = freeze(db, id, revision, epoch)?;
        let Some((repo, relative)) = tracking(&frozen.row)? else {
            return Ok(base(&frozen, "notTracked"));
        };
        let active = reserve(db, &frozen)?;
        (frozen, repo, relative, active)
    };
    let (downloaded, commit) =
        tokio::time::timeout(Duration::from_secs(60), download(repo.clone()))
            .await
            .map_err(|_| error("skillDownloadTimeout"))?
            .map_err(|_| error("skillDownloadFailed"))?;
    let _io = crate::config_io::lock();
    verify(db, &frozen)?;
    let Some(PreparedSource {
        source,
        execution,
        hash,
        next,
    }) = prepare_source(&frozen, &repo, &relative, &downloaded, &commit)?
    else {
        return Ok(base(&frozen, "sourceMissing"));
    };
    let mut result = base(
        &frozen,
        if frozen.row.content_hash.as_deref() == Some(&hash) {
            "upToDate"
        } else {
            "updateAvailable"
        },
    );
    let token = uuid::Uuid::new_v4().to_string();
    let conflicts = db
        .library_snapshot()?
        .machine
        .receipts
        .values()
        .filter(|r| {
            r.kind == super::ResourceKind::Skills
                && r.resource_id == id
                && !skill_resource::receipt_intact(r)
        })
        .map(|r| r.path.clone())
        .collect::<Vec<_>>();
    result["targetConflicts"] = json!(conflicts);
    result["sourceVerified"] = json!(skill_resource::artifact_for(db, id).is_ok());
    result["candidate"] = json!({"ticketId":token,"expiresAtMs":chrono::Utc::now().timestamp_millis()+TTL.as_millis() as i64,"artifactHash":hash,"commitSha":commit,"name":next.name,"description":next.description,"sourceUrl":next.readme_url});
    let copy_allowed = can_copy(&hash)?;
    result["candidate"]["canCopy"] = json!(copy_allowed);
    result["candidate"]["copyBlockedReason"] = json!(if copy_allowed {
        None
    } else {
        Some("library.skillArtifactHashOccupied")
    });
    let mut state = registry()?;
    state.active.remove(id);
    state.tickets.insert(
        token.clone(),
        Candidate {
            frozen,
            downloaded,
            source,
            execution,
            hash,
            commit,
            next,
            created: Instant::now(),
        },
    );
    drop(state);
    tokio::spawn(async move {
        tokio::time::sleep(TTL).await;
        if let Ok(mut state) = pending().lock() {
            if state
                .tickets
                .get(&token)
                .is_some_and(|candidate| candidate.created.elapsed() >= TTL)
            {
                state.tickets.remove(&token);
            }
        }
    });
    Ok(result)
}
pub async fn check(db: &Database, id: &str, revision: u64, epoch: u64) -> Result<Value, AppError> {
    check_with(db, id, revision, epoch, |repo| async move {
        SkillService::new()
            .download_repo(&repo)
            .await
            .map_err(|_| error("skillDownloadFailed"))
    })
    .await
}
pub fn confirm(
    db: &Database,
    ticket: &str,
    revision: u64,
    epoch: u64,
    mode: &str,
    name: Option<&str>,
    directory: Option<&str>,
) -> Result<Value, AppError> {
    if !["update", "copy"].contains(&mode) {
        return Err(error("invalidSkill"));
    }
    let _io = crate::config_io::lock();
    let mut state = registry()?;
    let c = state
        .tickets
        .get(ticket)
        .ok_or_else(|| error("skillUpdateExpired"))?;
    if c.created.elapsed() >= TTL {
        state.tickets.remove(ticket);
        return Err(error("skillUpdateExpired"));
    }
    if c.frozen.epoch != epoch || c.frozen.meta.revision != revision {
        return Err(error("revisionChanged"));
    }
    if let Err(failure) = verify(db, &c.frozen) {
        state.tickets.remove(ticket);
        return Err(failure);
    }
    if artifacts::source_hash(&c.source, &c.execution)
        .ok()
        .as_deref()
        != Some(&c.hash)
    {
        state.tickets.remove(ticket);
        return Err(error("skillArtifactChanged"));
    }
    let mut row = c.next.clone();
    if mode == "update" {
        skill_resource::artifact_for(db, &row.id).map_err(|_| error("skillSourceUnverified"))?;
        if db.library_snapshot()?.machine.receipts.values().any(|r| {
            r.kind == super::ResourceKind::Skills
                && r.resource_id == row.id
                && !skill_resource::receipt_intact(r)
        }) {
            return Err(error("skillTargetModified"));
        }
        if row.content_hash.as_deref() == Some(&c.hash) {
            let result = json!({"state":"unchanged","resourceId":row.id,"revision":revision,"artifactHash":c.hash,"commitSha":c.commit,"nativeApplyRequired":false});
            state.tickets.remove(ticket);
            return Ok(result);
        }
    } else {
        if !can_copy(&c.hash)? {
            return Err(error("skillArtifactHashOccupied"));
        }
        let directory = directory.ok_or_else(|| error("invalidSkill"))?;
        artifacts::safe_leaf(directory)?;
        if db
            .get_all_installed_skills()?
            .values()
            .any(|r| r.directory.eq_ignore_ascii_case(directory))
        {
            return Err(error("skillNameConflict"));
        }
        let name = name
            .filter(|name| !name.trim().is_empty() && name.len() <= 1024)
            .ok_or_else(|| error("invalidSkill"))?;
        row.id = format!("copy:{}", uuid::Uuid::new_v4());
        row.name = name.into();
        row.directory = directory.into();
        row.installed_at = chrono::Utc::now().timestamp();
    }
    let saved = skill_resource::save_draft_checked_with_execution(
        db,
        row,
        &c.source,
        if mode == "update" {
            Some(revision)
        } else {
            None
        },
        Some(&c.execution),
        Some(&c.hash),
    )?;
    let result = json!({"state":if mode=="update"{"updated"}else{"copied"},"resourceId":saved.id,"revision":if mode=="update"{revision+1}else{1},"artifactHash":saved.content_hash,"commitSha":c.commit,"nativeApplyRequired":mode=="update"});
    state.tickets.remove(ticket);
    Ok(result)
}
pub fn discard(ticket: &str) -> Result<bool, AppError> {
    Ok(registry()?.tickets.remove(ticket).is_some())
}
fn text(bytes: Option<&[u8]>) -> Option<String> {
    let bytes = bytes.filter(|b| b.len() <= 1024 * 1024)?;
    std::str::from_utf8(bytes)
        .ok()
        .filter(|t| !t.contains('\0'))
        .map(str::to_string)
}
pub fn compare(db: &Database, ticket: &str, relative: &str) -> Result<Value, AppError> {
    artifacts::safe_relative(relative)?;
    let _io = crate::config_io::lock();
    let mut state = registry()?;
    let c = state
        .tickets
        .get(ticket)
        .ok_or_else(|| error("skillUpdateExpired"))?;
    if c.created.elapsed() >= TTL {
        state.tickets.remove(ticket);
        return Err(error("skillUpdateExpired"));
    }
    if let Err(failure) = verify(db, &c.frozen) {
        state.tickets.remove(ticket);
        return Err(failure);
    }
    // Each entry list and its identity come from one bounded tree read.
    let after = artifacts::source_entries_checked(&c.source, &c.execution, &c.hash)?;
    let _keep_staging = &c.downloaded;
    let baseline = skill_resource::artifact_for(db, &c.frozen.row.id).and_then(|_| {
        artifacts::archive_entries(
            c.frozen
                .row
                .content_hash
                .as_deref()
                .ok_or_else(|| error("skillArtifactMissing"))?,
        )
    });
    let before_verified = baseline.is_ok();
    let before = baseline.map(|(_, entries)| entries).unwrap_or_default();
    let old = before
        .iter()
        .filter(|e| e.bytes.is_some())
        .map(|e| (e.path.clone(), e))
        .collect::<BTreeMap<_, _>>();
    let new = after
        .iter()
        .filter(|e| e.bytes.is_some())
        .map(|e| (e.path.clone(), e))
        .collect::<BTreeMap<_, _>>();
    let names = old
        .keys()
        .chain(new.keys())
        .collect::<std::collections::BTreeSet<_>>();
    if !names.iter().any(|path| path.as_str() == relative) {
        return Err(error("skillFileMissing"));
    }
    let files=names.into_iter().map(|path| {let before=old.get(path);let after=new.get(path);json!({"path":path,"status":if !before_verified {"unverified"} else {match (before,after) {(None,Some(_))=>"added",(Some(_),None)=>"removed",(Some(a),Some(b)) if a.bytes==b.bytes && a.executable==b.executable=>"unchanged",_=>"modified"}},"beforeExecutable":if before_verified {Some(before.is_some_and(|e|e.executable))}else{None},"afterExecutable":after.is_some_and(|e|e.executable)})}).collect::<Vec<_>>();
    Ok(
        json!({"beforeVerified":before_verified,"beforeState":if before_verified{"verified"}else{"unverified"},"canCopy":can_copy(&c.hash)?,"path":relative,"before":text(old.get(relative).and_then(|e|e.bytes.as_deref())),"after":text(new.get(relative).and_then(|e|e.bytes.as_deref())),"files":files}),
    )
}

pub(crate) async fn legacy_check_with<F, Fut>(
    db: &Database,
    mut download: F,
) -> Result<Vec<crate::services::skill::SkillUpdateInfo>, AppError>
where
    F: FnMut(SkillRepo) -> Fut,
    Fut: Future<Output = Result<(ExtractedDir, String), AppError>>,
{
    // Keep the legacy Vec contract without discarding failures or consuming UI tickets.
    let groups = {
        let _io = crate::config_io::lock();
        let snapshot = db.library_snapshot()?;
        let mut groups = BTreeMap::<(String, String, String), Vec<(Frozen, PathBuf)>>::new();
        for row in db.get_all_installed_skills()?.into_values() {
            let Some((repo, relative)) = tracking(&row)? else {
                continue;
            };
            let revision = snapshot
                .definitions
                .skills
                .get(&row.id)
                .ok_or_else(|| error("skillNeedsMigration"))?
                .revision;
            let frozen = freeze(db, &row.id, revision, snapshot.machine.epoch)?;
            skill_resource::artifact_for(db, &row.id)
                .map_err(|_| error("skillSourceUnverified"))?;
            groups
                .entry((repo.owner, repo.name, repo.branch))
                .or_default()
                .push((frozen, relative));
        }
        groups
    };
    if groups.is_empty() {
        return Err(error("skillLocalSource"));
    }
    let all = groups
        .values()
        .flatten()
        .map(|(f, _)| f.clone())
        .collect::<Vec<_>>();
    let mut updates = vec![];
    for ((owner, name, branch), rows) in groups {
        let repo = SkillRepo {
            owner,
            name,
            branch,
            enabled: true,
        };
        let (downloaded, commit) =
            tokio::time::timeout(Duration::from_secs(60), download(repo.clone()))
                .await
                .map_err(|_| error("skillDownloadTimeout"))?
                .map_err(|_| error("skillDownloadFailed"))?;
        let _io = crate::config_io::lock();
        for (frozen, relative) in rows {
            verify(db, &frozen)?;
            skill_resource::artifact_for(db, &frozen.row.id)
                .map_err(|_| error("skillSourceUnverified"))?;
            let prepared = prepare_source(&frozen, &repo, &relative, &downloaded, &commit)?
                .ok_or_else(|| error("skillSourceMissing"))?;
            if frozen.row.content_hash.as_deref() != Some(&prepared.hash) {
                updates.push(crate::services::skill::SkillUpdateInfo {
                    id: frozen.row.id,
                    name: frozen.row.name,
                    current_hash: frozen.row.content_hash,
                    remote_hash: prepared.hash,
                });
            }
        }
    }
    let _io = crate::config_io::lock();
    for frozen in &all {
        verify(db, frozen)?;
        skill_resource::artifact_for(db, &frozen.row.id)
            .map_err(|_| error("skillSourceUnverified"))?;
    }
    Ok(updates)
}
pub async fn legacy_check(
    db: &Database,
) -> Result<Vec<crate::services::skill::SkillUpdateInfo>, AppError> {
    legacy_check_with(db, |repo| async move {
        SkillService::new()
            .download_repo(&repo)
            .await
            .map_err(|_| error("skillDownloadFailed"))
    })
    .await
}
pub(crate) async fn legacy_update_with<F, Fut>(
    db: &Database,
    id: &str,
    download: F,
) -> Result<InstalledSkill, AppError>
where
    F: FnOnce(SkillRepo) -> Fut,
    Fut: Future<Output = Result<(ExtractedDir, String), AppError>>,
{
    let (frozen, repo, relative) = {
        let _io = crate::config_io::lock();
        let snapshot = db.library_snapshot()?;
        let revision = snapshot
            .definitions
            .skills
            .get(id)
            .ok_or_else(|| error("skillNeedsMigration"))?
            .revision;
        let frozen = freeze(db, id, revision, snapshot.machine.epoch)?;
        skill_resource::artifact_for(db, id).map_err(|_| error("skillSourceUnverified"))?;
        let (repo, relative) = tracking(&frozen.row)?.ok_or_else(|| error("skillLocalSource"))?;
        (frozen, repo, relative)
    };
    let (downloaded, commit) =
        tokio::time::timeout(Duration::from_secs(60), download(repo.clone()))
            .await
            .map_err(|_| error("skillDownloadTimeout"))?
            .map_err(|_| error("skillDownloadFailed"))?;
    let _io = crate::config_io::lock();
    verify(db, &frozen)?;
    skill_resource::artifact_for(db, id).map_err(|_| error("skillSourceUnverified"))?;
    if db.library_snapshot()?.machine.receipts.values().any(|r| {
        r.kind == super::ResourceKind::Skills
            && r.resource_id == id
            && !skill_resource::receipt_intact(r)
    }) {
        return Err(error("skillTargetModified"));
    }
    let prepared = prepare_source(&frozen, &repo, &relative, &downloaded, &commit)?
        .ok_or_else(|| error("skillSourceMissing"))?;
    if frozen.row.content_hash.as_deref() == Some(&prepared.hash) {
        return Ok(frozen.row);
    }
    skill_resource::save_draft_checked_with_execution(
        db,
        prepared.next,
        &prepared.source,
        Some(frozen.meta.revision),
        Some(&prepared.execution),
        Some(&prepared.hash),
    )
}
pub async fn legacy_update(db: &Database, id: &str) -> Result<InstalledSkill, AppError> {
    legacy_update_with(db, id, |repo| async move {
        SkillService::new()
            .download_repo(&repo)
            .await
            .map_err(|_| error("skillDownloadFailed"))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::super::{self as library, skill_resource};
    use super::*;
    use crate::app_config::InstalledSkill;
    use std::{
        fs,
        io::{Cursor, Write},
    };
    fn fixture(tracked: bool) -> Database {
        let db = Database::memory().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(
            source.path().join("SKILL.md"),
            "---\nname: sample\ndescription: Fixture\n---\nOld body",
        )
        .unwrap();
        fs::write(source.path().join("run.sh"), "echo fixture\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                source.path().join("run.sh"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }
        skill_resource::save_draft(
            &db,
            InstalledSkill {
                id: "sample".into(),
                name: "Sample".into(),
                description: None,
                directory: "sample".into(),
                repo_owner: tracked.then(|| "fixture".into()),
                repo_name: tracked.then(|| "skills".into()),
                repo_branch: tracked.then(|| "feature/shared".into()),
                readme_url: tracked.then(|| {
                    format!(
                        "https://github.com/fixture/skills/blob/{}/nested/sample/SKILL.md",
                        "a".repeat(40)
                    )
                }),
                apps: Default::default(),
                installed_at: 0,
                updated_at: 0,
                content_hash: None,
            },
            source.path(),
            None,
        )
        .unwrap();
        db
    }
    fn download(body: &str, executable: bool, path: &str) -> (ExtractedDir, String) {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        writer.start_file(format!("{path}/SKILL.md"), opts).unwrap();
        writer
            .write_all(format!("---\nname: sample\ndescription: Fixture\n---\n{body}").as_bytes())
            .unwrap();
        writer
            .start_file(
                format!("{path}/run.sh"),
                opts.unix_permissions(if executable { 0o755 } else { 0o644 }),
            )
            .unwrap();
        writer.write_all(b"echo fixture\n").unwrap();
        writer.start_file(format!("{path}/.hidden"), opts).unwrap();
        writer.write_all(b"hidden").unwrap();
        (
            artifacts::extract_zip(&writer.finish().unwrap().into_inner(), false).unwrap(),
            "b".repeat(40),
        )
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn checking_and_updating_use_the_reviewed_candidate_without_touching_native() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let op = skill_resource::preview(&db, "sample", &["codex".into()]).unwrap();
        assert_eq!(
            library::apply_operation(&db, &op.id).unwrap().status,
            "applied"
        );
        let native = skill_resource::native_paths().unwrap()["codex"].join("sample/SKILL.md");
        let old = fs::read(&native).unwrap();
        let before = db.library_snapshot().unwrap();
        let checked = check_with(&db, "sample", 1, 0, |repo| async move {
            assert_eq!(repo.branch, "feature/shared");
            Ok(download("New body", true, "nested/sample"))
        })
        .await
        .unwrap();
        assert_eq!(checked["state"], "updateAvailable");
        assert_eq!(db.library_snapshot().unwrap(), before);
        assert_eq!(fs::read(&native).unwrap(), old);
        let hash = checked["candidate"]["artifactHash"].as_str().unwrap();
        assert!(!artifacts::path(hash).unwrap().exists());
        let ticket = checked["candidate"]["ticketId"].as_str().unwrap();
        let result = confirm(&db, ticket, 1, 0, "update", None, None).unwrap();
        assert_eq!(result["state"], "updated");
        assert_eq!(fs::read(&native).unwrap(), old);
        assert_eq!(
            db.library_snapshot().unwrap().machine.receipts,
            before.machine.receipts
        );
        let row = db.get_installed_skill("sample").unwrap().unwrap();
        assert_eq!(row.content_hash.as_deref(), Some(hash));
        assert!(row.readme_url.unwrap().contains(&"b".repeat(40)));
        assert_eq!(
            fs::read_to_string(
                skill_resource::artifact_for(&db, "sample")
                    .unwrap()
                    .join(".hidden")
            )
            .unwrap(),
            "hidden"
        );
        assert!(artifacts::manifest(hash).unwrap()["run.sh"]);
        assert!(confirm(&db, ticket, 1, 0, "update", None, None).is_err());
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn errors_and_missing_sources_never_report_current_or_change_database() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let before = db.library_snapshot().unwrap();
        let failed = check_with(&db, "sample", 1, 0, |_| async {
            Err(AppError::Config("library.skillDownloadFailed".into()))
        })
        .await;
        assert!(failed.is_err());
        assert_eq!(db.library_snapshot().unwrap(), before);
        let missing = check_with(&db, "sample", 1, 0, |_| async {
            Ok(download("Other", false, "somewhere/sample"))
        })
        .await
        .unwrap();
        assert_eq!(missing["state"], "sourceMissing");
        assert!(missing["candidate"].is_null());
        assert_eq!(db.library_snapshot().unwrap(), before);
        let db = fixture(false);
        let local = check_with(&db, "sample", 1, 0, |_| async {
            panic!("local check downloaded")
        })
        .await
        .unwrap();
        assert_eq!(local["state"], "notTracked");
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn restoration_during_download_and_after_preview_invalidates_the_candidate() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let result = check_with(&db, "sample", 1, 0, |_| async {
            let before = db.library_snapshot().unwrap();
            let mut after = before.clone();
            after.machine.epoch += 1;
            db.library_commit(&before, &after).unwrap();
            Ok(download("New", false, "nested/sample"))
        })
        .await;
        assert!(result.is_err());
        let checked = check_with(&db, "sample", 1, 1, |_| async {
            Ok(download("New", false, "nested/sample"))
        })
        .await
        .unwrap();
        let ticket = checked["candidate"]["ticketId"].as_str().unwrap();
        let before = db.library_snapshot().unwrap();
        let mut after = before.clone();
        after.machine.epoch += 1;
        db.library_commit(&before, &after).unwrap();
        assert!(confirm(&db, ticket, 1, 1, "update", None, None).is_err());
        assert_eq!(
            db.get_installed_skill("sample")
                .unwrap()
                .unwrap()
                .content_hash,
            fixture(true)
                .get_installed_skill("sample")
                .unwrap()
                .unwrap()
                .content_hash
        );
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn candidate_copy_preserves_execution_and_has_no_assignments_or_overwrite() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let before = db.get_installed_skill("sample").unwrap().unwrap();
        let checked = check_with(&db, "sample", 1, 0, |_| async {
            Ok(download("New", true, "nested/sample"))
        })
        .await
        .unwrap();
        let result = confirm(
            &db,
            checked["candidate"]["ticketId"].as_str().unwrap(),
            1,
            0,
            "copy",
            Some("Copy"),
            Some("sample-copy"),
        )
        .unwrap();
        assert_eq!(result["state"], "copied");
        let copied = db
            .get_installed_skill(result["resourceId"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_ne!(copied.id, before.id);
        assert!(artifacts::manifest(copied.content_hash.as_ref().unwrap()).unwrap()["run.sh"]);
        assert_eq!(
            db.get_installed_skill("sample")
                .unwrap()
                .unwrap()
                .content_hash,
            before.content_hash
        );
        assert!(db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
        let checked = check_with(&db, "sample", 1, 0, |_| async {
            Ok(download("Newer", false, "nested/sample"))
        })
        .await
        .unwrap();
        assert!(confirm(
            &db,
            checked["candidate"]["ticketId"].as_str().unwrap(),
            1,
            0,
            "copy",
            Some("Collision"),
            Some("SAMPLE")
        )
        .is_err());
        assert_eq!(db.get_all_installed_skills().unwrap().len(), 2);
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn source_identity_never_falls_back_to_an_unrelated_root_or_branch_segment() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let source = skill_resource::artifact_for(&db, "sample").unwrap();
        let mut row = db.get_installed_skill("sample").unwrap().unwrap();
        row.readme_url = Some(
            "https://github.com/fixture/skills/blob/feature/shared/nested/sample/SKILL.md".into(),
        );
        let flags = artifacts::manifest(row.content_hash.as_ref().unwrap()).unwrap();
        skill_resource::save_draft_with_execution(&db, row, &source, Some(1), Some(&flags))
            .unwrap();
        let value = check_with(&db, "sample", 2, 0, |repo| async move {
            assert_eq!(repo.branch, "feature/shared");
            Ok(download("New", false, "nested/sample"))
        })
        .await
        .unwrap();
        assert_eq!(value["state"], "updateAvailable");
        let mut row = db.get_installed_skill("sample").unwrap().unwrap();
        row.readme_url = Some(format!(
            "https://github.com/different/skills/blob/{}/nested/sample/SKILL.md",
            "a".repeat(40)
        ));
        skill_resource::save_draft_with_execution(&db, row, &source, Some(2), Some(&flags))
            .unwrap();
        assert!(check_with(&db, "sample", 3, 0, |_| async {
            panic!("mismatched repository downloaded")
        })
        .await
        .is_err());
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn comparison_reads_only_the_frozen_candidate_and_expired_tickets_release_staging() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let checked = check_with(&db, "sample", 1, 0, |_| async {
            Ok(download("New body", true, "nested/sample"))
        })
        .await
        .unwrap();
        let ticket = checked["candidate"]["ticketId"].as_str().unwrap();
        let diff = compare(&db, ticket, "SKILL.md").unwrap();
        assert!(diff["before"].as_str().unwrap().contains("Old body"));
        assert!(diff["after"].as_str().unwrap().contains("New body"));
        assert!(diff["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["path"] == "run.sh" && v["afterExecutable"] == true));
        assert!(compare(&db, ticket, "../SKILL.md").is_err());
        let staged = {
            let mut state = registry().unwrap();
            let c = state.tickets.get_mut(ticket).unwrap();
            c.created = Instant::now() - TTL - Duration::from_secs(1);
            c.downloaded.path().to_path_buf()
        };
        assert!(confirm(&db, ticket, 1, 0, "update", None, None).is_err());
        assert!(!staged.exists());
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn a_modified_old_artifact_can_only_save_the_reviewed_remote_candidate_as_a_new_draft() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let path = skill_resource::artifact_for(&db, "sample")
            .unwrap()
            .join("SKILL.md");
        fs::write(&path, "User local changes").unwrap();
        let checked = check_with(&db, "sample", 1, 0, |_| async {
            Ok(download("Remote", false, "nested/sample"))
        })
        .await
        .unwrap();
        assert_eq!(checked["sourceVerified"], false);
        let result = confirm(
            &db,
            checked["candidate"]["ticketId"].as_str().unwrap(),
            1,
            0,
            "copy",
            Some("Remote Copy"),
            Some("remote-copy"),
        )
        .unwrap();
        assert_eq!(result["state"], "copied");
        assert_eq!(fs::read_to_string(path).unwrap(), "User local changes");
        assert_eq!(db.get_all_installed_skills().unwrap().len(), 2);
    }
    #[test]
    #[serial_test::serial]
    fn publisher_rejects_bytes_that_do_not_match_the_reviewed_candidate_hash() {
        let _sandbox = crate::test_sandbox::sandbox();
        let (staging, _) = download("Reviewed", false, "nested/sample");
        let source = staging.path().join("nested/sample");
        let flags = staging.execution_for(&source).unwrap();
        let expected = artifacts::source_hash(&source, &flags).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: sample\ndescription: Fixture\n---\nChanged after review",
        )
        .unwrap();
        assert!(artifacts::publish_checked(&source, Some(&flags), &expected).is_err());
        assert!(!artifacts::path(&expected).unwrap().exists());
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn a_recoverable_copy_name_error_preserves_the_same_reviewed_candidate() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let checked = check_with(&db, "sample", 1, 0, |_| async {
            Ok(download("Reviewed", false, "nested/sample"))
        })
        .await
        .unwrap();
        let ticket = checked["candidate"]["ticketId"].as_str().unwrap();
        assert!(confirm(&db, ticket, 1, 0, "copy", Some("Conflict"), Some("SAMPLE")).is_err());
        let result = confirm(&db, ticket, 1, 0, "copy", Some("Retried"), Some("retried")).unwrap();
        assert_eq!(result["commitSha"], "b".repeat(40));
        assert_eq!(result["state"], "copied");
    }
    #[test]
    #[serial_test::serial]
    fn hexadecimal_branch_prefix_is_not_silently_misread_as_a_commit_path() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let mut row = db.get_installed_skill("sample").unwrap().unwrap();
        row.repo_branch = Some(format!("{}/feature", "a".repeat(40)));
        row.readme_url = Some(format!(
            "https://github.com/fixture/skills/blob/{}/nested/sample/SKILL.md",
            row.repo_branch.as_ref().unwrap()
        ));
        assert!(tracking(&row).is_err());
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn a_late_native_copy_edit_allows_the_same_candidate_to_be_saved_separately() {
        let _sandbox = crate::test_sandbox::sandbox();
        let mut settings = crate::settings::get_settings();
        settings.skill_sync_method = crate::services::skill::SyncMethod::Copy;
        crate::settings::update_settings(settings).unwrap();
        let db = fixture(true);
        let op = skill_resource::preview(&db, "sample", &["codex".into()]).unwrap();
        library::apply_operation(&db, &op.id).unwrap();
        let checked = check_with(&db, "sample", 1, 0, |_| async {
            Ok(download("Reviewed", true, "nested/sample"))
        })
        .await
        .unwrap();
        let ticket = checked["candidate"]["ticketId"].as_str().unwrap();
        let native = skill_resource::native_paths().unwrap()["codex"].join("sample/SKILL.md");
        fs::write(&native, "User copy edit").unwrap();
        assert!(confirm(&db, ticket, 1, 0, "update", None, None).is_err());
        let copied = confirm(
            &db,
            ticket,
            1,
            0,
            "copy",
            Some("Separate"),
            Some("separate"),
        )
        .unwrap();
        assert_eq!(copied["commitSha"], "b".repeat(40));
        assert_eq!(fs::read_to_string(native).unwrap(), "User copy edit");
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn comparison_reports_an_unknown_baseline_when_the_old_artifact_changes_after_check() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let path = skill_resource::artifact_for(&db, "sample")
            .unwrap()
            .join("SKILL.md");
        let checked = check_with(&db, "sample", 1, 0, |_| async {
            Ok(download("Remote", true, "nested/sample"))
        })
        .await
        .unwrap();
        fs::write(path, "Later user edit").unwrap();
        let diff = compare(
            &db,
            checked["candidate"]["ticketId"].as_str().unwrap(),
            "SKILL.md",
        )
        .unwrap();
        assert_eq!(diff["beforeVerified"], false);
        assert!(diff["before"].is_null());
        assert!(diff["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["status"] == "unverified"));
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn a_corrupt_existing_hash_is_reported_as_a_copy_capability_block_without_replacing_native(
    ) {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let original = skill_resource::artifact_for(&db, "sample").unwrap();
        let hash = db
            .get_installed_skill("sample")
            .unwrap()
            .unwrap()
            .content_hash
            .unwrap();
        let (_, entries) = artifacts::archive_entries(&hash).unwrap();
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for entry in entries {
            if let Some(bytes) = entry.bytes {
                writer
                    .start_file(
                        format!("nested/sample/{}", entry.path),
                        zip::write::SimpleFileOptions::default()
                            .unix_permissions(if entry.executable { 0o755 } else { 0o644 }),
                    )
                    .unwrap();
                writer.write_all(&bytes).unwrap();
            }
        }
        let raw = writer.finish().unwrap().into_inner();
        fs::write(
            original.join("SKILL.md"),
            "User modified native link source",
        )
        .unwrap();
        let checked = check_with(&db, "sample", 1, 0, |_| async {
            Ok((artifacts::extract_zip(&raw, false).unwrap(), "b".repeat(40)))
        })
        .await
        .unwrap();
        assert_eq!(checked["candidate"]["artifactHash"], hash);
        assert_eq!(checked["candidate"]["canCopy"], false);
        assert_eq!(
            checked["candidate"]["copyBlockedReason"],
            "library.skillArtifactHashOccupied"
        );
        assert!(confirm(
            &db,
            checked["candidate"]["ticketId"].as_str().unwrap(),
            1,
            0,
            "copy",
            Some("Blocked"),
            Some("blocked")
        )
        .is_err());
        assert_eq!(
            fs::read_to_string(original.join("SKILL.md")).unwrap(),
            "User modified native link source"
        );
        assert_eq!(db.get_all_installed_skills().unwrap().len(), 1);
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn legacy_batch_groups_downloads_without_consuming_any_frozen_ticket_or_writing_database()
    {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let checked = check_with(&db, "sample", 1, 0, |_| async {
            Ok(download("Frozen", true, "nested/sample"))
        })
        .await
        .unwrap();
        let ticket = checked["candidate"]["ticketId"].as_str().unwrap();
        let original = db.get_installed_skill("sample").unwrap().unwrap();
        let source = skill_resource::artifact_for(&db, "sample").unwrap();
        let flags = artifacts::manifest(original.content_hash.as_ref().unwrap()).unwrap();
        for n in 1..9 {
            let mut row = original.clone();
            row.id = format!("sample-{n}");
            row.directory = format!("sample-{n}");
            row.readme_url = Some(format!(
                "https://github.com/fixture/skills/blob/{}/nested/sample-{n}/SKILL.md",
                "a".repeat(40)
            ));
            skill_resource::save_draft_with_execution(&db, row, &source, None, Some(&flags))
                .unwrap();
        }
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        for row in db.get_all_installed_skills().unwrap().values() {
            writer
                .start_file(format!("nested/{}/SKILL.md", row.directory), opts)
                .unwrap();
            writer
                .write_all(b"---\nname: sample\ndescription: Fixture\n---\nNew batch body")
                .unwrap();
        }
        let raw = writer.finish().unwrap().into_inner();
        let before = db.library_snapshot().unwrap();
        let mut count = 0;
        let updates = legacy_check_with(&db, |repo| {
            assert_eq!(repo.branch, "feature/shared");
            count += 1;
            let raw = raw.clone();
            async move { Ok((artifacts::extract_zip(&raw, false).unwrap(), "c".repeat(40))) }
        })
        .await
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(updates.len(), 9);
        assert_eq!(db.library_snapshot().unwrap(), before);
        let copy = confirm(
            &db,
            ticket,
            1,
            0,
            "copy",
            Some("Frozen copy"),
            Some("frozen-copy"),
        )
        .unwrap();
        assert_eq!(copy["commitSha"], "b".repeat(40));
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn legacy_update_checks_the_epoch_of_its_own_download_and_keeps_native_content() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        let op = skill_resource::preview(&db, "sample", &["codex".into()]).unwrap();
        library::apply_operation(&db, &op.id).unwrap();
        let native = skill_resource::native_paths().unwrap()["codex"].join("sample/SKILL.md");
        let old = fs::read(&native).unwrap();
        let updated = legacy_update_with(&db, "sample", |_| async {
            Ok(download("New legacy", true, "nested/sample"))
        })
        .await
        .unwrap();
        assert!(updated.readme_url.unwrap().contains(&"b".repeat(40)));
        assert_eq!(fs::read(&native).unwrap(), old);
        let before = db.get_installed_skill("sample").unwrap().unwrap();
        assert!(legacy_update_with(&db, "sample", |_| async {
            let snapshot = db.library_snapshot().unwrap();
            let mut restored = snapshot.clone();
            restored.machine.epoch += 1;
            db.library_commit(&snapshot, &restored).unwrap();
            Ok(download("Stale", false, "nested/sample"))
        })
        .await
        .is_err());
        assert_eq!(
            db.get_installed_skill("sample")
                .unwrap()
                .unwrap()
                .content_hash,
            before.content_hash
        );
        assert_eq!(fs::read(native).unwrap(), old);
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn legacy_batch_does_not_mix_restore_generations_or_treat_local_sources_as_current() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = fixture(true);
        assert!(legacy_check_with(&db, |_| async {
            let before = db.library_snapshot().unwrap();
            let mut restored = before.clone();
            restored.machine.epoch += 1;
            db.library_commit(&before, &restored).unwrap();
            Ok(download("New", false, "nested/sample"))
        })
        .await
        .is_err());
        let local = fixture(false);
        assert!(
            legacy_check_with(&local, |_| async { panic!("local batch downloaded") })
                .await
                .is_err()
        );
    }
}
