# 工具安装、资源库与用量分析实施记录

执行基线：`24a87849`。按[工具与资源库方案](superpowers/plans/2026-10-15-tool-catalog-and-library-design.md)和[用量方案](superpowers/plans/2026-10-15-usage-analytics-design.md)实施。

- I1 可选目录与本机入口：代码及阶段门禁完成；增加 9 个独立候选，默认折叠，本机模式不触发账户绑定。原生界面/Windows 留到最终验收。
- R0 资源所有权、持久状态与安全写入：代码、复审及阶段门禁完成，随后直接进入 R1。
- R1 资源库三页与原有全局适配：代码、独立评审、阶段门禁及沙箱原生基础交互验收完成。真实第三方客户端执行与 Windows 仍未验收；不代替全项目最终门禁。
- I2 逐工具安装及资源适配：进行中，先验证 Copilot CLI 的独立 npm 管理副本；其他候选和未验平台保持官方手动途径。
- U-A 统计口径与覆盖：四来源事务原子性首切片已完成阶段门禁；缓存证据、累计差分、快照修订、价格覆盖及来源状态正在实施，尚未完成阶段验收。
- U-B 工作台、请求恢复及刷新：前端与查询后端已通过专项验证及独立评审并合入；前端阶段门禁通过，Rust 全量最终门禁仍待执行。
- U-C 会话与项目分析：聚合核心已完成首轮 RED/GREEN；缓存、来源适配和页面尚待接通，不代表会话分析已经可用。
- U-D 新增统计来源：已取得部分安装版和原生合成 fixture 证据，尚未接入新 reader。
- 核心最终门禁与原生 UI 验收：待执行。
- Windows：待原生验收，不能用 macOS 类型检查替代。

规则：用户已授权 subagent 加速，按文件边界在独立工作区并行实现，集成前交叉独立评审；Rust 构建仍串行复用同一 target，低于 20 GiB 先清已完成缓存；用户文件测试只写沙箱；只 commit，不 push、不改版本、不打标签、不真卸载本机工具。

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

## R1 阶段实施记录

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
- macOS 26.5.1（Apple Silicon）原生沙箱完成未登录本机入口、默认折叠、资源三页、指令新建与双目标预览／应用、Skills 本机目录导入／核验正文／共享链接、MCP 协议失败／成功／取消及重启保留资源的操作检查。Tab、空格、标签方向键、Escape 与关闭后焦点恢复通过；900px 浅／深色和长路径已抽查并保存阶段截图。所有手工应用目标及 artifact 均确认位于临时目录，MCP fixture 只收到 discover/list，未调用工具、登录或卸载本机工具。
- 原生验收发现静态“连接未检查”与 shared 伪读取者两项 P2，分别取得前端／Rust RED 后修正；独立复审通过，并用重建的 Tauri 窗口复验。前端 typecheck、format、106 文件／735 项全量单测；Rust fmt、all-targets Clippy（-D warnings）、2251 library 单测及 95 integration 全部再次通过，9 项既有 ignored。屏幕共享标识遮住了交通灯，因此阶段截图不作为交通灯位置的新验收证据；真实第三方 MCP／技能执行与 Windows 仍未验。
- 下一步继续：I2、U-A 至 U-D，最后重跑完整本机门禁、安装和同 SHA Windows 验证包。

## I2 首个切片（代码与 macOS 沙箱基础生命周期已验）

- 首个自动生命周期目标为 Copilot CLI 的 macOS arm64 npm 副本。采用 OFox 专用 prefix，与已安装的外部副本、模型绑定、Key 和用户 npm 配置分开；不修改 shell PATH，界面说明实际管理位置并提供绝对启动命令。已有外部副本和其他平台保留手动途径，不把目录候选视作自动支持。
- 必须使用已有 Node ≥22／npm，冻结并重核运行时、包与 bin 身份。受控 cwd/env 和独立 npmrc 明确固定官方 scoped registry；不补装 Node、不修改用户镜像／npmrc、不继承 npm token。执行先订阅，owner job 持有写锁至进程树回收及重新探测；不复用带解绑／退 Key 的卸载流程。
- 原生来源特性检查：在临时 HOME／prefix／npm cache 中，以 Node 22.12.0、npm 10.9.0 安装官方 `@github/copilot@1.0.98`（禁用安装脚本）并仅运行 `--version`，成功返回 GitHub Copilot CLI 1.0.98。此结果只证明该临时安装及版本读取，不证明 OFox 生命周期接口、资源适配或 Windows 已完成；没有登录、运行 AI 或卸载本机副本。
- 官方约束来源：[Copilot 安装](https://docs.github.com/en/copilot/get-started/cli-quickstart)、[npm registry 选择](https://github.com/npm/npm-registry-fetch/blob/main/lib/index.js)。旧 Terminal 安装器超时后仍可能运行的边界，不能通过新增一个请求期 RAII 锁就宣称消除；新链必须管理自己的进程和写入范围。
- 已接通独立的检查／准备／执行／取消接口和工具目录管理弹窗。冻结官方版本、tarball SHA-512、Node/npm 全树身份及管理目录；持久 intent 先于执行，owner job 不依赖 IPC waiter。Unix 专用进程组回收完成后才释放任务；未知清理或来源变化保留待核对，不自动接管外部副本。
- 回归覆盖 FIFO／超量输入、物理目录替换、manifest 读取竞态、mtime 恢复后的 ctime 变化、receipt 祖先及叶节点替换、执行前目录改向、运行时失效、重复／过期任务、取消与 waiter 丢弃。npm 内置配置含非注释项即拒绝，复审发现的孤立 CR 注释绕过先 RED 后修复。
- 前端在执行前订阅并按 operationId 筛事件；目录层独立持有活动状态，关闭／重挂弹窗不恢复旧可信版本。操作结束刷新失效旧查询，同次结果共用刷新 promise，失败可真正重试。新增边界回归发现并补齐 `common.retry` 三语键；已运行的前端 typecheck、format 与 108 文件／752 项单测通过。
- 原生临时 prefix 核心链已通过：Node 22.12.0／npm 10.9.0 安装 Copilot 1.0.97，升级到 1.0.98，在 execute stage、spawn 前取消，再卸载临时副本。配置／授权／会话 sentinel 保留。该 opt-in 测试直接构造 Plan，不等于 Tauri 界面全流程；真实 npm 运行中取消由本地进程树 fixture 覆盖，未卸载本机工具。
- Copilot 1.0.98 `skill list --json` 从无关的空项目发现临时 HOME 下的 personal-Copilot 及 personal-Agents 技能，不运行技能脚本或 AI。资源预览新增 Copilot 的已知潜在共享读取者，明确不代表启用或实际加载；不增加 Copilot 分配目标，专用指令／MCP 和完整路径／禁用项解析仍未支持。
- 后端及界面独立复审已关闭当前 P1/P2；31 项生命周期及 17 项共享范围专测通过。当前代码 Rust fmt、all-targets Clippy（-D warnings）及全量测试通过（2283 library 单测、95 integration，9 项既有 ignored 加 1 项显式运行过的 opt-in 原生测试）；原生 900px 深浅色界面验收仍在收尾。以上均为 I2 阶段证据，不能代替 U-A 至 U-D 和全项目最终门禁；Windows 仍待原生验证。

## 安装目录界面修正

- 根据目录截图修正末行裁切、命令列错位及过长的管理按钮。真实根因是共享 CopyButton 的绝对定位读屏状态缺少定位父节点，隐藏内容逃出滚动区并撑高文档；在按钮内约束状态文本，不依赖全局禁止滚动。目录增加适量末尾留白，列宽固定并使用短管理标签，完整读屏名称保留。
- 19 个工具 ID 均使用本地产品 Logo；补齐缺失项，区分 Codex／ChatGPT、GitHub Copilot／Microsoft Copilot 和 DevEco Code／CLI。共享 ToolBadge 保留未知 ID 后备显示，深色 OpenCode 保留灰阶，未安装 ChatGPT 不强制白化。素材来源与许可见 [品牌素材](brand-assets.md)。
- 先取得目录短标签与缺失 Logo 的失败回归（11 项失败），以及浏览器文档高度、读屏状态越界的失败证据；修复后 109 文件／773 项前端单测、typecheck、format:check 通过。两路独立审查关闭已知 P2，无剩余确定 P1/P2；这只是此轮前端阶段门禁。
- macOS 原生调试沙箱在 900px、浅／深色检查目录顶部和底部；Goose 末行完整、页头固定，Tab 顺序及管理弹窗 Enter／Escape／关闭后焦点恢复通过。截图留在本机交付目录，未写入真实工具配置、未安装或卸载本机工具。自动生命周期的实际执行和最终全项目门禁仍待后续；Windows 待原生验收。

## I2 原生界面补验

- macOS 原生调试沙箱经管理弹窗准备并执行安装，核验 Node 22.12.0／Copilot 1.0.98；预览更新后取消，迟到计划未执行。随后预览并卸载同一临时受管副本，界面返回“尚无 OFox 管理副本”及成功回执，目录返回候选区。未运行 AI、代登录或改动本机外部副本。
- 临时 HOME 下的配置／授权／会话／共享 Skills 四个 sentinel 在卸载后逐项 hash 不变；npm 包目录已移除。真实版本升级由前述 1.0.97→1.0.98 核心原生测试覆盖，此次界面更新仅验证预览取消，不重复宣称真实升级验收。
- 界面发现上次 cleanup 状态遗留到下次 preview，先以 update／uninstall 两项 RED 复现，再将每次预览明确重置为 verify。取消再次触发状态变化并禁用按钮。独立复审通过；前端 typecheck、format、109 文件／775 项单测通过。
- 管理副本的检查与预览逐次重读 Node／npm／prefix 内容；debug 下耗时明显，快速重开和 StrictMode 还可能重复检查。该成本保留为已知限制，未通过删减写前校验掩盖；release 性能、最终应用安装与 Windows 仍未验收。

## U-A 首切片：同步批次原子性

- Claude／Codex／Gemini／OpenCode 的解析仍在锁外；同一批次的 facts、去重／prune 决策及全部 checkpoint 在一个 SQLite 事务中提交。预期 checkpoint 在写前 CAS，区分缺失与零值，并校验只读迁移依赖；任何事实、价格读取、checkpoint 或 COMMIT 错误使批次回滚，不把失败累计成 imported／skipped，也不推进水位。
- Codex 文件别名的最新 offset 在同一事务中读取；冲突忽略仅限 request_id，其他约束失败会返回错误。OpenCode SQLite 与旧 JSON 的相关水位在全部事实成功后一起发布。一个来源失败不撤回其他来源已提交数据；但推迟 pending maintenance，避免失败旧记录先被 backfill 归档、重试时又加 live。
- 扩展 RED 为 30 项、25 失败／5 通过，单独复现了归档重试 2≠1。补充既有行恢复、正常缺价与 deferred FK COMMIT 失败后，33 项原子性回归全部通过；最终 library 2316 项通过、10 项 ignored，Rust fmt 及 all-targets Clippy（-D warnings）通过。独立复审关闭 Codex fixture 覆盖缺口，无剩余确定 P1/P2。
- 集成最初只有旧 Windows 资源空数组断言失败；更新为实际 Tauri 资源展开验证后，当前 16 个 integration targets 共 94 项通过。该资源测试及其 Clippy 专项也通过。所有设置／环境／会话写入测试使用临时沙箱与默认 serial 锁，没有修改真实工具数据。schema v11 和现有 IPC 保持不变。
- 本切片没有修复 cache-only、同输出修订、累计／单次混合差分、半条 JSONL、大数截断、历史身份或新估价口径；这些仍需独立回归与实现。上述是阶段结果，不能替代全项目最终门禁或 Windows 原生验收。

## 用量阶段并行分工与当前证据

- 用户已授权并行开发。数据校准线负责四源与共享证据；前端线负责工作台、会话分析页面及独立的 WebDAV 并发修复；新来源线负责 WorkBuddy；集成线负责会话索引、查询整合及交叉评审。实现工作区隔离，Rust 构建串行使用同一 target。查询后端完成后，该工作线已转入 WorkBuddy，没有继续占用一个空闲实现位置。
- 四源缓存／证据及规范化已提交为 `cccffd0a`，这是中间保存点，不表示 U-A 完成。600 个未改变的 Claude／Codex 来源不重读正文；累计与单次混合、同秒／同输出修订、大数、UTF-8、合法 EOF／半条记录、模型修订及丢失证据等专项已进入 GREEN。一次 library 检查为 2400 通过／1 失败／10 ignored；唯一失败的 pending backfill 重试随后在 6 项专项中修复，尚未重跑全量，不能将该全量结果改记为通过。
- OpenCode 新增 4 个真实失败回归覆盖低水位数据库替换、旧零值补量、JSON／SQLite 父子关系单独变化；修复后来源语义整组 56 项通过。旧 receipt 升级、缺时间、换时区 prune 边界、来源健康和变更检测仍在实施，不能将已观察到的用量重新归并能力宣称为旧历史已经完全修复。
- 查询后端合入 `38d880ad`／`8eb198aa`／`8ebef5f2`：标准化毫秒半开查询、当前 OFox 基础价格目录、精确 Decimal 估价、价格覆盖与独立已保存金额、查询回执、冻结请求分页／详情／CSV。新分析专项 28 项、旧概览 30 项、旧定价 15 项及请求 4 项通过；Rust fmt 与 lib Clippy 通过，独立复审关闭已知 P1/P2。这些专项不代替最终 all-targets Clippy／全量测试。
- 前端工作台合入 `2e16504f`：共享时间／工具／模型筛选、90 天／自定义自然日、可访问大趋势图、请求分页／冻结详情／CSV、可见窗口刷新和账户区隔离。前端 typecheck、format 及全量 816 项单测通过。900px 浅／深色、图表方向键、请求详情 Escape／焦点恢复已用 renderer 示例数据检查；截图是开发预览，不作为真实本机统计或最终原生验收证据。
- 会话核心 16 项和首批无正文消息事实 6 项通过；已核实父子关系、精确分位数、未知项目与未解析项目、跨午夜及 23／25 小时自然日纳入回归。新增批量响应和消息 metadata 问题先取得 22 通过／5 失败的真实 RED，正在实现；索引、四来源适配及页面仍未接通，不表示分析页已经可用。
- WebDAV 在初次快照后并发提交的本机用量会被覆盖，已用 2 项真实失败回归复现；独立修复线会在最终数据库锁下重新复制本机保留表，并验证失败时保留并发提交。
- WorkBuddy reader 正在独立实现，先取得安装版 schema 证据并建立原生 SID／事件、目录 override、修订／搬移、半条 JSONL、缺毫秒时间及原子提交回归。当前仅有准备好的测试不算接入完成。
- 新来源证据只在只读检查或临时环境取得：WorkBuddy 5.7.7 的字段结构；Hermes 0.21.7+18.g590af66 的原生 SQLite 合成用量；dsh 0.1.2-rc.1 的原生 JSONL／zstd 写入回读；OpenClaw 2026.10.3 的纯规范化与内存会话。Hermes 的主请求／voice／aux 累计重叠、dsh 继承 seed 均有明确差异，不能照搬单一求和公式。OpenClaw 的实际 SQLite 用量写入仍未证明，当前 PATH 没有 Pi，二者不得声称原生验收通过。
- 以上是实施过程证据，最终提交仍须重跑完整前端和 Rust 门禁、原生界面及安装验收，并提供最终 SHA 的 Windows 验证包。Windows 仍待原生验收。
