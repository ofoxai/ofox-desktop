# 工具安装、资源库与用量分析实施记录

执行基线：`24a87849`。按[工具与资源库方案](superpowers/plans/2026-10-15-tool-catalog-and-library-design.md)和[用量方案](superpowers/plans/2026-10-15-usage-analytics-design.md)实施。

- I1 可选目录与本机入口：代码及阶段门禁完成；增加 9 个独立候选，默认折叠，本机模式不触发账户绑定。原生界面/Windows 留到最终验收。
- R0 资源所有权、持久状态与安全写入：代码、复审及阶段门禁完成，随后直接进入 R1。
- R1 资源库三页与原有全局适配：待实施。
- I2 逐工具安装及资源适配：待实施，按平台能力开放。
- U-A 统计口径与覆盖：待实施。
- U-B 工作台、请求恢复及刷新：待实施。
- U-C 会话与项目分析：待实施。
- U-D 新增统计来源：待实施。
- 核心最终门禁与原生 UI 验收：待执行。
- Windows：待原生验收，不能用 macOS 类型检查替代。

规则：一条实现线，独立评审；Rust 构建串行，低于 20 GiB 先清已完成缓存；用户文件测试只写沙箱；只 commit，不 push、不改版本、不打标签、不真卸载本机工具。

项目作用域、Hooks、RTK、跨设备合并及 Context 不属于首轮核心；高级图表按方案另行迭代。

## I1 阶段记录

- RED：本机路由、离开登录后的迟到请求、旧账户扫描、无账户用量刷新、目录探测与插值、刷新等待、portal 暂停；Codex desktop fallback 专项 Rust 测试复现误识别。评审补充的登录 busy 重置回归单独复现失败。
- GREEN：前端 typecheck、format:check、全量单测（103 文件、700 项）通过；Rust fmt、all-targets clippy（-D warnings）、all-targets 测试通过（2033 library 单测及 97 integration 测试，9 项既有 ignored）。
- 独立评审：后端与界面两路复审均无剩余 P1/P2。修复了退出登录状态、延迟 listener 清理、Codex/ChatGPT 产品身份、登录 busy 生命周期、三语插值、引导布局、目录刷新等待/重复探测、失败状态与隐藏页面 portal。
- renderer 浏览器预览：本机入口、默认折叠、搜索和旧后端只读降级已检查；这不代替真实 Tauri 探测或最终 900px/三语/原生窗口验收。
- 所有新增安装项目前只提供官方手动安装途径；自动安装/升级/卸载按 I2 实际平台验收开放。未安装或卸载任何本机工具。

## R0 审计结论

必须收口旧命令、deeplink、provider 后置同步、SQL/备份/WebDAV 恢复。Skills 的旧 SSOT 被工具 symlink 直接读取，恢复和更新须写不可变版本后显式应用；不能仅取消末尾 sync。共享锁须覆盖读改写，provider 完整配置写入也须保留资源子树。Applied/journal 保留为本机数据，恢复提升 epoch；损坏/未知版本拒绝写入。

## R0 当前验证记录

- 复审修复覆盖目录越界、完整旧定义 CAS、更新前旧 artifact 核对、链接失败转复制时的日志一致性、portable 执行位、私有存储与原生读取范围冲突、损坏 receipt 拒绝、来源歧义和环境快照等待。
- OpenCode 全局晚层文件和 inline 配置纳入预览、应用和恢复的依赖校验；动态路由键及无法解析的 Skills 路径引用明确阻塞。Hermes 补齐路径 trim 和裸 `~`。
- WebDAV 新格式使用独立 `skill-artifacts.zip` 与 encoding；旧平铺包只能恢复成草稿，禁止替换旧 SSOT 或原生目录。同步定义来自同一 DB 快照，Applied 和日志不出机。
- 阶段门禁：前端 typecheck、format:check、104 文件/701 项单测通过；Rust 全量测试通过（2184 library 单测及 95 integration，9 项既有 ignored），fmt 和 all-targets Clippy（-D warnings）通过。
- 指令/MCP/Skills 与恢复两路独立复审收尾无新增 P1/P2；旧 provider 合并深链接的原始 TOML 错误仍按 known-limitations 第 19 项保留，不能将新 parse/native 错误入口的修复说成所有旧错误均已脱敏。
- Rust 默认并发测试复现了三个旧 API-key 测试未使用共享串行锁造成的污染，已统一默认 serial 和沙箱；旧模型切换自动同步 MCP 的集成测试已改为验证原生 MCP 保留、资源须显式应用。
- 资源库页面、连接检查、专用可分享资源包及后续用量阶段尚未完成；Windows 原生文件/链接语义仍待验。本阶段只验证全局规则和沙箱写入，没有操作真实工具配置或卸载本机工具。
