//! Bounded text-only filesystem operations. No unredacted file bytes cross IPC.
use super::scope::{self, Access, Scope};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const MAX_BYTES: u64 = 384 * 1024;
const MAX_ENTRIES: usize = 10_000;

fn checked(scope: &Scope, path: &Path, outside: bool) -> Result<PathBuf, String> {
    let checked = scope.check(path)?;
    if checked.access == Access::Forbidden || (checked.access == Access::Outside && !outside) {
        return Err("assistant_path_denied".into());
    }
    Ok(checked.path)
}

pub fn stat(scope: &Scope, path: &Path, outside: bool) -> Result<Value, String> {
    let path = checked(scope, path, outside)?;
    match fs::metadata(path) {
        Ok(metadata) => Ok(
            json!({"exists":true,"directory":metadata.is_dir(),"file":metadata.is_file(),"size":metadata.len()}),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(json!({"exists":false,"directory":false,"file":false}))
        }
        Err(_) => Err("assistant_file_unreadable".into()),
    }
}

pub fn read(scope: &Scope, path: &Path, outside: bool) -> Result<String, String> {
    Ok(crate::redact::scrub(&read_original(scope, path, outside)?).into_owned())
}

pub(crate) fn read_original(scope: &Scope, path: &Path, outside: bool) -> Result<String, String> {
    read_bounded(scope, path, outside, MAX_BYTES)
}

fn read_bounded(
    scope: &Scope,
    path: &Path,
    outside: bool,
    max_bytes: u64,
) -> Result<String, String> {
    let path = checked(scope, path, outside)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(&path)
        .map_err(|_| "assistant_file_unreadable")?;
    let metadata = file.metadata().map_err(|_| "assistant_file_unreadable")?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err("assistant_file_too_large".into());
    }
    // Validate after opening, before consuming bytes, in case an ancestor moved.
    if scope::resolve(&path)? != path {
        return Err("assistant_path_denied".into());
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "assistant_file_unreadable")?;
    if bytes.len() as u64 > max_bytes {
        return Err("assistant_file_too_large".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "assistant_file_unreadable")?;
    if text.contains('\0') || (text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----")) {
        return Err("assistant_path_denied".into());
    }
    Ok(text)
}

pub fn read_skill(scope: &Scope, path: &Path) -> Result<String, String> {
    read_bounded(scope, path, false, 1024 * 1024)
        .map(|text| crate::redact::scrub(&text).into_owned())
}
pub fn skill_files(scope: &Scope, root: &Path) -> Result<Vec<String>, String> {
    walk(scope, root, false)?
        .into_iter()
        .map(|p| {
            p.strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .map_err(|_| "assistant_path_denied".into())
        })
        .collect()
}

pub fn list(scope: &Scope, path: &Path, limit: usize, outside: bool) -> Result<Vec<Value>, String> {
    let path = checked(scope, path, outside)?;
    let mut names = Vec::new();
    for (count, entry) in fs::read_dir(path)
        .map_err(|_| "assistant_file_unreadable")?
        .enumerate()
    {
        if count >= MAX_ENTRIES {
            return Err("assistant_scan_limit".into());
        }
        let entry = entry.map_err(|_| "assistant_file_unreadable")?;
        if entry.file_type().is_ok_and(|t| !t.is_symlink())
            && scope.check(&entry.path()).is_ok_and(|p| {
                p.access == Access::Inside || (outside && p.access == Access::Outside)
            })
        {
            names.push(json!({"name":entry.file_name().to_string_lossy(),"directory":entry.file_type().is_ok_and(|t|t.is_dir())}));
        }
    }
    names.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    names.truncate(limit.min(1000));
    Ok(names)
}

fn walk(scope: &Scope, root: &Path, outside: bool) -> Result<Vec<PathBuf>, String> {
    let root = checked(scope, root, outside)?;
    let filter_scope = scope.clone();
    let mut builder = ignore::WalkBuilder::new(&root);
    builder
        .hidden(false)
        .follow_links(false)
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .max_depth(Some(32));
    builder.filter_entry(move |entry| {
        entry.file_name() != ".git"
            && !entry.file_type().is_some_and(|t| t.is_symlink())
            && filter_scope.check(entry.path()).is_ok_and(|p| {
                p.access == Access::Inside || (outside && p.access == Access::Outside)
            })
    });
    let started = Instant::now();
    let mut files = Vec::new();
    for (count, entry) in builder.build().enumerate() {
        if count >= MAX_ENTRIES || started.elapsed() > Duration::from_secs(3) {
            return Err("assistant_scan_limit".into());
        }
        let entry = entry.map_err(|_| "assistant_file_unreadable")?;
        if entry.file_type().is_some_and(|t| t.is_file()) {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}

pub fn find(
    scope: &Scope,
    root: &Path,
    pattern: &str,
    limit: usize,
    outside: bool,
) -> Result<Vec<String>, String> {
    if pattern.len() > 512 {
        return Err("assistant_invalid_tool".into());
    }
    let pattern = glob::Pattern::new(pattern).map_err(|_| "assistant_invalid_tool")?;
    let root = checked(scope, root, outside)?;
    Ok(walk(scope, &root, outside)?
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(&root).ok()?;
            pattern
                .matches_path(relative)
                .then(|| relative.to_string_lossy().replace('\\', "/"))
        })
        .take(limit.min(1000))
        .collect())
}

pub fn grep(
    scope: &Scope,
    root: &Path,
    pattern: &str,
    limit: usize,
    outside: bool,
) -> Result<Vec<String>, String> {
    if pattern.len() > 512 {
        return Err("assistant_invalid_tool".into());
    }
    let pattern = regex::RegexBuilder::new(pattern)
        .size_limit(65536)
        .build()
        .map_err(|_| "assistant_invalid_tool")?;
    let root = checked(scope, root, outside)?;
    let mut matches = Vec::new();
    let mut bytes = 0;
    for path in walk(scope, &root, outside)? {
        let Ok(text) = read(scope, &path, outside) else {
            continue;
        };
        for (line, text) in text.lines().enumerate() {
            if pattern.is_match(text) {
                let found = format!(
                    "{}:{}:{}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    line + 1,
                    text.chars().take(1000).collect::<String>()
                );
                // Keep the reply well inside the runtime's line limit.
                bytes += found.len();
                if bytes > GREP_BYTES {
                    return Ok(matches);
                }
                matches.push(found);
                if matches.len() >= limit.min(500) {
                    return Ok(matches);
                }
            }
        }
    }
    Ok(matches)
}
const GREP_BYTES: usize = 256 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_files_scrub_before_return_and_refuse_private_key_content() {
        let tmp = tempfile::tempdir().unwrap();
        let scope = Scope::new(tmp.path().into(), vec![tmp.path().into()], vec![]).unwrap();
        let path = tmp.path().join("config.json");
        std::fs::write(&path, r#"{"api_key":"opaque-test-value","model":"safe"}"#).unwrap();
        let text = read(&scope, &path, false).unwrap();
        assert!(!text.contains("opaque-test-value"));
        assert!(text.contains("safe"));
        std::fs::write(&path, "-----BEGIN OPENSSH PRIVATE KEY-----\nsecret\n").unwrap();
        assert!(read(&scope, &path, false).is_err());
        let outside = tempfile::NamedTempFile::new().unwrap();
        assert!(read(&scope, outside.path(), false).is_err());
    }
    #[test]
    fn local_files_find_and_grep_respect_ignores_limits_and_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "needle").unwrap();
        std::fs::write(root.join("kept.txt"), "needle api_key=opaque-test-value\n").unwrap();
        let scope = Scope::new(root.into(), vec![root.into()], vec![]).unwrap();
        let found = find(&scope, root, "*.txt", 20, false).unwrap();
        assert_eq!(found, vec!["kept.txt"]);
        let matches = grep(&scope, root, "needle", 20, false).unwrap();
        assert_eq!(matches.len(), 1);
        assert!(!matches[0].contains("opaque-test-value"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("ignored.txt"), root.join("link.txt")).unwrap();
            assert!(!find(&scope, root, "*.txt", 20, false)
                .unwrap()
                .iter()
                .any(|s| s == "link.txt"));
        }
    }
    #[test]
    fn local_grep_stops_at_a_byte_budget_for_long_wide_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let line = format!("needle {}\n", "中".repeat(990));
        std::fs::write(root.join("notes.md"), line.repeat(120)).unwrap();
        let scope = Scope::new(root.into(), vec![root.into()], vec![]).unwrap();
        let matches = grep(&scope, root, "needle", 500, false).unwrap();
        assert!(!matches.is_empty());
        assert!(matches.iter().map(String::len).sum::<usize>() <= GREP_BYTES);
    }
}
