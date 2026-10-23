# 资源库工作台

资源库管理全局指令、MCP 和 Skills，独立于 OFox 模型绑定。指令写入适配器为 Claude Code、Codex、Gemini CLI、OpenCode、Hermes。MCP 与 Skills 覆盖全部 10 个可绑定客户端：另含 ChatGPT、OpenClaw、WorkBuddy、DeepSeek Harness、Pi；Codex / ChatGPT 合并成同一个物理配置目标。共享目录的其他读取者只作说明。项目级配置、Hooks、子代理、RTK、路由和网关不在此功能内。

## 从发现到应用

- 精选目录是随应用维护的配置入口。浏览不会连接 MCP、运行技能脚本或安装依赖。MCP 连接检查仍由用户明确启动，使用既有包运行器限制和原生授权流程。
- 原生发现只读扫描并返回有时限的内存票据。私密 MCP 定义留在后台；列表不含命令参数、环境变量值、请求头值或完整配置。原生指令中的用户正文与 OFox 标记块分开，旧提示词数据库另列来源。
- 选择性导入重新核验内容、物理身份、路由、数据库实例及状态 epoch，只创建未分配草稿，不移动原文件、不取得写入所有权、不启动资源。同名同内容可以归并；不同 MCP alias 保留原生功能身份；Skills 同名异内容使用独立目录变体。
- 推荐 Skills 通过现有仓库下载流程取得归档，只有所选技能进入不可变草稿，固定解析后的实际提交；不预装整库。依赖安装与脚本执行不在导入动作中。
- 点击工具 Logo 只暂存增量选择。预览按物理文件合成最终变化，明确确认后才执行。每个工具仍可选择不同指令集；同一物理指令文件不能同时接受不同套装。

## 批量操作与恢复

批量范围是当前子页所选资源。应用及撤回保留未选择工具的原有分配；MCP 共用配置文件只形成一次最终写入。Skills 共享读取与实际链接／复制方式独立说明。

本机批次日志位于版本化的 `library_batch_operations_v1` settings 项，SQL schema 保持 v11。它持有完整私有写计划，父批次、回执和不可独立执行的子日志通过同一 CAS 事务提交。旧单项执行、恢复和取消入口不能执行批次子日志。新版本的 SQL／WebDAV 导出与可分享资源包均排除批次日志；本机恢复保留本机日志并提升 epoch，旧计划失效。

每个物理组写入后核验并记账，跨文件可以部分完成。取消只停止未开始的组，成功项保留；中断后先核对实际状态，未完成项重新预览，不能重放旧写计划。“核对结果”不是任意撤销。历史容量管理保留未完成批次所需证据，完成历史与其子日志一起清理。

资源包可以导出全部或选中草稿。选中导出同时限制清单与技能 artifact 对象，不能把未选技能文件夹悄悄装入包内。MCP 私密值继续按原规则省略，导入后由用户补充。

## 状态与可访问性

列表区分已应用、待应用、共享可读、需处理。路径、版本、hash、字段引用与操作 ID 进入详情；状态不会仅靠颜色表达。工具 Logo 可通过键盘聚焦并读取名称、状态和不可用原因。MCP 只显示对应资源版本的上次显式检查结果及时间，不用“已应用”推断连接成功。

来源读取失败不等于空资源；过期票据、变更来源、部分成功及被跳过的本地修改均需显示。更新检查不会自行应用更新，有本地修改的来源或目标默认跳过，之后可进入单项差异检查。

## 已知边界

- Codex 的 `AGENTS.override.md` 独立列为只读来源，不把普通 `AGENTS.md` 误称为当前生效文件；既有 writer 的路径范围不扩大。
- Gemini CLI 0.64.0 原生 `url` 未指定 `type` 时会先尝试 HTTP 再回退 SSE；当前固定 transport 草稿不能无损表示此自动选择。扫描会明确标注相关未支持条目，保留其他可确认条目，不猜 transport 或发起网络检查。显式 `type` 和 `httpUrl` 仍可导入。
- OpenCode／Hermes 的可选全局配置层损坏时，只扫描能确认的默认与共享 Skills 目录，明确标注自定义范围不完整。
- 不认识批次日志的旧版本仍可能把新 settings 项包含在旧 SQL／WebDAV 导出里。创建批次后应使用支持批次过滤的新版本进行同步和导出；本次没有宣称旧客户端导出兼容安全。
- 原生 Windows 行为需要 Windows 验证，macOS 构建与测试不能替代该结果。实际客户端加载配置的验证应记录工具版本及范围，不用文件解析通过冒充原生消费通过。

## 第二轮：添加、定位与来源证据

当前分类的搜索同时覆盖资源库和本机发现项，匹配名称、用途、安全服务域名和来源工具，不索引凭据、命令参数或请求头值。首次访问在后台只读扫描；相关资源操作、目录设置变化和手动刷新会取得新票据。已收录项保留来源关系，可以展开定位到资源库。原生配置中的存在、OFox 写入回执及显式连接检查是三个独立事实。

来源详情通过不透明引用重新验证票据、数据库实例、配置根、路由、定义修订和恢复 epoch。文件夹定位不接受前端路径；复制路径和匹配项定位也重新核验来源。详情只返回字段名和类型，多层 OpenCode 来源展示有效合并结构，不能据此认定全部字段都写在所选单层文件中。列表不包含完整路径。内联和旧数据库来源没有文件定位操作。旧后端仍可管理资源，新定位／比较功能会说明不可用。

正向来源证明和缺失文件证明共同保护空扫描；空目录新增、原生文件从缺失变为存在、不可读来源或恢复后的旧票据都不能被解释为“本机没有配置”。同名异内容保留稳定身份并分组，比较仅返回字段路径和相同／不同／缺失状态，不返回秘密值，也不自动合并。

普通 MCP 新建和编辑共用表单。已有字段默认保留，普通编辑只提交变更的叶路径及原 `fieldRef`；改名不会改配置标识。新标识可读、可编辑，手动修改或首次提交后不再随名称变化。并发创建冲突不升级为覆盖。切换连接方式会列出需要清除的互斥字段并要求确认。

整条命令只做 POSIX／Windows 参数解析和确认填表，不执行 shell；不支持的 shell 操作明确拒绝。JSON 支持单项和指定外壳的多项选择，拒绝重复键、掩码、精度损失及无法确认的 transport。多项保存逐项报告成功与未完成；未完成项独立检查后保存，已成功项不会自动重放。应用目标默认不全选，保存与写入工具配置继续分离。

精选目录为 8 项 MCP、6 项 Skills 和 3 套指令，支持搜索和用途分类。已有识别由后端证据决定。Skills 的“已收录”只说明固定来源和有效本机 artifact，不代表远端最新版；更新需要明确检查。GitHub 的只读入口限制提供的工具集，实际权限仍由凭据决定。OAuth 仍在原生工具完成；包运行器不在 OFox 检查器允许范围时，应在对应工具中检查，不表示服务故障。

精选 Skills 收录及资源库单项更新检查解析实际提交，只获取所选目录的 blob；保留隐藏文件、目录内许可和执行权限清单，不执行脚本。Git tree、路径、模式、体积与 Git blob 摘要均核验后才形成 artifact。旧兼容的整库发现／批量更新检查仍有单独下载边界，不应将这些接口宣称为定向下载。原生平台和真实服务连接的验收记录与单元测试分开。

## 页内主流程

指令、MCP、Skills 都按“我的资源 → 工具中发现 → 发现资源／指令模板”组织。顶部分类保持可见；发现入口只定位页内区域并聚焦其独立搜索框。分类各自保留筛选、展开项和滚动位置。来源、状态收进筛选面板，排序和选择位于列表标题；资源包、历史和场景配置保留在更多菜单。

资源名称展开单个页内详情。指令支持 Markdown 编辑／安全预览、保存、放弃及 Cmd/Ctrl+S；MCP 展示连接摘要、应用关系和显式检查的完整工具清单；Skills 使用安全 Markdown 读取 `SKILL.md`，技术元数据与文件列表继续可查。原始 HTML 和远程图片不执行或自动加载。

未保存编辑的切换、分类导航及离开资源库统一经过保存／放弃／取消。普通 MCP 表单转高级编辑保留未保存标记；尚未解析确认的粘贴内容不能直接切入高级编辑。草稿只在内存中存在，原生关闭按钮隐藏窗口时保留编辑；退出进程后不会持久保存私密草稿。

“工具中发现”的来源 Logo 是只读来源入口；资源行的目标 Logo 则暂存增量选择。来源存在标记、应用确认、共享读取、待变更及异常分别表达；发现原生配置不取得所有权或推断成功连接。无冲突单项“加入”复用当前扫描票据，后端重新核验来源，成功后定位资源；冲突、过期或来源变化须比较／重扫，不绕过保护。

MCP 的可选 `displaySummary` / `iconId` 是展示提示。摘要只取既有安全服务域名、白名单启动程序及维护目录中的公开包名，不返回参数、完整 URL、环境变量、请求头或个人路径。图标来自固定随应用资源；不按用户 URL 下载、不凭显示名称认定配置相同。旧后端无这些字段时退回连接方式。

检查结果绑定资源、revision、epoch 和配置路由；变更后旧结果和未确认选择失效。检查准备、订阅、执行的每个异步边界都核对当前运行；取消、分类暂停或迟到的准备结果不能发布成功或继续旧执行。已完成且仍有效的工具清单在收起重开时保留，浏览和扫描不启动检查。

本轮界面验证使用 `scripts/library-interaction-fixture.html` 的合成数据，可通过 `pnpm exec vite scripts --config vite.config.ts --host 127.0.0.1 --port 3016 --strictPort` 运行。它封锁真实 IPC、配置写入、下载与付费调用；合成检查结果不代表原生客户端已消费资源。安装后只读窗口检查、Windows、公证及真实消费结果分别记录。

## 面向新用户的名称与来源关系

“我的资源”表示已保存到 Ofox 的资源定义，不保证已经应用或连接成功；“工具中发现”仅列出已支持工具配置里发现、尚未加入的内容。已匹配资源不再以“已收录”折叠区重复出现，其来源关系进入对应资源的页内“工具配置”。同名异内容仍保留独立变体与比较入口，不能因名称相同而隐藏。

单项加入仍复用只读扫描票据，并在后端重新核验；加入后定位我的资源，但不写入工具配置。工具配置已经全部加入时，提供“查看我的资源”入口，等待筛选后的列表实际挂载再滚动和聚焦。详情内重新扫描保留区域和按钮焦点，扫描期间立即移除旧来源结论。读取失败、票据过期与来源待确认仍可见。

界面、中英日提示、系统通知统一使用 **Ofox** / **Ofox Desktop**。持久化 provider 身份、钥匙串名称、协议标记与 `OFOX_` 环境变量保持兼容；展示名称修正不迁移或重写用户配置。

## Skills 目标范围与吸附栏修正（2026-10-21）

Skills 行内开关和安装选择器取 OFox 可绑定工具目录（`TOOL_ORDER`）与后端 Skills 适配列表的交集，再保留已检测到或已有分配的目标。Cursor、CodeBuddy、Claude Desktop 不属于 OFox 绑定范围，不显示为可分配目标；“全部”也不再新增这些目标。已有隐藏目标的分配保留，切换其他目标不会自动撤回历史配置。原生发现项仍可展示真实来源，来源不等于可分配目标。

当前可展示 Claude Code、Codex、OpenCode、OpenClaw、Hermes、Gemini CLI、WorkBuddy、DeepSeek Harness 和 Pi；未检测到且没有分配的工具不显示。ChatGPT 的 Codex 模式使用 Codex 的 Skills 目录，不新增独立 ChatGPT 列。没有新建绑定或写入配置时，不会因显示范围变化修改本机文件。

CC Switch 的帮助文案描述其自身同步范围。OpenClaw 的[官方 Skills 文档](https://docs.openclaw.ai/tools/skills)支持用户级 Skills。本次在 macOS 使用已安装的 OpenClaw 2026.10.3，以临时 `OPENCLAW_STATE_DIR`、配置文件和工作区隔离执行 `openclaw skills list --json`，成功发现临时 Skills 目录内的符号链接样例，并返回 `eligible: true`。这证明该版本的本地技能发现；不代表调用技能执行成功或 Windows 已验证。

资源库滚动容器移除顶部空隙，分类栏使用覆盖容器全宽的不透明 `canvas` 背景，避免内容从顶部、两侧和背景透出。合成浏览器检查覆盖 900px/1000px、亮色/深色、Tab/Shift+Tab 和“发现”定位后的焦点可见性。没有改动 MCP/Skills 文件写入、绑定、区域地址或数据库逻辑。

本次验证基线：`release/1.3.7` 的 `d3e1ad8a` 加当前工作区改动，macOS。`pnpm typecheck`、`pnpm format:check` 通过；三份相关测试共 71 项通过。完整前端测试为 1467 项通过、1 项失败：未修改的 `tests/guards/nativeAcceptance.test.ts` 在加载 `yargs/yargs` 时报告 `require is not defined in ES module scope`，定向复跑仍失败。本次未改 Rust、未重新打包安装，也未进行 Windows 原生验证。

### MCP 可绑定应用补齐（2026-10-21）

MCP 矩阵与添加表单覆盖 Claude Code、Codex / ChatGPT、OpenCode、OpenClaw、Hermes、Gemini CLI、WorkBuddy、DeepSeek Harness、Pi。Codex / ChatGPT 是一个文件目标，只写一次；未安装目标保留可见并说明原因。后端原生路径、发现、批量分配使用相同九目标注册表，旧收据与 schema v11 保持兼容。

新增适配：

| 目标             | 用户级配置                                                                                          | 写入格式与边界                                                                                                                                 |
| ---------------- | --------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| OpenClaw         | 配置路径覆盖或 `OPENCLAW_CONFIG_PATH` / `OPENCLAW_STATE_DIR` / `OPENCLAW_HOME` 下的 `openclaw.json` | JSON5 `mcp.servers`，stdio / streamable-http / SSE；保留模型与其他字段；外部 include 范围拒绝写入                                              |
| WorkBuddy        | `WORKBUDDY_CONFIG_DIR` 或 `~/.workbuddy/mcp.json`                                                   | 严格 JSON `mcpServers`；stdio / HTTP / SSE；识别 `disabled`；客户端授权文件不由资源库改写                                                      |
| DeepSeek Harness | `DSH_HOME` 或 `~/.dsh/cordis.patch.yml`                                                             | 每个 MCP 一个 `@deepseek-ai/dsh-mcp-client` 插件，stdio / streamable-http；home patch 对所有 profile 生效，检查 profile 同名冲突及预览后的变更 |
| Pi               | `PI_CODING_AGENT_DIR` 或 `~/.pi/agent/mcp.json`                                                     | 严格 JSON `mcpServers`，stdio / HTTP；原生 MCP 要求 0.99.0+；不支持 SSE，名称归一化冲突阻止写入                                                |

嵌套字段和 Cordis 插件复用文件事务、逐字段所有权、冲突检测及回滚。DeepSeek 的可执行 YAML 标签、未验证的环境表达式及 Pi 的 `!command` 值不会作为普通字符串迁移；遇到此类配置显示原因并保留原文件。WorkBuddy 通用 `${ENV}` 表达式不被假定为其原生环境展开语法。

DeepSeek 默认注释误报修复（2026-10-23）：dsh 0.2.1-rc.2 自动生成的 profile 注释包含 `!!js` 示例。原 MCP reader 对全文搜索标签标记，把正常注释误判为不支持的配置，导致本应为 0 的 MCP 数量显示为未知。现在通过 YAML token 检查实际标签；注释、引号、普通文本和块字符串中的示例正常读取，真正的标签、标签指令和重复 plugin ID 仍拒绝处理。home 与所有 profile、扫描与编辑预览共用这一检查。回归覆盖默认 profile、真实 MCP 发现/显式导入及增删后的无关内容保留；测试只写临时沙箱，Windows 原生验证另列。

该修复在旧实现上取得 3 项失败回归，修复后 DeepSeek 相关 90 项通过。最终 macOS arm64 全量验证为前端 1515 项、Rust 3421 项通过（14 项忽略），类型、格式及 `clippy --all-targets -- -D warnings` 通过；独立审查覆盖 token 解析器的内存生命周期、实际标签拒绝和配置保留。基线为 `d3e1ad8a` 加当前未提交的 release 工作区改动，日志前缀为 `/tmp/ofox-dsh-`。

macOS 隔离验证：OpenClaw 2026.10.3 `mcp list --json` 读取 stdio/HTTP 示例；Pi 1.3.0 的 `loadMcpConfig` 读取两类示例且无错误；DeepSeek MCP 0.2.1-rc.2 的原生 `Config` schema 接受两类示例。WorkBuddy 5.7.7 对照安装包的用户 MCP reader/form serializer，确认配置位置、传输键和原生授权边界。上述验证没有启动外部 MCP 服务或模型请求，不等于客户端完整会话验收。Windows 和 WorkBuddy 实际运行加载仍待验收。

900px 明暗主题检查使用合成页面，显示九目标并仅在表格内横向滚动；新增表单目标齐全、键盘和 Escape 关闭检查通过。截图不读取用户 MCP 配置。

验证记录：macOS arm64，基于 `d3e1ad8a` 的当前 release 工作区（含未提交修改）。最终 `cargo test` 共 3397 项通过、13 项忽略；`cargo clippy --all-targets -- -D warnings`、Rust 格式、TypeScript 和前端格式检查通过。前端全量 1472 项通过，1 项既有 `tests/guards/nativeAcceptance.test.ts` 的 Gemini/yargs ESM 加载失败；资源库相关用例通过。日志保存在 `/tmp/ofox-mcp-verified-all-rust.log`、`/tmp/ofox-mcp-verified-clippy.log`、`/tmp/ofox-mcp-final-all-ui.log`；隔离配置读取/schema 日志在 `/tmp/ofox-mcp-native-probe.log`。本轮没有打包、安装或发布；独立代码审查、Windows 和完整客户端会话验收未执行。

## MCP 状态读取反馈（2026-10-23）

首次读取应用 MCP 配置时，表格上方显示进度，数量和开关使用占位，读取完成后显示实际状态。重新扫描相同资源和配置路径时，暂时保留上次结果并明确标记更新中；旧扫描票据不用于导入或切换，操作等待当前扫描完成。配置路径或资源定义变化时不沿用旧结果。

整次扫描失败或票据过期时显示未知和重试。单个应用有任一来源不可读或不受支持时，该应用整列保持未知，其他应用仍显示已确认状态；同一应用的某个来源读取成功不能抵消另一来源的失败。重试把键盘焦点交给稳定的搜索框。

验证覆盖延迟读取、刷新时保留结果、失败重试、混合成功与失败来源，以及重试后的键盘焦点；相关 25 项测试和独立复查通过。900px 明暗主题和 Tab / Shift+Tab / Escape 使用隔离合成页面检查，不读取用户配置。本次为 renderer 改动，已安装应用及 Windows 原生窗口效果尚未验证。

最终检查：macOS arm64，基于 `d3e1ad8a` 的当前 release 工作区（包含既有未提交修改）。TypeScript、前端格式、1492 项前端测试、Rust 格式、全目标 Clippy 通过；Rust 测试 3406 项通过、13 项忽略。本轮未打包、安装或发布。

## MCP 弹窗退出入口（2026-10-23）

添加和编辑 MCP 弹窗提供右上角关闭按钮及固定底部的“取消”，表单滚动时仍可点击退出。未提交内容沿用 Esc 的丢弃行为，不触发保存；保存过程中关闭、取消及 Esc 均暂时禁用。关闭后焦点返回打开弹窗的按钮，按钮不可聚焦时返回列表搜索框。

新增四项退出回归先失败后通过，并经独立审查；覆盖添加的关闭／取消、编辑关闭、保存期间禁用及焦点交接。900px 明暗主题使用隔离合成页面验证，点击和 Tab / Shift+Tab / Enter 关闭可用。本轮完整前端测试 1496 项通过，类型和格式检查通过；未重新打包安装或进行 Windows 原生窗口验证。

最终检查基线仍为 macOS arm64、`d3e1ad8a` 加当前 release 工作区改动。Rust 格式、全目标 Clippy 及全量测试通过（3406 项通过、13 项忽略）；日志为 `/tmp/ofox-mcp-close-unit.log`、`/tmp/ofox-mcp-close-clippy.log` 和 `/tmp/ofox-mcp-close-rust-tests.log`。

## 下载与多目标分发（2026-10-22）

- MCP 和 Skills 的目标名单由同一注册表维护，ChatGPT 单独被发现时也能提供共享 Codex 目标。Pi 原生 MCP 仍要求可确认的受支持版本。
- 新的仓库收录不隐含启用 Claude。仓库身份按所有者、仓库及完整相对路径识别；仓库名忽略大小写，内部路径保留大小写。根目录是明确的空来源路径，安装文件夹名与来源路径分离。离线仓库返回检查失败。
- ZIP、本地目录和原生来源可先收录到独立存储，再选择多个目标。ZIP 按归档摘要及完整内部路径识别来源，逐项报告冲突和失败；同名外部内容默认保留。缺失或修改的 artifact 不得按“已安装”跳过。
- 追加目标只写新选中的目标，不重写先前已应用目标。每次收录返回 epoch、revision 和 artifact 摘要；部分失败后的重试只处理失败项，并重新验证这些版本。成功项不会重新下载。
- 全部客户端的 writer/schema、原生识别、真实 MCP 调用和最终账单是不同验收层级。新增验收范围可只选 MCP 与 Skills，但必须同时包含两者；原有三类资源验收保持兼容。ChatGPT 和 WorkBuddy 必须由桌面入口完成客户端验收。

本轮结果与外部阻塞单独记录于 [资源分发验收报告](resource-distribution-acceptance.md)。

## Skills 同名副本与 Magpie 对齐（2026-10-23）

- 按技能目录身份合并同名副本，优先显示共享文件夹中的版本，其次按应用顺序选择；SKILL.md 的显示名相同但目录不同的技能仍保留各自入口。
- 相同内容的应用显示为图标，内容不同的应用在右侧显示带圆点的琥珀色“Codex 中不同”标记。悬停或键盘聚焦可查看说明；导入当前版本不会改动不同副本。“全部导入”每组只提交选中的原始扫描 entry，不放宽票据或 epoch 检查。
- 已收录技能的直接应用目录遇到不同版本时，可选择“用资源库的”或“保留应用自己的”。前者复用预览、版本及路径检查和目录事务，先保留原副本再替换；后者只放弃该物理入口的所有者和分配，同时取消该入口尚未完成的普通及批量计划，旧预览不能重新取得授权。
- 链接替换不写其外部目标。备份保存可独立恢复的内容快照，并记录原应用路径和链接文本；覆盖绝对、相对链接的恢复回归。备份沿用现有 Skills 备份目录及不自动清理的原生备份策略。
- 嵌套或独立 reader 目录中的不同版本仍显示差异；这些位置不因展示或导入而获得写入权限，当前仅支持在已解析的直接应用目录中替换。Claude Desktop 自带技能仍只可保留。

验证：两项初始 UI 回归先失败后修复；独立审查发现并修复链接恢复、目录身份隐藏和旧替换预览复用三项问题，复查无新增问题。前端全量 1506 项通过；Rust Skills 定向 36 项通过，包含九个可绑定目标的副本保留、替换备份、原生文件变更、旧普通及批量预览撤销和链接恢复。900px 明暗主题、弹窗 Tab / Shift+Tab / Escape、返回焦点和合成导入完成后的焦点通过 computer use 检查。合成预览不写真实应用配置，不等于 Windows 原生验收。

完整检查：macOS arm64，当前 `d3e1ad8a` 加 release 工作区改动。TypeScript、前端格式、Rust 格式、全目标 Clippy 通过；全量前端 1506 项通过，Rust 3410 项通过、13 项忽略。日志为 `/tmp/ofox-skills-unit.log`、`/tmp/ofox-skills-rust-tests.log`、`/tmp/ofox-skills-clippy.log`。用户随后明确授权编译、替换本机应用并仅保留最新构建；安装结果另行记录。

本机安装：按用户授权执行 `pnpm tauri build --bundles app`，生成 1.3.7 的 macOS arm64 本机构建。Tauri 产物补齐本机 ad-hoc 包签名后通过 `codesign --verify --deep --strict`；直接移入 `/Applications/Ofox Desktop.app`，安装二进制 SHA-256 与安装记录一致。10 个同标识旧应用包移入废纸篓，旧 Downloads 安装镜像的挂载卷已卸载；最终仅该安装路径保留活动应用包，构建目录没有留下第二份 `.app`。安装记录在 `/tmp/ofox-installed-latest.json`，构建与安装日志在 `/tmp/ofox-skills-build.log` 和 `/tmp/ofox-skills-install.log`。

实际新应用已启动并读取现有资源；Skills 新文件图标、合并后的来源列表和可用的导入入口已在本机窗口观察。用户随后自行操作应用，停止后续 UI 输入；不同版本替换和保留的写入行为由隔离测试覆盖，本轮未通过自动 UI 操作改动用户的原生技能副本。Windows 原生运行仍未验证。


## Searchable Skills discovery (October 2026)

Skills discovery now has repository catalogs, skills.sh search, and a secondary curated tab. Repository search covers names, descriptions, full source paths and owner/repository; repository filters and confirmed collection status narrow results. The view renders at most 40 rows per page and caches read queries for five minutes. Repositories load independently: successful results stay available with explicit retry for failed sources. Repository management supports adding, enabling/disabling and removing configured sources.

Skills management and online discovery are separate views. Discovery is hidden and does not automatically request catalogs until the top Discover button is used. Back to Skills restores the list's scroll position and returns keyboard focus to that button; reopening discovery retains its filters, search and cached results. Choosing a result returns to the source/install review after the management DOM is visible, without collecting or writing anything. Explicit Configure Skills navigation from another page opens management at the source field. Hidden discovery pauses read queries and closes management/source dialogs so retained pages cannot revive a portal or move focus into hidden content.

Navigation regressions reproduced the old inline listing, stale dialogs and explicit-location scroll restoration before the fix. After the final change, 1,521 frontend tests and 3,431 Rust tests passed (14 ignored), along with TypeScript, formatting and all-targets Clippy. Independent review covered both explicit navigation cases. Synthetic 900px light/dark checks in zh/en/ja used 70 Agent rows, confirmed an exact 1,910px list-position round trip, kept the discovery search below the sticky header, returned to a visible install field after selection, and verified Enter/Escape focus handoff. These previews do not write native configurations or replace installed-app acceptance.

Repository discovery reads a bounded GitHub tree and only SKILL.md descriptors, with 16 concurrent reads, a 30-second overall deadline, per-document and aggregate byte limits, and Git object verification. Browsing does not download other repository assets or execute scripts. Selecting a skills.sh result first resolves actual paths against HEAD; the user confirms a concrete source before the existing epoch-bound collection and target review. This preserves nested/root identity and does not guess a path or assume the default branch is main. Public GitHub/skills.sh availability and network speed still affect the first discovery read.

Renamed or transferred GitHub repositories may return redirects. Discovery and selected-directory downloads follow at most three redirects for repository metadata on HTTPS api.github.com, keeping the exact commit/tree resource and query. Raw blob redirects, foreign hosts and changed resources remain blocked; commit, tree, size and blob-hash checks still apply. This supports the existing cexll/myclaude source after its transfer to [stellarlinkco/myclaude](https://github.com/stellarlinkco/myclaude), without rewriting configured source identities. GitHub rate limits and other network failures still surface as failed reads.

The October 23 regression pass reproduced the old redirect rejection before the fix. Six focused tests cover migrated catalogs and selected downloads, descriptor hash rejection, relative/canonical metadata paths, untrusted or changed resources, query retention, loops and hop bounds. Independent review found no blocker; the full project gates passed alongside the hourly analytics fix (1,517 frontend and 3,431 Rust tests, 14 Rust tests ignored). Public-source probes confirmed the transfer, but a later probe hit GitHub's rate limit; offline regressions do not claim that public network access is always available.

Read-only resource lists check native directories and the existing CLI fallback search paths without spawning CLI version commands. Windows desktop-only clients retain the existing bounded Store/WorkBuddy metadata reader outside the configuration lock; their cold presence lookup may still wait for PowerShell. Previews for MCP/instructions continue to validate current native installation/version. Content hashes are reused only inside one locked Skills projection; each link text/physical identity is checked, and the next scan checks contents again.
