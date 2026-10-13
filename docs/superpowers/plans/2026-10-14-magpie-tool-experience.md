# Magpie 功能借鉴与 OFox 工具体验实施计划

## 目标与开发基线

强化 OFox Desktop 现有工具管理和日常使用体验，实施模型选择体验、模型 Profiles、会话中心，并新增 DeepSeek Harness（dsh）和 Pi 的直连接入。保留现有 OFox 账号、独立工具 Key、绑定记录和区域协调流程。

开发分支为 `codex/magpie-tool-experience`，起点为 `71c885f8d64df6b24413a63794e671e650292e2e`。它包含本轮最新工具管理与兼容修复。创建时，发布分支 `release/1.3.6` 的 `5466546d` 在该提交上只增加四个版本文件的变更。

开发使用独立 worktree。已核对实际发布 tag `1.3.6_20261014_0219` 指向 `5466546d`，并将开发分支 rebase 到该提交。四处版本文件继承 1.3.6，不增加发布版本；当前发布 checkout、tag 和发布工作流保持隔离。实施进度和验证结果记录在本文的阶段检查项中。

用户要求完成后先交付测试，确认测试结果后再发布。开发阶段不触发正式发布，不创建发布 tag，也不更新正式更新源。

## 已确认的产品范围

- 第 1 项：工具行、托盘和管理弹窗的模型选择体验。普通单模型工具选择后自动保存；多模型工具保留完整受管集合。
- 第 2 项：OFox 模型 Profiles，只保存模型组合。OpenCode、WorkBuddy 保存有序模型集合；Codex 与 ChatGPT 共用一个配置写入目标。
- 第 4 项：会话列表、分组、元数据搜索、实际支持的正文读取、导出与续聊，以及文件回收站。搜索范围为标题、项目、工具和其他元数据。
- 新工具：dsh 和 Pi 接入检测、安装、更新、启动、绑定、解绑、Key 生命周期、模型目录和区域切换。
- 暂不引入本地网关、账号池、订阅代理、资源库迁移、跨会话正文索引，也不新增 effort 管理。

## 最新代码需要保留的行为

### OpenCode 的集合和当前模型

`get_active_ofox_models` / `set_active_ofox_models` 管理模型集合；保存返回 `SaveModelsReport { saved, skipped }`。模型协议按现有目录和检测缓存决定，保存不等待流式探测；后台验证结果仅产生建议。

OpenCode 的 TUI 当前 route 与受管集合是两个状态。旧单模型 setter 会重建单元素集合，不能用于快捷切换。首版工具行与托盘提供当前模型显示和多选入口；若增加直接切当前模型的交互，必须另加只激活 route 的接口，保留集合，并复用 `opencode_state`。

### ChatGPT 的显示和配置写入

ChatGPT 当前模型从 App recent state 读取；保存修改 Codex 配置默认模型。统一快照区分 `managedSelection` 与 `observedSelection`，并携带来源和写入目标。Profiles 的“正在使用”以受管配置为准，不能以 ChatGPT 最近显示的模型判断。

### 已存在的解析与兼容处理

复用 `live_model.rs`、`route.rs`、`chatgpt_state.rs` 和 `opencode_state.rs`，继续采纳用户在 CLI 中的模型变化。接入不变量被修改时报告冲突，模型、effort、MCP 和其他无关字段继续保留。

Codex 严格配置探测保持动态判定，不能恢复为固定版本阈值或无条件写入字段。探测在后台预热，避免阻塞快照读取。模型/Profile 写入的锁与 revision 检查必须覆盖启动时 Codex 配置清理；撤销只还原模型选择，不恢复整份旧配置。

## 并行开发安排

| 开发单元   | 可立即推进的内容                                      | 集成依赖                          |
| ---------- | ----------------------------------------------------- | --------------------------------- |
| 共用选择器 | 从管理弹窗提取单选、多选和搜索分页，保留当前行为      | 行内和托盘自动保存等待统一快照    |
| 工具适配   | Pi、dsh 配置 resolver、读取、合并写入、回滚和 fixture | 统一工具身份、Key 与区域生命周期  |
| 模型状态   | 受管值与观察值快照、一致读取、跨窗口事件              | 包装现有 reader/writer 和新适配器 |
| Profiles   | 数据结构、DAO、校验、预览和模型组合匹配               | Apply 等待快照和各工具 writer     |
| 会话中心   | Console 导航、现有 parser 接入、Pi/dsh 解析           | 动作按后端能力声明开放            |

新工具绑定与生命周期完成后，再将其纳入第 1、2 项的完整应用流程。会话扫描和正文读取可以独立推进。

## 新工具适配

### 统一工具身份与生命周期

为 `BindableTool` 增加稳定身份 `dsh`、`pi`，明确区分旧文件适配器与 WorkBuddy/dsh/Pi 原生适配器。旧 `AppType` 的 provider/proxy/MCP/资源体系保持现有用途，避免为新工具强制接入无关子系统。

当前 `account_keys::file_tool(None)` 代表 WorkBuddy，新增工具前必须改为显式原生工具分派。同步更新 Key 签发、替换、撤销、退出登录、缺记录对账、钥匙串 slot、绑定快照、启动协调与区域切换遍历。

同步工具元数据、首页、引导、托盘、检测白名单、安装器、更新包名和启动二进制。发现、绑定、模型读取、会话和启动共用配置目录 resolver；无法确定的相对路径明确报告。Windows 检测和启动使用同一 CLI 实例，并覆盖 `.cmd`。

### Pi

官方当前 npm 包为 `@earendil-works/pi-coding-agent`；识别既有 `@mariozechner/pi-coding-agent` 安装并保留其更新渠道。安装前按目标包 engines 检查 Node，当前官方源码要求 Node >=22.19.0。

配置目录遵循 `PI_CODING_AGENT_DIR`，默认 `~/.pi/agent`。独立管理 `models.json.providers.ofox-pi`，通过 `settings.json.defaultProvider/defaultModel` 设置新会话默认模型。

复用 Magpie 的 `piTakes`、`piScopeWith` 和 `piStartup`：已有非空 `enabledModels` 未覆盖所选模型时，追加精确模型引用；保留 glob 和 thinking 后缀，记录 OFox 添加的项目以供解绑恢复。保留其他 provider、扩展与用户字段。同名 provider/auth 冲突时报告，避免覆盖原生登录。

各模型协议和元数据来自当前 OFox 目录，支持 Chat、Responses、Anthropic。模型级 `api/baseUrl` 使用所选 apex 的 URL builder，Anthropic 使用 OFox 的 Anthropic base URL。绑定与默认模型选择要求明确模型 ID。

### dsh

官方 npm 包为 `@deepseek-ai/dsh`。配置目录遵循 `DSH_HOME`，默认 `~/.dsh`。接入现代 `llm-pi-ai` 自定义 provider 布局，处理 `profiles/<name>/cordis.patch.yml`、home 覆盖和实际生效的配置层。

为三种协议分别创建 OFox route，例如 `ofox-dsh`、`ofox-dsh-responses`、`ofox-dsh-anthropic`，由默认模型指向具体 route/model。保留用户 routes 和 DeepSeek 原 provider。未初始化与不兼容布局分别报告，避免将空目录当作旧版本。

使用独立 `OFOX_DSH_API_KEY` 引用，处理 `.env` 和 `.credentials.yaml.refs` 的实际读取优先级。未知凭据 schema、自定义 store 或同名冲突明确报告。patch、凭据引用、文件存在性与权限作为一个适配器事务处理，失败时恢复。

绑定记录保存实际管理的 profile 文件集合。新增 profile 不自动接管；已有受管 profile 模型不一致时显示“多个配置”。OFox Profile 应用前列出目标范围，应用后保留逐 profile 撤销信息。

### 区域与配置保护

Rust 使用 `ofox_apex.rs`，renderer 使用 `ofoxUrls.ts` 和 `useOfoxApex`。绑定前解析 apex，首次检测后协调已写入的受管 URL，重启与区域切换更新 OFox 条目。用户编辑的接入字段或无关配置保持原有冲突语义。

## 第 1 项 模型选择体验

先提取可复用 `ModelPicker` / `MultiModelPicker` 和搜索、分组、分页逻辑，保留 Dialog portal、滚动、小批量渲染、已选模型顺序与当前保存行为。

新增聚合快照，包含绑定状态、受管选择、观察值、来源、写入目标和 revision。在锁内完成应采纳的 live model 读取，避免先读模型再异步更新绑定的竞态。保存完成后发布原生事件，刷新 Console、托盘和管理弹窗。

普通单模型快捷选择沿用当前兼容规则和现有 writer。OpenCode/WorkBuddy 首版快捷入口打开共用多选流程；不得调用会缩减集合的单模型 setter。Pi 和 dsh 不套用旧工具“空模型即 OFox 默认路由”的语义。

## 第 2 项 模型 Profiles

使用现有 SQLite 数据库新增 Profiles 表和 DAO。recipe 只存工具目标和模型 ID；集合类型保留顺序。缺失目标表示不修改，显式默认仅用于支持该语义的工具。Codex/ChatGPT 归一为一个配置组，避免重复写入。

应用流程为预览、按当前工具规则预检、逐目标提交和结果报告。未安装或未绑定的目标跳过；硬配置错误、不可读配置及明确不兼容阻止相应写入。OpenCode 的部分 saved/skipped 结果必须展示，不把返回成功视为全部模型已应用。

保留现有文件事务、数据库回滚和配置 CAS。跨工具应用逐项执行，首个失败停止后续提交并报告已成功目标。提供短期模型撤销，撤销前校验账号、apex、配置作用域和 revision。跨工具不宣称文件与数据库全局原子提交。

“正在使用”从实际受管配置计算；不根据最后一次点击的 Profile 判断。recipe 不保存 Key、URL、模型目录元数据或兼容检测结果，应用时解析当前目录。

## 第 4 项 会话中心

将会话入口接入实际挂载的 Console，保持工具视图及账号状态，详情面板独立滚动。复用已有六种解析器、虚拟列表和元数据搜索，按后端声明显示动作。

| 来源     | 首版元数据                                   | 正文与 Markdown                 | 续聊                           |
| -------- | -------------------------------------------- | ------------------------------- | ------------------------------ |
| 现有工具 | 保留并接入 Console                           | 复用实际支持的读取器            | 按真实 CLI 与现有 resume 能力  |
| Pi       | 项目、标题、模型变化、自定义 session dir     | 回溯当前 JSONL 分支后读取和导出 | `pi --session`，使用原项目目录 |
| dsh      | 最高 generation、压缩 JSONL、parent/subagent | 首版关闭                        | 首版关闭通用续聊               |

Pi 复用 Magpie 按 parentId 回溯当前分支的逻辑，导出使用完整读取结果。Pi 恢复会话沿用会话内模型，新默认只影响新会话。

dsh 选择最高 generation，同代按修改时间决定来源，避免重复计数。支持压缩帧和 UTF-16 转义的 session ID 路径验证。Magpie 尚无原生 dsh 正文读取器；官方 headless `--session-id` 需要新 task 且有 cwd/preset/ownership 约束，通用会话 UI 不直接映射成继续对话按钮。

文件回收站先开放 Claude/Codex/Gemini，完整处理会话附属文件。OpenClaw 先提供现有浏览和导出能力。Pi/dsh 分别完成 bundle planner、活跃会话拒绝和恢复测试后开放回收站。dsh bundle 包括主会话全部 generation、锁、子 Agent 和对应 projection cache 行，不移动共享索引或整份共享数据库。

回收站先写 manifest，再记录移动进度，支持崩溃恢复；失败补偿，跨盘仅在 EXDEV 时复制，恢复前检查所有目标占用且禁止覆盖。路径与动作由服务端生成并校验，首次 UI 不暴露旧永久删除入口。

## Magpie 复用来源

| 领域             | 源码与 fixture                                                                                                                                                                 |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 模型选择与托盘   | `internal/gui/assets/app.js`、`trayclick.go`、`traymenu.go`；相关 picker/tray fixture；UI 行为移植到现有 React 组件                                                            |
| Profiles         | `internal/profile/profile.go`、`details.go`；`inuse_test.go`、`modellist_test.go`、`defaults_test.go`、`details_test.go`；复用缺失目标、显式默认、有序集合与实际使用判断的语义 |
| Pi 配置          | `internal/agent/agents.go`、`pi.go`；`pi_dir_test.go`、`pi_scope_test.go`、`pi_api_test.go`                                                                                    |
| dsh 配置         | `internal/agent/dsh.go`；`dsh_route_test.go`、`dsh_protocol_test.go`、`dsh_home_preserve_test.go`、`dsh_creds_test.go`                                                         |
| 安装更新         | `internal/agent/install.go`、`cliupdate.go`；沿用 OFox 已有平台实现                                                                                                            |
| Pi 会话          | `internal/sessions/pi.go`、`transcript.go`、`markdown.go`；Pi branch/session-dir fixture                                                                                       |
| dsh 会话与回收站 | `internal/sessions/dsh.go`、`manage.go`、`dsh_manage_test.go`                                                                                                                  |

移植算法、失败语义和回归 fixture，同时使用 OFox 现有数据库、事务、锁、Key 和 endpoint builder。记录移植来源；复制 substantial source 时保留 Magpie MIT copyright 和许可文本。

官方参考：[Pi 模型配置](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/models.md)、[Pi 包与 Node 要求](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/package.json)、[dsh CLI 配置层](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/cli/reference/README.md)、[dsh headless 会话恢复](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/bundle/headless/README.md)。

## 阶段检查项

- [x] 创建独立开发 worktree，核对最新 main 和 release/1.3.6 的差异。
- [x] 修正规划中的 OpenCode 多模型和 ChatGPT 显示/写入语义。
- [x] 提取共用模型选择器，保持管理弹窗行为并通过前端检查。
- [x] 完成统一快照、跨窗口刷新和模型写入并发校验。
- [x] 完成 Pi/dsh 适配与工具、Key、区域生命周期接入。
- [x] 接入工具行和托盘的模型选择。
- [x] 完成 Profiles 存储、预览、应用、结果报告与撤销。
- [x] 将会话中心接入 Console，增加 Pi/dsh parser 和能力声明。
- [x] 完成首批回收站；新工具分别通过 bundle 验证后开放。
- [x] 对齐最终发布基线。
- [x] 完成本地自动化检查、浏览器验收与 macOS 测试 App。
- [ ] 用户实机验收与正式发布。

前端检查使用 `pnpm typecheck`、`pnpm format:check`、相关 Vitest 和 `pnpm build:renderer`。Rust 改动执行 `cargo test`、`cargo fmt --check`、`cargo clippy -- -D warnings`，均指定 `src-tauri/Cargo.toml`。

首个实施单元验证：类型检查、全 src 格式检查、renderer 构建通过；ManageToolDialog、ConsolePage、model-fetch、manageToolChatgpt 四个测试文件共 82 个测试通过。该阶段修改共用 picker、管理弹窗接入和 zh/en/ja 文案，后端写入与发布文件保持现有状态。

新适配器重点覆盖目录 override、未初始化布局、原配置保留、同名冲突、多文件失败回滚、Key 对账、两个 apex、首次启动、区域切换、重启、macOS/Windows 和实际写入 URL。CLI 与网络实测在适合的隔离环境执行；真实 agent 请求需要具备可用环境。UI 改动完成时补截图；版本更新在后续发布阶段统一处理。

本轮实现已完成：统一模型快照和锁内提交版本、跨窗口刷新、快捷单选与有序多选、Profiles 乐观更新/预览/逐目标应用/部分结果/15 分钟模型撤销、Pi/dsh 独立适配与生命周期、Console 会话中心和安全文件回收站。模型提交凭据在实际 writer 锁内捕获；后续 CLI 选择被采纳后仍会阻止旧撤销。WorkBuddy 模型保存仅使用现有绑定 Key。

会话回收站开放 Claude/Codex/Gemini/Pi/dsh；Gemini 同时支持旧 JSON 与现代 JSONL，并处理主会话、子 Agent 与独立附属文件。共享数据库与索引保持原状。OpenClaw 仅开放浏览/导出；dsh 首版仅元数据。Pi 按当前 JSONL 分支读取、导出和续聊。

macOS 完整 Rust 检查：`cargo test` 共 1705 个测试通过、7 个既有人工/网络测试忽略，`cargo clippy --all-targets -- -D warnings` 通过。包含真实文件+数据库的“Profile 保存→CLI 改模型→诊断采纳→拒绝旧撤销”、删除配置后双向区域切换恢复、原项目目录续聊预览回归。现代 Pi/dsh CLI 与真实 OFox 请求尚需用户验收；当前机器没有 Pi，dsh 是旧布局 0.1.2-rc.1，未用它作为现代适配验证。

前端最终检查：typecheck、全量 format:check、61 文件 / 451 项 Vitest、renderer build 全部通过。浏览器验收修复了 360×620 托盘多选弹层裁切；保存 before/after 和 picker/Profiles/会话/回收站/托盘共 8 张截图，使用真实组件与合成 IPC 数据，临时 fixture 与 server 已移除。

本地已构建 ad-hoc 签名的 debug `Ofox Desktop Test.app`，不创建发行安装包、不上传 release。Linux / Windows 检查由 Draft PR 的现有 CI 与 Windows Probe 工作流运行，结果以 PR 检查状态为准。

用户验收步骤见 [测试说明](../../magpie-tool-experience-testing.md)。正式发布继续等待用户测试结果。
