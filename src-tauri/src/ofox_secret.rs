//! OFox OAuth token **和** 工具级 API key 的系统钥匙串持久化。
//!
//! 为什么单独抽一个模块：access_token / refresh_token 是用户级别的真实凭据
//! （前者直接绑用户身份调 `/openapi/*` 与 LLM gateway，后者能换出长期凭据），
//! 跟 cc-switch 的 client_id（公开标识）不是一回事，必须落到 OS 凭据库而不是
//! 明文 json。后来又加了"每工具一把 ofox API key"——同样要藏在 OS 凭据库里。
//! 本模块包一层 trait 让 `OfoxAuthManager` 与 `ofox_api_keys` 通过依赖注入解耦：
//! release 用 [`KeyringStore`]，debug 用 [`FileStore`]（0600 明文文件，避免开发期
//! 被钥匙串授权框反复打断），单测用 [`InMemoryStore`]。生产路径统一从
//! [`default_store`] 取 store，CI 不依赖真实 keychain。
//!
//! ### 两组数据，两个 service
//!
//! Keychain 用 `(service, account)` tuple 索引条目。同一个 service 下 account
//! 必须唯一，而 cc-switch 现在持两类完全不同语义的密钥：
//!
//!   - **OAuth token**（`access_token` / `refresh_token`）—— cc-switch 自己用于
//!     调 ofox `/openapi/*` 与刷 token，从不外泄给第三方工具
//!   - **API key**（每个工具一把）—— 用户主动让 cc-switch 写到工具配置文件里，
//!     由工具拿去调 ofox LLM gateway
//!
//! 拆成两个 service（`*.oauth` 与 `*.apikey`）有几点好处：
//!   1. 用 Keychain Access.app 时一眼能看出"哪个是登录态、哪个是工具配置"
//!   2. 用户在 ofox 后台批量 revoke 工具 key 时，cc-switch 本地清理只动 apikey
//!      service，OAuth 登录态不受影响
//!   3. account 命名空间互不污染——`access_token` / `claude` 这两个 account 哪怕
//!      同属一个 service 也不会撞，但语义上还是分开放更稳
//!
//! ### 跨平台后端
//!
//! `keyring` v4 默认 feature `v1` 已自动拉入三平台后端：
//!   - macOS  → Apple Security Framework（login.keychain-db）
//!   - Windows → Win32 Credential Manager（wincred）
//!   - Linux  → DBus Secret Service（GNOME Keyring / KWallet）
//!
//! 三者对 `(service, account)` tuple 都是 UPSERT 语义（macOS 走
//! `SecItemAdd` + `SecItemUpdate` 回退），不会因为反复 `set_password` 产生
//! 多条 ACL。所以本模块只暴露最小三件套 save / load / clear。
//!
//! ### `save("")` 是 footgun
//!
//! 写空串会让条目仍存在但 value 为空，`get_password` 后续返回 `Ok("")`——这
//! 跟"从未存过"语义混淆。所以 [`KeyringStore::save`] 拒绝空值；想清理时必须
//! 调 [`SecretStore::clear`]。

use std::sync::Mutex;

use keyring::Entry;

use crate::app_config::BindableTool;

/// OAuth token 用的 service。与 bundle identifier `ai.ofox.desktop` 对齐，
/// `security dump-keychain` 时能直接对应到 app 身份。改名时间点是 ofox-desktop
/// 第一次正式发布前——dev 期重命名，没有"老用户兼容"的负担。
///
/// **debug 构建带 `.dev` 后缀**：`pnpm tauri dev` 出来的包是 ad-hoc 签名，
/// 跟正式版（Developer ID 签名）签名身份不同。keychain 条目走传统 ACL（绑
/// 具体签名身份），两个身份访问**同一**条目时，系统会反复弹"请输入密码"
/// 授权框。给 dev 包用独立 service 名，让它和正式版各写各的条目、互不打架，
/// 开发期来回切就不再被授权框打断。release 构建保持原名，正式用户零影响。
const SERVICE_OAUTH: &str = if cfg!(debug_assertions) {
    "ai.ofox.desktop.dev.oauth"
} else {
    "ai.ofox.desktop.oauth"
};

/// 工具级 API key 用的 service——跟 OAuth 物理隔离，便于 Keychain Access.app
/// 里目视区分，也方便"撤销所有工具 key"这种批量动作只动这一组。
/// debug 构建带 `.dev` 后缀，原因同 [`SERVICE_OAUTH`]。
const SERVICE_APIKEY: &str = if cfg!(debug_assertions) {
    "ai.ofox.desktop.dev.apikey"
} else {
    "ai.ofox.desktop.apikey"
};

/// 钥匙串里我们存的几个槽位。account 字段直接就是变体语义对应的字符串，
/// 跟 OAuth 协议字段名 / cc-switch 工具 slug 一致——日后排障
/// `security dump-keychain` 能一眼看明白。
///
/// `Copy`：所有 payload 都是 `Copy`（`AppType` 加了 `Copy` derive），让调用方
/// 可以无脑按值传递、不用考虑借用语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    AccessToken,
    RefreshToken,
    /// 工具级 ofox LLM API key——每工具一把。account 用 `AppType::as_str()`
    /// 返回的 slug（`claude` / `codex` / `gemini` / `opencode` / `openclaw` /
    /// `hermes`）。
    ApiKey {
        tool: BindableTool,
    },
}

impl Slot {
    /// 该 slot 写入哪个 service。
    fn service(self) -> &'static str {
        match self {
            Slot::AccessToken | Slot::RefreshToken => SERVICE_OAUTH,
            Slot::ApiKey { .. } => SERVICE_APIKEY,
        }
    }

    /// 该 slot 在 keychain 里的 account 字段。
    ///
    /// 返回 `&'static str`：OAuth 两根本来就是字面量；`AppType::as_str` 也
    /// 返回 `&'static str`，所以整个签名能保持零分配。
    fn account(self) -> &'static str {
        match self {
            Slot::AccessToken => "access_token",
            Slot::RefreshToken => "refresh_token",
            Slot::ApiKey { tool } => tool.as_str(),
        }
    }
}

/// OAuth token / API key 持久化抽象。
///
/// 所有方法返回 `Result<_, String>`——把后端千姿百态的 error 类型脱敏成单一
/// 字符串，调用方不需要 match 跨平台细节。区分"没条目"和"读失败"靠 `load`
/// 的 `Option`：`Ok(None)` 表示 keychain 里没存或已被清掉，`Err` 才是真异常。
pub trait SecretStore: Send + Sync {
    fn load(&self, slot: Slot) -> Result<Option<String>, String>;
    /// 写入。`value` **不可为空串**——空值约定调 [`SecretStore::clear`]。
    fn save(&self, slot: Slot, value: &str) -> Result<(), String>;
    /// 删除。不存在不报错（吞 `NoEntry`）。
    fn clear(&self, slot: Slot) -> Result<(), String>;
}

// ─── 生产实现：系统钥匙串 ─────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl KeyringStore {
    pub const fn new() -> Self {
        Self
    }

    fn entry(slot: Slot) -> Result<Entry, String> {
        let service = slot.service();
        let account = slot.account();
        Entry::new(service, account).map_err(|e| {
            format!("keychain entry init failed (service={service}, account={account}): {e}")
        })
    }
}

/// 钥匙串读取被用户拒绝时错误信息的开头（见 [`is_keychain_denied`]）。
pub const KEYCHAIN_DENIED: &str = "KEYCHAIN_DENIED";

/// 读取失败是不是因为用户在系统弹窗里点了「拒绝」。
///
/// App 换了签名（例如 1.3.2 → 1.3.4 改用公司证书）后，macOS 会按条询问能否读取
/// 旧条目；拒绝时界面不该显示成「未登录」却不说原因。
pub fn is_keychain_denied(error: &str) -> bool {
    error.starts_with(KEYCHAIN_DENIED)
}

/// macOS 把「拒绝」报成 errSecUserCanceled（-128）或 errSecAuthFailed（-25293），
/// keyring 包成 `PlatformFailure`；按错误码认，不看会随系统语言变化的文字。
fn denied_by_user(error: &keyring::Error) -> bool {
    let keyring::Error::PlatformFailure(inner) = error else {
        return false;
    };
    let debug = format!("{inner:?}");
    debug
        .split("code: ")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| c != '-' && !c.is_ascii_digit()).next())
        .and_then(|code| code.parse::<i32>().ok())
        .is_some_and(|code| code == -128 || code == -25293)
}

impl SecretStore for KeyringStore {
    fn load(&self, slot: Slot) -> Result<Option<String>, String> {
        let entry = Self::entry(slot)?;
        match entry.get_password() {
            // `Ok("")` 与 `NoEntry` 都按"未设置"对待——前者通常是别处误写空值，
            // 我们按缺失重走登录流程比假装"有 token"更安全。
            Ok(v) if v.is_empty() => Ok(None),
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) if denied_by_user(&e) => Err(format!(
                "{KEYCHAIN_DENIED}: keychain access denied ({})",
                slot.account()
            )),
            Err(e) => Err(format!("keychain read failed ({}): {e}", slot.account())),
        }
    }

    fn save(&self, slot: Slot, value: &str) -> Result<(), String> {
        if value.is_empty() {
            // 防呆：让调用方显式选择 clear，避免"以为删了实际只是写了空串"
            return Err(format!(
                "refuse to save empty value to keychain ({}); call clear() instead",
                slot.account()
            ));
        }
        let entry = Self::entry(slot)?;
        entry
            .set_password(value)
            .map_err(|e| format!("keychain write failed ({}): {e}", slot.account()))
    }

    fn clear(&self, slot: Slot) -> Result<(), String> {
        let entry = Self::entry(slot)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("keychain delete failed ({}): {e}", slot.account())),
        }
    }
}

// ─── 开发实现：明文文件 ───────────────────────────────────────────────────

/// **仅 debug 构建**：把 secret 明文存到 app 配置目录下一个 0600 的 JSON 文件。
///
/// 为什么需要它：macOS 钥匙串的 ACL 绑定代码签名，而 debug 二进制每次
/// `cargo build` 后 ad-hoc 签名都会变，于是每个条目都要重新弹窗授权 —— oauth 2 条
/// （access/refresh）加每个工具一条 apikey，最多 8 次。开发期反复输登录密码没有
/// 任何收益，还会诱导开发者点"拒绝"从而掩盖真实问题。
///
/// **不要在 release 用**：值是明文。安全边界靠三条：
///   1. 只在 `cfg!(debug_assertions)` 下被 [`default_store`] 选中
///   2. 文件权限 0600（仅所有者可读写）
///   3. 文件名带 `dev-` 前缀，跟正式数据区分明显；正式版走钥匙串，根本不读它
pub struct FileStore {
    path: std::path::PathBuf,
}

impl FileStore {
    /// 生产（dev）路径：`<app_config_dir>/dev-secrets.json`。
    pub fn new() -> Self {
        Self {
            path: crate::config::get_app_config_dir().join("dev-secrets.json"),
        }
    }

    /// 注入路径，测试专用（`#[cfg(test)]` 让它不编进生产二进制）。
    #[cfg(test)]
    pub fn with_path(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    /// 测试专用：用来断言文件权限。
    #[cfg(all(test, unix))]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// key 用 `<service>/<account>`，跟钥匙串的寻址方式保持一致 —— 这样同一个
    /// Slot 在两种 store 里的身份是对应的，排查问题时好对照。
    fn key(slot: Slot) -> String {
        format!("{}/{}", slot.service(), slot.account())
    }

    /// 读全量。读不到或解析不了都按"空"处理。
    ///
    /// 故意不返回 Err：这条路在应用启动时就会走，一个坏文件不该让应用起不来。
    /// 后续任何 save 都会整体覆盖它，所以坏文件能自愈。
    fn read_all(&self) -> std::collections::BTreeMap<String, String> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn write_all(&self, map: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
        use std::io::Write;

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("dev secret dir failed: {e}"))?;
        }

        let json = serde_json::to_string_pretty(map)
            .map_err(|e| format!("dev secret encode failed: {e}"))?;

        // 创建时就带 0600，不给"默认权限落盘 → 再 chmod"之间的窗口期。
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }

        let mut f = opts
            .open(&self.path)
            .map_err(|e| format!("dev secret open failed: {e}"))?;
        f.write_all(json.as_bytes())
            .map_err(|e| format!("dev secret write failed: {e}"))?;

        // 文件可能是早先用默认权限建的 —— OpenOptions 的 mode 只作用于新建，
        // 所以这里再显式收一次权限。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }
}

impl Default for FileStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for FileStore {
    fn load(&self, slot: Slot) -> Result<Option<String>, String> {
        // 空串按"未设置"处理，与 KeyringStore 对齐
        Ok(self
            .read_all()
            .get(&Self::key(slot))
            .filter(|v| !v.is_empty())
            .cloned())
    }

    fn save(&self, slot: Slot, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Err(format!(
                "refuse to save empty value to dev secret file ({}); call clear() instead",
                slot.account()
            ));
        }
        let mut map = self.read_all();
        map.insert(Self::key(slot), value.to_string());
        self.write_all(&map)
    }

    fn clear(&self, slot: Slot) -> Result<(), String> {
        let mut map = self.read_all();
        if map.remove(&Self::key(slot)).is_none() {
            // 本来就没有，不写盘也不报错
            return Ok(());
        }
        self.write_all(&map)
    }
}

/// 生产路径统一从这里拿 store：release 用钥匙串，debug 用明文文件。
///
/// debug 下想验证真实钥匙串行为时，设 `OFOX_DEV_USE_KEYCHAIN=1` 切回去。
pub fn default_store() -> std::sync::Arc<dyn SecretStore> {
    #[cfg(debug_assertions)]
    {
        if std::env::var_os("OFOX_DEV_USE_KEYCHAIN").is_none() {
            return std::sync::Arc::new(FileStore::new());
        }
    }
    std::sync::Arc::new(KeyringStore::new())
}

// ─── 测试实现：内存字典 ───────────────────────────────────────────────────

/// 单测用：进程内 HashMap 替代 OS 凭据库。线程安全，因为 `OfoxAuthManager`
/// 把 store 包在 `Arc<dyn SecretStore>` 里跨 await 共享。
#[derive(Debug, Default)]
#[allow(dead_code)] // Production builds do not construct the test secret store.
pub struct InMemoryStore {
    // 用 Mutex<Vec<(Slot, String)>> 比 HashMap 简单——total 槽位 < 10，Vec 顺序
    // 扫线性开销可忽略，省一个 Eq+Hash bound 的样板。
    inner: Mutex<Vec<(Slot, String)>>,
}

impl InMemoryStore {
    #[allow(dead_code)] // Used by unit tests compiled with the test harness.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for InMemoryStore {
    fn load(&self, slot: Slot) -> Result<Option<String>, String> {
        let guard = self.inner.lock().map_err(|e| format!("poisoned: {e}"))?;
        Ok(guard
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, v)| v.clone()))
    }

    fn save(&self, slot: Slot, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Err("InMemoryStore::save refuses empty value".to_string());
        }
        let mut guard = self.inner.lock().map_err(|e| format!("poisoned: {e}"))?;
        if let Some(entry) = guard.iter_mut().find(|(s, _)| *s == slot) {
            entry.1 = value.to_string();
        } else {
            guard.push((slot, value.to_string()));
        }
        Ok(())
    }

    fn clear(&self, slot: Slot) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|e| format!("poisoned: {e}"))?;
        guard.retain(|(s, _)| *s != slot);
        Ok(())
    }
}

#[cfg(test)]
mod file_store_tests {
    use super::*;
    use crate::app_config::AppType;
    use std::io::Write;

    /// 包一层，免得 `Slot::ApiKey { tool: ... }` 这个 struct literal 在断言里
    /// 被 rustfmt 展开成四行，读起来全是噪音。
    fn api_key(tool: AppType) -> Slot {
        Slot::ApiKey { tool: tool.into() }
    }

    fn store() -> (FileStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore::with_path(dir.path().join("dev-secrets.json"));
        (store, dir)
    }

    #[test]
    fn round_trips_a_slot() {
        let (store, _dir) = store();
        assert_eq!(store.load(Slot::AccessToken).unwrap(), None);

        store.save(Slot::AccessToken, "at-1").unwrap();

        assert_eq!(
            store.load(Slot::AccessToken).unwrap().as_deref(),
            Some("at-1")
        );
    }

    #[test]
    fn slots_are_isolated() {
        let (store, _dir) = store();
        store.save(Slot::AccessToken, "at").unwrap();
        store.save(api_key(AppType::Codex), "codex-key").unwrap();

        assert_eq!(store.load(Slot::RefreshToken).unwrap(), None);
        assert_eq!(
            store.load(api_key(AppType::Claude)).unwrap(),
            None,
            "不同工具的 api key 不能互相串"
        );
        assert_eq!(
            store.load(api_key(AppType::Codex)).unwrap().as_deref(),
            Some("codex-key")
        );
    }

    #[test]
    fn save_then_clear_then_load_returns_none() {
        let (store, _dir) = store();
        store.save(Slot::AccessToken, "at").unwrap();
        store.clear(Slot::AccessToken).unwrap();

        assert_eq!(store.load(Slot::AccessToken).unwrap(), None);
    }

    #[test]
    fn clearing_absent_slot_is_ok() {
        let (store, _dir) = store();

        assert!(store.clear(Slot::RefreshToken).is_ok());
    }

    #[test]
    fn overwrites_existing_value() {
        let (store, _dir) = store();
        store.save(Slot::AccessToken, "old").unwrap();
        store.save(Slot::AccessToken, "new").unwrap();

        assert_eq!(
            store.load(Slot::AccessToken).unwrap().as_deref(),
            Some("new")
        );
    }

    #[test]
    fn refuses_empty_value() {
        // 与 KeyringStore / InMemoryStore 行为一致：空值必须显式走 clear，
        // 否则"以为删了、实际写了空串"会让上层误判成"有 token"。
        let (store, _dir) = store();

        assert!(store.save(Slot::AccessToken, "").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn file_is_only_readable_by_owner() {
        // 安全关键：dev 的 token 是明文落盘的，权限必须是 0600。
        use std::os::unix::fs::PermissionsExt;

        let (store, _dir) = store();
        store.save(Slot::AccessToken, "at").unwrap();

        let mode = std::fs::metadata(store.path())
            .expect("file exists")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(
            mode, 0o600,
            "实际权限 {mode:o}，token 明文文件不能让其他用户读到"
        );
    }

    #[test]
    fn missing_file_reads_as_empty_not_error() {
        let (store, _dir) = store();

        // 没写过任何东西，文件还不存在——这是正常的首次启动状态，不该报错
        assert_eq!(store.load(Slot::AccessToken).unwrap(), None);
    }

    #[test]
    fn corrupted_file_does_not_panic() {
        let (store, dir) = store();
        let mut f = std::fs::File::create(dir.path().join("dev-secrets.json")).unwrap();
        f.write_all(b"{ this is not json").unwrap();
        drop(f);

        // 坏文件可以报错，但绝不能 panic——启动路径上 panic 会让应用起不来
        let _ = store.load(Slot::AccessToken);
    }

    #[test]
    fn survives_a_corrupted_file_by_overwriting() {
        let (store, dir) = store();
        std::fs::write(dir.path().join("dev-secrets.json"), b"not json").unwrap();

        store.save(Slot::AccessToken, "at").unwrap();

        assert_eq!(
            store.load(Slot::AccessToken).unwrap().as_deref(),
            Some("at"),
            "坏文件不应把 store 永久卡死，写入要能自愈"
        );
    }
}

#[cfg(test)]
mod tests {
    /// Mimics `security_framework::base::Error`'s Debug output.
    struct OsStatus(i32);
    impl std::fmt::Debug for OsStatus {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "Error {{ code: {}, message: \"用户已取消操作。\" }}",
                self.0
            )
        }
    }
    impl std::fmt::Display for OsStatus {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "OSStatus {}", self.0)
        }
    }
    impl std::error::Error for OsStatus {}

    #[test]
    fn a_denied_keychain_prompt_is_told_apart_from_other_failures() {
        let failure = |code| keyring::Error::PlatformFailure(Box::new(OsStatus(code)));
        assert!(super::denied_by_user(&failure(-128)));
        assert!(super::denied_by_user(&failure(-25293)));
        assert!(!super::denied_by_user(&failure(-25300)));
        assert!(!super::denied_by_user(&failure(-1280)));
        assert!(!super::denied_by_user(&keyring::Error::NoEntry));
        assert!(super::is_keychain_denied(&format!(
            "{}: keychain access denied (access_token)",
            super::KEYCHAIN_DENIED
        )));
        assert!(!super::is_keychain_denied(
            "keychain read failed (access_token): x"
        ));
    }

    use super::*;
    use crate::app_config::AppType;

    #[test]
    fn in_memory_round_trips_both_slots() {
        let store = InMemoryStore::new();
        assert_eq!(store.load(Slot::AccessToken).unwrap(), None);
        assert_eq!(store.load(Slot::RefreshToken).unwrap(), None);

        store.save(Slot::AccessToken, "at-1").unwrap();
        store.save(Slot::RefreshToken, "rt-1").unwrap();

        assert_eq!(
            store.load(Slot::AccessToken).unwrap().as_deref(),
            Some("at-1")
        );
        assert_eq!(
            store.load(Slot::RefreshToken).unwrap().as_deref(),
            Some("rt-1")
        );
    }

    #[test]
    fn slots_are_isolated() {
        // 防回归：之前在 PoC 阶段写过把两个 slot 共用同一 key 的 bug。
        let store = InMemoryStore::new();
        store.save(Slot::AccessToken, "at").unwrap();
        assert_eq!(store.load(Slot::RefreshToken).unwrap(), None);
        store.clear(Slot::AccessToken).unwrap();
        store.save(Slot::RefreshToken, "rt").unwrap();
        assert_eq!(store.load(Slot::AccessToken).unwrap(), None);
        assert_eq!(
            store.load(Slot::RefreshToken).unwrap().as_deref(),
            Some("rt")
        );
    }

    #[test]
    fn save_then_clear_then_load_returns_none() {
        let store = InMemoryStore::new();
        store.save(Slot::AccessToken, "at").unwrap();
        store.clear(Slot::AccessToken).unwrap();
        assert_eq!(store.load(Slot::AccessToken).unwrap(), None);
    }

    #[test]
    fn clear_missing_slot_is_ok() {
        // KeyringStore 的 clear 也吞 NoEntry——内存版要保证同样的幂等语义，
        // 否则单测会跟生产行为脱节。
        let store = InMemoryStore::new();
        store.clear(Slot::AccessToken).unwrap();
        store.clear(Slot::AccessToken).unwrap();
    }

    #[test]
    fn save_rejects_empty_value() {
        // footgun 防御：写空串必须立刻报错，调用方应当用 clear。
        let store = InMemoryStore::new();
        assert!(store.save(Slot::AccessToken, "").is_err());
        // 也确认即便 save 失败，原有值不被破坏
        store.save(Slot::AccessToken, "at").unwrap();
        assert!(store.save(Slot::AccessToken, "").is_err());
        assert_eq!(
            store.load(Slot::AccessToken).unwrap().as_deref(),
            Some("at")
        );
    }

    #[test]
    fn save_overwrites_existing_value() {
        let store = InMemoryStore::new();
        store.save(Slot::AccessToken, "old").unwrap();
        store.save(Slot::AccessToken, "new").unwrap();
        assert_eq!(
            store.load(Slot::AccessToken).unwrap().as_deref(),
            Some("new")
        );
    }

    // ─── ApiKey 变体：新增覆盖 ─────────────────────────────────────────

    #[test]
    fn api_key_slots_are_per_tool_isolated() {
        // 每工具一把 key 的核心约束——给 Claude 写的 key 不能被 Codex 读到，
        // 反之亦然。如果这条挂了，bind Codex 时会拿到 Claude 的 key，整个产品
        // 假设就破了。
        let store = InMemoryStore::new();
        store
            .save(
                Slot::ApiKey {
                    tool: AppType::Claude.into(),
                },
                "sk-claude",
            )
            .unwrap();
        store
            .save(
                Slot::ApiKey {
                    tool: AppType::Codex.into(),
                },
                "sk-codex",
            )
            .unwrap();

        assert_eq!(
            store
                .load(Slot::ApiKey {
                    tool: AppType::Claude.into()
                })
                .unwrap()
                .as_deref(),
            Some("sk-claude")
        );
        assert_eq!(
            store
                .load(Slot::ApiKey {
                    tool: AppType::Codex.into()
                })
                .unwrap()
                .as_deref(),
            Some("sk-codex")
        );
        // 没存过的 tool 应当读出 None
        assert_eq!(
            store
                .load(Slot::ApiKey {
                    tool: AppType::Gemini.into()
                })
                .unwrap(),
            None
        );
    }

    #[test]
    fn api_key_does_not_collide_with_oauth_tokens() {
        // OAuth 与 ApiKey 走两个 service，且 InMemoryStore 用 Slot 整体作为
        // key——所以即便有人手贱把 SERVICE 拼错让它们撞同一个 service，本测试
        // 仍然能因 Slot 变体不等而通过；这是双保险。
        let store = InMemoryStore::new();
        store.save(Slot::AccessToken, "oauth-at").unwrap();
        store
            .save(
                Slot::ApiKey {
                    tool: AppType::Claude.into(),
                },
                "tool-key",
            )
            .unwrap();

        assert_eq!(
            store.load(Slot::AccessToken).unwrap().as_deref(),
            Some("oauth-at")
        );
        assert_eq!(
            store
                .load(Slot::ApiKey {
                    tool: AppType::Claude.into()
                })
                .unwrap()
                .as_deref(),
            Some("tool-key")
        );

        // 清 Claude 的 API key 不应影响 OAuth token
        store
            .clear(Slot::ApiKey {
                tool: AppType::Claude.into(),
            })
            .unwrap();
        assert_eq!(
            store.load(Slot::AccessToken).unwrap().as_deref(),
            Some("oauth-at")
        );
        assert_eq!(
            store
                .load(Slot::ApiKey {
                    tool: AppType::Claude.into()
                })
                .unwrap(),
            None
        );
    }

    #[test]
    fn slot_service_routes_to_distinct_services() {
        // 防回归：拆 SERVICE_OAUTH / SERVICE_APIKEY 的核心目的就是物理隔离，
        // 这里直接验路由表，不依赖 InMemoryStore 实现。
        assert_eq!(Slot::AccessToken.service(), SERVICE_OAUTH);
        assert_eq!(Slot::RefreshToken.service(), SERVICE_OAUTH);
        assert_eq!(
            Slot::ApiKey {
                tool: AppType::Claude.into()
            }
            .service(),
            SERVICE_APIKEY
        );
        assert_eq!(
            Slot::ApiKey {
                tool: AppType::Hermes.into()
            }
            .service(),
            SERVICE_APIKEY
        );
    }

    #[test]
    fn slot_account_uses_tool_slug_for_api_key() {
        // account 字段必须跟 AppType slug 一致——`security find-generic-password`
        // 排障时这是用户能看到的字段。
        for tool in AppType::all() {
            let slot = Slot::ApiKey { tool: tool.into() };
            assert_eq!(slot.account(), tool.as_str());
        }
    }
}
