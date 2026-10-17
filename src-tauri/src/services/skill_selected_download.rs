//! Fetch only the explicitly selected skill subtree at a resolved commit.
//! Metadata is bounded; no downloaded file is executed or projected to an agent.
use super::SkillRepo;
use crate::services::library::skill_artifact::{self as artifacts, ExtractedDir};
use anyhow::{anyhow, Result};
use serde::{de::DeserializeOwned, Deserialize};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    future::Future,
    io::{Cursor, Write},
};
use url::Url;

trait Transport: Sync {
    fn read(&self, url: Url, max_bytes: u64) -> impl Future<Output = Result<Vec<u8>>> + Send;
}
struct HttpTransport(reqwest::Client);
impl Transport for HttpTransport {
    async fn read(&self, url: Url, max_bytes: u64) -> Result<Vec<u8>> {
        // The client has redirects disabled. Do not follow URLs supplied by tree entries.
        if url.scheme() != "https"
            || !matches!(
                url.host_str(),
                Some("api.github.com" | "raw.githubusercontent.com")
            )
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(failure("skillRepositoryInvalid"));
        }
        let mut response = self
            .0
            .get(url)
            .header("User-Agent", "ofox-desktop")
            .send()
            .await
            .map_err(|_| failure("skillDownloadFailed"))?;
        if !response.status().is_success() {
            return Err(failure("skillDownloadFailed"));
        }
        if response
            .content_length()
            .is_some_and(|size| size > max_bytes)
        {
            return Err(failure("skillTooLarge"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| failure("skillDownloadFailed"))?
        {
            if bytes.len().saturating_add(chunk.len()) as u64 > max_bytes {
                return Err(failure("skillTooLarge"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}
#[derive(Clone, Copy)]
struct Limits {
    metadata_bytes: u64,
    tree_entries: usize,
    selected_entries: usize,
    blob_bytes: u64,
    total_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            metadata_bytes: 8 * 1024 * 1024,
            tree_entries: 100_000,
            selected_entries: 10_000,
            blob_bytes: artifacts::MAX_BYTES,
            total_bytes: artifacts::MAX_BYTES,
        }
    }
}
#[derive(Deserialize)]
struct Object {
    sha: String,
}
#[derive(Deserialize)]
struct CommitBody {
    tree: Object,
}
#[derive(Deserialize)]
struct Commit {
    sha: String,
    commit: CommitBody,
}
#[derive(Deserialize)]
struct GitTree {
    sha: String,
    truncated: bool,
    tree: Vec<TreeEntry>,
}
#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    size: Option<u64>,
}
fn failure(key: &str) -> anyhow::Error {
    anyhow!("library.{key}")
}
fn valid_sha(sha: &str) -> bool {
    sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn validate_repo(repo: &SkillRepo) -> Result<&str> {
    if [&repo.owner, &repo.name].iter().any(|part| {
        part.is_empty()
            || part.len() > 100
            || [".", ".."].contains(&part.as_str())
            || !part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    }) {
        return Err(failure("skillRepositoryInvalid"));
    }
    let reference = if repo.branch.trim().is_empty() {
        "HEAD"
    } else {
        repo.branch.as_str()
    };
    if reference.len() > 256 || reference.chars().any(char::is_control) {
        return Err(failure("skillRepositoryInvalid"));
    }
    Ok(reference)
}
fn endpoint(host: &str, segments: &[&str]) -> Result<Url> {
    let mut url =
        Url::parse(&format!("https://{host}/")).map_err(|_| failure("skillRepositoryInvalid"))?;
    url.path_segments_mut()
        .map_err(|_| failure("skillRepositoryInvalid"))?
        .extend(segments);
    Ok(url)
}
async fn read<T: Transport>(transport: &T, url: Url, limit: u64) -> Result<Vec<u8>> {
    let bytes = transport.read(url, limit).await.map_err(|error| {
        if error.to_string() == "library.skillTooLarge" {
            failure("skillTooLarge")
        } else {
            failure("skillDownloadFailed")
        }
    })?;
    if bytes.len() as u64 > limit {
        return Err(failure("skillTooLarge"));
    }
    Ok(bytes)
}
async fn metadata<T: Transport, D: DeserializeOwned>(
    transport: &T,
    url: Url,
    limit: u64,
) -> Result<D> {
    let bytes = read(transport, url, limit).await?;
    serde_json::from_slice(&bytes).map_err(|_| failure("skillRepositoryInvalid"))
}
fn selected_prefix(entries: &[TreeEntry], directory: &str) -> Result<String> {
    // An empty, already validated tracking path means the repository root, never a leaf lookup.
    if directory.is_empty() {
        return if entries
            .iter()
            .any(|entry| entry.path == "SKILL.md" && entry.kind == "blob")
        {
            Ok(String::new())
        } else {
            Err(failure("skillSourceMissing"))
        };
    }
    if entries
        .iter()
        .any(|entry| entry.path == format!("{directory}/SKILL.md") && entry.kind == "blob")
    {
        return Ok(directory.to_string());
    }
    if directory.contains('/') {
        return Err(failure("skillSourceMissing"));
    }
    let mut candidates = entries.iter().filter_map(|entry| {
        if entry.kind != "blob" {
            return None;
        }
        let parent = entry.path.strip_suffix("/SKILL.md")?;
        // Match the existing leaf lookup; direct paths can still explicitly select hidden parents.
        if parent.split('/').any(|part| part.starts_with('.')) {
            return None;
        }
        parent
            .rsplit('/')
            .next()
            .filter(|leaf| leaf.eq_ignore_ascii_case(directory))
            .map(|_| parent)
    });
    if let Some(first) = candidates.next() {
        if candidates.next().is_some() {
            return Err(failure("skillSourceAmbiguous"));
        }
        return Ok(first.to_string());
    }
    if entries
        .iter()
        .any(|entry| entry.path == "SKILL.md" && entry.kind == "blob")
    {
        return Ok(String::new());
    }
    Err(failure("skillSourceMissing"))
}
fn selected<'a>(tree: &'a GitTree, prefix: &str, limits: Limits) -> Result<Vec<&'a TreeEntry>> {
    let prefix_slash = format!("{prefix}/");
    let mut selected = tree
        .tree
        .iter()
        .filter(|entry| {
            prefix.is_empty() || entry.path == prefix || entry.path.starts_with(&prefix_slash)
        })
        .collect::<Vec<_>>();
    if selected.len() > limits.selected_entries {
        return Err(failure("skillTooLarge"));
    }
    let mut explicit = BTreeMap::new();
    let mut prefixes = BTreeMap::new();
    let mut total = 0u64;
    for entry in &selected {
        artifacts::safe_relative(&entry.path).map_err(|_| failure("invalidSkillPath"))?;
        if !valid_sha(&entry.sha) {
            return Err(failure("skillRepositoryInvalid"));
        }
        let is_directory = match (entry.kind.as_str(), entry.mode.as_str()) {
            ("tree", "040000") => true,
            ("blob", "100644" | "100755") => false,
            _ => return Err(failure("skillLinkUnsupported")),
        };
        if explicit
            .insert(entry.path.to_lowercase(), is_directory)
            .is_some()
        {
            return Err(failure("skillPathCollision"));
        }
        let mut path = String::new();
        for component in entry.path.split('/') {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(component);
            if prefixes
                .insert(path.to_lowercase(), path.clone())
                .is_some_and(|previous| previous != path)
            {
                return Err(failure("skillPathCollision"));
            }
        }
        if !is_directory {
            let size = entry
                .size
                .ok_or_else(|| failure("skillRepositoryInvalid"))?;
            if size > limits.blob_bytes {
                return Err(failure("skillTooLarge"));
            }
            total = total
                .checked_add(size)
                .ok_or_else(|| failure("skillTooLarge"))?;
            if total > limits.total_bytes {
                return Err(failure("skillTooLarge"));
            }
        }
    }
    for entry in &selected {
        let mut parent = entry.path.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if explicit.get(&prefix.to_lowercase()) == Some(&false) {
                return Err(failure("skillPathCollision"));
            }
            parent = prefix;
        }
    }
    selected.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(selected)
}
fn matches_git_blob(entry: &TreeEntry, bytes: &[u8]) -> bool {
    if entry.size != Some(bytes.len() as u64) {
        return false;
    }
    let mut digest = Sha1::new();
    digest.update(format!("blob {}\0", bytes.len()).as_bytes());
    digest.update(bytes);
    format!("{:x}", digest.finalize()).eq_ignore_ascii_case(&entry.sha)
}
// A pinned Git hash can identify a pointer without supplying the large file it names.
// Keep the selected tree unpublished rather than silently installing pointer-only fonts/assets.
fn is_git_lfs_pointer(bytes: &[u8]) -> bool {
    if bytes.len() >= 1024 {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut lines = text.lines();
    if lines.next() != Some("version https://git-lfs.github.com/spec/v1") {
        return false;
    }
    let mut keys = std::collections::BTreeSet::new();
    let mut oid = false;
    let mut size = false;
    for line in lines {
        let Some((key, value)) = line.split_once(' ') else {
            return false;
        };
        if key.is_empty()
            || value.is_empty()
            || value.contains('\r')
            || !key.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte)
            })
            || !keys.insert(key)
        {
            return false;
        }
        match key {
            "oid" => {
                oid = value.strip_prefix("sha256:").is_some_and(|hash| {
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                });
                if !oid {
                    return false;
                }
            }
            "size" => {
                size =
                    value.bytes().all(|byte| byte.is_ascii_digit()) && value.parse::<u64>().is_ok();
                if !size {
                    return false;
                }
            }
            "version" => return false,
            _ => {}
        }
    }
    oid && size
}
pub(super) async fn download(repo: &SkillRepo, directory: &str) -> Result<(ExtractedDir, String)> {
    // Reuse the no-redirect client factory, including the user's existing proxy choice.
    let transport = HttpTransport(
        crate::proxy::http_client::mcp_check_client()
            .map_err(|_| failure("skillDownloadFailed"))?,
    );
    download_with(repo, directory, &transport, Limits::default()).await
}
async fn download_with<T: Transport>(
    repo: &SkillRepo,
    directory: &str,
    transport: &T,
    limits: Limits,
) -> Result<(ExtractedDir, String)> {
    let reference = validate_repo(repo)?;
    // New installations validate a non-empty directory before calling this method.
    // Frozen update tracking may explicitly identify the repository root as an empty path.
    if !directory.is_empty() {
        artifacts::safe_relative(directory).map_err(|_| failure("invalidSkillPath"))?;
    }
    let commit: Commit = metadata(
        transport,
        endpoint(
            "api.github.com",
            &["repos", &repo.owner, &repo.name, "commits", reference],
        )?,
        limits.metadata_bytes,
    )
    .await?;
    if !valid_sha(&commit.sha)
        || !valid_sha(&commit.commit.tree.sha)
        || (valid_sha(reference) && !reference.eq_ignore_ascii_case(&commit.sha))
    {
        return Err(failure("skillRepositoryInvalid"));
    }
    let mut tree_url = endpoint(
        "api.github.com",
        &[
            "repos",
            &repo.owner,
            &repo.name,
            "git",
            "trees",
            &commit.commit.tree.sha,
        ],
    )?;
    tree_url.query_pairs_mut().append_pair("recursive", "1");
    let tree: GitTree = metadata(transport, tree_url, limits.metadata_bytes).await?;
    if tree.truncated || !tree.sha.eq_ignore_ascii_case(&commit.commit.tree.sha) {
        return Err(failure("skillRepositoryInvalid"));
    }
    if tree.tree.len() > limits.tree_entries {
        return Err(failure("skillTooLarge"));
    }
    let prefix = selected_prefix(&tree.tree, directory)?;
    let entries = selected(&tree, &prefix, limits)?;
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for entry in entries {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(if entry.mode == "100755" || entry.kind == "tree" {
                0o755
            } else {
                0o644
            });
        if entry.kind == "tree" {
            archive
                .add_directory(format!("{}/", entry.path), options)
                .map_err(|_| failure("skillArchiveInvalid"))?;
            continue;
        }
        let mut segments = vec![repo.owner.as_str(), repo.name.as_str(), commit.sha.as_str()];
        segments.extend(entry.path.split('/'));
        let bytes = read(
            transport,
            endpoint("raw.githubusercontent.com", &segments)?,
            entry.size.unwrap_or(0),
        )
        .await?;
        if !matches_git_blob(entry, &bytes) {
            return Err(failure("skillDownloadFailed"));
        }
        if is_git_lfs_pointer(&bytes) {
            return Err(failure("skillGitLfsUnsupported"));
        }
        archive
            .start_file(&entry.path, options)
            .map_err(|_| failure("skillArchiveInvalid"))?;
        archive
            .write_all(&bytes)
            .map_err(|_| failure("skillWriteFailed"))?;
    }
    let bytes = archive
        .finish()
        .map_err(|_| failure("skillArchiveInvalid"))?
        .into_inner();
    if bytes.len() as u64 > artifacts::MAX_BYTES {
        return Err(failure("skillTooLarge"));
    }
    let extracted =
        artifacts::extract_zip(&bytes, false).map_err(|_| failure("skillArchiveInvalid"))?;
    Ok((extracted, commit.sha.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use sha1::{Digest, Sha1};
    use std::{collections::BTreeMap, sync::Mutex};

    const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TREE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    #[derive(Default)]
    struct Fixture {
        responses: BTreeMap<String, Vec<u8>>,
        requests: Mutex<Vec<String>>,
    }
    impl Transport for Fixture {
        async fn read(&self, url: Url, _max_bytes: u64) -> Result<Vec<u8>> {
            self.requests.lock().unwrap().push(url.to_string());
            self.responses
                .get(url.as_str())
                .cloned()
                .ok_or_else(|| anyhow!("private-transport-sentinel"))
        }
    }
    fn repo() -> SkillRepo {
        SkillRepo {
            owner: "fixture".into(),
            name: "skills".into(),
            branch: "main".into(),
            enabled: true,
        }
    }
    fn git_hash(bytes: &[u8]) -> String {
        let mut hash = Sha1::new();
        hash.update(format!("blob {}\0", bytes.len()).as_bytes());
        hash.update(bytes);
        format!("{:x}", hash.finalize())
    }
    fn blob(path: &str, bytes: &[u8], executable: bool) -> Value {
        json!({"path":path,"mode":if executable {"100755"} else {"100644"},"type":"blob","sha":git_hash(bytes),"size":bytes.len()})
    }
    fn fixture(entries: Vec<Value>, contents: &[(&str, &[u8])]) -> Fixture {
        let mut fixture = Fixture::default();
        fixture.responses.insert(
            "https://api.github.com/repos/fixture/skills/commits/main".into(),
            serde_json::to_vec(&json!({"sha":COMMIT,"commit":{"tree":{"sha":TREE}}})).unwrap(),
        );
        fixture.responses.insert(
            format!("https://api.github.com/repos/fixture/skills/git/trees/{TREE}?recursive=1"),
            serde_json::to_vec(&json!({"sha":TREE,"truncated":false,"tree":entries})).unwrap(),
        );
        for (path, bytes) in contents {
            fixture.responses.insert(
                format!("https://raw.githubusercontent.com/fixture/skills/{COMMIT}/{path}"),
                bytes.to_vec(),
            );
        }
        fixture
    }
    fn requests(fixture: &Fixture) -> Vec<String> {
        fixture.requests.lock().unwrap().clone()
    }
    async fn failure(fixture: &Fixture, path: &str, limits: Limits) -> String {
        match download_with(&repo(), path, fixture, limits).await {
            Ok(_) => panic!("unsafe selection unexpectedly downloaded"),
            Err(error) => error.to_string(),
        }
    }
    #[tokio::test]
    async fn selected_download_fetches_only_chosen_blobs_preserves_hidden_license_and_execution() {
        let files: [(&str, &[u8]); 5] = [
            ("skills/chosen/SKILL.md", b"# Chosen"),
            ("skills/chosen/LICENSE.txt", b"Selected license"),
            ("skills/chosen/.hidden", b"hidden"),
            ("skills/chosen/run.sh", b"#!/bin/sh\necho not-run\n"),
            ("skills/chosen/fonts/FONT-LICENSE.txt", b"font terms"),
        ];
        let mut entries = files
            .iter()
            .map(|(path, bytes)| blob(path, bytes, path.ends_with("run.sh")))
            .collect::<Vec<_>>();
        entries.push(blob(
            "skills/unselected/private-sentinel.txt",
            b"must not download",
            false,
        ));
        let fixture = fixture(entries, &files);
        let (downloaded, commit) =
            download_with(&repo(), "skills/chosen", &fixture, Limits::default())
                .await
                .unwrap();
        assert_eq!(commit, COMMIT);
        for (path, bytes) in files {
            assert_eq!(std::fs::read(downloaded.path().join(path)).unwrap(), bytes);
        }
        assert!(!downloaded.path().join("skills/unselected").exists());
        let execution = downloaded
            .execution_for(&downloaded.path().join("skills/chosen"))
            .unwrap();
        assert_eq!(execution.get("run.sh"), Some(&true));
        assert_eq!(execution.get("LICENSE.txt"), Some(&false));
        let requests = requests(&fixture);
        assert_eq!(requests.len(), 7);
        assert!(requests
            .iter()
            .all(|url| !url.contains("archive") && !url.contains("unselected")));
        assert!(requests
            .iter()
            .filter(|url| url.contains("raw.githubusercontent.com"))
            .all(|url| url.contains(COMMIT) && !url.contains("/main/")));
    }
    #[tokio::test]
    async fn selected_download_supports_unique_leaf_and_repository_root_skills() {
        for (requested, actual) in [
            ("chosen", "skills/chosen/SKILL.md"),
            ("root-alias", "SKILL.md"),
        ] {
            let fixture = fixture(
                vec![blob(actual, b"# Skill", false)],
                &[(actual, b"# Skill")],
            );
            let (downloaded, _) = download_with(&repo(), requested, &fixture, Limits::default())
                .await
                .unwrap();
            assert!(downloaded.path().join(actual).is_file());
        }
    }
    #[tokio::test]
    async fn selected_download_accepts_explicit_repository_root_update_without_leaf_fallback() {
        let fixture = fixture(
            vec![
                blob("SKILL.md", b"# Root skill", false),
                blob("LICENSE.txt", b"Root license", false),
            ],
            &[
                ("SKILL.md", b"# Root skill"),
                ("LICENSE.txt", b"Root license"),
            ],
        );
        let (downloaded, commit) = download_with(&repo(), "", &fixture, Limits::default())
            .await
            .unwrap();
        assert_eq!(commit, COMMIT);
        assert_eq!(
            std::fs::read(downloaded.path().join("SKILL.md")).unwrap(),
            b"# Root skill"
        );
        assert_eq!(
            std::fs::read(downloaded.path().join("LICENSE.txt")).unwrap(),
            b"Root license"
        );
    }
    #[tokio::test]
    async fn selected_download_reports_missing_root_without_using_a_nested_skill() {
        let fixture = fixture(vec![blob("nested/chosen/SKILL.md", b"# Other", false)], &[]);
        assert_eq!(
            failure(&fixture, "", Limits::default()).await,
            "library.skillSourceMissing"
        );
        assert_eq!(requests(&fixture).len(), 2);
    }
    #[tokio::test]
    async fn selected_download_rejects_ambiguous_leaf_and_missing_skill_before_blobs() {
        for entries in [
            vec![
                blob("a/chosen/SKILL.md", b"a", false),
                blob("b/chosen/SKILL.md", b"b", false),
            ],
            vec![blob("skills/chosen/readme.md", b"not a skill", false)],
        ] {
            let fixture = fixture(entries, &[]);
            assert!(failure(&fixture, "chosen", Limits::default())
                .await
                .starts_with("library."));
            assert_eq!(requests(&fixture).len(), 2);
        }
    }
    #[tokio::test]
    async fn selected_download_rejects_truncated_and_wrong_tree_metadata_before_blobs() {
        for update in [json!({"truncated":true}), json!({"sha":"c".repeat(40)})] {
            let mut fixture = fixture(vec![blob("skills/chosen/SKILL.md", b"a", false)], &[]);
            let key =
                format!("https://api.github.com/repos/fixture/skills/git/trees/{TREE}?recursive=1");
            let mut tree: Value = serde_json::from_slice(&fixture.responses[&key]).unwrap();
            for (field, value) in update.as_object().unwrap() {
                tree[field] = value.clone();
            }
            fixture
                .responses
                .insert(key, serde_json::to_vec(&tree).unwrap());
            assert!(failure(&fixture, "skills/chosen", Limits::default())
                .await
                .starts_with("library."));
            assert_eq!(requests(&fixture).len(), 2);
        }
    }
    #[tokio::test]
    async fn selected_download_rejects_symlink_submodule_and_path_collisions_before_blobs() {
        for bad in [
            json!({"path":"skills/chosen/link","mode":"120000","type":"blob","sha":TREE,"size":0}),
            json!({"path":"skills/chosen/module","mode":"160000","type":"commit","sha":TREE}),
            blob("skills/chosen/../escape", b"a", false),
            blob("skills/chosen/skill.md", b"a", false),
            json!({"path":"skills/chosen/fifo","mode":"100600","type":"blob","sha":TREE,"size":0}),
        ] {
            let fixture = fixture(vec![blob("skills/chosen/SKILL.md", b"a", false), bad], &[]);
            assert!(failure(&fixture, "skills/chosen", Limits::default())
                .await
                .starts_with("library."));
            assert_eq!(requests(&fixture).len(), 2);
        }
    }
    #[tokio::test]
    async fn selected_download_rejects_invalid_selection_and_repository_before_network() {
        let fixture = Fixture::default();
        for path in [
            "../chosen",
            "skills/../chosen",
            "C:\\chosen",
            "/chosen",
            "skills//chosen",
        ] {
            assert!(failure(&fixture, path, Limits::default())
                .await
                .starts_with("library."));
        }
        let mut invalid = repo();
        invalid.owner = "owner/private".into();
        assert!(
            download_with(&invalid, "skills/chosen", &fixture, Limits::default())
                .await
                .is_err()
        );
        assert!(requests(&fixture).is_empty());
    }
    #[tokio::test]
    async fn selected_download_checks_blob_bytes_against_git_object_hash_and_size() {
        for bytes in [b"corrupt!".as_slice(), b"short".as_slice()] {
            let fixture = fixture(
                vec![blob("skills/chosen/SKILL.md", b"# Chosen", false)],
                &[("skills/chosen/SKILL.md", bytes)],
            );
            assert_eq!(
                failure(&fixture, "skills/chosen", Limits::default()).await,
                "library.skillDownloadFailed"
            );
        }
    }
    #[tokio::test]
    async fn selected_download_enforces_metadata_entry_blob_and_total_budgets() {
        let entries = vec![
            blob("skills/chosen/SKILL.md", b"content", false),
            blob("skills/chosen/LICENSE", b"license", false),
        ];
        for limits in [
            Limits {
                metadata_bytes: 10,
                ..Limits::default()
            },
            Limits {
                tree_entries: 1,
                ..Limits::default()
            },
            Limits {
                selected_entries: 1,
                ..Limits::default()
            },
            Limits {
                blob_bytes: 3,
                ..Limits::default()
            },
            Limits {
                total_bytes: 10,
                ..Limits::default()
            },
        ] {
            let fixture = fixture(entries.clone(), &[]);
            assert_eq!(
                failure(&fixture, "skills/chosen", limits).await,
                "library.skillTooLarge"
            );
            assert!(requests(&fixture)
                .iter()
                .all(|url| !url.contains("raw.githubusercontent.com")));
        }
    }
    #[tokio::test]
    async fn selected_download_rejects_standard_git_lfs_pointers_without_expanding_them() {
        for (separator, extension) in [("\n", ""), ("\r\n", ""), ("\n", "ext-0-test sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n")] {
            let pointer = format!("version https://git-lfs.github.com/spec/v1{separator}{extension}oid sha256:{}{separator}size 12345{separator}", "a".repeat(64));
            let path = "skills/chosen/fonts/font.ttf";
            let fixture = fixture(vec![blob("skills/chosen/SKILL.md",b"# Skill",false),blob(path,pointer.as_bytes(),false)], &[("skills/chosen/SKILL.md",b"# Skill"),(path,pointer.as_bytes())]);
            assert_eq!(failure(&fixture,"skills/chosen",Limits::default()).await,"library.skillGitLfsUnsupported");
            assert!(requests(&fixture).iter().all(|url| url.starts_with("https://api.github.com/") || url.starts_with("https://raw.githubusercontent.com/")));
            assert_eq!(requests(&fixture).len(),4);
        }
    }
    #[tokio::test]
    async fn selected_download_does_not_mistake_documentation_or_invalid_pointer_text_for_lfs() {
        for text in ["# Git LFS example\nversion https://git-lfs.github.com/spec/v1\noid sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nsize 1\n", "version https://git-lfs.github.com/spec/v1\noid sha256:not-an-object-id\nsize 1\n", "version https://git-lfs.github.com/spec/v1\noid sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nsize unknown\n"] {
            let path = "skills/chosen/reference.txt";
            let fixture = fixture(vec![blob("skills/chosen/SKILL.md",b"# Skill",false),blob(path,text.as_bytes(),false)], &[("skills/chosen/SKILL.md",b"# Skill"),(path,text.as_bytes())]);
            let (downloaded,_) = download_with(&repo(),"skills/chosen",&fixture,Limits::default()).await.unwrap();
            assert_eq!(std::fs::read(downloaded.path().join(path)).unwrap(),text.as_bytes());
        }
    }
    #[tokio::test]
    async fn selected_download_sanitizes_transport_errors_and_never_fetches_sibling_symlinks() {
        let empty = Fixture::default();
        let error = failure(&empty, "skills/chosen", Limits::default()).await;
        assert_eq!(error, "library.skillDownloadFailed");
        assert!(!error.contains("private-transport-sentinel"));
        let fixture = fixture(
            vec![
                blob("skills/chosen/SKILL.md", b"a", false),
                json!({"path":"other/link","mode":"120000","type":"blob","sha":TREE,"size":0}),
            ],
            &[("skills/chosen/SKILL.md", b"a")],
        );
        let (downloaded, _) = download_with(&repo(), "skills/chosen", &fixture, Limits::default())
            .await
            .unwrap();
        assert!(downloaded.path().join("skills/chosen/SKILL.md").is_file());
        assert!(!downloaded.path().join("other").exists());
    }
}
