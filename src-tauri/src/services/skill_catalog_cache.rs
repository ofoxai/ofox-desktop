//! The last catalog each repository returned. The Discover page shows it while
//! GitHub is rate-limited or unreachable; installing always resolves the
//! repository again, so a saved copy never decides what gets installed.
use super::{DiscoverableSkill, SkillRepo};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{
    future::Future,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryCatalog {
    pub skills: Vec<DiscoverableSkill>,
    /// Unix seconds of the saved copy shown because GitHub could not be read now.
    pub saved_at: Option<u64>,
    /// The `library.*` reason the live read failed, set together with `saved_at`.
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    saved_at: u64,
    skills: Vec<DiscoverableSkill>,
}

pub(super) fn dir() -> PathBuf {
    crate::config::get_app_config_dir().join("skill-repo-catalogs")
}

fn file(dir: &Path, repo: &SkillRepo) -> PathBuf {
    let branch = match repo.branch.trim() {
        "" => "HEAD",
        branch => branch,
    };
    let key = format!(
        "{}/{}@{branch}",
        repo.owner.to_ascii_lowercase(),
        repo.name.to_ascii_lowercase()
    );
    dir.join(format!("{:x}.json", sha2::Sha256::digest(key.as_bytes())))
}

/// Only "could not reach GitHub" falls back. A repository whose content is
/// invalid or too large keeps reporting that, even with an older copy saved.
fn unreachable(message: &str) -> bool {
    message.starts_with("library.githubRateLimited")
        || [
            "library.skillDownloadFailed",
            "library.skillDownloadTimeout",
        ]
        .contains(&message)
}

pub(super) async fn read_through(
    dir: &Path,
    repo: &SkillRepo,
    now: u64,
    live: impl Future<Output = Result<Vec<DiscoverableSkill>>>,
) -> Result<RepositoryCatalog> {
    let path = file(dir, repo);
    let error = match live.await {
        Ok(skills) => {
            let saved = Saved {
                saved_at: now,
                skills,
            };
            let written = std::fs::create_dir_all(dir)
                .map_err(anyhow::Error::from)
                .and_then(|()| Ok(serde_json::to_vec(&saved)?))
                .and_then(|bytes| Ok(crate::config::atomic_write(&path, &bytes)?));
            if let Err(error) = written {
                log::warn!(
                    "[Skills] could not save the {}/{} catalog: {error}",
                    repo.owner,
                    repo.name
                );
            }
            return Ok(RepositoryCatalog {
                skills: saved.skills,
                saved_at: None,
                reason: None,
            });
        }
        Err(error) if unreachable(&error.to_string()) => error,
        Err(error) => return Err(error),
    };
    let Some(saved) = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
    else {
        return Err(error);
    };
    log::warn!(
        "[Skills] {}/{} unreachable ({error}); showing the catalog saved at {}",
        repo.owner,
        repo.name,
        saved.saved_at
    );
    Ok(RepositoryCatalog {
        skills: saved.skills,
        saved_at: Some(saved.saved_at),
        reason: Some(error.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> SkillRepo {
        SkillRepo {
            owner: "Fixture".into(),
            name: "Skills".into(),
            branch: "main".into(),
            enabled: true,
        }
    }
    fn skill(name: &str) -> DiscoverableSkill {
        DiscoverableSkill {
            key: format!("fixture/skills:{name}"),
            name: name.into(),
            description: String::new(),
            directory: name.into(),
            readme_url: None,
            repo_owner: "Fixture".into(),
            repo_name: "Skills".into(),
            repo_branch: "main".into(),
        }
    }
    fn names(catalog: &RepositoryCatalog) -> Vec<&str> {
        catalog
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect()
    }

    #[tokio::test]
    async fn an_unreachable_github_shows_the_last_saved_catalog_and_why() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = read_through(dir.path(), &repo(), 100, async { Ok(vec![skill("first")]) })
            .await
            .unwrap();
        assert_eq!(names(&fresh), ["first"]);
        assert_eq!((fresh.saved_at, fresh.reason), (None, None));

        for reason in [
            "library.githubRateLimited@1793901507",
            "library.skillDownloadFailed",
            "library.skillDownloadTimeout",
        ] {
            let saved = read_through(dir.path(), &repo(), 200, async {
                Err(anyhow::anyhow!(reason))
            })
            .await
            .unwrap();
            assert_eq!(names(&saved), ["first"]);
            assert_eq!(saved.saved_at, Some(100));
            assert_eq!(saved.reason.as_deref(), Some(reason));
        }
    }

    #[tokio::test]
    async fn an_invalid_repository_or_a_first_read_still_fails() {
        let dir = tempfile::tempdir().unwrap();
        let error = read_through(dir.path(), &repo(), 1, async {
            Err(anyhow::anyhow!("library.githubRateLimited@1793901507"))
        })
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "library.githubRateLimited@1793901507");

        read_through(dir.path(), &repo(), 1, async { Ok(vec![skill("first")]) })
            .await
            .unwrap();
        let error = read_through(dir.path(), &repo(), 2, async {
            Err(anyhow::anyhow!("library.skillRepositoryInvalid"))
        })
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "library.skillRepositoryInvalid");
    }

    #[tokio::test]
    async fn each_repository_and_branch_keeps_its_own_copy() {
        let dir = tempfile::tempdir().unwrap();
        read_through(dir.path(), &repo(), 1, async { Ok(vec![skill("main")]) })
            .await
            .unwrap();
        let other = SkillRepo {
            branch: "next".into(),
            ..repo()
        };
        let error = read_through(dir.path(), &other, 2, async {
            Err(anyhow::anyhow!("library.skillDownloadFailed"))
        })
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "library.skillDownloadFailed");
        // Owner and name are matched case-insensitively, as GitHub does.
        let lower = SkillRepo {
            owner: "fixture".into(),
            name: "skills".into(),
            ..repo()
        };
        let saved = read_through(dir.path(), &lower, 3, async {
            Err(anyhow::anyhow!("library.skillDownloadFailed"))
        })
        .await
        .unwrap();
        assert_eq!(names(&saved), ["main"]);
    }
}
