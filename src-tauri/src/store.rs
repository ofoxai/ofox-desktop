use crate::database::Database;
use crate::services::tool_health::ToolHealthCache;
use crate::services::{ProxyService, UsageCache};
use std::sync::Arc;

/// 全局应用状态
pub struct AppState {
    pub db: Arc<Database>,
    pub proxy_service: ProxyService,
    pub usage_cache: Arc<UsageCache>,
    /// 工具健康检查结果缓存（后端循环写入，前端通过事件订阅）
    pub tool_health_cache: ToolHealthCache,
}

impl AppState {
    /// 创建新的应用状态
    pub fn new(db: Arc<Database>) -> Self {
        let proxy_service = ProxyService::new(db.clone());

        Self {
            db,
            proxy_service,
            usage_cache: Arc::new(UsageCache::new()),
            tool_health_cache: ToolHealthCache::new(),
        }
    }
}
