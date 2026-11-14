//! Run-local skill authority. Names and paths from model arguments never grant access.
pub mod bundled;

use super::{
    host::{blocking, AssistantHost, RunControl},
    tools::{
        files,
        scope::{self, Scope},
    },
};
use crate::services::library::assistant_target::{
    self, AssistantSkill, AssistantSkillProof, AssistantSkillSnapshot, SkillAccess, SkillMode,
    VerifiedSkillAccess,
};
use crate::{error::AppError, Database};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub type SkillRead = Box<dyn FnOnce(&VerifiedSkillAccess<'_>) -> Result<Value, AppError> + Send>;
pub(super) const PPTX_BUILTIN: &str = "ofox-office-pptx";
pub fn snapshot(db: &Database) -> Result<AssistantSkillSnapshot, String> {
    assistant_target::assistant_skill_snapshot(db, &assistant_target::reviewed_names())
        .map_err(|e| e.to_string())
}
pub fn read_verified(
    db: &Database,
    proof: &AssistantSkillProof,
    access: SkillAccess,
    read: SkillRead,
) -> Result<Value, String> {
    assistant_target::with_revalidated_skill(db, proof, access, read).map_err(|e| e.to_string())
}
#[derive(Default)]
pub struct SkillRunState {
    entries: BTreeMap<String, (AssistantSkill, AssistantSkillProof)>,
    bundled: Vec<bundled::BundledSkill>,
    selected: BTreeMap<String, AssistantSkillProof>,
    denied: Vec<PathBuf>,
    revision: u64,
}
impl SkillRunState {
    pub fn replace(
        &mut self,
        snapshot: AssistantSkillSnapshot,
        bundled: Vec<bundled::BundledSkill>,
    ) -> Value {
        let mut entries = BTreeMap::new();
        for skill in &snapshot.skills {
            if skill.broken.is_some() {
                continue;
            }
            if let Some(proof) = snapshot.proof(&skill.name) {
                for path in [proof.lexical_target(), proof.canonical_dir()] {
                    // resolve normalizes trusted Windows canonical prefixes.
                    if let Ok(path) = scope::resolve(path) {
                        if !self.denied.contains(&path) {
                            self.denied.push(path);
                        }
                    }
                }
                entries.insert(skill.name.clone(), (skill.clone(), proof.clone()));
            }
        }
        self.selected.retain(|name, old| {
            if let Some((_, new)) = entries.get(name) {
                if old.same_identity(new) {
                    *old = new.clone();
                    return true;
                }
            }
            false
        });
        self.entries = entries;
        self.bundled = bundled;
        self.revision += 1;
        self.public()
    }
    /// The user's skills, then each bundled skill none of them shadows.
    pub fn public(&self) -> Value {
        let user = self.entries.values().map(|(s,p)|json!({"name":s.name,"description":crate::redact::scrub(&s.description),"dir":p.lexical_target(),"mode":s.mode,"hasScripts":s.has_scripts,"reviewed":s.reviewed}));
        let bundled = self
            .bundled
            .iter()
            .filter(|skill| self.unshadowed(skill))
            .map(|skill| json!({"name":skill.name,"description":crate::redact::scrub(&skill.description),"dir":skill.dir,"mode":SkillMode::Auto,"hasScripts":false,"reviewed":true,"bundled":true}));
        json!({"revision":self.revision,"skills":user.chain(bundled).collect::<Vec<_>>()})
    }
    fn unshadowed(&self, skill: &bundled::BundledSkill) -> bool {
        let free = !self.entries.contains_key(&skill.name);
        if !free {
            log::info!(
                "[Assistant] the user's skill {} replaces the bundled one",
                skill.name
            );
        }
        free
    }
    fn proof(&self, name: &str, selected: bool) -> Result<AssistantSkillProof, String> {
        let (metadata, proof) = self
            .entries
            .get(name)
            .ok_or("not_available: unknown or broken skill")?;
        if !selected && metadata.mode != SkillMode::Auto {
            return Err("not_available: manual skill requires user selection".into());
        }
        Ok(proof.clone())
    }
    pub fn grants(&self) -> Vec<(AssistantSkillProof, SkillAccess)> {
        self.entries
            .values()
            .filter(|(s, _)| s.mode == SkillMode::Auto)
            .map(|(_, p)| (p.clone(), SkillAccess::AutoUse))
            .chain(
                self.selected
                    .values()
                    .map(|p| (p.clone(), SkillAccess::UserSelected)),
            )
            .collect()
    }
    /// A bundled skill no user skill shadows.
    pub fn bundled_skill(&self, name: &str) -> Option<&bundled::BundledSkill> {
        if self.entries.contains_key(name) {
            return None;
        }
        self.bundled.iter().find(|skill| skill.name == name)
    }
    pub fn denied(&self) -> &[PathBuf] {
        &self.denied
    }
}
fn unavailable(e: String) -> AppError {
    AppError::Message(e)
}
fn load(access: &VerifiedSkillAccess<'_>) -> Result<Value, AppError> {
    read_dir(access.dir(), access.target()).map_err(unavailable)
}
/// A skill folder's bounded body and relative file list; `shown` is the dir the model sees.
fn read_dir(dir: &Path, shown: &Path) -> Result<Value, String> {
    let root = scope::resolve(dir)?;
    let scope = Scope::new(root.clone(), vec![root.clone()], vec![])?;
    // Task3 accepts 1 MiB SKILL files; the normal generic reader has a lower limit.
    let body = files::read_skill(&scope, &root.join("SKILL.md"))?;
    let mut paths = files::skill_files(&scope, &root)?;
    let truncated = body.chars().count() > 60000 || paths.len() > 200;
    paths.truncate(200);
    Ok(
        json!({"dir":shown,"body":body.chars().take(60000).collect::<String>(),"files":paths,"truncated":truncated}),
    )
}
/// Priming loads only the complete builtin instruction body, not its potentially
/// large resource inventory. No path or name comes from a model or request.
fn read_pptx_priming_body(skill: &bundled::BundledSkill) -> Result<Value, String> {
    if skill.name != PPTX_BUILTIN
        || std::fs::canonicalize(&skill.dir).map_err(|e| e.to_string())? != skill.dir
    {
        return Err("assistant_skill_unavailable".into());
    }
    let root = scope::resolve(&skill.dir)?;
    let scope = Scope::new(root.clone(), vec![root.clone()], vec![])?;
    let body = files::read_skill(&scope, &root.join("SKILL.md"))?;
    let (name, _) = assistant_target::metadata_bytes(body.as_bytes()).map_err(|e| e.to_string())?;
    if name != PPTX_BUILTIN || body.chars().count() > 60000 {
        return Err("assistant_skill_unavailable".into());
    }
    Ok(
        json!({"name":PPTX_BUILTIN,"dir":skill.dir,"body":body,"files":["SKILL.md"],"truncated":false}),
    )
}
/// The runtime's bundled skills; none when the runtime cannot be found.
fn bundled_in(env: &dyn super::host::Environment) -> Vec<bundled::BundledSkill> {
    match env.runtime_dir() {
        Ok(dir) => bundled::bundled_skills(&dir),
        Err(error) => {
            log::warn!("[Assistant] bundled skills are unavailable: {error}");
            Vec::new()
        }
    }
}
impl AssistantHost {
    pub(super) async fn prime_pptx_skill(
        &self,
        run: &Arc<RunControl>,
        selected: Option<&str>,
        tools: bool,
    ) -> Result<Option<Value>, String> {
        if !run.presentation_context
            || selected.is_some()
            || !tools
            || self.runtime().is_err()
            || !self.engines().formats().contains(&"pptx")
        {
            return Ok(None);
        }
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err("assistant_identity_changed".into());
        }
        let skill = {
            let state = run
                .skills
                .lock()
                .map_err(|_| "assistant_state_unavailable")?;
            // Preserve user shadowing, including manual skills. Never fall
            // through to the user-skill loader during automatic priming.
            if state.entries.contains_key(PPTX_BUILTIN) {
                return Ok(None);
            }
            state
                .bundled_skill(PPTX_BUILTIN)
                .cloned()
                .ok_or("assistant_skill_unavailable")?
        };
        let result = blocking(move || read_pptx_priming_body(&skill)).await;
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err("assistant_identity_changed".into());
        }
        let mut result = result.map_err(|_| "assistant_skill_unavailable")?;
        result["scriptsUnavailable"] = json!(!run.local_enabled);
        Ok(Some(result))
    }

    pub async fn list_skills(&self) -> Result<Value, String> {
        let env = self.env.clone();
        let (snapshot, bundled) =
            blocking(move || Ok((env.skill_snapshot()?, bundled_in(env.as_ref())))).await?;
        let user: Vec<_> = snapshot
            .skills
            .iter()
            .filter(|s| s.broken.is_none() && snapshot.proof(&s.name).is_some())
            .collect();
        let bundled = bundled
            .iter()
            .filter(|b| !user.iter().any(|s| s.name == b.name))
            .map(|b| json!({"name":b.name,"description":crate::redact::scrub(&b.description),"mode":SkillMode::Auto,"reviewed":true,"origin":"bundled"}));
        Ok(json!(user.iter().map(|s| json!({"name":s.name,"description":crate::redact::scrub(&s.description),"mode":s.mode,"reviewed":s.reviewed,"origin":s.origin})).chain(bundled).collect::<Vec<_>>()))
    }

    pub async fn refresh_run_skills(&self, run: &Arc<RunControl>) -> Result<Value, String> {
        self.refresh_skills(run, false).await
    }
    /// Chat admission: damaged skill state must not stop ordinary chat. The run
    /// gets no skills, and a picked skill then reports itself unavailable.
    pub async fn admit_run_skills(&self, run: &Arc<RunControl>) -> Result<Value, String> {
        self.refresh_skills(run, true).await
    }
    async fn refresh_skills(&self, run: &Arc<RunControl>, degrade: bool) -> Result<Value, String> {
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err("assistant_identity_changed".into());
        }
        // Bundled skills come with the runtime: a damaged library keeps them.
        let env = self.env.clone();
        let bundled = blocking(move || Ok(bundled_in(env.as_ref()))).await?;
        let env = self.env.clone();
        let snapshot = match blocking(move || env.skill_snapshot()).await {
            Ok(snapshot) => snapshot,
            Err(error) if degrade => {
                log::warn!("[Assistant] skills are unavailable for this run: {error}");
                Default::default()
            }
            Err(error) => return Err(error),
        };
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err("assistant_identity_changed".into());
        }
        Ok(run
            .skills
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .replace(snapshot, bundled))
    }
    /// Task5 only: call during admission, after resolving the user's effective selected name.
    pub async fn select_run_skill(
        &self,
        run: &Arc<RunControl>,
        name: &str,
    ) -> Result<Value, String> {
        self.load_run_skill(run, name, true).await
    }
    pub(super) async fn load_run_skill(
        &self,
        run: &Arc<RunControl>,
        name: &str,
        selected: bool,
    ) -> Result<Value, String> {
        if name.is_empty() || name.contains("..") || name.contains(['/', '\\']) {
            return Err("not_available: invalid name".into());
        }
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err("assistant_identity_changed".into());
        }
        let bundled = run
            .skills
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .bundled_skill(name)
            .cloned();
        if let Some(skill) = bundled {
            return self.load_bundled(run, skill).await;
        }
        let proof = run
            .skills
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .proof(name, selected)?;
        let expected = proof.clone();
        let env = self.env.clone();
        let mut result = blocking(move || {
            env.skill_read(
                &proof,
                if selected {
                    SkillAccess::UserSelected
                } else {
                    SkillAccess::AutoUse
                },
                Box::new(load),
            )
        })
        .await
        .map_err(|_| "not_available: skill changed or cannot be read".to_string())?;
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err("assistant_identity_changed".into());
        }
        let mut state = run
            .skills
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        if state.proof(name, selected)? != expected {
            return Err("not_available: skill changed".into());
        }
        if selected {
            state.selected.insert(name.into(), expected);
        }
        result["name"] = json!(name);
        result["scriptsUnavailable"] = json!(!run.local_enabled);
        Ok(result)
    }
    /// A bundled skill is the app's own file: no proof, no library read.
    async fn load_bundled(
        &self,
        run: &Arc<RunControl>,
        skill: bundled::BundledSkill,
    ) -> Result<Value, String> {
        let name = skill.name.clone();
        let mut result = blocking(move || bundled::read_bundled(&skill))
            .await
            .map_err(|error| {
                log::warn!("[Assistant] bundled skill {name} cannot be read: {error}");
                "not_available: skill cannot be read".to_string()
            })?;
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err("assistant_identity_changed".into());
        }
        result["name"] = json!(name);
        result["scriptsUnavailable"] = json!(!run.local_enabled);
        Ok(result)
    }
    pub(super) async fn skill_scope(
        &self,
        run: &Arc<RunControl>,
        mut scope: Scope,
    ) -> Result<Scope, String> {
        let (grants, denied) = {
            let state = run
                .skills
                .lock()
                .map_err(|_| "assistant_state_unavailable")?;
            (state.grants(), state.denied.clone())
        };
        let env = self.env.clone();
        let roots = blocking(move || {
            let mut roots = Vec::new();
            // One fresh scan for the scope, not a complete catalog rescan for
            // every grant. The actual read still has pre/post proof validation.
            if !grants.is_empty() {
                let current = env.skill_snapshot()?;
                for metadata in &current.skills {
                    if let Some(proof) = current.proof(&metadata.name) {
                        if grants.iter().any(|(old, _)| old == proof) {
                            roots.push((
                                proof.lexical_target().to_path_buf(),
                                scope::resolve(proof.canonical_dir())?,
                            ));
                        }
                    }
                }
            }
            Ok(roots)
        })
        .await?;
        scope.protect(&self.env.local_private_root(), &denied, roots)?;
        Ok(scope)
    }
}

/// Revalidate the matching capability around a synchronous generic read.
pub(super) fn read_in_skill(
    env: &dyn super::host::Environment,
    grants: Vec<(AssistantSkillProof, SkillAccess)>,
    path: &Path,
    read: Box<dyn FnOnce() -> Result<Value, String> + Send>,
) -> Result<Value, String> {
    for (proof, access) in grants {
        let real = scope::resolve(proof.canonical_dir())?;
        if path.starts_with(&real) {
            return env.skill_read(
                &proof,
                access,
                Box::new(move |_| read().map_err(unavailable)),
            );
        }
    }
    read()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[serial_test::serial]
    fn selected_grant_refresh_preserves_only_unchanged_identity_and_retains_denials() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let dir = assistant_target::skills_dir().join("sample");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: sample\ndescription: Example\n---\nBody",
        )
        .unwrap();
        assistant_target::set_skill_mode("sample", SkillMode::Manual).unwrap();
        let mut state = SkillRunState::default();
        state.replace(snapshot(&db).unwrap(), Vec::new());
        let proof = state.proof("sample", true).unwrap();
        assert!(state.proof("sample", false).is_err());
        state.selected.insert("sample".into(), proof.clone());
        let before = db.library_snapshot().unwrap();
        let mut after = before.clone();
        after.machine.epoch += 1;
        db.library_commit(&before, &after).unwrap();
        state.replace(snapshot(&db).unwrap(), Vec::new());
        assert!(state.selected.contains_key("sample"));
        assert_ne!(state.selected["sample"], proof);
        assert!(state.selected["sample"].same_identity(&proof));
        std::fs::write(dir.join("reference.txt"), "changed").unwrap();
        state.replace(snapshot(&db).unwrap(), Vec::new());
        assert!(state.selected.is_empty());
        assert!(!state.denied.is_empty());
        let next = state.proof("sample", true).unwrap();
        state.selected.insert("sample".into(), next);
        assistant_target::set_skill_mode("sample", SkillMode::Auto).unwrap();
        state.replace(snapshot(&db).unwrap(), Vec::new());
        assert!(state.selected.is_empty());
        let before = state.denied.clone();
        std::fs::remove_dir_all(dir).unwrap();
        state.replace(snapshot(&db).unwrap(), Vec::new());
        assert!(state.entries.is_empty());
        assert_eq!(state.denied, before);
    }
    #[test]
    #[serial_test::serial]
    fn public_puts_user_skills_first_and_drops_colliding_bundled() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let dir = assistant_target::skills_dir().join("ofox-office-pdf");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: ofox-office-pdf\ndescription: Mine\n---\nBody",
        )
        .unwrap();
        let bundled = |name: &str| bundled::BundledSkill {
            name: name.into(),
            description: format!("Bundled {name}"),
            dir: PathBuf::from("/runtime/skills").join(name),
        };
        let mut state = SkillRunState::default();
        let public = state.replace(
            snapshot(&db).unwrap(),
            vec![bundled("ofox-office-docx"), bundled("ofox-office-pdf")],
        );
        let skills = public["skills"].as_array().unwrap();
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0]["name"], "ofox-office-pdf");
        assert_eq!(skills[0]["description"], "Mine");
        assert!(skills[0].get("bundled").is_none());
        assert_eq!(
            skills[1],
            json!({"name":"ofox-office-docx","description":"Bundled ofox-office-docx","dir":bundled("ofox-office-docx").dir,"mode":"auto","hasScripts":false,"reviewed":true,"bundled":true})
        );
        assert!(state.bundled_skill("ofox-office-pdf").is_none());
        assert_eq!(
            state.bundled_skill("ofox-office-docx"),
            Some(&bundled("ofox-office-docx"))
        );
    }
    #[test]
    #[serial_test::serial]
    fn bounded_load_uses_lexical_target_and_rejects_changed_proofs() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let dir = assistant_target::skills_dir().join("sample");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!(
                "---\nname: sample\ndescription: Example\n---\n{}",
                "😀".repeat(60001)
            ),
        )
        .unwrap();
        for i in 0..205 {
            std::fs::write(dir.join(format!("file-{i}")), "hello").unwrap();
        }
        let snap = snapshot(&db).unwrap();
        let proof = snap.proof("sample").unwrap();
        let value = read_verified(&db, proof, SkillAccess::AutoUse, Box::new(load)).unwrap();
        assert_eq!(value["body"].as_str().unwrap().chars().count(), 60000);
        assert_eq!(value["files"].as_array().unwrap().len(), 200);
        assert_eq!(value["dir"], json!(dir));
        assert_eq!(value["truncated"], true);
        std::fs::rename(&dir, dir.with_file_name("old")).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: sample\ndescription: Other\n---\nchanged",
        )
        .unwrap();
        assert!(read_verified(&db, proof, SkillAccess::AutoUse, Box::new(load)).is_err());
    }
}

#[cfg(test)]
mod primer_tests {
    use super::*;
    use crate::services::assistant::{
        catalog,
        host::{Admit, BoxFuture, Environment, Identity},
        types::{id, Event, SendRequest},
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    struct PrimerEnvironment {
        config: PathBuf,
        runtime: PathBuf,
        db: Mutex<Option<Arc<Database>>>,
        identity_calls: AtomicUsize,
        stale_at: AtomicUsize,
        external_reads: AtomicUsize,
        forbidden_calls: AtomicUsize,
    }

    fn identity() -> Identity {
        Identity {
            owner: "ab".repeat(32),
            apex: "ofox.ai".into(),
        }
    }

    impl Environment for PrimerEnvironment {
        fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
            let call = self.identity_calls.fetch_add(1, Ordering::SeqCst) + 1;
            let mut value = identity();
            let stale_at = self.stale_at.load(Ordering::SeqCst);
            if stale_at > 0 && call >= stale_at {
                value.owner = "cd".repeat(32);
            }
            Box::pin(async move { Ok(value) })
        }
        fn api_key<'a>(
            &'a self,
            _: &'a Identity,
            _: Option<&'a str>,
            _: &'a Admit<'a>,
        ) -> BoxFuture<'a, Result<String, String>> {
            self.forbidden_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err("fixture_no_key".into()) })
        }
        fn key_ready<'a>(&'a self, _: &'a Identity) -> BoxFuture<'a, bool> {
            self.forbidden_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { false })
        }
        fn models<'a>(
            &'a self,
            _: &'a str,
            _: catalog::Source,
        ) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>> {
            self.forbidden_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err("fixture_no_models".into()) })
        }
        fn gateway_base(&self, _: &str) -> Result<String, String> {
            self.forbidden_calls.fetch_add(1, Ordering::SeqCst);
            Err("fixture_no_gateway".into())
        }
        fn config_dir(&self) -> PathBuf {
            self.config.clone()
        }
        fn runtime_dir(&self) -> Result<PathBuf, String> {
            Ok(self.runtime.clone())
        }
        fn emit(&self, _: Event) {
            self.forbidden_calls.fetch_add(1, Ordering::SeqCst);
        }
        fn skill_snapshot(&self) -> Result<AssistantSkillSnapshot, String> {
            match self.db.lock().unwrap().as_ref() {
                Some(db) => snapshot(db),
                None => Ok(Default::default()),
            }
        }
        fn skill_read(
            &self,
            _: &AssistantSkillProof,
            _: SkillAccess,
            _: SkillRead,
        ) -> Result<Value, String> {
            self.external_reads.fetch_add(1, Ordering::SeqCst);
            Err("fixture_external_read_forbidden".into())
        }
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        env: Arc<PrimerEnvironment>,
        host: Arc<AssistantHost>,
        run: Arc<RunControl>,
        skill: PathBuf,
    }
    impl Fixture {
        async fn new(context: bool, style: Option<&str>, office: bool) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let runtime = temp.path().join("runtime");
            for file in ["node/node", "node/node.exe", "runtime/src/main.mjs"] {
                let path = runtime.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, "fixture, never executed").unwrap();
            }
            if office {
                std::fs::write(runtime.join("officecli"), "fixture").unwrap();
            }
            std::fs::write(
                runtime.join("manifest.json"),
                serde_json::to_vec(&json!({"engines":{"officecli":{"path":"officecli"}}})).unwrap(),
            )
            .unwrap();
            let skill = bundled::write_skill(
                &runtime,
                PPTX_BUILTIN,
                PPTX_BUILTIN,
                "Use new_deck, never raw Presentation().",
            );
            let env = Arc::new(PrimerEnvironment {
                config: temp.path().join("config"),
                runtime,
                db: Mutex::new(None),
                identity_calls: AtomicUsize::new(0),
                stale_at: AtomicUsize::new(0),
                external_reads: AtomicUsize::new(0),
                forbidden_calls: AtomicUsize::new(0),
            });
            let host = AssistantHost::new(env.clone());
            let request: SendRequest = serde_json::from_value(json!({"sessionId":id(),"runId":id(),"operationId":id(),"text":"Topic","presentationWorkflowVersion":1,"presentationStyleId":style})).unwrap();
            let mut run = RunControl::new(identity(), &request, "synthetic-primer-key".into());
            run.presentation_context = context;
            let run = Arc::new(run);
            host.admit_run_skills(&run).await.unwrap();
            env.identity_calls.store(0, Ordering::SeqCst);
            Self {
                _temp: temp,
                env,
                host,
                run,
                skill,
            }
        }
        fn no_side_effects(&self) {
            assert_eq!(self.env.forbidden_calls.load(Ordering::SeqCst), 0);
            assert_eq!(self.env.external_reads.load(Ordering::SeqCst), 0);
            assert!(self.run.skills.lock().unwrap().selected.is_empty());
        }
    }

    #[tokio::test]
    async fn pptx_primer_only_primes_explicit_ppt_context_with_the_real_capability() {
        for (context, style, tools, office, selected, expected) in [
            (false, None, true, true, None, false),
            (true, Some("auto"), true, true, None, true),
            (true, Some("cartesian"), true, true, None, true),
            (true, None, true, true, None, true), // Host-verified PPT page reference.
            (true, Some("auto"), false, true, None, false),
            (true, Some("auto"), true, false, None, false),
            (true, Some("auto"), true, true, Some("chosen-skill"), false),
            (true, Some("auto"), true, true, Some(PPTX_BUILTIN), false),
        ] {
            let f = Fixture::new(context, style, office).await;
            let result = f
                .host
                .prime_pptx_skill(&f.run, selected, tools)
                .await
                .unwrap();
            assert_eq!(
                result.is_some(),
                expected,
                "{context:?} {style:?} {tools:?} {office:?} {selected:?}"
            );
            if let Some(result) = result {
                assert_eq!(result["name"], PPTX_BUILTIN);
                assert!(result["body"].as_str().unwrap().contains("Use new_deck"));
                assert_eq!(result["truncated"], false);
                assert_eq!(result["scriptsUnavailable"], true);
            }
            f.no_side_effects();
        }
        let f = Fixture::new(true, Some("auto"), true).await;
        std::fs::remove_file(f.env.runtime.join("runtime/src/main.mjs")).unwrap();
        assert!(f
            .host
            .prime_pptx_skill(&f.run, None, true)
            .await
            .unwrap()
            .is_none());
        f.no_side_effects();
    }

    #[tokio::test]
    async fn pptx_primer_reads_the_complete_body_without_truncating_on_the_resource_inventory() {
        let f = Fixture::new(true, Some("auto"), true).await;
        for i in 0..205 {
            std::fs::write(f.skill.join(format!("resource-{i}.txt")), "asset").unwrap();
        }
        let result = f
            .host
            .prime_pptx_skill(&f.run, None, true)
            .await
            .unwrap()
            .unwrap();
        assert!(result["body"]
            .as_str()
            .unwrap()
            .ends_with("Use new_deck, never raw Presentation()."));
        assert_eq!(result["files"], json!(["SKILL.md"]));
        assert_eq!(result["truncated"], false);
        f.no_side_effects();
    }

    #[tokio::test]
    async fn pptx_primer_rejects_missing_changed_invalid_or_truncated_builtin_body() {
        for case in ["missing", "deleted", "renamed", "truncated", "invalid"] {
            let f = Fixture::new(true, Some("auto"), true).await;
            match case {
                "missing" => {
                    f.run.skills.lock().unwrap().bundled.clear();
                }
                "deleted" => std::fs::remove_file(f.skill.join("SKILL.md")).unwrap(),
                "renamed" => {
                    std::fs::write(
                        f.skill.join("SKILL.md"),
                        "---\nname: other\ndescription: Other\n---\nOther body",
                    )
                    .unwrap();
                }
                "truncated" => {
                    std::fs::write(
                        f.skill.join("SKILL.md"),
                        format!(
                            "---\nname: {PPTX_BUILTIN}\ndescription: Long\n---\n{}",
                            "a".repeat(60001)
                        ),
                    )
                    .unwrap();
                }
                _ => {
                    std::fs::write(f.skill.join("SKILL.md"), [0xff, 0xfe]).unwrap();
                }
            }
            assert_eq!(
                f.host
                    .prime_pptx_skill(&f.run, None, true)
                    .await
                    .unwrap_err(),
                "assistant_skill_unavailable",
                "{case}"
            );
            f.no_side_effects();
        }
    }

    #[tokio::test]
    async fn pptx_primer_rejects_stale_identity_before_and_after_read_and_cancellation() {
        for stale_at in [1, 2] {
            let f = Fixture::new(true, Some("auto"), true).await;
            f.env.stale_at.store(stale_at, Ordering::SeqCst);
            assert_eq!(
                f.host
                    .prime_pptx_skill(&f.run, None, true)
                    .await
                    .unwrap_err(),
                "assistant_identity_changed"
            );
            f.no_side_effects();
        }
        let f = Fixture::new(true, Some("auto"), true).await;
        f.run.cancel.send_replace(true);
        assert_eq!(
            f.host
                .prime_pptx_skill(&f.run, None, true)
                .await
                .unwrap_err(),
            "assistant_cancelled"
        );
        f.no_side_effects();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pptx_primer_rejects_a_skill_file_replaced_with_an_external_symlink() {
        let f = Fixture::new(true, Some("auto"), true).await;
        let outside = f._temp.path().join("outside.md");
        std::fs::write(
            &outside,
            format!("---\nname: {PPTX_BUILTIN}\ndescription: Outside\n---\nExternal body"),
        )
        .unwrap();
        std::fs::remove_file(f.skill.join("SKILL.md")).unwrap();
        std::os::unix::fs::symlink(outside, f.skill.join("SKILL.md")).unwrap();
        assert_eq!(
            f.host
                .prime_pptx_skill(&f.run, None, true)
                .await
                .unwrap_err(),
            "assistant_skill_unavailable"
        );
        f.no_side_effects();
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn pptx_primer_never_automatically_reads_a_user_skill_shadow() {
        let _sandbox = crate::test_sandbox::sandbox();
        let f = Fixture::new(true, Some("auto"), true).await;
        let db = Arc::new(Database::memory().unwrap());
        let dir = assistant_target::skills_dir().join(PPTX_BUILTIN);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {PPTX_BUILTIN}\ndescription: User choice\n---\nUser body"),
        )
        .unwrap();
        assistant_target::set_skill_mode(PPTX_BUILTIN, SkillMode::Manual).unwrap();
        *f.env.db.lock().unwrap() = Some(db);
        f.host.admit_run_skills(&f.run).await.unwrap();
        assert!(f
            .host
            .prime_pptx_skill(&f.run, None, true)
            .await
            .unwrap()
            .is_none());
        f.no_side_effects();
    }
}
