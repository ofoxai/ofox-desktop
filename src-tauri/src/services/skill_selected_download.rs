//! Fetch only the explicitly selected skill subtree at a resolved commit.
//! Metadata is bounded; no downloaded file is executed or projected to an agent.
use super::SkillRepo;
use crate::services::library::skill_artifact::{self as artifacts, ExtractedDir};
use anyhow::{anyhow, Result};
use serde::{de::DeserializeOwned, Deserialize};
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    io::{Cursor, Write},
};
use url::Url;

trait Transport: Sync {
    fn read(&self, url: Url, max_bytes: u64) -> impl Future<Output = Result<Vec<u8>>> + Send;
}
struct HttpTransport(reqwest::Client);
enum HttpRead {
    Bytes(Vec<u8>),
    Redirect {
        status: u16,
        location: Option<String>,
    },
}
trait HttpRequester: Sync {
    fn request(&self, url: &Url, max_bytes: u64) -> impl Future<Output = Result<HttpRead>> + Send;
}
impl HttpRequester for HttpTransport {
    async fn request(&self, url: &Url, max_bytes: u64) -> Result<HttpRead> {
        let mut response = self
            .0
            .get(url.clone())
            .header("User-Agent", "ofox-desktop")
            .send()
            .await
            .map_err(|_| failure("skillDownloadFailed"))?;
        if response.status().is_redirection() {
            return Ok(HttpRead::Redirect {
                status: response.status().as_u16(),
                location: response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
            });
        }
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
        Ok(HttpRead::Bytes(bytes))
    }
}
impl Transport for HttpTransport {
    async fn read(&self, url: Url, max_bytes: u64) -> Result<Vec<u8>> {
        read_http_response(self, url, max_bytes).await
    }
}
fn validate_http_url(url: &Url) -> Result<()> {
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
    Ok(())
}
/// Only GitHub repository metadata may move. Preserve the requested commit/tree
/// resource and query; raw blob redirects and arbitrary API paths stay refused.
fn repository_resource(url: &Url) -> Option<Vec<&str>> {
    if url.host_str() != Some("api.github.com") {
        return None;
    }
    let segments: Vec<_> = url.path_segments()?.collect();
    let resource = match segments.as_slice() {
        ["repos", owner, name, rest @ ..]
            if valid_repo_component(owner) && valid_repo_component(name) =>
        {
            rest
        }
        ["repositories", id, rest @ ..]
            if !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            rest
        }
        _ => return None,
    };
    match resource {
        ["commits", reference] if !reference.is_empty() => Some(resource.to_vec()),
        ["git", "trees", sha] if valid_sha(sha) => Some(resource.to_vec()),
        _ => None,
    }
}
fn repository_redirect(from: &Url, status: u16, location: Option<&str>) -> Result<Url> {
    let reject = || failure("skillDownloadFailed");
    if ![301, 302, 307, 308].contains(&status) {
        return Err(reject());
    }
    let location = location
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 2048
                && value.trim() == *value
                && !value.chars().any(char::is_control)
        })
        .ok_or_else(reject)?;
    let next = from.join(location).map_err(|_| reject())?;
    validate_http_url(&next).map_err(|_| reject())?;
    let original = repository_resource(from).ok_or_else(reject)?;
    if repository_resource(&next).as_ref() != Some(&original) || from.query() != next.query() {
        return Err(reject());
    }
    Ok(next)
}
async fn read_http_response<T: HttpRequester>(
    requester: &T,
    mut url: Url,
    max_bytes: u64,
) -> Result<Vec<u8>> {
    // The shared client itself keeps redirects disabled, including for MCP
    // probes. Never let reqwest follow a Location before this policy checks it.
    let mut visited = BTreeSet::new();
    for redirects in 0..=3 {
        validate_http_url(&url)?;
        if !visited.insert(url.to_string()) {
            return Err(failure("skillDownloadFailed"));
        }
        match requester.request(&url, max_bytes).await? {
            HttpRead::Bytes(bytes) => return Ok(bytes),
            HttpRead::Redirect { status, location } if redirects < 3 => {
                url = repository_redirect(&url, status, location.as_deref())?;
            }
            HttpRead::Redirect { .. } => return Err(failure("skillDownloadFailed")),
        }
    }
    Err(failure("skillDownloadFailed"))
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
#[derive(Clone, Deserialize)]
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
fn valid_repo_component(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 100
        && ![".", ".."].contains(&part)
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}
fn validate_repo(repo: &SkillRepo) -> Result<&str> {
    if [&repo.owner, &repo.name]
        .iter()
        .any(|part| !valid_repo_component(part))
    {
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
/// Discover descriptors without downloading repository assets, dependencies or archives.
pub(super) async fn catalog(repo: &SkillRepo) -> Result<Vec<super::DiscoverableSkill>> {
    let transport = HttpTransport(
        crate::proxy::http_client::mcp_check_client()
            .map_err(|_| failure("skillDownloadFailed"))?,
    );
    catalog_with(repo, &transport).await
}
async fn catalog_with<T: Transport>(
    repo: &SkillRepo,
    transport: &T,
) -> Result<Vec<super::DiscoverableSkill>> {
    use futures::{stream, StreamExt, TryStreamExt};
    let reference = validate_repo(repo)?;
    let limits = Limits::default();
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
    let mut url = endpoint(
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
    url.query_pairs_mut().append_pair("recursive", "1");
    let tree: GitTree = metadata(transport, url, limits.metadata_bytes).await?;
    if tree.truncated
        || !tree.sha.eq_ignore_ascii_case(&commit.commit.tree.sha)
        || tree.tree.len() > limits.tree_entries
    {
        return Err(failure("skillRepositoryInvalid"));
    }
    let documents: Vec<_> = tree
        .tree
        .iter()
        .filter(|entry| entry.path == "SKILL.md" || entry.path.ends_with("/SKILL.md"))
        .cloned()
        .collect();
    if documents.len() > 4000 {
        return Err(failure("skillTooLarge"));
    }
    let mut total = 0u64;
    for entry in &documents {
        artifacts::safe_relative(&entry.path).map_err(|_| failure("invalidSkillPath"))?;
        if entry.kind != "blob"
            || !["100644", "100755"].contains(&entry.mode.as_str())
            || !valid_sha(&entry.sha)
        {
            return Err(failure("skillLinkUnsupported"));
        }
        let size = entry
            .size
            .ok_or_else(|| failure("skillRepositoryInvalid"))?;
        total = total
            .checked_add(size)
            .ok_or_else(|| failure("skillTooLarge"))?;
        if size > 512 * 1024 || total > 64 * 1024 * 1024 {
            return Err(failure("skillTooLarge"));
        }
    }
    let commit = &commit.sha;
    let mut result: Vec<_> = stream::iter(documents.into_iter().map(|entry| async move {
        let mut segments = vec![repo.owner.as_str(), repo.name.as_str(), commit.as_str()];
        segments.extend(entry.path.split('/'));
        let bytes = read(
            transport,
            endpoint("raw.githubusercontent.com", &segments)?,
            512 * 1024,
        )
        .await?;
        if !matches_git_blob(&entry, &bytes) {
            return Err(failure("skillRepositoryInvalid"));
        }
        let content = std::str::from_utf8(&bytes).map_err(|_| failure("skillRepositoryInvalid"))?;
        let meta = super::SkillService::parse_skill_metadata_content(content);
        let path = entry.path.strip_suffix("/SKILL.md").unwrap_or_default();
        let directory = if path.is_empty() {
            repo.name.as_str()
        } else {
            path
        };
        Ok(super::DiscoverableSkill {
            key: format!(
                "{}/{}:{}",
                repo.owner.to_ascii_lowercase(),
                repo.name.to_ascii_lowercase(),
                path
            ),
            name: meta
                .name
                .unwrap_or_else(|| directory.rsplit('/').next().unwrap_or(directory).to_owned()),
            description: meta.description.unwrap_or_default(),
            directory: directory.to_owned(),
            readme_url: Some(super::SkillService::build_skill_doc_url(
                &repo.owner,
                &repo.name,
                commit,
                &entry.path,
            )),
            repo_owner: repo.owner.clone(),
            repo_name: repo.name.clone(),
            repo_branch: repo.branch.clone(),
        })
    }))
    .buffer_unordered(16)
    .try_collect()
    .await?;
    result.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(result)
}

pub(super) async fn download(
    repo: &SkillRepo,
    directory: &str,
    exact: bool,
) -> Result<(ExtractedDir, String)> {
    // Reuse the no-redirect client factory, including the user's existing proxy choice.
    let transport = HttpTransport(
        crate::proxy::http_client::mcp_check_client()
            .map_err(|_| failure("skillDownloadFailed"))?,
    );
    download_with_mode(repo, directory, &transport, Limits::default(), exact).await
}
#[cfg(test)]
async fn download_with<T: Transport>(
    repo: &SkillRepo,
    directory: &str,
    transport: &T,
    limits: Limits,
) -> Result<(ExtractedDir, String)> {
    download_with_mode(repo, directory, transport, limits, false).await
}
async fn download_with_mode<T: Transport>(
    repo: &SkillRepo,
    directory: &str,
    transport: &T,
    limits: Limits,
    exact: bool,
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
    let prefix = if exact {
        let document = if directory.is_empty() {
            "SKILL.md".into()
        } else {
            format!("{directory}/SKILL.md")
        };
        if !tree
            .tree
            .iter()
            .any(|entry| entry.path == document && entry.kind == "blob")
        {
            return Err(failure("skillSourceMissing"));
        }
        directory.to_owned()
    } else {
        selected_prefix(&tree.tree, directory)?
    };
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
    #[derive(Default)]
    struct HttpFixture {
        responses: BTreeMap<String, HttpRead>,
        requests: Mutex<Vec<String>>,
    }
    impl HttpRequester for HttpFixture {
        async fn request(&self, url: &Url, _max_bytes: u64) -> Result<HttpRead> {
            self.requests.lock().unwrap().push(url.to_string());
            match self.responses.get(url.as_str()) {
                Some(HttpRead::Bytes(bytes)) => Ok(HttpRead::Bytes(bytes.clone())),
                Some(HttpRead::Redirect { status, location }) => Ok(HttpRead::Redirect {
                    status: *status,
                    location: location.clone(),
                }),
                None => Err(anyhow!("private-transport-sentinel")),
            }
        }
    }
    impl Transport for HttpFixture {
        async fn read(&self, url: Url, max_bytes: u64) -> Result<Vec<u8>> {
            read_http_response(self, url, max_bytes).await
        }
    }
    fn moved_fixture() -> HttpFixture {
        let descriptor = b"---\nname: moved skill\ndescription: stays discoverable after a repository move\n---\n";
        let source = fixture(
            vec![blob("skills/chosen/SKILL.md", descriptor, false)],
            &[("skills/chosen/SKILL.md", descriptor)],
        );
        let mut data = HttpFixture::default();
        for (url, bytes) in source.responses {
            if url.starts_with("https://api.github.com/repos/fixture/skills/") {
                let moved = url.replace("/repos/fixture/skills/", "/repositories/123/");
                data.responses.insert(
                    url,
                    HttpRead::Redirect {
                        status: 301,
                        location: Some(moved.clone()),
                    },
                );
                data.responses.insert(moved, HttpRead::Bytes(bytes));
            } else {
                data.responses.insert(url, HttpRead::Bytes(bytes));
            }
        }
        data
    }
    #[tokio::test]
    async fn moved_repository_catalog_follows_only_same_github_repository_redirects() {
        let data = moved_fixture();
        let skills = catalog_with(&repo(), &data).await.unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].key, "fixture/skills:skills/chosen");
        assert_eq!(skills[0].name, "moved skill");
        assert_eq!(data.requests.lock().unwrap().len(), 5);
    }
    #[tokio::test]
    async fn moved_repository_selected_download_retains_pinned_blob_validation() {
        let data = moved_fixture();
        let (downloaded, commit) =
            download_with(&repo(), "skills/chosen", &data, Limits::default())
                .await
                .unwrap();
        assert_eq!(commit, COMMIT);
        assert!(downloaded.path().join("skills/chosen/SKILL.md").is_file());
        let requests = data.requests.lock().unwrap();
        assert_eq!(requests.len(), 5);
        assert!(requests.last().unwrap().contains(COMMIT));
    }
    fn redirect_fixture(from: &str, status: u16, location: Option<&str>) -> HttpFixture {
        let mut data = HttpFixture::default();
        data.responses.insert(
            from.to_owned(),
            HttpRead::Redirect {
                status,
                location: location.map(str::to_owned),
            },
        );
        data
    }
    #[tokio::test]
    async fn repository_redirects_accept_relative_and_canonical_paths_without_changing_resource() {
        let from = "https://api.github.com/repos/fixture/skills/commits/main";
        for status in [301, 302, 307, 308] {
            for location in [
                "/repositories/123/commits/main",
                "https://api.github.com/repos/moved/skills/commits/main",
            ] {
                let mut data = redirect_fixture(from, status, Some(location));
                let target = Url::parse(from).unwrap().join(location).unwrap();
                data.responses
                    .insert(target.to_string(), HttpRead::Bytes(b"body".to_vec()));
                assert_eq!(
                    read_http_response(&data, Url::parse(from).unwrap(), 4)
                        .await
                        .unwrap(),
                    b"body"
                );
                assert_eq!(data.requests.lock().unwrap().len(), 2);
            }
        }
    }
    #[tokio::test]
    async fn repository_redirects_reject_untrusted_or_changed_resources_before_following() {
        let from = "https://api.github.com/repos/fixture/skills/commits/main";
        for location in [
            None,
            Some(""),
            Some("https://evil.invalid/repositories/123/commits/main"),
            Some("https://api.github.com.evil.invalid/repositories/123/commits/main"),
            Some("https://raw.githubusercontent.com/repositories/123/commits/main"),
            Some("http://api.github.com/repositories/123/commits/main"),
            Some("https://user:private@api.github.com/repositories/123/commits/main"),
            Some("https://api.github.com:444/repositories/123/commits/main"),
            Some("https://api.github.com/repositories/123/commits/main#fragment"),
            Some("https://api.github.com/repositories/123/commits/other"),
            Some("https://api.github.com/repositories/123/commits/main?extra=1"),
            Some("https://api.github.com/users/123/commits/main"),
            Some("https://api.github.com/repositories/not-numeric/commits/main"),
            Some("https://api.github.com/repositories/123/commits/main\n"),
        ] {
            let data = redirect_fixture(from, 301, location);
            assert!(
                read_http_response(&data, Url::parse(from).unwrap(), 4)
                    .await
                    .is_err(),
                "accepted {location:?}"
            );
            assert_eq!(
                data.requests.lock().unwrap().len(),
                1,
                "followed {location:?}"
            );
        }
        for status in [300, 303, 304, 305, 306] {
            let data = redirect_fixture(from, status, Some("/repositories/123/commits/main"));
            assert!(read_http_response(&data, Url::parse(from).unwrap(), 4)
                .await
                .is_err());
            assert_eq!(data.requests.lock().unwrap().len(), 1);
        }
        let raw = "https://raw.githubusercontent.com/fixture/skills/main/SKILL.md";
        let data = redirect_fixture(
            raw,
            301,
            Some("https://raw.githubusercontent.com/moved/skills/main/SKILL.md"),
        );
        assert!(read_http_response(&data, Url::parse(raw).unwrap(), 4)
            .await
            .is_err());
        assert_eq!(data.requests.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn repository_redirects_preserve_tree_query_and_bound_hops_and_loops() {
        let from =
            format!("https://api.github.com/repos/fixture/skills/git/trees/{TREE}?recursive=1");
        let target =
            format!("https://api.github.com/repositories/123/git/trees/{TREE}?recursive=1");
        let mut data = redirect_fixture(&from, 301, Some(&target));
        data.responses
            .insert(target, HttpRead::Bytes(b"tree".to_vec()));
        assert_eq!(
            read_http_response(&data, Url::parse(&from).unwrap(), 4)
                .await
                .unwrap(),
            b"tree"
        );
        let data = redirect_fixture(&from, 301, Some(&from));
        assert!(read_http_response(&data, Url::parse(&from).unwrap(), 4)
            .await
            .is_err());
        assert_eq!(data.requests.lock().unwrap().len(), 1);
        let from = "https://api.github.com/repositories/1/commits/main";
        let mut data = HttpFixture::default();
        for id in 1..=4 {
            data.responses.insert(
                format!("https://api.github.com/repositories/{id}/commits/main"),
                HttpRead::Redirect {
                    status: 301,
                    location: Some(format!("/repositories/{}/commits/main", id + 1)),
                },
            );
        }
        assert!(read_http_response(&data, Url::parse(from).unwrap(), 4)
            .await
            .is_err());
        assert_eq!(data.requests.lock().unwrap().len(), 4);
    }
    #[tokio::test]
    async fn moved_repository_catalog_still_rejects_unverified_descriptor_bytes() {
        let mut data = moved_fixture();
        let raw = format!(
            "https://raw.githubusercontent.com/fixture/skills/{COMMIT}/skills/chosen/SKILL.md"
        );
        data.responses
            .insert(raw, HttpRead::Bytes(b"changed".to_vec()));
        assert_eq!(
            catalog_with(&repo(), &data)
                .await
                .err()
                .unwrap()
                .to_string(),
            "library.skillRepositoryInvalid"
        );
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
    async fn catalog_reads_only_descriptors_and_preserves_nested_and_root_identity() {
        let root = b"---\nname: root skill\ndescription: root description\n---\n";
        let nested = b"---\nname: nested skill\ndescription: searchable words\n---\n";
        let data = fixture(
            vec![
                blob("SKILL.md", root, false),
                blob("skills/nested/SKILL.md", nested, false),
                blob("large-unused.bin", b"unrelated", false),
            ],
            &[("SKILL.md", root), ("skills/nested/SKILL.md", nested)],
        );
        let skills = catalog_with(&repo(), &data).await.unwrap();
        assert_eq!(skills.len(), 2);
        let nested = skills
            .iter()
            .find(|skill| skill.name == "nested skill")
            .unwrap();
        assert_eq!(nested.key, "fixture/skills:skills/nested");
        assert_eq!(nested.directory, "skills/nested");
        assert_eq!(nested.description, "searchable words");
        let root = skills
            .iter()
            .find(|skill| skill.name == "root skill")
            .unwrap();
        assert_eq!(root.key, "fixture/skills:");
        assert_eq!(root.directory, "skills");
        assert_eq!(requests(&data).len(), 4);
        assert!(requests(&data)
            .iter()
            .all(|url| !url.contains("large-unused.bin")));
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
    async fn exact_selection_never_falls_back_to_a_moved_or_same_name_skill() {
        let fixture = fixture(
            vec![
                blob("nested/chosen/SKILL.md", b"# Other", false),
                blob("SKILL.md", b"# Root", false),
            ],
            &[],
        );
        assert_eq!(
            download_with_mode(&repo(), "chosen", &fixture, Limits::default(), true)
                .await
                .err()
                .unwrap()
                .to_string(),
            "library.skillSourceMissing"
        );
        assert_eq!(requests(&fixture).len(), 2);
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
