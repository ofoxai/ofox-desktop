//! Skills shipped with the runtime (`runtime/skills/ofox-*`). The app's own
//! resources are their authority, so they need no library proof; a user's
//! skill with the same name always wins.
use crate::services::library::assistant_target;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Reaches the model through Pi's own skill paths, never this list.
const LOCAL_CONFIG: &str = "local-config";
const PREFIX: &str = "ofox-";
const MAX_SKILL_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundledSkill {
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
}

/// The `ofox-*` skills under `runtime_dir/runtime/skills`, by name. A folder
/// whose frontmatter does not name it, or whose name lacks the prefix, is
/// skipped with a warning.
pub fn bundled_skills(runtime_dir: &Path) -> Vec<BundledSkill> {
    let base = runtime_dir.join("runtime/skills");
    let entries = match fs::canonicalize(&base).and_then(fs::read_dir) {
        Ok(entries) => entries,
        Err(error) => {
            log::warn!(
                "[Assistant] bundled skills are unavailable at {}: {error}",
                base.display()
            );
            return Vec::new();
        }
    };
    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let folder = entry.file_name().to_string_lossy().into_owned();
        if folder == LOCAL_CONFIG || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        match skill(&entry.path(), &folder) {
            Ok(skill) => skills.push(skill),
            Err(reason) => log::warn!("[Assistant] skipped bundled skill {folder}: {reason}"),
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn skill(dir: &Path, folder: &str) -> Result<BundledSkill, String> {
    let file = dir.join("SKILL.md");
    if fs::metadata(&file).map_err(|e| e.to_string())?.len() > MAX_SKILL_BYTES {
        return Err("SKILL.md exceeds 1 MiB".into());
    }
    let bytes = fs::read(&file).map_err(|e| e.to_string())?;
    let (name, description) =
        assistant_target::metadata_bytes(&bytes).map_err(|e| e.to_string())?;
    if name != folder || !name.starts_with(PREFIX) {
        return Err(format!(
            "name {name:?} must match its folder and start with {PREFIX:?}"
        ));
    }
    Ok(BundledSkill {
        name,
        description,
        dir: dir.to_path_buf(),
    })
}

/// The same shape as a verified user skill read: `body`, `dir`, `files`, `truncated`.
pub fn read_bundled(skill: &BundledSkill) -> Result<Value, String> {
    super::read_dir(&skill.dir, &skill.dir)
}

#[cfg(test)]
pub(crate) fn write_skill(runtime_dir: &Path, dir: &str, name: &str, body: &str) -> PathBuf {
    let dir = runtime_dir.join("runtime/skills").join(dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Use for {name}\n---\n{body}"),
    )
    .unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_skills_scans_runtime_dir_and_skips_local_config() {
        let runtime = tempfile::tempdir().unwrap();
        write_skill(runtime.path(), "local-config", "local-config", "Local");
        write_skill(runtime.path(), "bad-name", "bad-name", "Unprefixed");
        write_skill(runtime.path(), "ofox-renamed", "ofox-other", "Mismatched");
        let pdf = write_skill(
            runtime.path(),
            "ofox-office-pdf",
            "ofox-office-pdf",
            "Typst",
        );
        std::fs::write(
            runtime.path().join("runtime/skills/README.md"),
            "not a skill",
        )
        .unwrap();
        assert_eq!(
            bundled_skills(runtime.path()),
            vec![BundledSkill {
                name: "ofox-office-pdf".into(),
                description: "Use for ofox-office-pdf".into(),
                dir: std::fs::canonicalize(pdf).unwrap(),
            }]
        );
    }

    #[test]
    fn every_shipped_skill_but_local_config_passes_the_scan() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime/skills");
        let runtime = tempfile::tempdir().unwrap();
        let mut shipped = Vec::new();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let folder = entry.file_name().to_string_lossy().into_owned();
            let dir = runtime.path().join("runtime/skills").join(&folder);
            fs::create_dir_all(&dir).unwrap();
            fs::copy(entry.path().join("SKILL.md"), dir.join("SKILL.md")).unwrap();
            if folder != LOCAL_CONFIG {
                shipped.push(folder);
            }
        }
        shipped.sort();
        let names: Vec<_> = bundled_skills(runtime.path())
            .into_iter()
            .map(|skill| skill.name)
            .collect();
        assert_eq!(names, shipped);
    }

    #[test]
    fn bundled_skills_is_empty_without_a_skills_dir() {
        let runtime = tempfile::tempdir().unwrap();
        assert!(bundled_skills(runtime.path()).is_empty());
    }

    #[test]
    fn read_bundled_returns_the_verified_read_shape() {
        let runtime = tempfile::tempdir().unwrap();
        write_skill(
            runtime.path(),
            "ofox-office-pdf",
            "ofox-office-pdf",
            "Typst body",
        );
        let skill = bundled_skills(runtime.path()).remove(0);
        let value = read_bundled(&skill).unwrap();
        assert!(value["body"].as_str().unwrap().contains("Typst body"));
        assert_eq!(value["dir"], serde_json::json!(skill.dir));
        assert_eq!(value["files"], serde_json::json!(["SKILL.md"]));
        assert_eq!(value["truncated"], false);
    }
}
