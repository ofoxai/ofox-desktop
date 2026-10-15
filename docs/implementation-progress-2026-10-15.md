# 工具安装、资源库与用量分析实施记录

执行基线：`24a87849`。按[工具与资源库方案](superpowers/plans/2026-10-15-tool-catalog-and-library-design.md)和[用量方案](superpowers/plans/2026-10-15-usage-analytics-design.md)实施。

- I1 可选目录与本机入口：代码及阶段门禁完成；增加 9 个独立候选，默认折叠，本机模式不触发账户绑定。原生界面/Windows 留到最终验收。
- R0 资源所有权、持久状态与安全写入：代码、复审及阶段门禁完成，随后直接进入 R1。
- R1 资源库三页与原有全局适配：进行中。三页、草稿编辑／导入、完整目标预览／应用／恢复已接线；专用资源包已接线，MCP 复制与显式接管、Skills 固定版本更新及连接检查流程已完成代码、评审与阶段门禁；原生界面验收仍未完成。
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
- 资源专用 ZIP 包已接入全库导出、恢复预览、跳过／另存与逐项结果。包中无原生路径、分配、Applied 或执行记录；MCP 仅保留完整环境变量引用和根传输类型，省略值在导入端也强制校验。共享指令及技能原文件仍需分享前人工检查，此包不等于整库私有备份。恢复核对完整文件指纹、定义 revision 与 epoch，只新增草稿；提升 epoch，旧操作不得续写。
- 资源包独立复审关闭了同名资源往返不一致、UTF-8 长名称／目录复制后缀超限两项问题，均先复现失败；7 项后端专测、26 项 WebDAV 兼容测试和 19 项界面／IPC 专测通过。前端全量 106 文件／719 项通过（随后新增的异步预览与 IPC 回归在上述 19 项中通过）；typecheck、format、Rust fmt 和 all-targets Clippy 通过。没有将这些阶段结果宣称为全部规划最终门禁。
- MCP 已支持后端引用复制为新 ID／草稿，源 revision、epoch 与 spec hash 均冻结；私有配置不经脱敏 DTO 重建。普通同名目标仍冲突；显式接管只认与定义一致的原生字段，重新预览后另行确认，保留未知字段和注释。`adoptedServer` 与新建父表分别记录；无用户扩展时可撤回空项，有扩展则保留待处理。更新删除最后嵌套字段导致空祖先阻塞撤回的问题先 RED 复现，已修复并经五适配循环复验。
- MCP 两路独立复审无剩余 P1/P2；5 项新后端回归、资源库 111 项通过；前端 typecheck、format 与全量 106 文件／723 项通过，Rust fmt、all-targets Clippy（-D warnings）和全量测试通过（2208 library 单测、95 integration，9 项既有 ignored）。五适配测试均在沙箱，仅证明文件格式／所有权流程；这轮仍未以真实客户端版本验证 MCP 启动或授权。
- Skills 更新由用户明确检查触发，按完整来源/ref 解析并固定 GitHub 提交，先比较候选与当前不可变内容，再更新源草稿或另存；目标副本的修改阻止原位更新。检查、比较、确认均冻结 revision/epoch，候选有时限、数量和下载时长上限，可显式丢弃，不执行技能脚本。发布前核对同一内存树的 hash，失败不应用到原生工具。
- 比较失败保留同一候选及文件列表以重试，比较成功前禁止确认；改名冲突等可恢复错误保留候选，来源失效或状态变更则废弃。旧批量入口共用来源解析，仍按仓库合并下载且不占用候选；错误不再显示“已最新”。损坏旧对象、同 hash 已被修改对象的限制及旧更新命令按调用时 ref 解析的兼容语义已记入 known-limitations，不冒充已修复。
- Skills 更新先取得后端及界面 RED，16 项专测通过；前端 typecheck、format 和全量 106 文件／727 项通过，Rust fmt、all-targets Clippy（-D warnings）和全量测试通过（2224 library 单测、95 integration，9 项既有 ignored）。后端及界面独立复审关闭 P1/P2。所有新下载测试使用注入的沙箱 ZIP，未运行真实 MCP／技能或改变原生配置；原生客户端版本及 Windows 仍未验证。
- 指令总览按工具展示原生位置、自有／受管行数、期望和实际可读集合、最后受管 revision、共享读取者；同一物理组只读取一次。不可读、手改和外来 marker 不伪装成空；旧位置 receipt 单列待核对，不宣称已撤回。总览复用完整目标预览，浏览不写入配置。Skills 详情显示严格校验的来源、跟踪 ref、记录的固定提交、内容 hash 和毫秒时间；正文读取冻结 revision/epoch/hash，旧后端返回不同 hash 也拒绝混用。
- 总览／来源先取得后端及界面 RED，独立评审发现的“未知可读状态误报未分配”另以 2 项失败复现后修复、复审关闭。前端 typecheck、format、全量 106 文件／731 项通过；Rust fmt、all-targets Clippy（-D warnings）、全量测试通过（2227 library 单测、95 integration，9 项既有 ignored）。只使用沙箱和 renderer 单测，没有修改原生工具文件；原生 UI 与 Windows 待验。
- MCP 显式检查只在用户点击开始后准备冻结的定义并连接；支持 stdio、Streamable HTTP JSON/SSE 及显式旧 SSE。先订阅再运行，按 operationId 区分进度；取消与进程启动使用同一 gate，迟到的准备／监听不会执行。配置不写入原生工具，结果只在内存；不调用服务端工具、不代登录或安装包。
- 检查限制消息、总接收量、分页、工具数、字段展开和脱敏预算；保留半条迟到握手、支持服务端 ping，HTTP 不跟随重定向或跨域 SSE endpoint。已知凭据先统一控制字符处理，再合并全部重叠匹配，错误不含 stderr／响应正文。Unix 专用进程组及 Windows Job 请求回收进程树；实际外部程序不是 OS 沙箱，详见 [连接检查说明](mcp-connection-checks.md)。
- MCP 回归先取得策略、协议、界面、取消竞态和复审问题的 RED；24 项后端专测通过。前端 typecheck、format、全量 106 文件／735 项通过；Rust fmt、all-targets Clippy（-D warnings）、全量测试通过（2251 library 单测、95 integration，9 项既有 ignored）。后端／协议及界面独立复审无剩余 P1/P2。
- macOS 使用 Node 22.12.0 沙箱进程及本地 HTTP fixtures，覆盖迟到半条握手、ping、stderr 排空、取消后子进程不再写入；没有执行真实 MCP 服务、授权或原生工具配置。Windows Job／执行策略仅通过隔离的交叉类型检查，原生 Job 行为仍待 Windows 验证，不能将其写为平台验收通过。
- 下一步继续：R1 沙箱原生界面验收；之后按 I2、U-A 至 U-D、最终本机及 Windows 验证包推进。
