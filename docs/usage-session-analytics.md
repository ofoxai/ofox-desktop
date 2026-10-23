# 会话分析缓存与查询

会话分析只读本机缓存；查询筛选和打开详情不重新解析原生正文，也不写入请求账本。前端、缓存查询、共享刷新、公共来源健康和能力开关已经接通并通过专项评审。最终全项目门禁与原生窗口验收以[实施记录](implementation-progress-2026-10-16.md)为准。

## 数据与口径

价格按当前 Ofox 单价优先；型号下架或缺少可用价格时，回退到厂商官方 API 标价，并在回执和界面中保留来源。厂商目录、离线缓存、别名匹配和估算边界见[官方价格回退](usage-official-price-fallback.md)。

- `get_session_usage_analytics({query})` 接收既有 `UsageQueryV1`，全部时间戳为 Unix 毫秒，窗口为 `[startMs,endMs)`。回执包含原查询、查询指纹和快照版本。
- 同一 SQLite 只读事务取得 Analytics 缓存与当前 apex 的价格目录，释放锁后聚合。账户余额和已保存金额不参与本机现价估算。
- 主 Tokens 为新输入加输出；缓存读取、写入分别显示。会话范围仅包含可识别会话，未关联的用量另外返回，不能为了与概览一致而丢掉或猜关联。
- 每个原生会话使用 provider／native SID 的稳定哈希。项目 ID 使用原生完整目录的独立哈希，显示路径保留原值；不会按同名目录或去掉有意义的空格来合并项目。未知目录与未解析目录不同。
- 分位数基于筛选和时间范围内每个会话的主 Tokens。已知零值纳入，未知用量排除，并返回样本数和排除数。中位数使用精确十进制字符串，避免大数或 `.5` 丢失。
- 只有已验证的原生父子关系用于用量归根；子会话消息不冒充主线程消息。消息只统计可见 user／assistant，活跃度是同会话中严格小于 5 分钟的相邻消息间隔之和，按查询窗口和本地自然日裁切。并发会话时长可相加。
- 消息没有可靠时间、原生身份或可见内容时，不显示假零。模型／来源筛选不能可靠归因消息与活跃时，指标为空并返回原因。
- 每日边界复用 `usage_time`，支持 23／25 小时自然日和被跳过的日期。全部范围从实际缓存事件开始，空缓存不会生成从 1970 年起的日期。

## 缓存与来源协调

`session_meta_cache` 的版本化 envelope 内，Metadata、UsageSync、Analytics 各有独立 revision。Analytics 用 CAS 更新自己的部分，保留其他 writer 和未知字段；未来或损坏的自有版本拒绝覆盖。数据库 schema 保持 v11。

`discover_sources()` 仅枚举和读取文件属性；`source_revision(spec)` 包含正文文件身份／时间、Gemini 的 `.project_root`、Codex 的 `session_index.jsonl`，以及 OpenCode 数据库 WAL 或 JSON session/message/part 依赖。正文或依赖变化才重新解析，解析前后再次核对来源版本。

- Claude／Codex 复用既有标题候选与规范化用量语义；Codex 标题索引变化也会失效缓存。
- Gemini 保留原生消息和项目侧文件的独立状态。已适配 0.64.0 JSONL：上下文回退后的当前消息与已经发生的用量分别投影，同一原生消息的重复 Token 快照只保留最终值。请求账本仍只发现 `chats/` 平层文件；会话分析保持既有有界递归，可读嵌套 child，但没有持久化父 SID 时不按目录名猜归并。原生 `kind=subagent` 不进入主会话消息／活跃时长；缺失或未知 JSONL 角色保持未知，旧 JSON 无 `kind` 保持兼容。
- Codex 原生 fork／replay 的多 `session_meta` 物理来源没有唯一会话归属时，缓存明确保存 partial 与原因，不产生推测的分析指标；未变化来源仍可暖命中。真正损坏的 JSON／不可读来源继续失败并保留旧事实，不以空 partial 覆盖上次成功统计。旧请求账本身份没有因此得到修复。
- OpenCode SQLite 的会话、消息、parts 和用量来自同一原生只读事务，保留一库多会话；旧 JSON 存储同样以原生 ID 去重。没有原生消息 ID 的文件副本不能成为额外请求。
- 物理来源删除后保留已知统计，清除管理跳转引用并标为不可用。枚举失败不等于删除：失败 provider 保留上次事实与引用、暴露读取失败，其余 provider 继续刷新。
- 文件搬移并修订后，当前观察到的同原生事件替代缺失／失败副本的旧缓存；没有新副本的历史仍保留并说明来源缺失。两个同时存在的冲突副本仍拒绝任意选择。
- 同原生事件的相同副本只计一次；冲突副本不按路径或 mtime 任意选一个。冲突消息身份、项目和父子 metadata 均显式降级。
- 缺失、跨工具或成环的原生父子链只降级相关组件，不阻止其他工具的查询；不将其消息指标伪装为已知零值。
- 公共来源健康与缓存在同一查询事务读取。旧缓存解析成功不能提升公共通道的 unknown／pending／failed；未来来源 stamp 的未知字段保留，暖缓存比较仅使用当前已知观察字段，不改变账本确认或负身份的证明规则。
- `RefreshReport.providerStates` 只覆盖实际观察到的来源。空缺不构成成功证明，公共健康 writer 必须单独处理没有文件／未尝试的情况。

缓存有来源数、单个 envelope 和查询批次总字节限制，超过预算失败并保留旧数据。当前冷读文本仍有整文件解析成本，不能把暖缓存无重复解析说成所有原生 I/O 均已流式化。

分析查询另保留一份无正文的已验证输入投影，十分钟有效、估算持有量上限128MiB。每次查询仍在同一SQLite事务读取全部原始缓存列、完整公共健康投影和最新价格目录；只有原始列与健康状态均相同时才跳过JSON解码和去重。价格、apex、时间范围和时区仍按本次请求计算，不把旧金额或旧筛选结果当作新结果。SQLite错误、删除、损坏及超预算不能回退到旧值。内存上限仅约束缓存持有量，在途请求的旧Arc可能增加瞬时总量；这份输入缓存与排行快照的128MiB预算分别计算。

## 已验证及待验

- 60 项会话分析回归：核心聚合、消息、缓存、原生 OpenCode、查询、来源发现、健康合流、坏父子图隔离与文件搬移均通过。新增边界先取得 55 通过／5 失败的 RED，修复经独立评审。601 个未变化来源第二轮解析数为零，查询后缓存字节不变。
- 10,000 会话、50,000 用量事实和 50,000 消息、1,460 天的 debug 聚合约 1.27 秒；返回 Top 100，序列化结果小于 1 MiB。这不是 release 的 p95 验收结果。
- 页面使用测试数据检查过 900px 浅／深色和详情 Escape 回焦点；不是实际本机统计或最终原生界面的验收。
- 最初阶段只接入 Claude、Codex、Gemini 和 OpenCode；之后新增 WorkBuddy、Pi、dsh、OpenClaw 和 Hermes，当前共有九个会话／项目分析桥接，覆盖所有可绑定工具（ChatGPT 的 Codex 模式计入 Codex）。新增桥接复用各自的原生来源与归属证据，具体能力见[原生分析桥接](native-analysis-bridges.md)；dsh／OpenClaw 的消息数与活跃时长仍保持未知。Hermes 按会话 × 模型的累计行计入，整行落在其最后活跃的那一天，与 magpie 相同。
- 概览／请求页的共享项目筛选和概览项目排行见[项目归属说明](usage-project-filter.md)。旧后端只有会话项目能力时仍保留原有的局部筛选降级。
- 早期集成曾完成当时版本的代码门禁、原生复验及真实CSV聚合对账，不能代替本轮最终SHA的验收。请求与概览按同一冻结范围核对；会话分析保留可核实的归属子集，未知历史不为配平而加入。2026-10-16 阶段的原样10k release热查询p95为264.8ms，属于[该阶段实施记录](implementation-progress-2026-10-16.md)，不能替代新增四个桥接后的最终测量。当前门禁、安装、原生结果和性能分别按最终源码记录；Windows原生行为仍待验证，不以macOS的类型检查替代。

## 会话工作台扩展

分析响应在同一个缓存／价格投影中增加以下兼容字段，不增加数据库迁移：

- `knownMetrics`、`metricCoverage`：只累计消息身份、角色和时间完整的已识别会话。旧 totals 与 series 的 nullable 语义不变；每日的 `knownMessages`／`knownActiveMs` 与相同子集对账。有效与排除数不是全量完成率，无法量化的来源缺口单独声明。模型或来源不可归因时，新指标同样不可用。
- `activityByHour`：有效消息间隔先按查询窗口裁切，再切到本地小时，汇总为星期一至星期日、0–23点。前跳小时为空，回拨的同名小时合并；总和与已知活动时长相同。时区投影和规则版本进入快照版本。
- `compactTop`：完整筛选 cohort 的 Tokens／可计算估价／活跃排行各六条。未知指标不填零，费用使用十进制比较；部分金额仍明确标为部分。
- `distributions`：Tokens、消息数和活动时长分布，已知零有效，未知排除；边界为各桶包含的下界，末桶无上限。不是从 Top100 推算。

`get_session_analytics_page({analyticsSnapshotId,expectedReceipt,sortBy,page,pageSize,search?,managerKey?})` 只读完整 cohort 的无正文内存快照。每页1–100条，搜索只影响列表，`managerKey` 精确匹配后端签发引用；`appliedView` 和 `viewFingerprint` 与统计范围回执分开。快照沿用10分钟、最多4份／128MiB、单份100k行的上限；超限明确返回分页不可用，不截断冒充完整。后台更新不替换已打开的排行，apex不匹配或过期需重新查询。

UI保留六格KPI、紧凑趋势与两栏排行；900px仍为双列，窄于720px改为单列。项目／工具切换与模型排行默认各六条，继续展示分批有界；隐藏只是显示偏好，不改变统计。日历按年查看当前范围；活动指标是消息间隔估计，不代表人的工作时间。

## 概览与会话列表的显示口径补充

概览的项目排行使用全宽紧凑卡片，默认显示六项，每次“更多”增加50项；工具、模型排行同样有界。项目名称列设上限，长名称截断，完整目录保留在标题提示中，不再重复占用整行。筛选、隐藏、恢复和继续展示仍可用，隐藏不改变统计。同步时间、完整价格目录版本和一次费用说明统一收进可展开的“数据状态”；来源失败／部分／未同步、价格不可用／陈旧／未计价等警告仍直接显示。会话页的静态能力说明按工具归组后默认折叠；文件大小、原生ID和I/O分解移至详情，列表保留主要识别与用量信息。

Codex会话列表曾按原生 `session_meta.id` 与旧账本 `session_id` 精确关联；子线程两种身份可以不同，因此原生用量存在时，旧关联仍会为空。本次仅修正显示投影：对身份、计数和来源可证明的Codex线程，显示该原生线程自身的新输入、输出、缓存读取和缓存写入，以及当前区域OFox目录的现价估算。父线程和子线程分别显示，不用归根后的父子混合总额回填子线程，也不回退到旧billing关联补造金额。

可选的 `usageDisplay` 回执包含 `state`、`basis`、`scope`、`reasons`、`estimate`、`pricingCoverage`、`modelEstimates`、`parentSessionId` 和 `billingSessionId`。当前 `basis=currentOfoxEstimate`、`scope=nativeThread`；可用的线程计数不等于价格完整。未知金额保持未知，部分计价保留“+”，可验证的零值仍显示零。父SID和billing SID用于说明来源，不是新的账本合并许可。前端会话查询缓存按apex隔离，价格目录每次在同一读取事务重新取得；切换区域不复用旧区域金额回执。字段缺失时保留旧后端兼容，不据此宣称原生归属已验证。

无正文的线程来源证明与Analytics输入投影共享有界 `Arc` 缓存。暖读仍逐次核验原生来源stat和缓存／健康观察，匹配时不重读正文、不重复JSON解码；只复用投影，不缓存旧价格。相同SID的多个副本需要完整来源字节的SHA-256 digest证明内容相同，仅Token事实相同不足以合并；来源字节不同保守返回未知。多header、fork、继承基线不明、不完整计数、冲突副本、陈旧来源和缺索引均保留不可用原因。此读取优化不扩大共享envelope writer／CAS原有的可写格式或修订兼容范围：旧Entry数组可以经查询兼容读取，但自有CAS仍按原规则拒绝，不会自动重建或写回。

这不是历史账本修复：不迁移旧 `request_id` 或billing `session_id`，不重写已保存费用，也不承诺补齐会话分析桥接的全部历史。线程显示证据与筛选后会话分析cohort仍是不同口径。代码入口见[线程投影](../src-tauri/src/services/codex_thread_usage.rs)、[列表金额回执](../src-tauri/src/session_manager/usage.rs)与[前端契约](../src/lib/api/sessions.ts)。代码提交 `a8ae76c1` 的本次前端与Rust门禁及两线独立复审已通过；macOS构建、安装及列明的真实数据显示复验已通过；Windows与完整AX回跳验收仍待验，精确数量与交付状态单列于[实施记录](implementation-progress-2026-10-16.md)。

## 本地正文搜索与阅读

正文搜索显式调用 `prepare_session_search`，先订阅 `session-search-progress`，再 `run_session_search`；`cancel_session_search` 设置同一任务取消标记。请求、事件和结果均以 operation／request ID 隔离。候选是已发现、后端签发的会话引用，不接受任意文件路径；不扫描回收站，不建立持久化正文索引，不上传查询或正文。

搜索使用原生 reader 还原后的可见消息，保留 Gemini rewind、Pi active branch、WorkBuddy快照替换等语义。命中包含 `sessionKey/readRevision/messageOrdinal/contentHash`；`get_session_transcript` 打开时验证来源与定位，变化时要求重新搜索。缓存中的旧定位不能绕过重新验证。进度和片段在返回前清理可识别凭据；显式阅读正文不改原生文件。

每批最多200个命中、每会话最多3个片段；默认只含用户与助手，过程内容需明确开启。时间、解码、结果上限以及变化／失败／不支持来源分别报告，不合成假零。普通元数据筛选不读取全部正文。阅读器保留虚拟化，增加安全Markdown、逐条和代码复制、时间、提问目录及本会话查找；HTML作为文本显示，图片不自动联网，链接只允许现有外部打开器支持的HTTP(S)。

Codex原生 `archived_sessions` 纳入发现、读取和导出。原生归档与OFox回收站不同；归档或跨根副本不因可读而开放继续／删除。没有前缀证明的缓存消息数保持未知，正文投影按可证明的身份处理副本。

安全读取预算与快照限制见[已知边界](known-limitations.md#session-workbench-acceptance-boundaries-2026-10-16)。普通阅读和生成继续命令也遵循经过验证的会话引用；阅读不会隐式执行工具。

## Interaction insights

The session analysis view now includes a visible session-shape histogram (messages, active minutes, tool calls per user prompt), frequent Skills, and tool usage with category composition and the latest 16 calendar weeks. The existing token distribution remains available in the expanded distribution panel. Messages and active time use the existing full-cohort projections. Tool summaries are computed before Top100 session truncation and use the same query, snapshot and timezone. No extra transcript reads occur while changing chart tabs.

Tool metadata is extracted during the existing native-source read: Claude assistant tool blocks, owned Codex response calls, OpenCode structured tool parts, Gemini toolCalls, Pi's active owned branch, WorkBuddy function calls, and Hermes's deduplicated SQLite message rows. Only bounded names, timestamps and hashed native call identities are indexed. Arguments, commands, tool output and file paths are not retained in this projection. Skills come from explicit Skill/activate_skill calls or direct Read/read/read_file calls for SKILL.md; arbitrary mentions in prompts are not counted. Tool outputs never create calls. Identical native call replicas count once; conflicting calls are excluded and mark coverage incomplete.

Coverage distinguishes fully counted sessions from unsupported/incomplete sessions. Tool/skill totals may retain verified calls from incomplete sessions and are labeled with “+”; calls-per-prompt distributions require complete call evidence and a known nonzero user-prompt denominator. An older snapshot or unsupported reader is unavailable rather than measured zero. Model/source attribution is not inferred from a nearby usage fact: those filters mark interaction statistics unavailable. Agent, project and half-open date filters still apply. Verified child calls roll up to their identified root conversation; unknown ancestry is not guessed.

Daily trends use wider stacked bars for up to 31 time buckets, then straight stacked areas over exact points. Real zero remains zero; null splits the area and isolated known points remain visible. There is no smoothing or weekly summation of daily session counts. The same keyboard-accessible points and exact data table remain available.

An explicit range within one local natural day (including Today) also returns an optional receipt-bound hourly trend for all chart metrics. The original daily series and natural-day activity KPIs remain unchanged. Hourly Tokens and estimates use the same filtered, deduplicated facts and captured prices; messages, runtime and session membership use the same cohort and metric eligibility. Native runtime gaps belong wholly to their later event, while legacy message gaps retain their clipped interval allocation. A session spanning hours counts once in each hour where it contributes; summing hourly session counts does not produce the range's distinct session total. The final hour stops at the exclusive query end. Chronological hour identities retain both occurrences during a fall-back, omit nonexistent spring-forward hours, and respect fractional timezone offsets. Multi-day and unbounded ranges retain the daily chart. Older backends without this field retain their existing daily chart.

The October 23 regression pass first reproduced the daily-only result, then verified the hourly projection, all metrics, null coverage, model filters, quarter-hour timezone offsets and 23/25-hour days. The renderer consumes the returned trend granularity, preserving the applied-query receipt during filter changes. Synthetic 900px previews passed in Chinese, English and Japanese, light and dark themes, with metric-arrow and hourly-point End-key navigation. Full gates passed: 1,517 frontend tests, 3,431 Rust tests (14 ignored), TypeScript, formatting and all-targets Clippy. Installed-app verification is recorded separately from these synthetic checks.

Verification for the interaction-insights change (2026-10-21, macOS, working changes over d3e1ad8a70131b8efd3aeca9f87d1645f1e893f4): typecheck, Prettier, rustfmt and diff whitespace checks pass. The complete renderer suite has 1,460 passing tests and one pre-existing Gemini/yargs ESM failure in nativeAcceptance.test.ts. Independent static review and re-review are complete, including the half-open window replica conflict, timezone, Hermes missing-ID/conflict, and partial-versus-zero cases. Synthetic 900px light/dark previews and keyboard metric/shape navigation were checked. The subsequent local installation was authorized and completed on macOS 26.5.1 arm64: all-targets Clippy passed; 3,390 Rust tests passed and 13 were ignored. The production 1.3.7 app was signed with the existing Developer ID identity, verified with deep/strict codesign checks, backed up and replaced locally. Real local history populated the session-shape, Skills and tool-usage cards; the Codex generation marker advanced to native parser v4. Windows and paid agent-request acceptance remain unverified. No release was published.

## Compact distribution cards (2026-10-21)

The expandable distribution panel uses three intrinsic-width cards rather than a viewport breakpoint above the native app's normal width. At 900px the Tokens, messages and activity cards fit side by side; narrower containers wrap without horizontal overflow. Each row shows the exact session count and its share of that metric's measured sample set. Bar length uses that same share, not the largest bucket. Excluded sessions and incomplete sources remain separate from measured zero.

Visible ranges use compact units; semantic list-item names and hover descriptions retain exact inclusive-lower/exclusive-upper bounds. Activity labels remain in minutes rather than mixing minutes and hours. Native disclosure keyboard handling and focus styles remain available. A new regression suite first failed on the previous panel, then passed after the redesign. Independent review and re-review passed. Synthetic browser previews checked 900px light/dark, 380px English, Japanese layout, no horizontal overflow and Enter/Space disclosure operation. These previews do not establish Windows native acceptance.

Final source checks for the card redesign (working changes over d3e1ad8a70131b8efd3aeca9f87d1645f1e893f4, macOS 26.5.1 arm64): typecheck, Prettier, rustfmt and all-targets Clippy passed. The renderer suite has 1,463 passing tests and the same one pre-existing Gemini/yargs ESM loader failure. All 3,390 Rust tests passed, with 13 ignored. The signed 1.3.7 package was backed up and replaced locally. The native app displayed all three compact cards side by side using real history: 417 measured token sessions, 415 measured message sessions and 408 measured activity sessions; exclusions and source gaps remained visible. Deep/strict codesign and installed/backup binary hashes were verified. No release was published. The packaged runtime-source fingerprint is `cb09be437634fb74e3bc97887e62e38ca6557a3fb3cdaa5180186b157dab754d`, captured before the renderer build. Concurrent edits to three unrelated library/console source files after that renderer build are outside this artifact and these check results; those working-tree edits were preserved.


## Warm filter reads and refresh feedback (October 2026)

Managed analytics queries can reuse a verified projection without rereading all raw JSON when the database instance identity, SQLite data_version/schema_version and total_changes are unchanged. The proof and read share an established read transaction; autocommit/write transactions take full validation. SQL imports and binary restores explicitly rotate the database identity because SQLite Backup need not increment total_changes. External commits, schema changes, mutations, TTL expiry, corruption and restore revoke the shortcut. Prices are still read fresh in the same snapshot. Unmanaged connections retain full content validation.

Local statistics publish as soon as native synchronization and their query complete, independently of pending or failed account endpoints. Manual failures are visible while retaining the last validated data. UsagePage owns publication of query-mode refresh results; usage-synced does not additionally refetch an old query anchor. Sync and staged reads have visible progress, a spinning refresh action and aria-busy. During filter changes the date summary follows the displayed result receipt, with a highlighted pending/previous-result notice. The last graph and its local state remain available while waiting; a late response cannot publish to a different query/apex owner.

A synthetic debug benchmark of 32 MiB raw cache measured approximately 1.5 seconds for full warm validation versus less than 1 millisecond for an unchanged managed SQLite observation. This measures projection reuse only, excluding native synchronization, aggregation, IPC, rendering and first-load network requests; it is not an end-to-end app speed claim.
