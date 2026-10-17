//! Skills 服务层
//!
//! v3.10.0+ 统一管理架构：
//! - SSOT（单一事实源）：`~/.ofox-desktop/skills/`
//! - 安装时下载到 SSOT，按需同步到各应用目录
//! - 数据库存储安装记录和启用状态

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::time::timeout;

use crate::app_config::{AppType, InstalledSkill, SkillApps, UnmanagedSkill};
use crate::config::{get_app_config_dir, get_home_dir};
use crate::database::Database;
use crate::error::format_skill_error;

#[path = "skill_selected_download.rs"]
mod selected_download;

// ========== 数据结构 ==========

/// Skill 同步方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SyncMethod {
    /// 自动选择：优先 symlink，失败时回退到 copy
    #[default]
    Auto,
    /// 符号链接（推荐，节省磁盘空间）
    Symlink,
    /// 文件复制（兼容模式）
    Copy,
}

/// Skill 存储位置（SSOT 目录选择）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillStorageLocation {
    /// Ofox Switch 管理目录 (~/.ofox-desktop/skills/)
    #[default]
    CcSwitch,
    /// Agent Skills 统一标准目录 (~/.agents/skills/)
    Unified,
}

/// 可发现的技能（来自仓库）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverableSkill {
    /// 唯一标识: "owner/name:directory"
    pub key: String,
    /// 显示名称 (从 SKILL.md 解析)
    pub name: String,
    /// 技能描述
    pub description: String,
    /// 目录名称 (安装路径的最后一段)
    pub directory: String,
    /// GitHub README URL
    #[serde(rename = "readmeUrl")]
    pub readme_url: Option<String>,
    /// 仓库所有者
    #[serde(rename = "repoOwner")]
    pub repo_owner: String,
    /// 仓库名称
    #[serde(rename = "repoName")]
    pub repo_name: String,
    /// 分支名称
    #[serde(rename = "repoBranch")]
    pub repo_branch: String,
}

/// 技能对象（兼容旧 API，内部使用 DiscoverableSkill）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// 唯一标识: "owner/name:directory" 或 "local:directory"
    pub key: String,
    /// 显示名称 (从 SKILL.md 解析)
    pub name: String,
    /// 技能描述
    pub description: String,
    /// 目录名称 (安装路径的最后一段)
    pub directory: String,
    /// GitHub README URL
    #[serde(rename = "readmeUrl")]
    pub readme_url: Option<String>,
    /// 是否已安装
    pub installed: bool,
    /// 仓库所有者
    #[serde(rename = "repoOwner")]
    pub repo_owner: Option<String>,
    /// 仓库名称
    #[serde(rename = "repoName")]
    pub repo_name: Option<String>,
    /// 分支名称
    #[serde(rename = "repoBranch")]
    pub repo_branch: Option<String>,
}

/// 仓库配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRepo {
    /// GitHub 用户/组织名
    pub owner: String,
    /// 仓库名称
    pub name: String,
    /// 分支 (默认 "main")
    pub branch: String,
    /// 是否启用
    pub enabled: bool,
}

/// 技能安装状态（旧版兼容）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillState {
    /// 是否已安装
    pub installed: bool,
    /// 安装时间
    #[serde(rename = "installedAt")]
    pub installed_at: DateTime<Utc>,
}

/// 持久化存储结构（仓库配置）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillStore {
    /// directory -> 安装状态（旧版兼容，新版不使用）
    pub skills: HashMap<String, SkillState>,
    /// 仓库列表
    pub repos: Vec<SkillRepo>,
}

impl Default for SkillStore {
    fn default() -> Self {
        SkillStore {
            skills: HashMap::new(),
            repos: vec![
                SkillRepo {
                    owner: "anthropics".to_string(),
                    name: "skills".to_string(),
                    branch: "main".to_string(),
                    enabled: true,
                },
                SkillRepo {
                    owner: "ComposioHQ".to_string(),
                    name: "awesome-claude-skills".to_string(),
                    branch: "master".to_string(),
                    enabled: true,
                },
                SkillRepo {
                    owner: "cexll".to_string(),
                    name: "myclaude".to_string(),
                    branch: "master".to_string(),
                    enabled: true,
                },
                SkillRepo {
                    owner: "JimLiu".to_string(),
                    name: "baoyu-skills".to_string(),
                    branch: "main".to_string(),
                    enabled: true,
                },
            ],
        }
    }
}

/// Skill 卸载结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUninstallResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
}

/// Skill 更新检测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdateInfo {
    /// Skill ID
    pub id: String,
    /// Skill 名称
    pub name: String,
    /// 当前本地哈希
    pub current_hash: Option<String>,
    /// 远程最新哈希
    pub remote_hash: String,
}

/// Skill 存储位置迁移结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationResult {
    pub migrated_count: usize,
    pub skipped_count: usize,
    pub errors: Vec<String>,
}

// ========== skills.sh API 类型 ==========

/// skills.sh API 原始响应
///
/// 注意：API 命名不一致（searchType 是 camelCase，duration_ms 是 snake_case），
/// 因此不能用 rename_all，需要逐字段指定。
#[derive(Debug, Clone, Deserialize)]
struct SkillsShApiResponse {
    pub query: String,
    #[serde(rename = "searchType")]
    #[allow(dead_code)]
    pub search_type: String,
    pub skills: Vec<SkillsShApiSkill>,
    pub count: usize,
    #[allow(dead_code)]
    pub duration_ms: u64,
}

/// skills.sh API 原始技能条目
#[derive(Debug, Clone, Deserialize)]
struct SkillsShApiSkill {
    pub id: String,
    #[serde(rename = "skillId")]
    pub skill_id: String,
    pub name: String,
    pub installs: u64,
    pub source: String,
}

/// skills.sh 搜索结果（返回给前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsShSearchResult {
    pub skills: Vec<SkillsShDiscoverableSkill>,
    pub total_count: usize,
    pub query: String,
}

/// skills.sh 可安装技能（返回给前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsShDiscoverableSkill {
    pub key: String,
    pub name: String,
    pub directory: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub repo_branch: String,
    pub installs: u64,
    pub readme_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillBackupEntry {
    pub backup_id: String,
    pub backup_path: String,
    pub created_at: i64,
    pub skill: InstalledSkill,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillBackupMetadata {
    skill: InstalledSkill,
    backup_created_at: i64,
    source_path: String,
    #[serde(default)]
    execution: Option<crate::services::library::skill_artifact::ExecutionMap>,
}

const SKILL_BACKUP_RETAIN_COUNT: usize = 20;

/// 技能元数据 (从 SKILL.md 解析)
#[derive(Debug, Clone, Deserialize)]
pub struct SkillMetadata {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// 导入已有 Skill 时，前端显式提交的启用应用选择
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSkillSelection {
    pub directory: String,
    #[serde(default)]
    pub apps: SkillApps,
}

fn get_agents_skills_dir() -> Option<PathBuf> {
    let path = get_home_dir().join(".agents/skills");
    path.is_dir().then_some(path)
}

// ========== SkillService ==========

pub struct SkillService;

impl Default for SkillService {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillService {
    pub fn new() -> Self {
        Self
    }

    /// 构建 Skill 文档 URL（指向仓库中的 SKILL.md 文件）
    fn build_skill_doc_url(owner: &str, repo: &str, branch: &str, doc_path: &str) -> String {
        format!("https://github.com/{owner}/{repo}/blob/{branch}/{doc_path}")
    }

    // ========== 路径管理 ==========

    /// 获取 SSOT 目录（根据设置返回 ~/.ofox-desktop/skills/ 或 ~/.agents/skills/）
    pub fn get_ssot_dir() -> Result<PathBuf> {
        Ok(match crate::settings::get_skill_storage_location() {
            SkillStorageLocation::CcSwitch => get_app_config_dir().join("skills"),
            SkillStorageLocation::Unified => get_home_dir().join(".agents").join("skills"),
        })
    }

    /// 获取 Skill 卸载备份目录（~/.ofox-desktop/skill-backups/）
    fn get_backup_dir() -> Result<PathBuf> {
        let dir = get_app_config_dir().join("skill-backups");
        crate::services::library::skill_artifact::validate_private_location(&dir)?;
        crate::config::create_private_dir_all(&dir)?;
        Ok(dir)
    }

    /// 获取应用的 skills 目录
    pub fn get_app_skills_dir(app: &AppType) -> Result<PathBuf> {
        // 目录覆盖：优先使用用户在 settings.json 中配置的 override 目录
        match app {
            // Claude Code and Codex are resolved below through `agent_env` (the settings
            // override, else CLAUDE_CONFIG_DIR / CODEX_HOME, else the default directory).
            AppType::Claude | AppType::Codex => {}
            AppType::Gemini => {
                if let Some(custom) = crate::settings::get_gemini_override_dir() {
                    return Ok(custom.join("skills"));
                }
            }
            AppType::OpenCode => {
                if let Some(custom) = crate::settings::get_opencode_override_dir() {
                    return Ok(custom.join("skills"));
                }
            }
            AppType::OpenClaw => {
                if let Some(custom) = crate::settings::get_openclaw_override_dir() {
                    return Ok(custom.join("skills"));
                }
            }
            AppType::Hermes => {
                if let Some(custom) = crate::settings::get_hermes_override_dir() {
                    return Ok(custom.join("skills"));
                }
            }
        }

        // Use the same home resolver as tool configuration. Windows ignores
        // HOME, so bypassing it would also bypass the isolated test home.
        let home = get_home_dir();

        Ok(match app {
            AppType::Claude => crate::config::get_claude_config_dir().join("skills"),
            AppType::Codex => crate::codex_config::get_codex_config_dir().join("skills"),
            AppType::Gemini => crate::gemini_config::get_gemini_dir().join("skills"),
            AppType::OpenCode => crate::opencode_config::get_opencode_dir().join("skills"),
            AppType::OpenClaw => home.join(".openclaw").join("skills"),
            AppType::Hermes => crate::hermes_config::get_hermes_dir().join("skills"),
        })
    }

    // ========== 统一管理方法 ==========

    /// 获取所有已安装的 Skills
    pub fn get_all_installed(db: &Arc<Database>) -> Result<Vec<InstalledSkill>> {
        let _io = crate::config_io::lock();
        let snapshot = db.library_snapshot()?;
        let roots = crate::services::library::skill_resource::native_paths()?;
        db.get_all_installed_skills()?
            .into_values()
            .map(|mut row| {
                crate::services::library::skill_resource::validate_skill(&row)?;
                let definition_hash = crate::services::library::skill_resource::spec_hash(&row)?;
                row.apps = SkillApps::default();
                for (tool, root) in &roots {
                    let physical = crate::services::library::skill_resource::entry_id(
                        &root.join(&row.directory),
                    )?;
                    if snapshot
                        .machine
                        .receipts
                        .get(&format!("skills:{physical}"))
                        .is_some_and(|receipt| {
                            receipt.kind == crate::services::library::ResourceKind::Skills
                                && receipt.resource_id == row.id
                                && receipt.owners.contains(tool)
                                && snapshot
                                    .machine
                                    .assignments
                                    .get(&format!("skills:{}:{tool}", row.id))
                                    == Some(&row.id)
                                && snapshot
                                    .definitions
                                    .skills
                                    .get(&row.id)
                                    .is_some_and(|meta| {
                                        meta.revision == receipt.resource_revision
                                            && meta.hash == receipt.definition_hash
                                            && meta.hash == definition_hash
                                    })
                                && crate::services::library::skill_resource::receipt_intact(receipt)
                        })
                    {
                        row.apps.set_enabled_for(&tool.parse()?, true);
                    }
                }
                Ok(row)
            })
            .collect()
    }

    /// 安装 Skill
    ///
    /// 流程：
    /// 1. 下载到 SSOT 目录
    /// 2. 保存到数据库
    /// 3. 同步到启用的应用目录
    pub async fn install(
        &self,
        db: &Arc<Database>,
        skill: &DiscoverableSkill,
        current_app: &AppType,
    ) -> Result<InstalledSkill> {
        crate::agent_env::ready().await;
        let _ = current_app;
        crate::services::library::skill_artifact::safe_relative(&skill.directory)?;
        crate::services::library::skill_resource::ensure_metadata(db)?;
        let existing = db.get_all_installed_skills()?;
        if let Some(row) = existing.values().find(|row| {
            row.directory.eq_ignore_ascii_case(
                skill
                    .directory
                    .rsplit('/')
                    .next()
                    .unwrap_or(&skill.directory),
            )
        }) {
            if row.id == skill.key {
                return Ok(row.clone());
            }
            return Err(anyhow!("library.skillNameConflict"));
        }
        let repo = SkillRepo {
            owner: skill.repo_owner.clone(),
            name: skill.repo_name.clone(),
            branch: skill.repo_branch.clone(),
            enabled: true,
        };
        let (downloaded, commit) = timeout(
            std::time::Duration::from_secs(60),
            self.download_selected_repo(&repo, &skill.directory),
        )
        .await
        .map_err(|_| anyhow!("library.skillDownloadTimeout"))??;
        let source = Self::resolve_skill_source_dir(downloaded.path(), &skill.directory)
            .ok_or_else(|| anyhow!("library.skillSourceAmbiguous"))?;
        let directory = skill
            .directory
            .rsplit('/')
            .next()
            .ok_or_else(|| anyhow!("library.invalidSkillPath"))?
            .to_string();
        let doc = source
            .strip_prefix(downloaded.path())?
            .join("SKILL.md")
            .to_string_lossy()
            .replace('\\', "/");
        let row = InstalledSkill {
            id: skill.key.clone(),
            name: skill.name.clone(),
            description: Some(skill.description.clone()),
            directory,
            repo_owner: Some(repo.owner.clone()),
            repo_name: Some(repo.name.clone()),
            repo_branch: Some(repo.branch),
            readme_url: Some(Self::build_skill_doc_url(
                &repo.owner,
                &repo.name,
                &commit,
                &doc,
            )),
            apps: SkillApps::default(),
            installed_at: Utc::now().timestamp(),
            content_hash: None,
            updated_at: 0,
        };
        Ok(
            crate::services::library::skill_resource::save_draft_with_execution(
                db,
                row,
                &source,
                None,
                Some(&downloaded.execution_for(&source)?),
            )?,
        )
    }

    /// 卸载 Skill
    ///
    /// 流程：
    /// 1. 从所有应用目录删除
    /// 2. 从 SSOT 删除
    /// 3. 从数据库删除
    pub fn uninstall(db: &Arc<Database>, id: &str) -> Result<SkillUninstallResult> {
        let _io = crate::config_io::lock();
        crate::services::library::skill_resource::ensure_metadata(db)?;
        let skill = db
            .get_installed_skill(id)?
            .ok_or_else(|| anyhow!("library.resourceMissing"))?;
        // Ownership is checked before backup or deletion; no native entry is touched.
        let snapshot = db.library_snapshot()?;
        if snapshot.machine.receipts.values().any(|receipt| {
            receipt.kind == crate::services::library::ResourceKind::Skills
                && receipt.resource_id == id
        }) || snapshot
            .machine
            .assignments
            .iter()
            .any(|(key, value)| key.starts_with("skills:") && value == id)
        {
            return Err(anyhow!("library.withdrawRequired"));
        }
        let backup_path =
            Self::create_uninstall_backup(&skill)?.map(|path| path.to_string_lossy().into_owned());
        crate::services::library::skill_resource::delete_draft(db, id)?;
        Ok(SkillUninstallResult { backup_path })
    }

    // ========== 更新检测 ==========

    /// 计算目录内容的 SHA-256 哈希
    ///
    /// 递归遍历目录下所有非隐藏文件，按相对路径字典序排列，
    /// 将 "相对路径\0内容\0" 逐文件 feed 给同一个 hasher。
    pub fn compute_dir_hash(dir: &Path) -> Result<String> {
        Ok(crate::services::library::skill_artifact::tree_hash(dir)?)
    }

    /// 检查所有已安装 Skill 的更新
    ///
    /// 仅检查有 repo_owner 的 Skill（本地 Skill 跳过），
    /// 按仓库分组下载，避免重复下载同一仓库。
    pub async fn check_updates(&self, db: &Arc<Database>) -> Result<Vec<SkillUpdateInfo>> {
        crate::agent_env::ready().await;
        Ok(crate::services::library::skill_actions::legacy_check(db).await?)
    }

    /// Legacy callers resolve their tracking ref at this call, and update only a draft.
    /// The resource-library UI uses its frozen ticket instead.
    pub async fn update_skill(&self, db: &Arc<Database>, skill_id: &str) -> Result<InstalledSkill> {
        crate::agent_env::ready().await;
        Ok(crate::services::library::skill_actions::legacy_update(db, skill_id).await?)
    }

    /// 为缺少 content_hash 的已安装 Skill 补算哈希
    pub fn backfill_content_hashes(db: &Arc<Database>) -> Result<usize> {
        let before = db.library_snapshot()?.definitions.skills.len();
        crate::services::library::skill_resource::ensure_metadata(db)?;
        Ok(db
            .library_snapshot()?
            .definitions
            .skills
            .len()
            .saturating_sub(before))
    }

    /// 迁移 Skill 存储位置（在两个 SSOT 目录间移动文件）
    ///
    /// 安全策略：先移文件，后改设置。中途崩溃时设置仍指向旧目录。
    pub fn migrate_storage(
        db: &Arc<Database>,
        target: SkillStorageLocation,
    ) -> Result<MigrationResult> {
        let _ = (db, target);
        Err(anyhow!("library.previewRequired"))
    }

    pub fn list_backups() -> Result<Vec<SkillBackupEntry>> {
        let backup_dir = Self::get_backup_dir()?;
        let mut entries = Vec::new();

        for entry in fs::read_dir(&backup_dir)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    log::warn!("读取 Skill 备份目录项失败: {err}");
                    continue;
                }
            };
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            match Self::read_backup_metadata(&path) {
                Ok(metadata) => entries.push(SkillBackupEntry {
                    backup_id: entry.file_name().to_string_lossy().to_string(),
                    backup_path: path.to_string_lossy().to_string(),
                    created_at: metadata.backup_created_at,
                    skill: metadata.skill,
                }),
                Err(err) => {
                    log::warn!("解析 Skill 备份失败 {}: {err:#}", path.display());
                }
            }
        }

        entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
        Ok(entries)
    }

    pub fn delete_backup(backup_id: &str) -> Result<()> {
        let backup_path = Self::backup_path_for_id(backup_id)?;
        let metadata = fs::symlink_metadata(&backup_path)
            .with_context(|| format!("failed to access {}", backup_path.display()))?;

        if !metadata.is_dir() {
            return Err(anyhow!(
                "Skill backup is not a directory: {}",
                backup_path.display()
            ));
        }

        fs::remove_dir_all(&backup_path)
            .with_context(|| format!("failed to delete {}", backup_path.display()))?;

        log::info!("Skill 备份已删除: {}", backup_path.display());
        Ok(())
    }

    pub fn restore_from_backup(
        db: &Arc<Database>,
        backup_id: &str,
        current_app: &AppType,
    ) -> Result<InstalledSkill> {
        let _ = current_app;
        let backup = Self::backup_path_for_id(backup_id)?;
        let metadata = Self::read_backup_metadata(&backup)?;
        crate::services::library::skill_artifact::safe_leaf(&metadata.skill.directory)?;
        let mut skill = metadata.skill;
        skill.apps = SkillApps::default();
        skill.installed_at = Utc::now().timestamp();
        skill.updated_at = 0;
        Ok(
            crate::services::library::skill_resource::save_draft_with_execution(
                db,
                skill,
                &backup.join("skill"),
                None,
                metadata.execution.as_ref(),
            )?,
        )
    }

    /// 切换应用启用状态
    ///
    /// 启用：复制到应用目录
    /// 禁用：从应用目录删除
    pub fn toggle_app(db: &Arc<Database>, id: &str, app: &AppType, enabled: bool) -> Result<()> {
        let _ = (db, id, app, enabled);
        Err(anyhow!("library.previewRequired"))
    }

    /// 扫描未管理的 Skills
    ///
    /// 扫描各应用目录，找出未被 Ofox Switch 管理的 Skills
    pub fn scan_unmanaged(db: &Arc<Database>) -> Result<Vec<UnmanagedSkill>> {
        let managed_skills = db.get_all_installed_skills()?;
        let managed_dirs: HashSet<String> = managed_skills
            .values()
            .map(|s| s.directory.clone())
            .collect();

        // 收集所有待扫描的目录及其来源标签
        let mut scan_sources: Vec<(PathBuf, String)> = Vec::new();
        for app in AppType::all() {
            if let Ok(d) = Self::get_app_skills_dir(&app) {
                scan_sources.push((d, app.as_str().to_string()));
            }
        }
        if let Some(agents_dir) = get_agents_skills_dir() {
            scan_sources.push((agents_dir, "agents".to_string()));
        }
        if let Ok(ssot_dir) = Self::get_ssot_dir() {
            scan_sources.push((ssot_dir, "ofox-switch".to_string()));
        }

        let mut unmanaged: HashMap<String, UnmanagedSkill> = HashMap::new();

        for (scan_dir, label) in &scan_sources {
            let entries = match fs::read_dir(scan_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let dir_name = entry.file_name().to_string_lossy().to_string();
                if dir_name.starts_with('.') || managed_dirs.contains(&dir_name) {
                    continue;
                }

                let skill_md = path.join("SKILL.md");
                if !skill_md.exists() {
                    continue;
                }
                let (name, description) = Self::read_skill_name_desc(&skill_md, &dir_name);

                unmanaged
                    .entry(dir_name.clone())
                    .and_modify(|s| s.found_in.push(label.clone()))
                    .or_insert(UnmanagedSkill {
                        directory: dir_name,
                        name,
                        description,
                        found_in: vec![label.clone()],
                        path: path.display().to_string(),
                    });
            }
        }

        Ok(unmanaged.into_values().collect())
    }

    /// 从应用目录导入 Skills
    ///
    /// 将未管理的 Skills 导入到 Ofox Switch 统一管理
    pub fn import_from_apps(
        db: &Arc<Database>,
        imports: Vec<ImportSkillSelection>,
    ) -> Result<Vec<InstalledSkill>> {
        let _guard = crate::config_io::lock();
        let mut prepared = Vec::new();
        for selection in imports {
            crate::services::library::skill_artifact::safe_leaf(&selection.directory)?;
            let mut candidates = Vec::new();
            let apps = if selection.apps.is_empty() {
                AppType::all().collect()
            } else {
                selection.apps.enabled_apps()
            };
            for app in apps {
                let source = Self::get_app_skills_dir(&app)?.join(&selection.directory);
                if source.is_dir() {
                    candidates.push(source.canonicalize()?);
                }
            }
            if selection.apps.is_empty() {
                let source = get_home_dir()
                    .join(".agents")
                    .join("skills")
                    .join(&selection.directory);
                if source.is_dir() {
                    candidates.push(source.canonicalize()?);
                }
            }
            candidates.sort();
            candidates.dedup();
            let mut hashes = HashSet::new();
            for source in &candidates {
                hashes.insert(Self::compute_dir_hash(source)?);
            }
            if hashes.len() > 1 {
                return Err(anyhow!("library.skillSourceAmbiguous"));
            }
            let source = candidates
                .first()
                .ok_or_else(|| anyhow!("library.skillSourceMissing"))?
                .clone();
            let (name, description) =
                Self::read_skill_name_desc(&source.join("SKILL.md"), &selection.directory);
            prepared.push((
                InstalledSkill {
                    id: format!("local:{}", selection.directory),
                    name,
                    description,
                    directory: selection.directory,
                    repo_owner: None,
                    repo_name: None,
                    repo_branch: None,
                    readme_url: None,
                    apps: SkillApps::default(),
                    installed_at: Utc::now().timestamp(),
                    content_hash: None,
                    updated_at: 0,
                },
                source,
            ));
        }
        let mut imported = Vec::new();
        for (skill, source) in prepared {
            if let Some(existing) = db.get_installed_skill(&skill.id)? {
                imported.push(existing);
                continue;
            }
            imported.push(crate::services::library::skill_resource::save_draft(
                db, skill, &source, None,
            )?);
        }
        Ok(imported)
    }

    // ========== 文件同步方法 ==========

    /// 同步 Skill 到应用目录（使用 symlink 或 copy）
    ///
    /// 根据配置和平台选择最佳同步方式：
    /// - Auto: 优先尝试 symlink，失败时回退到 copy
    /// - Symlink: 仅使用 symlink
    /// - Copy: 仅使用文件复制
    pub fn sync_to_app_dir(directory: &str, app: &AppType) -> Result<()> {
        let _ = (directory, app);
        Err(anyhow!("library.previewRequired"))
    }

    /// 复制 Skill 到应用目录（保留用于向后兼容）
    #[deprecated(note = "请使用 sync_to_app_dir() 代替")]
    pub fn copy_to_app(directory: &str, app: &AppType) -> Result<()> {
        Self::sync_to_app_dir(directory, app)
    }

    /// 从应用目录删除 Skill（支持 symlink 和真实目录）
    pub fn remove_from_app(directory: &str, app: &AppType) -> Result<()> {
        let _ = (directory, app);
        Err(anyhow!("library.previewRequired"))
    }

    /// 同步所有已启用的 Skills 到指定应用
    pub fn sync_to_app(db: &Arc<Database>, app: &AppType) -> Result<()> {
        let _ = app;
        db.library_snapshot()?;
        Ok(())
    }

    // ========== 发现功能（保留原有逻辑）==========

    /// 列出所有可发现的技能（从仓库获取）
    pub async fn discover_available(
        &self,
        repos: Vec<SkillRepo>,
    ) -> Result<Vec<DiscoverableSkill>> {
        let mut skills = Vec::new();

        // 仅使用启用的仓库
        let enabled_repos: Vec<SkillRepo> = repos.into_iter().filter(|repo| repo.enabled).collect();

        let fetch_tasks = enabled_repos
            .iter()
            .map(|repo| self.fetch_repo_skills(repo));

        let results: Vec<Result<Vec<DiscoverableSkill>>> =
            futures::future::join_all(fetch_tasks).await;

        for (repo, result) in enabled_repos.into_iter().zip(results) {
            match result {
                Ok(repo_skills) => skills.extend(repo_skills),
                Err(e) => log::warn!("获取仓库 {}/{} 技能失败: {}", repo.owner, repo.name, e),
            }
        }

        // 去重并排序
        Self::deduplicate_discoverable_skills(&mut skills);
        skills.sort_by_key(|skill| skill.name.to_lowercase());

        Ok(skills)
    }

    /// 列出所有技能（兼容旧 API）
    pub async fn list_skills(
        &self,
        repos: Vec<SkillRepo>,
        db: &Arc<Database>,
    ) -> Result<Vec<Skill>> {
        // 获取可发现的技能
        let discoverable = self.discover_available(repos).await?;

        // 获取已安装的技能
        let installed = db.get_all_installed_skills()?;
        let installed_dirs: HashSet<String> =
            installed.values().map(|s| s.directory.clone()).collect();

        // 转换为 Skill 格式
        let mut skills: Vec<Skill> = discoverable
            .into_iter()
            .map(|d| {
                let install_name = Path::new(&d.directory)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| d.directory.clone());

                Skill {
                    key: d.key,
                    name: d.name,
                    description: d.description,
                    directory: d.directory,
                    readme_url: d.readme_url,
                    installed: installed_dirs.contains(&install_name),
                    repo_owner: Some(d.repo_owner),
                    repo_name: Some(d.repo_name),
                    repo_branch: Some(d.repo_branch),
                }
            })
            .collect();

        // 添加本地已安装但不在仓库中的技能
        for skill in installed.values() {
            let already_in_list = skills.iter().any(|s| {
                let s_install_name = Path::new(&s.directory)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| s.directory.clone());
                s_install_name == skill.directory
            });

            if !already_in_list {
                skills.push(Skill {
                    key: skill.id.clone(),
                    name: skill.name.clone(),
                    description: skill.description.clone().unwrap_or_default(),
                    directory: skill.directory.clone(),
                    readme_url: skill.readme_url.clone(),
                    installed: true,
                    repo_owner: skill.repo_owner.clone(),
                    repo_name: skill.repo_name.clone(),
                    repo_branch: skill.repo_branch.clone(),
                });
            }
        }

        skills.sort_by_key(|skill| skill.name.to_lowercase());

        Ok(skills)
    }

    /// 从仓库获取技能列表
    async fn fetch_repo_skills(&self, repo: &SkillRepo) -> Result<Vec<DiscoverableSkill>> {
        let (temp_dir, resolved_branch) =
            timeout(std::time::Duration::from_secs(60), self.download_repo(repo))
                .await
                .map_err(|_| {
                    anyhow!(format_skill_error(
                        "DOWNLOAD_TIMEOUT",
                        &[
                            ("owner", &repo.owner),
                            ("name", &repo.name),
                            ("timeout", "60")
                        ],
                        Some("checkNetwork"),
                    ))
                })??;

        let mut skills = Vec::new();
        let scan_dir = temp_dir.path();
        let mut resolved_repo = repo.clone();
        resolved_repo.branch = resolved_branch;
        self.scan_dir_recursive(scan_dir, scan_dir, &resolved_repo, &mut skills)?;
        for skill in &mut skills {
            skill.repo_branch = repo.branch.clone();
        }

        // TempDir removes unpublished files on every exit.

        Ok(skills)
    }

    /// 递归扫描目录查找 SKILL.md
    fn scan_dir_recursive(
        &self,
        current_dir: &Path,
        base_dir: &Path,
        repo: &SkillRepo,
        skills: &mut Vec<DiscoverableSkill>,
    ) -> Result<()> {
        let skill_md = current_dir.join("SKILL.md");

        if skill_md.exists() {
            let directory = if current_dir == base_dir {
                repo.name.clone()
            } else {
                current_dir
                    .strip_prefix(base_dir)
                    .unwrap_or(current_dir)
                    .to_string_lossy()
                    .to_string()
            };

            let doc_path = skill_md
                .strip_prefix(base_dir)
                .unwrap_or(skill_md.as_path())
                .to_string_lossy()
                .replace('\\', "/");

            if let Ok(skill) =
                self.build_skill_from_metadata(&skill_md, &directory, &doc_path, repo)
            {
                skills.push(skill);
            }

            return Ok(());
        }

        for entry in fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                self.scan_dir_recursive(&path, base_dir, repo, skills)?;
            }
        }

        Ok(())
    }

    /// 从 SKILL.md 构建技能对象
    fn build_skill_from_metadata(
        &self,
        skill_md: &Path,
        directory: &str,
        doc_path: &str,
        repo: &SkillRepo,
    ) -> Result<DiscoverableSkill> {
        let meta = self.parse_skill_metadata(skill_md)?;

        Ok(DiscoverableSkill {
            key: format!("{}/{}:{}", repo.owner, repo.name, directory),
            name: meta.name.unwrap_or_else(|| directory.to_string()),
            description: meta.description.unwrap_or_default(),
            directory: directory.to_string(),
            readme_url: Some(Self::build_skill_doc_url(
                &repo.owner,
                &repo.name,
                &repo.branch,
                doc_path,
            )),
            repo_owner: repo.owner.clone(),
            repo_name: repo.name.clone(),
            repo_branch: repo.branch.clone(),
        })
    }

    /// 解析技能元数据
    fn parse_skill_metadata(&self, path: &Path) -> Result<SkillMetadata> {
        Self::parse_skill_metadata_static(path)
    }

    /// 静态方法：解析技能元数据
    fn parse_skill_metadata_static(path: &Path) -> Result<SkillMetadata> {
        let content = fs::read_to_string(path)?;
        let content = content.trim_start_matches('\u{feff}');

        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            return Ok(SkillMetadata {
                name: None,
                description: None,
            });
        }

        let front_matter = parts[1].trim();
        let meta: SkillMetadata = serde_yaml::from_str(front_matter).unwrap_or(SkillMetadata {
            name: None,
            description: None,
        });

        Ok(meta)
    }

    /// 从 SKILL.md 读取名称和描述，不存在则用目录名兜底
    pub(crate) fn read_skill_name_desc(
        skill_md: &Path,
        fallback_name: &str,
    ) -> (String, Option<String>) {
        if skill_md.exists() {
            match Self::parse_skill_metadata_static(skill_md) {
                Ok(meta) => (
                    meta.name.unwrap_or_else(|| fallback_name.to_string()),
                    meta.description,
                ),
                Err(_) => (fallback_name.to_string(), None),
            }
        } else {
            (fallback_name.to_string(), None)
        }
    }

    /// 校验并规范化技能源路径（允许多级目录），拒绝路径穿越和绝对路径
    fn sanitize_skill_source_path(raw: &str) -> Option<PathBuf> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut normalized = PathBuf::new();
        let mut has_component = false;

        for component in Path::new(trimmed).components() {
            match component {
                Component::Normal(name) => {
                    let segment = name.to_string_lossy().trim().to_string();
                    if segment.is_empty() || segment == "." || segment == ".." {
                        return None;
                    }
                    normalized.push(segment);
                    has_component = true;
                }
                Component::CurDir
                | Component::ParentDir
                | Component::RootDir
                | Component::Prefix(_) => {
                    return None;
                }
            }
        }

        has_component.then_some(normalized)
    }

    /// 在目录树中查找名称匹配且包含 SKILL.md 的子目录
    ///
    /// 用于 skills.sh 安装回退：API 只返回 skillId（如 "find-skills"），
    /// 但实际文件可能在仓库子目录中（如 "skills/find-skills"）。
    fn find_skill_dir_by_name(root: &Path, target_name: &str) -> Option<Vec<PathBuf>> {
        fn walk(dir: &Path, target: &str, depth: usize, matches: &mut Vec<PathBuf>) -> Option<()> {
            if depth > 32 {
                return None;
            }
            for entry in fs::read_dir(dir).ok()? {
                let entry = entry.ok()?;
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).ok()?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    continue;
                }
                let name = entry.file_name();
                let name = name.to_str()?;
                if name.starts_with('.') {
                    continue;
                }
                if name.eq_ignore_ascii_case(target) && path.join("SKILL.md").is_file() {
                    matches.push(path.clone());
                }
                walk(&path, target, depth + 1, matches)?;
            }
            Some(())
        }
        let mut matches = Vec::new();
        walk(root, target_name, 0, &mut matches)?;
        Some(matches)
    }

    /// 将 discoverable skill 的目录信息重新解析为解压目录中的真实源目录。
    ///
    /// 兼容三种情况：
    /// 1. `skills/foo` 这类直接相对路径；
    /// 2. 仅持有安装名 `foo`，需要在仓库中递归查找真实目录；
    /// 3. 仓库根目录本身就是 skill，此时回退到解压根目录。
    fn resolve_skill_source_dir(root: &Path, raw_directory: &str) -> Option<PathBuf> {
        let source_rel = Self::sanitize_skill_source_path(raw_directory)?;
        let direct = root.join(&source_rel);
        if direct.is_dir() && direct.join("SKILL.md").is_file() {
            return Some(direct);
        }
        if source_rel.components().count() > 1 {
            return None;
        }
        let target_name = source_rel.file_name()?.to_string_lossy().to_string();
        let mut candidates = Self::find_skill_dir_by_name(root, &target_name)?;
        if candidates.len() > 1 {
            return None;
        }
        if let Some(found) = candidates.pop() {
            log::info!(
                "Skill directory '{}' not found at direct path, using fallback: {}",
                target_name,
                found.display()
            );
            return Some(found);
        }

        if root.is_dir() && root.join("SKILL.md").exists() {
            log::info!(
                "Skill directory '{}' not found, but SKILL.md exists at root, using repo root",
                target_name,
            );
            return Some(root.to_path_buf());
        }

        None
    }

    /// 去重技能列表（基于完整 key，不同仓库的同名 skill 分开显示）
    fn deduplicate_discoverable_skills(skills: &mut Vec<DiscoverableSkill>) {
        let mut seen = HashMap::new();
        skills.retain(|skill| {
            // 使用完整 key（owner/repo:directory）作为唯一标识
            // 这样不同仓库的同名 skill 会分开显示
            let unique_key = skill.key.to_lowercase();
            if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(unique_key) {
                e.insert(true);
                true
            } else {
                false
            }
        });
    }

    /// Download only a selected immutable skill tree; browsing/discovery keeps its separate path.
    pub(crate) async fn download_selected_repo(
        &self,
        repo: &SkillRepo,
        directory: &str,
    ) -> Result<(
        crate::services::library::skill_artifact::ExtractedDir,
        String,
    )> {
        selected_download::download(repo, directory).await
    }

    /// 下载仓库
    pub(crate) async fn download_repo(
        &self,
        repo: &SkillRepo,
    ) -> Result<(
        crate::services::library::skill_artifact::ExtractedDir,
        String,
    )> {
        if [&repo.owner, &repo.name].iter().any(|part| {
            part.is_empty()
                || part.len() > 100
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
                || [".", ".."].contains(&part.as_str())
        }) {
            return Err(anyhow!("library.skillRepositoryInvalid"));
        }
        let reference = if repo.branch.trim().is_empty() {
            "HEAD"
        } else {
            repo.branch.as_str()
        };
        if reference.len() > 256 || reference.chars().any(char::is_control) {
            return Err(anyhow!("library.skillRepositoryInvalid"));
        }
        let mut api = url::Url::parse("https://api.github.com/")?;
        api.path_segments_mut()
            .map_err(|_| anyhow!("library.skillRepositoryInvalid"))?
            .extend(["repos", &repo.owner, &repo.name, "commits", reference]);
        let client = crate::proxy::http_client::get();
        let response = client
            .get(api)
            .header("User-Agent", "ofox-desktop")
            .send()
            .await?
            .error_for_status()?;
        let commit: serde_json::Value = response.json().await?;
        let sha = commit
            .get("sha")
            .and_then(serde_json::Value::as_str)
            .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| anyhow!("library.skillRepositoryInvalid"))?
            .to_string();
        let mut archive = url::Url::parse("https://github.com/")?;
        archive
            .path_segments_mut()
            .map_err(|_| anyhow!("library.skillRepositoryInvalid"))?
            .extend([
                repo.owner.as_str(),
                repo.name.as_str(),
                "archive",
                &format!("{sha}.zip"),
            ]);
        let mut response = client.get(archive).send().await?.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len().saturating_add(chunk.len()) as u64
                > crate::services::library::skill_artifact::MAX_BYTES
            {
                return Err(anyhow!("library.skillTooLarge"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((
            crate::services::library::skill_artifact::extract_zip(&bytes, true)?,
            sha,
        ))
    }

    fn resolve_uninstall_backup_source(skill: &InstalledSkill) -> Result<Option<PathBuf>> {
        crate::services::library::skill_artifact::safe_leaf(&skill.directory)?;
        if let Some(hash) = &skill.content_hash {
            let path = crate::services::library::skill_artifact::path(hash)?;
            if path.is_dir() {
                return Ok(Some(path));
            }
        }
        let ssot_path = Self::get_ssot_dir()?.join(&skill.directory);
        if ssot_path.is_dir() {
            return Ok(Some(ssot_path));
        }

        for app in AppType::all() {
            let app_dir = match Self::get_app_skills_dir(&app) {
                Ok(dir) => dir,
                Err(_) => continue,
            };
            let candidate = app_dir.join(&skill.directory);
            if candidate.is_dir() {
                return Ok(Some(candidate));
            }
        }

        Ok(None)
    }

    fn sanitize_backup_segment(segment: &str) -> String {
        let sanitized = segment
            .chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
                _ => '-',
            })
            .collect::<String>()
            .trim_matches('-')
            .to_string();

        if sanitized.is_empty() {
            "skill".to_string()
        } else {
            sanitized
        }
    }

    fn cleanup_old_skill_backups(dir: &Path) -> Result<()> {
        let mut entries = fs::read_dir(dir)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let metadata = entry.metadata().ok()?;
                if !metadata.is_dir() {
                    return None;
                }
                Some((entry.path(), metadata.modified().ok()))
            })
            .collect::<Vec<_>>();

        if entries.len() <= SKILL_BACKUP_RETAIN_COUNT {
            return Ok(());
        }

        entries.sort_by_key(|(_, modified)| *modified);
        let remove_count = entries.len().saturating_sub(SKILL_BACKUP_RETAIN_COUNT);

        for (path, _) in entries.into_iter().take(remove_count) {
            fs::remove_dir_all(&path)?;
        }

        Ok(())
    }

    fn backup_path_for_id(backup_id: &str) -> Result<PathBuf> {
        if backup_id.contains("..")
            || backup_id.contains('/')
            || backup_id.contains('\\')
            || backup_id.trim().is_empty()
        {
            return Err(anyhow!("Invalid backup id: {backup_id}"));
        }

        Ok(Self::get_backup_dir()?.join(backup_id))
    }

    fn read_backup_metadata(backup_path: &Path) -> Result<SkillBackupMetadata> {
        let metadata_path = backup_path.join("meta.json");
        let content = fs::read_to_string(&metadata_path)
            .with_context(|| format!("failed to read {}", metadata_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", metadata_path.display()))
    }

    fn create_uninstall_backup(skill: &InstalledSkill) -> Result<Option<PathBuf>> {
        let Some(source_path) = Self::resolve_uninstall_backup_source(skill)? else {
            log::warn!(
                "Skill {} 卸载前未找到可备份的目录，将跳过备份",
                skill.directory
            );
            return Ok(None);
        };

        let backup_root = Self::get_backup_dir()?;
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let slug = Self::sanitize_backup_segment(&skill.directory);
        let mut backup_path = backup_root.join(format!("{timestamp}_{slug}"));
        let mut counter = 1;
        while backup_path.exists() {
            backup_path = backup_root.join(format!("{timestamp}_{slug}_{counter}"));
            counter += 1;
        }

        let write_backup = || -> Result<()> {
            let skill_backup_dir = backup_path.join("skill");
            crate::services::library::skill_artifact::copy_verified(
                &source_path,
                &skill_backup_dir,
            )?;

            let metadata = SkillBackupMetadata {
                skill: skill.clone(),
                backup_created_at: Utc::now().timestamp(),
                source_path: source_path.to_string_lossy().to_string(),
                execution: skill
                    .content_hash
                    .as_deref()
                    .map(crate::services::library::skill_artifact::manifest)
                    .transpose()?,
            };
            let metadata_path = backup_path.join("meta.json");
            let metadata_json = serde_json::to_string_pretty(&metadata)
                .context("failed to serialize skill backup metadata")?;
            crate::config::atomic_write(&metadata_path, metadata_json.as_bytes())
                .with_context(|| format!("failed to write {}", metadata_path.display()))?;
            Ok(())
        };

        if let Err(err) = write_backup() {
            let _ = fs::remove_dir_all(&backup_path);
            return Err(err);
        }

        if let Err(err) = Self::cleanup_old_skill_backups(&backup_root) {
            log::warn!("清理旧 Skill 备份失败: {err:#}");
        }

        log::info!(
            "Skill {} 已在卸载前备份到 {}",
            skill.name,
            backup_path.display()
        );

        Ok(Some(backup_path))
    }

    // ========== 从 ZIP 文件安装 ==========

    /// 从本地 ZIP 文件安装 Skills
    ///
    /// 流程：
    /// 1. 解压 ZIP 到临时目录
    /// 2. 扫描目录查找包含 SKILL.md 的技能
    /// 3. 复制到 SSOT 并保存到数据库
    /// 4. 同步到当前应用目录
    pub fn install_from_zip(
        db: &Arc<Database>,
        zip_path: &Path,
        current_app: &AppType,
    ) -> Result<Vec<InstalledSkill>> {
        let _ = current_app;
        use std::io::Read;
        let mut bytes = Vec::new();
        fs::File::open(zip_path)?
            .take(crate::services::library::skill_artifact::MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        let extracted = crate::services::library::skill_artifact::extract_zip(&bytes, false)?;
        let dirs = Self::scan_skills_in_dir(extracted.path())?;
        if dirs.is_empty() {
            return Err(anyhow!("library.skillDocumentMissing"));
        }
        let mut names = HashSet::new();
        let mut prepared = Vec::new();
        for source in dirs {
            let directory = if source == extracted.path() {
                zip_path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| anyhow!("library.invalidSkillPath"))?
                    .to_string()
            } else {
                source
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| anyhow!("library.invalidSkillPath"))?
                    .to_string()
            };
            crate::services::library::skill_artifact::safe_leaf(&directory)?;
            if !names.insert(directory.to_lowercase()) {
                return Err(anyhow!("library.skillPathCollision"));
            }
            Self::compute_dir_hash(&source)?;
            let (name, description) =
                Self::read_skill_name_desc(&source.join("SKILL.md"), &directory);
            prepared.push((
                InstalledSkill {
                    id: format!("local:{directory}"),
                    name,
                    description,
                    directory,
                    repo_owner: None,
                    repo_name: None,
                    repo_branch: None,
                    readme_url: None,
                    apps: SkillApps::default(),
                    installed_at: Utc::now().timestamp(),
                    content_hash: None,
                    updated_at: 0,
                },
                source,
            ));
        }
        let existing = db.get_all_installed_skills()?;
        if prepared.iter().any(|(skill, _)| {
            existing
                .values()
                .any(|row| row.directory.eq_ignore_ascii_case(&skill.directory))
        }) {
            return Err(anyhow!("library.skillNameConflict"));
        }
        let mut imported = Vec::new();
        for (skill, source) in prepared {
            imported.push(
                crate::services::library::skill_resource::save_draft_with_execution(
                    db,
                    skill,
                    &source,
                    None,
                    Some(&extracted.execution_for(&source)?),
                )?,
            );
        }
        Ok(imported)
    }

    /// 递归扫描目录查找包含 SKILL.md 的技能目录
    fn scan_skills_in_dir(dir: &Path) -> Result<Vec<PathBuf>> {
        let mut skill_dirs = Vec::new();
        Self::scan_skills_recursive(dir, &mut skill_dirs)?;
        Ok(skill_dirs)
    }

    /// 递归扫描辅助函数
    fn scan_skills_recursive(current: &Path, results: &mut Vec<PathBuf>) -> Result<()> {
        // 检查当前目录是否包含 SKILL.md
        let skill_md = current.join("SKILL.md");
        if skill_md.exists() {
            results.push(current.to_path_buf());
            // 找到后不再递归子目录（一个 skill 目录）
            return Ok(());
        }

        // 递归子目录
        if let Ok(entries) = fs::read_dir(current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // 跳过隐藏目录
                    let dir_name = entry.file_name().to_string_lossy().to_string();
                    if dir_name.starts_with('.') {
                        continue;
                    }
                    Self::scan_skills_recursive(&path, results)?;
                }
            }
        }

        Ok(())
    }

    // ========== 仓库管理（保留原有逻辑）==========

    /// 列出仓库
    pub fn list_repos(&self, store: &SkillStore) -> Vec<SkillRepo> {
        store.repos.clone()
    }

    /// 添加仓库
    pub fn add_repo(&self, store: &mut SkillStore, repo: SkillRepo) -> Result<()> {
        if let Some(pos) = store
            .repos
            .iter()
            .position(|r| r.owner == repo.owner && r.name == repo.name)
        {
            store.repos[pos] = repo;
        } else {
            store.repos.push(repo);
        }

        Ok(())
    }

    /// 删除仓库
    pub fn remove_repo(&self, store: &mut SkillStore, owner: String, name: String) -> Result<()> {
        store
            .repos
            .retain(|r| !(r.owner == owner && r.name == name));

        Ok(())
    }

    // ========== skills.sh 搜索 ==========

    /// 搜索 skills.sh 公共目录
    pub async fn search_skills_sh(
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<SkillsShSearchResult> {
        let client = crate::proxy::http_client::get();

        let url = url::Url::parse_with_params(
            "https://skills.sh/api/search",
            &[
                ("q", query),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        )?;

        let resp = client
            .get(url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?
            .json::<SkillsShApiResponse>()
            .await?;

        let skills = resp
            .skills
            .into_iter()
            .filter_map(|s| {
                let parts: Vec<&str> = s.source.splitn(2, '/').collect();
                if parts.len() != 2 {
                    return None;
                }
                let (owner, repo) = (parts[0].to_string(), parts[1].to_string());
                // 过滤非 GitHub 来源（如 "skills.volces.com"、"mcp-hub.momenta.works"）
                if owner.contains('.') || repo.contains('.') {
                    return None;
                }
                Some(SkillsShDiscoverableSkill {
                    key: s.id,
                    name: s.name,
                    directory: s.skill_id.clone(),
                    repo_owner: owner.clone(),
                    repo_name: repo.clone(),
                    repo_branch: "main".to_string(),
                    installs: s.installs,
                    readme_url: Some(format!("https://github.com/{}/{}", owner, repo)),
                })
            })
            .collect();

        Ok(SkillsShSearchResult {
            skills,
            total_count: resp.count,
            query: resp.query,
        })
    }
}

// ========== 迁移支持 ==========

/// 从 lock 文件信息构建 skill 的 ID、仓库字段和 readme URL
///
/// 返回 (id, repo_owner, repo_name, repo_branch, readme_url)
pub fn migrate_skills_to_ssot(db: &Arc<Database>) -> Result<usize> {
    db.library_snapshot()?;
    // Old enablement snapshots cannot establish ownership. Import is explicit.
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    #[serial_test::serial]
    fn old_enabled_flags_are_not_reported_as_applied_skills() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Arc::new(crate::Database::memory().unwrap());
        let row = InstalledSkill {
            id: "local:fixture".into(),
            name: "Fixture".into(),
            description: None,
            directory: "fixture".into(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps {
                claude: true,
                ..Default::default()
            },
            installed_at: 0,
            content_hash: None,
            updated_at: 0,
        };
        db.save_skill(&row).unwrap();
        let listed = SkillService::get_all_installed(&db).unwrap();
        assert!(!listed[0].apps.claude);
        assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn a_modified_definition_is_not_projected_as_an_applied_native_skill() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = tempdir().unwrap();
        write_skill(source.path(), "Fixture");
        let db = Arc::new(crate::Database::memory().unwrap());
        let row = InstalledSkill {
            id: "local:fixture".into(),
            name: "Fixture".into(),
            description: None,
            directory: "fixture".into(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: Default::default(),
            installed_at: 0,
            content_hash: None,
            updated_at: 0,
        };
        let mut row =
            crate::services::library::skill_resource::save_draft(&db, row, source.path(), None)
                .unwrap();
        let op = crate::services::library::skill_resource::preview(&db, &row.id, &["codex".into()])
            .unwrap();
        crate::services::library::apply_operation(&db, &op.id).unwrap();
        assert!(SkillService::get_all_installed(&db).unwrap()[0].apps.codex);
        let before = db.library_snapshot().unwrap();
        row.content_hash = Some("0".repeat(64));
        db.save_skill(&row).unwrap();
        assert!(!SkillService::get_all_installed(&db).unwrap()[0].apps.codex);
        assert_eq!(db.library_snapshot().unwrap(), before);
    }

    #[test]
    fn immutable_content_identity_includes_hidden_files() {
        let dir = tempdir().unwrap();
        write_skill(dir.path(), "Fixture");
        fs::write(dir.path().join(".configuration"), "first").unwrap();
        let before = SkillService::compute_dir_hash(dir.path()).unwrap();
        fs::write(dir.path().join(".configuration"), "changed").unwrap();
        assert_ne!(SkillService::compute_dir_hash(dir.path()).unwrap(), before);
    }

    #[test]
    #[cfg(unix)]
    fn immutable_content_identity_includes_script_execution_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        write_skill(dir.path(), "Fixture");
        let script = dir.path().join("run.sh");
        fs::write(&script, "echo example").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
        let before = SkillService::compute_dir_hash(dir.path()).unwrap();
        fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
        assert_ne!(before, SkillService::compute_dir_hash(dir.path()).unwrap());
    }

    #[test]
    fn a_source_name_fallback_requires_one_candidate() {
        let dir = tempdir().unwrap();
        write_skill(dir.path(), "Root");
        write_skill(&dir.path().join("a/fixture"), "A");
        write_skill(&dir.path().join("b/fixture"), "B");
        assert!(SkillService::resolve_skill_source_dir(dir.path(), "fixture").is_none());
        assert_eq!(
            SkillService::resolve_skill_source_dir(dir.path(), "a/fixture").unwrap(),
            dir.path().join("a/fixture")
        );
    }

    #[test]
    #[cfg(unix)]
    #[serial_test::serial]
    fn literal_backslash_names_are_not_rewritten_into_directories() {
        let _sandbox = crate::test_sandbox::sandbox();
        let dir = tempdir().unwrap();
        write_skill(dir.path(), "Fixture");
        fs::create_dir(dir.path().join("a")).unwrap();
        fs::write(dir.path().join("a\\b"), "user content").unwrap();
        assert!(crate::services::library::skill_artifact::publish(dir.path()).is_err());
        assert!(!crate::services::library::skill_artifact::root().exists());
    }

    #[test]
    #[serial_test::serial]
    fn a_legacy_sync_cannot_replace_an_unowned_skill_directory() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = SkillService::get_ssot_dir().unwrap().join("fixture");
        write_skill(&source, "Library");
        let target = SkillService::get_app_skills_dir(&AppType::Claude)
            .unwrap()
            .join("fixture");
        write_skill(&target, "User");
        fs::write(target.join("personal"), "keep").unwrap();
        assert!(SkillService::sync_to_app_dir("fixture", &AppType::Claude).is_err());
        assert_eq!(fs::read_to_string(target.join("personal")).unwrap(), "keep");
    }

    fn skill_zip(path: &Path, extra: Option<(&str, &str)>) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("fixture/SKILL.md", options).unwrap();
        zip.write_all(b"---\nname: fixture\ndescription: Test\n---\nnew draft")
            .unwrap();
        if let Some((name, body)) = extra {
            zip.start_file(name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn native_import_only_creates_a_draft_and_preserves_the_source() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = SkillService::get_app_skills_dir(&AppType::Claude)
            .unwrap()
            .join("fixture");
        write_skill(&source, "User");
        let before = fs::read(source.join("SKILL.md")).unwrap();
        let db = Arc::new(Database::memory().unwrap());
        let rows = SkillService::import_from_apps(
            &db,
            vec![ImportSkillSelection {
                directory: "fixture".into(),
                apps: SkillApps::only(&AppType::Claude),
            }],
        )
        .unwrap();
        assert!(rows[0].apps.is_empty());
        assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), before);
        assert!(db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
        assert!(!SkillService::get_ssot_dir()
            .unwrap()
            .join("fixture")
            .exists());
    }

    #[test]
    #[serial_test::serial]
    fn zip_import_does_not_rewrite_a_source_already_visible_to_an_agent() {
        let _sandbox = crate::test_sandbox::sandbox();
        let source = SkillService::get_ssot_dir().unwrap().join("fixture");
        write_skill(&source, "Old user content");
        let before = fs::read(source.join("SKILL.md")).unwrap();
        let db = Arc::new(Database::memory().unwrap());
        let tmp = tempdir().unwrap();
        let zip = tmp.path().join("skills.zip");
        skill_zip(&zip, None);
        let result = SkillService::install_from_zip(&db, &zip, &AppType::Claude);
        assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), before);
        assert!(result.unwrap()[0].apps.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn an_unsafe_archive_is_rejected_before_any_draft_is_saved() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Arc::new(Database::memory().unwrap());
        let tmp = tempdir().unwrap();
        let zip = tmp.path().join("skills.zip");
        skill_zip(&zip, Some(("../outside", "bad")));
        assert!(SkillService::install_from_zip(&db, &zip, &AppType::Claude).is_err());
        assert!(db.get_all_installed_skills().unwrap().is_empty());
        assert!(!SkillService::get_ssot_dir()
            .unwrap()
            .join("fixture")
            .exists());
    }

    /// The agents' skills live inside the directory they were told to use.
    mod app_skills_dir {
        use super::*;
        use crate::test_sandbox::sandbox;
        use serial_test::serial;

        #[test]
        #[serial]
        fn the_defaults_are_below_the_home() {
            let sandbox = sandbox();

            assert_eq!(
                SkillService::get_app_skills_dir(&AppType::Claude).unwrap(),
                sandbox.home().join(".claude").join("skills")
            );
            assert_eq!(
                SkillService::get_app_skills_dir(&AppType::Codex).unwrap(),
                sandbox.home().join(".codex").join("skills")
            );
        }

        #[test]
        #[serial]
        fn claude_config_dir_and_codex_home_move_the_skills() {
            let sandbox = sandbox();
            let claude = sandbox.home().join("data").join("claude-state");
            let codex = sandbox.home().join("data").join("codex-state");
            std::env::set_var("CLAUDE_CONFIG_DIR", &claude);
            std::env::set_var("CODEX_HOME", &codex);

            assert_eq!(
                SkillService::get_app_skills_dir(&AppType::Claude).unwrap(),
                claude.join("skills")
            );
            assert_eq!(
                SkillService::get_app_skills_dir(&AppType::Codex).unwrap(),
                codex.join("skills")
            );
        }

        #[test]
        #[serial]
        fn a_settings_override_beats_the_environment_variable() {
            let sandbox = sandbox();
            let typed = sandbox.home().join("typed-codex");
            let mut settings = crate::settings::get_settings();
            settings.codex_config_dir = Some(typed.to_string_lossy().into_owned());
            crate::settings::update_settings(settings).expect("save the override");
            std::env::set_var(
                "CODEX_HOME",
                sandbox.home().join("data").join("codex-state"),
            );

            assert_eq!(
                SkillService::get_app_skills_dir(&AppType::Codex).unwrap(),
                typed.join("skills")
            );
        }
    }

    fn write_skill(dir: &Path, name: &str) {
        fs::create_dir_all(dir).expect("create skill dir");
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Test skill\n---\n"),
        )
        .expect("write SKILL.md");
    }

    #[test]
    fn resolve_skill_source_dir_returns_repo_root_for_root_level_skill() {
        let temp = tempdir().expect("tempdir");
        write_skill(temp.path(), "Root Skill");

        let resolved = SkillService::resolve_skill_source_dir(temp.path(), "last30days-skill-cn")
            .expect("root-level skill should resolve to the extracted repo root");

        assert_eq!(resolved, temp.path());
    }

    #[test]
    fn resolve_skill_source_dir_returns_direct_nested_directory_when_present() {
        let temp = tempdir().expect("tempdir");
        let nested = temp.path().join("skills").join("nested-skill");
        write_skill(&nested, "Nested Skill");

        let resolved = SkillService::resolve_skill_source_dir(temp.path(), "skills/nested-skill")
            .expect("nested skill should resolve from its relative source path");

        assert_eq!(resolved, nested);
    }

    #[test]
    fn resolve_skill_source_dir_falls_back_to_matching_install_name() {
        let temp = tempdir().expect("tempdir");
        let nested = temp.path().join("skills").join("nested-skill");
        write_skill(&nested, "Nested Skill");

        let resolved = SkillService::resolve_skill_source_dir(temp.path(), "nested-skill")
            .expect("install name should fall back to the matching discovered skill directory");

        assert_eq!(resolved, nested);
    }
}
