# 资源库读取与加载状态

资源库使用现有 FoxLoader。路由配置未取得时不读取默认目录；初次配置失败时显示错误，重试先重新读取配置。指令、MCP、技能三种页面共享同样的就绪与可见性门禁。隐藏页面不启动新的目录读取，30 秒内重返复用结果，窗口焦点本身不触发目录重扫。用户刷新和资源操作完成后的更新仍是显式读取。

发现结果依赖扫描票据。`useLibraryInventory` 对新的 kind/epoch/revision/rescan，从首个提交帧即报告 pending；过期但未请求续期的票据不是 loading，可以显式重扫。旧票据、旧路由或旧定义不能为新的操作授权。

这里没有恢复旧的 LoadingState 组件、CLI 版本探测或旧技能目标列表。后台发现仍走当前只读 discovery；读取、模式切换与事务授权继续由 Host 验证。

验证入口：`tests/components/library/LibraryLoading.test.tsx` 覆盖三种页面的就绪、隐藏、错误重试与热缓存；`tests/hooks/useLibraryInventory.test.tsx` 覆盖首帧 pending 和过期重试。现有 MCP/技能组件测试继续覆盖主要操作与焦点行为。这些模拟测试不代表真实配置读取延迟或 Windows GUI 验收。
