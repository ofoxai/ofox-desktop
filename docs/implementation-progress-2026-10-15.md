# 工具安装、资源库与用量分析实施记录

执行基线：`24a87849`。按[工具与资源库方案](superpowers/plans/2026-10-15-tool-catalog-and-library-design.md)和[用量方案](superpowers/plans/2026-10-15-usage-analytics-design.md)实施。

- I1 可选目录与本机入口：代码及阶段门禁完成；增加 9 个独立候选，默认折叠，本机模式不触发账户绑定。原生界面/Windows 留到最终验收。
- R0 资源所有权、持久状态与安全写入：代码、复审及阶段门禁完成，随后直接进入 R1。
- R1 资源库三页与原有全局适配：进行中。三页、草稿编辑／导入、完整目标预览／应用／恢复已接线；专用资源包、MCP 连接检查／接管、更新流程及原生界面验收仍未完成。
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


## R1 当前实施记录（尚未完成）

- 四导航及指令／MCP／Skills 子页首访加载；资源状态独立于登录和模型绑定。指令可新建、编辑、复制、Markdown／旧提示词草稿导入；MCP 按字段引用保持／设置／清除；Skills 接入目录、ZIP、GitHub/ref、已有技能及备份草稿导入和经核验 artifact 的只读详情。
- 预览发送完整期望目标集合，按物理组展示受管内容、字段名、分发方式和共享读取范围；保存、预览、删除冻结 revision 与恢复 epoch。后台刷新与隐藏页面不丢草稿／选择／预览，不允许重复提交。
- 独立评审已推动修复 pending DTO 不能应用、空目标撤回、旧删除弹窗、共享子树判定、portal 状态丢失、文件倒序覆盖、坏文件后选择器丢失及无 epoch 后端仍可写等问题。旧失败日志可显式结束，已结束日志不得再应用或复活；Skills 仅清理已核验的新暂存副本，旧备份及修改材料保留待核对。
- 已完成 RED/GREEN 的阶段回归；当前代码前端 typecheck、format 与全量单测通过（106 文件／717 项）；Rust fmt、all-targets Clippy（-D warnings）和全量测试通过（2196 library 单测、95 integration，9 项既有 ignored）。这是 R1 当前基础代码的阶段结果，不能替代全部规划完成后的最终门禁。
- 浏览器仅核对未登录入口、四导航和缺 Tauri 后端的降级状态；不代表真实原生资源分发、900px／深浅色／Windows 已验收。没有改动真实工具配置或卸载工具；这轮尚未逐一验证已安装原生客户端版本，Windows 待原生验证。
- 下一步继续：资源专用备份／恢复与冲突预览、MCP 显式连接检查及匹配项接管、技能更新草稿流程，再完成 R1 评审和阶段门禁；之后按 I2、U-A 至 U-D、最终本机及 Windows 验证包推进。
