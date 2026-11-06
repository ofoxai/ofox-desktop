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
