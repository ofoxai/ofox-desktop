use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use crate::app_config::AppType;
use crate::error::AppError;
use crate::services::skill::{SkillStorageLocation, SyncMethod};

/// 自定义端点配置（历史兼容，实际存储在 provider.meta.custom_endpoints）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomEndpoint {
    pub url: String,
    pub added_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
}

fn default_true() -> bool {
    true
}

/// 主页面显示的应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleApps {
    #[serde(default = "default_true")]
    pub claude: bool,
    #[serde(default = "default_true")]
    pub codex: bool,
    #[serde(default = "default_true")]
    pub gemini: bool,
    #[serde(default = "default_true")]
    pub opencode: bool,
    #[serde(default = "default_true")]
    pub openclaw: bool,
    #[serde(default)]
    pub hermes: bool,
}

impl Default for VisibleApps {
    fn default() -> Self {
        Self {
            claude: true,
            codex: true,
            gemini: true,
            opencode: true,
            openclaw: true,
            hermes: false, // 默认不显示，需用户手动启用
        }
    }
}

impl VisibleApps {
    /// Check if the specified app is visible
    pub fn is_visible(&self, app: &AppType) -> bool {
        match app {
            AppType::Claude => self.claude,
            AppType::Codex => self.codex,
            AppType::Gemini => self.gemini,
            AppType::OpenCode => self.opencode,
            AppType::OpenClaw => self.openclaw,
            AppType::Hermes => self.hermes,
        }
    }
}

/// WebDAV 同步状态（持久化同步进度信息）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WebDavSyncStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_remote_etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_local_manifest_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_remote_manifest_hash: Option<String>,
}

fn default_remote_root() -> String {
    "ofox-switch-sync".to_string()
}
fn default_profile() -> String {
    "default".to_string()
}

/// WebDAV 同步设置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebDavSyncSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub auto_sync: bool,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_remote_root")]
    pub remote_root: String,
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(default)]
    pub status: WebDavSyncStatus,
}

impl Default for WebDavSyncSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_sync: false,
            base_url: String::new(),
            username: String::new(),
            password: String::new(),
            remote_root: default_remote_root(),
            profile: default_profile(),
            status: WebDavSyncStatus::default(),
        }
    }
}

impl WebDavSyncSettings {
    pub fn validate(&self) -> Result<(), crate::error::AppError> {
        if self.base_url.trim().is_empty() {
            return Err(crate::error::AppError::localized(
                "webdav.base_url.required",
                "WebDAV 地址不能为空",
                "WebDAV URL is required.",
            ));
        }
        if self.username.trim().is_empty() {
            return Err(crate::error::AppError::localized(
                "webdav.username.required",
                "WebDAV 用户名不能为空",
                "WebDAV username is required.",
            ));
        }
        Ok(())
    }

    pub fn normalize(&mut self) {
        self.base_url = self.base_url.trim().to_string();
        self.username = self.username.trim().to_string();
        self.remote_root = self.remote_root.trim().to_string();
        self.profile = self.profile.trim().to_string();
        if self.remote_root.is_empty() {
            self.remote_root = default_remote_root();
        }
        if self.profile.is_empty() {
            self.profile = default_profile();
        }
    }

    /// Returns true if all credential fields are blank (no config to persist).
    fn is_empty(&self) -> bool {
        self.base_url.is_empty() && self.username.is_empty() && self.password.is_empty()
    }
}

/// 单个工具对应的 ofox LLM API key 元数据。
///
/// key 本体（`sk-...`）**不**在这里——它在 [`crate::ofox_secret`] 的 keychain
/// 槽位 `Slot::ApiKey { tool }` 里。本结构体存可见的"账面信息"：
///
///   - `key_id`：ofox-core 返回的服务端 id，撤销 / 查询用量等服务端操作的索引
///   - `alias`：用户起的别名（可选；默认 `None`），用于 UI 标识"这把是给 Codex 用的"
///   - `created_at` / `last_used_at`：人读的时间戳，便于排障
///
/// 字段保持紧凑——这份元数据要随 `settings.json` 落盘，里面任何敏感信息都跟
/// "整把 key 进 keychain"的安全决策相违。所以**严禁**在这里加 key 的 secret /
/// hash / 任何能反推 key 的字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyMeta {
    /// 关联的工具。整个 `ofox_api_keys` Vec 里 `tool` 是 unique key。
    pub tool: AppType,
    /// 服务端返回的 key id。空串非法——上游若不返 id 我们就拒绝持久化。
    pub key_id: String,
    /// 用户起的别名。`None` 表示没起。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// 服务端返回的明文 key 前缀（如 `sk-of-AbCdEf`），UI 列表用来辅助识别
    /// "哪一把"。`None` 表示老版本数据 / 端点接入前的 stub 路径未填。**不能**
    /// 用前缀反推 key 本体——key 本体在 keychain。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_start: Option<String>,
    /// key 创建时间，unix epoch 秒。`upsert` 时不动（保留原始创建时间）。
    pub created_at: i64,
    /// 最近一次被注入到工具配置的时间，unix epoch 秒。`None` 表示从未注入过
    /// （刚创建尚未 bind）。每次 bind 成功时由调用方更新。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<i64>,
}

/// 应用设置结构
///
/// 存储设备级别设置，保存在本地 `~/.ofox-switch/settings.json`，不随数据库同步。
/// 这确保了云同步场景下多设备可以独立运作。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    // ===== 设备级 UI 设置 =====
    #[serde(default = "default_show_in_tray")]
    pub show_in_tray: bool,
    #[serde(default = "default_minimize_to_tray_on_close")]
    pub minimize_to_tray_on_close: bool,
    #[serde(default)]
    pub use_app_window_controls: bool,
    /// 是否启用 Claude 插件联动
    #[serde(default)]
    pub enable_claude_plugin_integration: bool,
    /// 是否跳过 Claude Code 初次安装确认
    #[serde(default)]
    pub skip_claude_onboarding: bool,
    /// 是否开机自启
    #[serde(default)]
    pub launch_on_startup: bool,
    /// 静默启动（程序启动时不显示主窗口，仅托盘运行）
    #[serde(default)]
    pub silent_startup: bool,
    /// 是否在主页面启用本地代理功能（默认关闭）
    #[serde(default)]
    pub enable_local_proxy: bool,
    /// User has confirmed the local proxy first-run notice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_confirmed: Option<bool>,
    /// User has confirmed the usage query first-run notice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_confirmed: Option<bool>,
    /// User has confirmed the stream check first-run notice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_check_confirmed: Option<bool>,
    /// Whether to show the failover toggle independently on the main page
    #[serde(default)]
    pub enable_failover_toggle: bool,
    /// User has confirmed the failover toggle first-run notice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failover_confirmed: Option<bool>,
    /// User has confirmed the first-run welcome notice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_run_notice_confirmed: Option<bool>,
    /// User has confirmed the common config first-run notice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub common_config_confirmed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    // ===== 主页面显示的应用 =====
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_apps: Option<VisibleApps>,

    // ===== 设备级目录覆盖 =====
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opencode_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openclaw_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hermes_config_dir: Option<String>,

    // ===== 当前供应商 ID（设备级）=====
    /// 当前 Claude 供应商 ID（本地存储，优先于数据库 is_current）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_claude: Option<String>,
    /// 当前 Codex 供应商 ID（本地存储，优先于数据库 is_current）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_codex: Option<String>,
    /// 当前 Gemini 供应商 ID（本地存储，优先于数据库 is_current）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_gemini: Option<String>,
    /// 当前 OpenCode 供应商 ID（本地存储，对 OpenCode 可能无意义，但保持结构一致）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_opencode: Option<String>,
    /// 当前 OpenClaw 供应商 ID（本地存储，对 OpenClaw 可能无意义，但保持结构一致）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_openclaw: Option<String>,
    /// 当前 Hermes 供应商 ID（本地存储，保持结构一致）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_hermes: Option<String>,

    // ===== Skill 同步设置 =====
    /// Skill 同步方式：auto（默认，优先 symlink）、symlink、copy
    #[serde(default)]
    pub skill_sync_method: SyncMethod,
    /// Skill 存储位置：cc_switch（默认）或 unified（~/.agents/skills/）
    #[serde(default)]
    pub skill_storage_location: SkillStorageLocation,

    // ===== WebDAV 同步设置 =====
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webdav_sync: Option<WebDavSyncSettings>,

    // ===== WebDAV 备份设置（旧版，保留向后兼容）=====
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webdav_backup: Option<serde_json::Value>,

    // ===== 备份策略设置 =====
    /// Auto-backup interval in hours (default 24, 0 = disabled)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_interval_hours: Option<u32>,
    /// Maximum number of backup files to retain (default 10)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_retain_count: Option<u32>,

    // ===== 终端设置 =====
    /// 首选终端应用（可选，默认使用系统默认终端）
    /// - macOS: "terminal" | "iterm2" | "warp" | "alacritty" | "kitty" | "ghostty" | "wezterm" | "kaku"
    /// - Windows: "cmd" | "powershell" | "wt" (Windows Terminal)
    /// - Linux: "gnome-terminal" | "konsole" | "xfce4-terminal" | "alacritty" | "kitty" | "ghostty"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_terminal: Option<String>,

    // ===== OFox 偏好设置 =====
    /// 启用低余额提醒（默认 true）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_balance_enabled: Option<bool>,
    /// 低余额阈值（美元；默认 10.0）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_balance_threshold_usd: Option<f64>,
    /// 工具健康检查间隔："off" | "1h" | "6h" | "24h"（默认 "6h"）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_interval: Option<String>,
    /// 上次告警时使用的阈值（与当前阈值不一致时清除冷却）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_balance_last_alert_threshold: Option<f64>,
    /// 上次告警时间戳（unix-ms）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_balance_last_alert_at: Option<i64>,
    /// 上次成功同步 Ofox 模型 pricing 的时间戳（unix 秒）。
    /// 由 [`crate::services::pricing_sync`] 写入；启动时 / 24h 维护 timer
    /// 通过 `sync_pricing_if_needed` 读取以做 cooldown 门控。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pricing_sync_at: Option<i64>,
    /// 已绑定工具列表（镜像自前端 localStorage `BOUND_TOOLS_STORAGE_KEY`）
    /// 用于后台健康检查循环知道该探测哪些工具
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_tools: Option<Vec<String>>,

    // ===== OFox 区域（apex）=====
    /// OFox 访问域名 apex："ofox.ai"（海外）或 "ofox.io"（国内镜像）。
    /// 影响所有 OAuth、LLM 网关、外链、ofox-* 工具的 base_url。
    /// `None` 表示尚未通过 ip-api 探测过——`ofoxApexResolved == Some(true)` 后该值
    /// 会被钉死，再次启动不会重新探测。Dev 模式下读这个值不会影响实际请求 URL
    /// （所有 URL 都打 localhost），仅 release build 才生效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ofox_apex: Option<String>,
    /// 是否已完成首次区域探测。`Some(true)` 表示 `ofox_apex` 是当前权威值，
    /// 启动钩子不会再次 probe ip-api；用户在 UI 里手动切换也写 `Some(true)`。
    /// 删除 settings.json / 显式置 `None` 会让下一次启动重新探测。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ofox_apex_resolved: Option<bool>,

    // ===== OFox 工具级 API key 元数据 =====
    /// 每个工具一把 ofox API key 的元数据列表（不含 key 本体——本体在
    /// keychain）。条目按 `tool` 唯一，`upsert_api_key_meta` 负责"存在则替换"。
    ///
    /// 为什么不进 SQLite：key 总数封顶就 6（工具数），写入是低频动作（用户
    /// 主动 bind 工具时才动），用扁平 JSON 数组比加一张 DB 表 + 写 migration
    /// 划算多了。如果以后要加用量统计、按时间窗口查询等需求，再考虑迁。
    ///
    /// `#[serde(default)]`：保证升级用户的老 settings.json（没这字段）读起来
    /// 是空 Vec，向后兼容。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ofox_api_keys: Vec<ApiKeyMeta>,
}

fn default_show_in_tray() -> bool {
    true
}

fn default_minimize_to_tray_on_close() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            show_in_tray: true,
            minimize_to_tray_on_close: true,
            use_app_window_controls: false,
            enable_claude_plugin_integration: false,
            skip_claude_onboarding: false,
            launch_on_startup: false,
            silent_startup: false,
            enable_local_proxy: false,
            proxy_confirmed: None,
            usage_confirmed: None,
            stream_check_confirmed: None,
            enable_failover_toggle: false,
            failover_confirmed: None,
            first_run_notice_confirmed: None,
            common_config_confirmed: None,
            language: None,
            visible_apps: None,
            claude_config_dir: None,
            codex_config_dir: None,
            gemini_config_dir: None,
            opencode_config_dir: None,
            openclaw_config_dir: None,
            hermes_config_dir: None,
            current_provider_claude: None,
            current_provider_codex: None,
            current_provider_gemini: None,
            current_provider_opencode: None,
            current_provider_openclaw: None,
            current_provider_hermes: None,
            skill_sync_method: SyncMethod::default(),
            skill_storage_location: SkillStorageLocation::default(),
            webdav_sync: None,
            webdav_backup: None,
            backup_interval_hours: None,
            backup_retain_count: None,
            preferred_terminal: None,
            low_balance_enabled: None,
            low_balance_threshold_usd: None,
            health_check_interval: None,
            low_balance_last_alert_threshold: None,
            low_balance_last_alert_at: None,
            last_pricing_sync_at: None,
            bound_tools: None,
            ofox_apex: None,
            ofox_apex_resolved: None,
            ofox_api_keys: Vec::new(),
        }
    }
}

impl AppSettings {
    fn settings_path() -> Option<PathBuf> {
        // settings.json 保留用于旧版本迁移和无数据库场景
        Some(
            crate::config::get_home_dir()
                .join(".ofox-switch")
                .join("settings.json"),
        )
    }

    fn normalize_paths(&mut self) {
        self.claude_config_dir = self
            .claude_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.codex_config_dir = self
            .codex_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.gemini_config_dir = self
            .gemini_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.opencode_config_dir = self
            .opencode_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.openclaw_config_dir = self
            .openclaw_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.hermes_config_dir = self
            .hermes_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.language = self
            .language
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| matches!(*s, "en" | "zh" | "ja"))
            .map(|s| s.to_string());

        if let Some(sync) = &mut self.webdav_sync {
            sync.normalize();
            if sync.is_empty() {
                self.webdav_sync = None;
            }
        }

        // OFox apex 白名单：只接受 "ofox.ai" / "ofox.io"，其他值（hand-edit、
        // 旧字段、笔误）一律置 None 让启动钩子重新探测。这避免一个被破坏的
        // settings.json 把 URL 拼成 `https://app.ofox.evil.com`。
        self.ofox_apex = self
            .ofox_apex
            .as_ref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| s == "ofox.ai" || s == "ofox.io");
        // 一致性：apex 被清掉，resolved 也跟着清，否则启动会"已 resolved 但
        // apex 是 None"卡住——`current_apex()` 会 fallback `ofox.ai` 但不再触发
        // ip-api 探测。
        if self.ofox_apex.is_none() {
            self.ofox_apex_resolved = None;
        }
    }

    fn load_from_file() -> Self {
        let Some(path) = Self::settings_path() else {
            return Self::default();
        };
        if let Ok(content) = fs::read_to_string(&path) {
            match serde_json::from_str::<AppSettings>(&content) {
                Ok(mut settings) => {
                    settings.normalize_paths();
                    settings
                }
                Err(err) => {
                    log::warn!(
                        "解析设置文件失败，将使用默认设置。路径: {}, 错误: {}",
                        path.display(),
                        err
                    );
                    Self::default()
                }
            }
        } else {
            Self::default()
        }
    }
}

fn save_settings_file(settings: &AppSettings) -> Result<(), AppError> {
    let mut normalized = settings.clone();
    normalized.normalize_paths();
    let Some(path) = AppSettings::settings_path() else {
        return Err(AppError::Config("无法获取用户主目录".to_string()));
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let json = serde_json::to_string_pretty(&normalized)
        .map_err(|e| AppError::JsonSerialize { source: e })?;
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| AppError::io(&path, e))?;
        file.write_all(json.as_bytes())
            .map_err(|e| AppError::io(&path, e))?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&path, json).map_err(|e| AppError::io(&path, e))?;
    }

    Ok(())
}

static SETTINGS_STORE: OnceLock<RwLock<AppSettings>> = OnceLock::new();

fn settings_store() -> &'static RwLock<AppSettings> {
    SETTINGS_STORE.get_or_init(|| RwLock::new(AppSettings::load_from_file()))
}

fn resolve_override_path(raw: &str) -> PathBuf {
    if raw == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(stripped) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if let Some(stripped) = raw.strip_prefix("~\\") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }

    PathBuf::from(raw)
}

pub fn get_settings() -> AppSettings {
    settings_store()
        .read()
        .unwrap_or_else(|e| {
            log::warn!("设置锁已毒化，使用恢复值: {e}");
            e.into_inner()
        })
        .clone()
}

pub fn get_settings_for_frontend() -> AppSettings {
    let mut settings = get_settings();
    if let Some(sync) = &mut settings.webdav_sync {
        sync.password.clear();
    }
    settings.webdav_backup = None;
    settings
}

pub fn update_settings(mut new_settings: AppSettings) -> Result<(), AppError> {
    new_settings.normalize_paths();
    save_settings_file(&new_settings)?;

    let mut guard = settings_store().write().unwrap_or_else(|e| {
        log::warn!("设置锁已毒化，使用恢复值: {e}");
        e.into_inner()
    });
    *guard = new_settings;
    Ok(())
}

/// Read-modify-write helper for in-process callers (ip-api 探测、apex 切换命令、
/// 工具健康检查等）。比 `update_settings` 安全：不会替换无关字段，只把 mutator
/// 的修改持久化。
pub fn mutate_settings<F>(mutator: F) -> Result<(), AppError>
where
    F: FnOnce(&mut AppSettings),
{
    let mut guard = settings_store().write().unwrap_or_else(|e| {
        log::warn!("设置锁已毒化，使用恢复值: {e}");
        e.into_inner()
    });
    let mut next = guard.clone();
    mutator(&mut next);
    next.normalize_paths();
    save_settings_file(&next)?;
    *guard = next;
    Ok(())
}

/// 从文件重新加载设置到内存缓存
/// 用于导入配置等场景，确保内存缓存与文件同步
pub fn reload_settings() -> Result<(), AppError> {
    let fresh_settings = AppSettings::load_from_file();
    let mut guard = settings_store().write().unwrap_or_else(|e| {
        log::warn!("设置锁已毒化，使用恢复值: {e}");
        e.into_inner()
    });
    *guard = fresh_settings;
    Ok(())
}

pub fn get_claude_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .claude_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

pub fn get_codex_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .codex_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

pub fn get_gemini_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .gemini_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

pub fn get_opencode_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .opencode_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

pub fn get_openclaw_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .openclaw_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

pub fn get_hermes_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .hermes_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

// ===== 当前供应商管理函数 =====

/// 获取指定应用类型的当前供应商 ID（从本地 settings 读取）
///
/// 这是设备级别的设置，不随数据库同步。
/// 如果本地没有设置，调用者应该 fallback 到数据库的 `is_current` 字段。
pub fn get_current_provider(app_type: &AppType) -> Option<String> {
    let settings = settings_store().read().ok()?;
    match app_type {
        AppType::Claude => settings.current_provider_claude.clone(),
        AppType::Codex => settings.current_provider_codex.clone(),
        AppType::Gemini => settings.current_provider_gemini.clone(),
        AppType::OpenCode => settings.current_provider_opencode.clone(),
        AppType::OpenClaw => settings.current_provider_openclaw.clone(),
        AppType::Hermes => settings.current_provider_hermes.clone(),
    }
}

/// 设置指定应用类型的当前供应商 ID（保存到本地 settings）
///
/// 这是设备级别的设置，不随数据库同步。
/// 传入 `None` 会清除当前供应商设置。
pub fn set_current_provider(app_type: &AppType, id: Option<&str>) -> Result<(), AppError> {
    let id_owned = id.map(|s| s.to_string());
    mutate_settings(|settings| match app_type {
        AppType::Claude => settings.current_provider_claude = id_owned.clone(),
        AppType::Codex => settings.current_provider_codex = id_owned.clone(),
        AppType::Gemini => settings.current_provider_gemini = id_owned.clone(),
        AppType::OpenCode => settings.current_provider_opencode = id_owned.clone(),
        AppType::OpenClaw => settings.current_provider_openclaw = id_owned.clone(),
        AppType::Hermes => settings.current_provider_hermes = id_owned.clone(),
    })
}

/// 获取有效的当前供应商 ID（验证存在性）
///
/// 逻辑：
/// 1. 从本地 settings 读取当前供应商 ID
/// 2. 验证该 ID 在数据库中存在
/// 3. 如果不存在则清理本地 settings，fallback 到数据库的 is_current
///
/// 这确保了返回的 ID 一定是有效的（在数据库中存在）。
/// 多设备云同步场景下，配置导入后本地 ID 可能失效，此函数会自动修复。
pub fn get_effective_current_provider(
    db: &crate::database::Database,
    app_type: &AppType,
) -> Result<Option<String>, AppError> {
    // 1. 从本地 settings 读取
    if let Some(local_id) = get_current_provider(app_type) {
        // 2. 验证该 ID 在数据库中存在
        let providers = db.get_all_providers(app_type.as_str())?;
        if providers.contains_key(&local_id) {
            // 存在，直接返回
            return Ok(Some(local_id));
        }

        // 3. 不存在，清理本地 settings
        log::warn!(
            "本地 settings 中的供应商 {} ({}) 在数据库中不存在，将清理并 fallback 到数据库",
            local_id,
            app_type.as_str()
        );
        let _ = set_current_provider(app_type, None);
    }

    // Fallback 到数据库的 is_current
    db.get_current_provider(app_type.as_str())
}

// ===== Skill 同步方式管理函数 =====

/// 获取 Skill 同步方式配置
pub fn get_skill_sync_method() -> SyncMethod {
    settings_store()
        .read()
        .unwrap_or_else(|e| {
            log::warn!("设置锁已毒化，使用恢复值: {e}");
            e.into_inner()
        })
        .skill_sync_method
}

// ===== Skill 存储位置管理函数 =====

/// 获取 Skill 存储位置配置
pub fn get_skill_storage_location() -> SkillStorageLocation {
    settings_store()
        .read()
        .unwrap_or_else(|e| {
            log::warn!("设置锁已毒化，使用恢复值: {e}");
            e.into_inner()
        })
        .skill_storage_location
}

/// 设置 Skill 存储位置
pub fn set_skill_storage_location(location: SkillStorageLocation) -> Result<(), AppError> {
    mutate_settings(|s| {
        s.skill_storage_location = location;
    })
}

// ===== 备份策略管理函数 =====

/// Get the effective auto-backup interval in hours (default 24)
pub fn effective_backup_interval_hours() -> u32 {
    settings_store()
        .read()
        .unwrap_or_else(|e| {
            log::warn!("设置锁已毒化，使用恢复值: {e}");
            e.into_inner()
        })
        .backup_interval_hours
        .unwrap_or(24)
}

/// Get the effective backup retain count (default 10, minimum 1)
pub fn effective_backup_retain_count() -> usize {
    settings_store()
        .read()
        .unwrap_or_else(|e| {
            log::warn!("设置锁已毒化，使用恢复值: {e}");
            e.into_inner()
        })
        .backup_retain_count
        .map(|n| (n as usize).max(1))
        .unwrap_or(10)
}

// ===== 终端设置管理函数 =====

/// 获取首选终端应用
pub fn get_preferred_terminal() -> Option<String> {
    settings_store()
        .read()
        .unwrap_or_else(|e| {
            log::warn!("设置锁已毒化，使用恢复值: {e}");
            e.into_inner()
        })
        .preferred_terminal
        .clone()
}

// ===== OFox 偏好（低余额 / 健康检查）管理函数 =====

/// 低余额提醒是否启用（默认 true）
pub fn low_balance_alert_enabled() -> bool {
    settings_store()
        .read()
        .map(|s| s.low_balance_enabled.unwrap_or(true))
        .unwrap_or(true)
}

/// 低余额阈值（美元，默认 10.0）
pub fn low_balance_threshold_usd() -> f64 {
    settings_store()
        .read()
        .map(|s| s.low_balance_threshold_usd.unwrap_or(10.0))
        .unwrap_or(10.0)
}

/// 上次告警的阈值（用于检测阈值变化重置冷却）
pub fn low_balance_last_alert_threshold() -> Option<f64> {
    settings_store()
        .read()
        .ok()
        .and_then(|s| s.low_balance_last_alert_threshold)
}

/// 上次告警时间戳（unix-ms）
pub fn low_balance_last_alert_at() -> Option<i64> {
    settings_store()
        .read()
        .ok()
        .and_then(|s| s.low_balance_last_alert_at)
}

/// 写回低余额闩锁（告警发生后调用）
pub fn set_low_balance_latch(threshold: f64, at_ms: i64) -> Result<(), AppError> {
    mutate_settings(|s| {
        s.low_balance_last_alert_threshold = Some(threshold);
        s.low_balance_last_alert_at = Some(at_ms);
    })
}

/// 工具健康检查间隔（默认 "6h"）
pub fn health_check_interval() -> String {
    settings_store()
        .read()
        .map(|s| {
            s.health_check_interval
                .clone()
                .unwrap_or_else(|| "6h".to_string())
        })
        .unwrap_or_else(|_| "6h".to_string())
}

/// 已绑定工具列表（前端 localStorage 的镜像）
pub fn get_bound_tools() -> Vec<String> {
    settings_store()
        .read()
        .ok()
        .and_then(|s| s.bound_tools.clone())
        .unwrap_or_default()
}

// ===== WebDAV 同步设置管理函数 =====

/// 获取 WebDAV 同步设置
pub fn get_webdav_sync_settings() -> Option<WebDavSyncSettings> {
    settings_store().read().ok()?.webdav_sync.clone()
}

/// 保存 WebDAV 同步设置
pub fn set_webdav_sync_settings(settings: Option<WebDavSyncSettings>) -> Result<(), AppError> {
    mutate_settings(|current| {
        current.webdav_sync = settings;
    })
}

/// 仅更新 WebDAV 同步状态，避免覆写 credentials/root/profile 等字段
pub fn update_webdav_sync_status(status: WebDavSyncStatus) -> Result<(), AppError> {
    mutate_settings(|current| {
        if let Some(sync) = current.webdav_sync.as_mut() {
            sync.status = status;
        }
    })
}

// ─── OFox 工具级 API key 元数据 helpers ───────────────────────────────────
//
// 三件套语义跟 keychain 那一层（`crate::ofox_secret`）解耦：本组只动 `settings.
// json` 上的元数据列表；key 本体的存取调用方各自走 `SecretStore`。两边由
// `ofox_api_keys::fetch_or_create_api_key` 协调一致。

/// 取某工具的 API key 元数据（不读 keychain，所以拿不到 key 本体）。
pub fn get_api_key_meta(tool: AppType) -> Option<ApiKeyMeta> {
    settings_store()
        .read()
        .ok()
        .and_then(|s| s.ofox_api_keys.iter().find(|m| m.tool == tool).cloned())
}

/// 列出所有工具的 API key 元数据，按 `AppType::all()` 顺序未必稳定，但条目本
/// 身按 `tool` 唯一。
pub fn list_api_key_metas() -> Vec<ApiKeyMeta> {
    settings_store()
        .read()
        .map(|s| s.ofox_api_keys.clone())
        .unwrap_or_default()
}

/// 写入或更新一个工具的 API key 元数据。
///
/// 唯一键：`meta.tool`。已存在则**整条替换**——上游约定 `key_id` 变化即"换了
/// 一把 key"，元数据要随新 id 一起更新；不存在则追加。
///
/// 这里**不**自动维护 `created_at`——由调用方决定（首次创建用当前时间；后续
/// 更新通常保留旧值或上游下发的新值）。让 `ApiKeyMeta` 本身就是单一事实来源，
/// 而不是在 helper 里搞隐式合并。
pub fn upsert_api_key_meta(meta: ApiKeyMeta) -> Result<(), AppError> {
    if meta.key_id.trim().is_empty() {
        return Err(AppError::Config(format!(
            "ApiKeyMeta.key_id is empty for tool={:?}; refuse to persist",
            meta.tool
        )));
    }
    mutate_settings(|s| {
        if let Some(slot) = s.ofox_api_keys.iter_mut().find(|m| m.tool == meta.tool) {
            *slot = meta;
        } else {
            s.ofox_api_keys.push(meta);
        }
    })
}

/// 删除某工具的 API key 元数据。不存在不报错（幂等）——跟
/// `SecretStore::clear` 的语义对齐，方便调用方一把"清干净"两边而不用 match
/// 出"是不是真的有"。
pub fn remove_api_key_meta(tool: AppType) -> Result<(), AppError> {
    mutate_settings(|s| {
        s.ofox_api_keys.retain(|m| m.tool != tool);
    })
}

#[cfg(test)]
mod api_key_meta_tests {
    //! 这组测试只覆盖 helpers 对 `AppSettings.ofox_api_keys` 字段的纯函数
    //! 行为——不触磁盘、不触 keychain。涉及 `settings_store` 全局单例的
    //! 集成行为靠 `mutate_settings` 自己的串行测试覆盖。

    use super::*;

    fn sample_meta(tool: AppType, key_id: &str) -> ApiKeyMeta {
        ApiKeyMeta {
            tool,
            key_id: key_id.to_string(),
            alias: None,
            key_start: None,
            created_at: 1_700_000_000,
            last_used_at: None,
        }
    }

    #[test]
    fn upsert_refuses_empty_key_id() {
        let meta = ApiKeyMeta {
            key_id: "   ".into(), // trim 后为空也算空
            ..sample_meta(AppType::Claude, "")
        };
        let err = upsert_api_key_meta(meta).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("key_id"), "expected key_id error, got: {msg}");
    }

    /// 直接对 Vec 操作做断言（不经过 settings 单例，避免污染全局状态）。
    /// upsert / remove 内部的 mutator 逻辑跟下面这个等价闭包一致，所以
    /// 这里覆盖等价语义就够了。
    fn apply_upsert(list: &mut Vec<ApiKeyMeta>, meta: ApiKeyMeta) {
        if let Some(slot) = list.iter_mut().find(|m| m.tool == meta.tool) {
            *slot = meta;
        } else {
            list.push(meta);
        }
    }

    fn apply_remove(list: &mut Vec<ApiKeyMeta>, tool: AppType) {
        list.retain(|m| m.tool != tool);
    }

    #[test]
    fn upsert_appends_then_replaces_in_place() {
        let mut list = vec![];
        apply_upsert(&mut list, sample_meta(AppType::Claude, "k1"));
        apply_upsert(&mut list, sample_meta(AppType::Codex, "k2"));
        assert_eq!(list.len(), 2);

        // 替换 Claude 的 key
        apply_upsert(&mut list, sample_meta(AppType::Claude, "k1-new"));
        assert_eq!(list.len(), 2, "tool 必须唯一");
        let claude = list.iter().find(|m| m.tool == AppType::Claude).unwrap();
        assert_eq!(claude.key_id, "k1-new");
    }

    #[test]
    fn remove_is_idempotent() {
        let mut list = vec![sample_meta(AppType::Claude, "k1")];
        apply_remove(&mut list, AppType::Claude);
        assert!(list.is_empty());
        apply_remove(&mut list, AppType::Claude); // 二次删，不应 panic
        assert!(list.is_empty());
    }

    #[test]
    fn remove_does_not_touch_other_tools() {
        let mut list = vec![
            sample_meta(AppType::Claude, "k1"),
            sample_meta(AppType::Codex, "k2"),
        ];
        apply_remove(&mut list, AppType::Claude);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].tool, AppType::Codex);
    }

    /// `key_start` 是 Option，serde 时 None 不该出现在 JSON 里——避免老
    /// settings.json 反序列化失败 / 文件里出现大量 `"keyStart":null` 噪声。
    #[test]
    fn key_start_serde_skips_when_none() {
        let meta = sample_meta(AppType::Claude, "k1");
        let json = serde_json::to_string(&meta).unwrap();
        assert!(
            !json.contains("keyStart"),
            "None 的 keyStart 不该出现在序列化输出里: {json}"
        );
    }

    /// `key_start` 有值时正确 round-trip——服务端 201 响应里返的就是这个
    /// 字段，UI 列表用前缀辅助辨认。
    #[test]
    fn key_start_round_trips_when_some() {
        let meta = ApiKeyMeta {
            key_start: Some("sk-of-AbCdEf".into()),
            ..sample_meta(AppType::Claude, "k1")
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("\"keyStart\":\"sk-of-AbCdEf\""), "got: {json}");

        let decoded: ApiKeyMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.key_start.as_deref(), Some("sk-of-AbCdEf"));
    }

    /// 老 settings.json 里没有 `keyStart` 字段时反序列化要兜底 None，不能
    /// 把整条 meta 读丢——`#[serde(default)]` 必须就位。
    #[test]
    fn key_start_defaults_when_missing_in_json() {
        let json = r#"{
            "tool": "claude",
            "keyId": "k1",
            "createdAt": 1700000000
        }"#;
        let decoded: ApiKeyMeta = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.key_start, None);
        assert_eq!(decoded.key_id, "k1");
    }
}

