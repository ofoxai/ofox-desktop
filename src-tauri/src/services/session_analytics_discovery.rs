//! Enumerate native sources and their metadata dependencies without reading bodies.
use super::session_analytics_index::SourceSpec;
use std::path::PathBuf;
#[derive(Default)]
pub(crate) struct Discovery {
    pub specs: Vec<SourceSpec>,
    pub batches: Vec<BatchSource>,
    pub failures: Vec<DiscoveryFailure>,
    pub roots: Vec<(String, PathBuf)>,
    pub require_route_match: bool,
}
#[derive(Clone)]
pub(crate) struct BatchSource {
    pub provider: String,
    pub roots: Vec<PathBuf>,
}
pub(crate) struct DiscoveryFailure {
    pub provider: String,
    pub code: &'static str,
}
#[cfg(test)]
pub(crate) fn discover_sources() -> Discovery {
    discover_in(&[
        ("claude", crate::config::get_claude_config_dir()),
        ("codex", crate::codex_config::get_codex_config_dir()),
        ("gemini", crate::gemini_config::get_gemini_dir()),
        (
            "opencode",
            crate::session_manager::providers::opencode::get_opencode_base_dir(),
        ),
    ])
}
pub(crate) fn discover_in(roots: &[(&str, PathBuf)]) -> Discovery {
    let mut result = Discovery {
        roots: roots
            .iter()
            .map(|(p, r)| ((*p).into(), r.clone()))
            .collect(),
        ..Default::default()
    };
    for (provider, root) in roots {
        if matches!(*provider, "workbuddy" | "pi" | "dsh" | "openclaw") {
            if let Some(batch) = result
                .batches
                .iter_mut()
                .find(|batch| batch.provider == *provider)
            {
                if !batch.roots.contains(root) {
                    batch.roots.push(root.clone());
                }
            } else {
                result.batches.push(BatchSource {
                    provider: (*provider).into(),
                    roots: vec![root.clone()],
                });
            }
            continue;
        }
        match provider_sources(provider, root) {
            Ok(mut specs) => result.specs.append(&mut specs),
            Err(()) => result.failures.push(DiscoveryFailure {
                provider: (*provider).into(),
                code: "sourceDiscoveryFailed",
            }),
        }
    }
    result
        .specs
        .sort_by(|a, b| (&a.provider, &a.path).cmp(&(&b.provider, &b.path)));
    result
}
const MAX_SOURCES: usize = 20_000;
const MAX_DIRECTORIES: usize = 100_000;
fn directory(path: &std::path::Path) -> Result<bool, ()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        _ => Err(()),
    }
}
fn children(path: &std::path::Path) -> Result<Vec<PathBuf>, ()> {
    let mut result = std::fs::read_dir(path)
        .map_err(|_| ())?
        .map(|e| e.map(|e| e.path()).map_err(|_| ()))
        .collect::<Result<Vec<_>, _>>()?;
    if result.len() > MAX_DIRECTORIES {
        return Err(());
    }
    result.sort();
    Ok(result)
}
fn walk(
    path: &std::path::Path,
    depth: usize,
    dirs: &mut usize,
    extensions: &[&str],
    files: &mut Vec<PathBuf>,
) -> Result<(), ()> {
    if depth > 8 || *dirs > MAX_DIRECTORIES || files.len() > MAX_SOURCES {
        return Err(());
    }
    if !directory(path)? {
        return Ok(());
    }
    *dirs += 1;
    for path in children(path)? {
        let meta = std::fs::symlink_metadata(&path).map_err(|_| ())?;
        if meta.file_type().is_symlink() {
            return Err(());
        }
        if meta.is_dir() {
            walk(&path, depth + 1, dirs, extensions, files)?;
        } else if meta.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| extensions.contains(&e))
        {
            if path.to_str().is_none() || files.len() >= MAX_SOURCES {
                return Err(());
            }
            files.push(path);
        }
    }
    Ok(())
}
fn provider_sources(provider: &str, root: &std::path::Path) -> Result<Vec<SourceSpec>, ()> {
    if !directory(root)? {
        return Ok(vec![]);
    }
    let mut files = vec![];
    let mut dirs = 0;
    let mut specs = vec![];
    match provider {
        "claude" => walk(&root.join("projects"), 0, &mut dirs, &["jsonl"], &mut files)?,
        "codex" => {
            walk(
                &root.join("sessions"),
                0,
                &mut dirs,
                &["jsonl", "zst", "zstd"],
                &mut files,
            )?;
            walk(
                &root.join("archived_sessions"),
                0,
                &mut dirs,
                &["jsonl", "zst", "zstd"],
                &mut files,
            )?;
        }
        "gemini" => {
            let tmp = root.join("tmp");
            if !directory(&tmp)? {
                return Ok(specs);
            }
            for project in children(&tmp)? {
                let metadata = std::fs::symlink_metadata(&project).map_err(|_| ())?;
                if metadata.file_type().is_symlink() {
                    return Err(());
                }
                if !metadata.is_dir() {
                    continue;
                }
                let mut chats = vec![];
                walk(
                    &project.join("chats"),
                    0,
                    &mut dirs,
                    &["json", "jsonl"],
                    &mut chats,
                )?;
                for path in chats {
                    specs.push(SourceSpec {
                        provider: provider.into(),
                        path,
                        subagent: false,
                        project_file: Some(project.join(".project_root")),
                        title_file: None,
                    });
                }
                if specs.len() > MAX_SOURCES {
                    return Err(());
                }
            }
        }
        "opencode" => {
            let db = root.join("opencode.db");
            match std::fs::symlink_metadata(&db) {
                Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => files.push(db),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(()),
            }
            let storage = root.join("storage");
            if directory(&storage)? {
                files.push(storage);
            }
        }
        _ => return Err(()),
    }
    for path in files {
        let subagent =
            provider == "claude" && path.components().any(|p| p.as_os_str() == "subagents");
        specs.push(SourceSpec {
            provider: provider.into(),
            path,
            subagent,
            project_file: None,
            title_file: (provider == "codex").then(|| root.join("session_index.jsonl")),
        });
    }
    Ok(specs)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn roots(dir: &std::path::Path) -> Vec<(&'static str, PathBuf)> {
        ["claude", "codex", "gemini", "opencode"]
            .into_iter()
            .map(|p| (p, dir.join(p)))
            .collect()
    }
    fn touch(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "invalid private body must not be read by discovery").unwrap();
    }
    #[test]
    fn session_analytics_discovery_finds_native_roster_children_archived_and_dependencies_without_body_reads(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        for path in [
            "claude/projects/p/s.jsonl",
            "claude/projects/p/s/subagents/agent-c.jsonl",
            "codex/sessions/2026/10/15/s.jsonl",
            "codex/archived_sessions/a.jsonl",
            "gemini/tmp/hash/chats/session-g.jsonl",
            "opencode/opencode.db",
            "opencode/storage/session/p/s.json",
        ] {
            touch(&dir.path().join(path));
        }
        let found = discover_in(&roots);
        assert!(found.failures.is_empty());
        assert_eq!(found.specs.len(), 7);
        assert!(found
            .specs
            .iter()
            .any(|s| s.provider == "claude" && s.subagent));
        assert_eq!(
            found
                .specs
                .iter()
                .filter(|s| s.provider == "opencode")
                .count(),
            2
        );
        assert!(found
            .specs
            .iter()
            .filter(|s| s.provider == "codex")
            .all(|s| s.title_file.as_deref()
                == Some(dir.path().join("codex/session_index.jsonl").as_path())));
        assert!(found
            .specs
            .iter()
            .filter(|s| s.provider == "gemini")
            .all(|s| s.project_file.as_deref()
                == Some(dir.path().join("gemini/tmp/hash/.project_root").as_path())));
    }
    #[test]
    fn session_analytics_discovery_one_unreadable_root_does_not_turn_into_empty_or_block_other_providers(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        touch(&dir.path().join("claude"));
        touch(&dir.path().join("codex/sessions/s.jsonl"));
        let found = discover_in(&roots);
        assert_eq!(found.specs.len(), 1);
        assert_eq!(found.specs[0].provider, "codex");
        assert_eq!(found.failures.len(), 1);
        assert_eq!(found.failures[0].provider, "claude");
        assert_eq!(found.failures[0].code, "sourceDiscoveryFailed");
    }
}
