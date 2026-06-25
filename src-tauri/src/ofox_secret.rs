//! OFox OAuth token **和** 工具级 API key 的系统钥匙串持久化。
//!
//! 为什么单独抽一个模块：access_token / refresh_token 是用户级别的真实凭据
//! （前者直接绑用户身份调 `/openapi/*` 与 LLM gateway，后者能换出长期凭据），
//! 跟 cc-switch 的 client_id（公开标识）不是一回事，必须落到 OS 凭据库而不是
//! 明文 json。后来又加了"每工具一把 ofox API key"——同样要藏在 OS 凭据库里。
//! 本模块包一层 trait 让 `OfoxAuthManager` 与 `ofox_api_keys` 通过依赖注入解耦：
//! 生产用 [`KeyringStore`]，单测用 [`InMemoryStore`]，CI 不依赖真实 keychain。
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

use crate::app_config::AppType;

/// OAuth token 用的 service。与 bundle identifier `ai.ofox.desktop` 对齐，
/// `security dump-keychain` 时能直接对应到 app 身份。改名时间点是 ofox-desktop
/// 第一次正式发布前——dev 期重命名，没有"老用户兼容"的负担。
const SERVICE_OAUTH: &str = "ai.ofox.desktop.oauth";

/// 工具级 API key 用的 service——跟 OAuth 物理隔离，便于 Keychain Access.app
/// 里目视区分，也方便"撤销所有工具 key"这种批量动作只动这一组。
const SERVICE_APIKEY: &str = "ai.ofox.desktop.apikey";

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
    ApiKey { tool: AppType },
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

impl SecretStore for KeyringStore {
    fn load(&self, slot: Slot) -> Result<Option<String>, String> {
        let entry = Self::entry(slot)?;
        match entry.get_password() {
            // `Ok("")` 与 `NoEntry` 都按"未设置"对待——前者通常是别处误写空值，
            // 我们按缺失重走登录流程比假装"有 token"更安全。
            Ok(v) if v.is_empty() => Ok(None),
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
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

// ─── 测试实现：内存字典 ───────────────────────────────────────────────────

/// 单测用：进程内 HashMap 替代 OS 凭据库。线程安全，因为 `OfoxAuthManager`
/// 把 store 包在 `Arc<dyn SecretStore>` 里跨 await 共享。
#[derive(Debug, Default)]
pub struct InMemoryStore {
    // 用 Mutex<Vec<(Slot, String)>> 比 HashMap 简单——total 槽位 < 10，Vec 顺序
    // 扫线性开销可忽略，省一个 Eq+Hash bound 的样板。
    inner: Mutex<Vec<(Slot, String)>>,
}

impl InMemoryStore {
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
mod tests {
    use super::*;

    #[test]
    fn in_memory_round_trips_both_slots() {
        let store = InMemoryStore::new();
        assert_eq!(store.load(Slot::AccessToken).unwrap(), None);
        assert_eq!(store.load(Slot::RefreshToken).unwrap(), None);

        store.save(Slot::AccessToken, "at-1").unwrap();
        store.save(Slot::RefreshToken, "rt-1").unwrap();

        assert_eq!(store.load(Slot::AccessToken).unwrap().as_deref(), Some("at-1"));
        assert_eq!(store.load(Slot::RefreshToken).unwrap().as_deref(), Some("rt-1"));
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
        assert_eq!(store.load(Slot::RefreshToken).unwrap().as_deref(), Some("rt"));
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
        assert_eq!(store.load(Slot::AccessToken).unwrap().as_deref(), Some("at"));
    }

    #[test]
    fn save_overwrites_existing_value() {
        let store = InMemoryStore::new();
        store.save(Slot::AccessToken, "old").unwrap();
        store.save(Slot::AccessToken, "new").unwrap();
        assert_eq!(store.load(Slot::AccessToken).unwrap().as_deref(), Some("new"));
    }

    // ─── ApiKey 变体：新增覆盖 ─────────────────────────────────────────

    #[test]
    fn api_key_slots_are_per_tool_isolated() {
        // 每工具一把 key 的核心约束——给 Claude 写的 key 不能被 Codex 读到，
        // 反之亦然。如果这条挂了，bind Codex 时会拿到 Claude 的 key，整个产品
        // 假设就破了。
        let store = InMemoryStore::new();
        store
            .save(Slot::ApiKey { tool: AppType::Claude }, "sk-claude")
            .unwrap();
        store
            .save(Slot::ApiKey { tool: AppType::Codex }, "sk-codex")
            .unwrap();

        assert_eq!(
            store
                .load(Slot::ApiKey { tool: AppType::Claude })
                .unwrap()
                .as_deref(),
            Some("sk-claude")
        );
        assert_eq!(
            store
                .load(Slot::ApiKey { tool: AppType::Codex })
                .unwrap()
                .as_deref(),
            Some("sk-codex")
        );
        // 没存过的 tool 应当读出 None
        assert_eq!(
            store.load(Slot::ApiKey { tool: AppType::Gemini }).unwrap(),
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
            .save(Slot::ApiKey { tool: AppType::Claude }, "tool-key")
            .unwrap();

        assert_eq!(
            store.load(Slot::AccessToken).unwrap().as_deref(),
            Some("oauth-at")
        );
        assert_eq!(
            store
                .load(Slot::ApiKey { tool: AppType::Claude })
                .unwrap()
                .as_deref(),
            Some("tool-key")
        );

        // 清 Claude 的 API key 不应影响 OAuth token
        store
            .clear(Slot::ApiKey { tool: AppType::Claude })
            .unwrap();
        assert_eq!(
            store.load(Slot::AccessToken).unwrap().as_deref(),
            Some("oauth-at")
        );
        assert_eq!(
            store.load(Slot::ApiKey { tool: AppType::Claude }).unwrap(),
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
            Slot::ApiKey { tool: AppType::Claude }.service(),
            SERVICE_APIKEY
        );
        assert_eq!(
            Slot::ApiKey { tool: AppType::Hermes }.service(),
            SERVICE_APIKEY
        );
    }

    #[test]
    fn slot_account_uses_tool_slug_for_api_key() {
        // account 字段必须跟 AppType slug 一致——`security find-generic-password`
        // 排障时这是用户能看到的字段。
        for tool in AppType::all() {
            let slot = Slot::ApiKey { tool };
            assert_eq!(slot.account(), tool.as_str());
        }
    }
}
