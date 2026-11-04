# 资源库补资源、助手接上技能和 MCP

## 为什么做

1.5.0 发布后的下一轮，用户提了两件事：

1. **助手继续迭代。** 我们对比了几个 agent：pi、opencode、claw-code、AionUi/AionCore/aionrs，以及 dsh、WorkBuddy 两个桌面 agent。用户的要求是「先做简单有效的」，不要一次做太复杂。
2. **资源库的优质资源太少。** 指令模板只有 3 个，卡片全是同一个灰色文档图标。用户要参考 magpie：补充资源，像 magpie 那样给卡片配上图标，增加趣味性。

调研结论：
- **资源库**：magpie 的 MCP 目录有 27 个，我们只有 8 个。magpie 不用搜索就列出 skills.sh 的热门技能，带安装量和作者头像。magpie 是 MIT 协议，数据可以注明出处后直接用。
- **助手**：现在不加载任何技能，也不连 MCP。资源库里给其他工具装的技能和 MCP，助手自己用不上。dsh 和 WorkBuddy 都支持技能：只给模型看名字和描述，要用时再读全文；WorkBuddy 还能在对话里找技能、装技能、存成技能。

## 目标

**资源库（第一部分）**
- MCP 推荐从 8 个增加到约 26 个，卡片标明「需登录」「需要 API key」。
- 指令模板从 3 个增加到 12 个。
- 技能页一打开就有热门技能，带安装量、官方标记和作者头像。
- 每张卡片都有图标：品牌 logo，或者带颜色的图标或首字母方块。不再出现千篇一律的灰色文档图标。

**助手（第二部分）**
- 三个小改动，让助手更稳：模型知道今天的日期；工具和搜索次数上限放宽；请求还没被处理就失败时自动重试。另外 Pi 升到 1.5.0。
- 技能：资源库里勾给「Ofox 助手」的技能，助手能用。规则：
  - 平时只给模型看名字和描述，用时再读全文。
  - 输入框打 `/` 能手动选技能。
  - 每个技能有三档：自动用 / 只能手动用 / 关。
  - 对话里能找技能、装技能，但要用户点了才装。
  - 能把对话里的做法「存成技能」。
- MCP：资源库里勾给「Ofox 助手」的 MCP 服务，助手能调用。不是只读的调用，要按权限模式审批。

## 不做的（这一轮）

以下已经和用户讨论过，用户要求先做简单有效的，所以这些先不做：
- 运行预算和收尾：按时间、花费上限，快到时先总结再停，加「继续」按钮。
- 重复调用检测、通用的进度清单、带选项的提问卡片。
- 记忆、预设助手、报错旁的「让助手看看」、定时任务。
- 普通对话里直接跑技能带的脚本：要有执行环境、沙盒和自带 Python，放到下一轮和「产出文件」一起做。这一轮只有开了「本机配置」才能跑。
- 需要浏览器登录（OAuth）的 MCP 服务给助手用，比如 Notion、Linear。资源库照样推荐它们给 Claude Code 等工具用，但「Ofox 助手」这个目标先不支持。
- MCP 服务常驻进程。这一轮只量一下要等多久。
- 从 skills.sh 实时抓热门列表。这一轮用 app 里自带的快照，搜索照旧实时查询。
- 用服务商实际计费的字段改花费显示。
- 不自动读 `~/.agents/skills` 或 `~/.claude/skills`。助手的技能统一从资源库来。

## 约束

- **不影响正在发布的 1.5.0。**
  - 工作区：`/Users/yunshen/code/ofox-desktop-1.5.1`，分支：`feat/assistant-1.5.1`，从 `release/1.5.0`（2d7886ac）开出。
  - 不往 `release/1.5.0`、`feat/assistant-media-agent` 推提交，也不改这两个分支。
  - 只在本地提交，用户说「提交」之前不推送、不开 PR。
- **开源边界。** 本仓库是开源的：
  - 只借鉴思路，不照搬闭源产品的代码、提示词或工具说明。
  - 提交、文档、Fizzy 里都不提闭源产品的名字。
  - magpie（MIT，© 2026 yetone）的数据可以用，在 `src/assets/library-logos/NOTICE.md` 和新数据文件的头部注明出处。
  - AionCore 里的 PDF 技能版权归 Anthropic，不能拿。
- **图标不在运行时联网获取。** 沿用 `src/assets/library-logos/NOTICE.md` 的规矩，图标都打包进 app。
- **界面文字三种语言。** 中、英、日都要有，放进 `src/i18n/locales/{zh,en,ja}.json`，不能写死在代码里。
- **Ofox 地址按区域生成。** 新加的功能如果用到 Ofox 的网址，用 `src/lib/ofoxUrls.ts` + `useOfoxApex`，不写死 `.ai` 或 `.io`。
- **Windows 和 macOS 行为一致。** 路径用平台 API 拼接。npx 在 Windows 上是 `npx.cmd`。
- **项目规范。**
  - 用 pnpm；提交信息简洁，用 Conventional Commits 风格，不带生成信息。
  - 每个提交都要能编译、测试全过。
  - Rust 改动要过 `cargo fmt --check` 和 `cargo clippy -- -D warnings`。
- **磁盘。** 开工前和每次 Rust 全量编译前查一下 `df -h /System/Volumes/Data`。可用空间低于 20 GB 时，可以删除不再用的编译产物，范围只限：
  - `/Users/yunshen/code/ofox-desktop-assistant-media/src-tauri/target`
  - `/Users/yunshen/code/ofox-desktop/src-tauri/target`
  - 本工作区自己的 `target/debug/incremental`

  别的都不删。

## 第一部分：资源库

### A1. MCP 推荐补到约 26 个

- 从 magpie 的 `internal/library/market.go`（`featured` 列表和 `homepages`）搬这 18 个：exa、memory、sequential-thinking、git、time、sentry、linear、notion、atlassian、supabase、stripe、vercel、neon、cloudflare-docs、huggingface、brave-search、firecrawl、tavily。
- 搬之前逐个确认还能用：
  - npx 包：`npm view` 能查到；
  - uvx 包：PyPI 上有；
  - 远程地址：能连上，返回的不是 404 或 DNS 错误。

  不能用的就不加，在提交信息里说明。
- 每个服务按我们的数据结构写，放在 `src/components/library/libraryRecommendations.ts`：
  - 填好分类、发布方、许可证、`initialDraft`（启动命令或地址）和描述；
  - 描述的中文、英文用 magpie 的，日文自己翻；
  - magpie 里要填的 API key、请求头这类，放进 `initialDraft` 的 `environment` / `headers`，名字填好、值留空。
- 分类：沿用现有 6 类，新加一类 `search`（「搜索」），放 exa、brave-search、firecrawl、tavily。
- 卡片上的标记：
  - 需要浏览器登录的显示「需登录」，就是 magpie 里 `SignIn` 为真的 8 个；
  - 要填 key 或 token 的显示「需要 API key」。

  标记放在卡片上，不只藏在详情里。

### A2. 指令模板补到 12 个

现在有 3 个：通用编程、中文沟通、代码审查。新加 9 个：

| id | 名称 | 内容要点 |
|---|---|---|
| tdd | 测试先行 | 先写失败的测试，再写最少的代码让它通过，然后整理 |
| debugging | 先找根因 | 先复现、读报错、查最近改动，找到原因再修；同一问题失败三次就停下来换思路 |
| git-commits | 提交与 PR 规范 | 小步提交，Conventional Commits，提交信息说明为什么改，PR 写清改了什么、怎么验证的 |
| frontend | 前端界面 | 沿用项目已有的组件和样式，照顾可访问性、暗色模式和窄屏，文字走 i18n |
| docs-writing | 文档写作 | 先写结论，短句，示例能直接运行，不堆术语 |
| security | 安全检查 | 不提交密钥，用户输入要校验，注意注入和越权，依赖要固定版本 |
| refactor | 小步重构 | 行为不变，每一步都能跑测试，一次只改一件事 |
| plan-first | 先规划再动手 | 复杂任务先列步骤和验收标准，确认后再写代码 |
| concise | 简洁回答 | 先给答案，少铺垫，不重复用户的话 |

- 每个模板都写中、英、日三份正文，每份 10 到 25 行。语气和现有的 3 个一致，写给 AI 编程工具看。
- 分类沿用现有的 `category`。

### A3. 每张卡片都有图标

- **MCP 和推荐技能**：用品牌 logo，按下面的顺序找：
  1. `@lobehub/icons-static-svg` 里有的，直接用。这个包已经是项目依赖。要先确认文件确实存在，比如 `github.svg`、`vercel.svg`、`notion.svg`、`huggingface.svg`、`cloudflare.svg`。
  2. 没有的，开发时从 `https://github.com/<org>.png?size=96` 下载一次，存成 `src/assets/library-logos/<id>.png`，在 `NOTICE.md` 里记下来源。
- **热门技能**：用作者头像。只打包快照里出现的作者，大约 20 来个，同样开发时下载一次。
- **指令模板**：每个模板配一个 lucide 图标，放在带颜色的方块里，颜色按模板固定。例如：中文沟通用 `Languages`，代码审查用 `ScanSearch`，安全检查用 `ShieldCheck`，测试先行用 `FlaskConical`。颜色用项目已有的 Tailwind 色值，亮色和暗色模式下都要看得清。
- **都没有时**：显示名字首字母的方块，颜色由名字算出、固定不变。用在搜索结果、用户自己加的资源这类场合。这个规则写进 `LibraryResourceIcon.tsx`，所有卡片共用。
- **已安装的资源**：后端 `src-tauri/src/services/library/display.rs` 按推荐 id 匹配图标，要把新加的 id 也匹配上。

### A4. 技能页一打开就有热门技能

- 从 magpie 的 `internal/library/market_skills.json` 生成一份快照：每个仓库最多 4 个，按安装量取前 60 个，放进 `src/components/library/` 下的一个 JSON 文件。
  - 字段：来源仓库、技能 id、名称、安装量、是否官方、英文描述。
  - 描述在生成快照时从 `https://skills.sh/<owner>/<repo>/<skill>` 页面的 meta description 一次性抓好存进去，运行时不去抓。
  - 生成快照的脚本放在 `scripts/` 下，以后可以重新生成。
- 技能页的「发现」默认就显示热门：卡片网格，参考 magpie 的样式。每张卡片有作者头像、名称、官方标记 ✓、来源仓库、安装量（显示成「1.2M」这种短格式），以及「添加」按钮。点「添加」走现有的 skills.sh 安装流程。
- 搜索照旧实时查 skills.sh，结果也显示安装量和图标。头像没打包的作者，显示首字母方块。

## 第二部分：助手

### B1. 三个小改动 + Pi 升级

- **日期**：系统提示里加一行，写明今天的日期和星期、所在时区（IANA 名称）、界面语言。日期和时区由运行时在本机算；界面语言由主程序传进来。位置在 `assistant-runtime/src/runtime.mjs` 的 `systemPrompt()`。
- **上限**（「本机配置」任务自己的上限不变）：
  - 普通对话每次回答最多调用工具的次数：20 → 40（`runtime.mjs` 的 `MAX_TOOL_CALLS`）；
  - 搜索次数：3 → 6（`search.mjs` 的 `MAX_SEARCHES`）。
- **自动重试**：只对对话模型的请求生效。
  - 什么时候重试：还没收到响应的任何内容就失败了，并且属于以下三种之一：连接没建立、请求没发出去，或者服务端返回 429 / 502 / 503。
  - 怎么重试：最多 2 次，间隔 1 秒、3 秒。遇到 429 时如果有 `Retry-After`，按它等，但最多等 10 秒。界面显示「重试中」。
  - 什么时候不重试：已经收到部分内容，或者可能已经被处理了（超时、其他 5xx）。这时照旧显示失败，让用户手动重试。
  - 生成图片、视频、语音的重试规则不变。
- **Pi 升级**：
  - `@earendil-works/pi-ai`、`pi-coding-agent`、`pi-mcp` 从 1.4.0 升到 1.5.0。`pi-codemode` 如果在用，也一起升。
  - `undici` 的 override 提到 Pi 1.5.0 要求的版本。
  - 升级后 `pnpm assistant:prepare`、`pnpm assistant:test` 要全过。1.5.0 对工具参数的校验更严，要确认我们所有工具的参数还能通过校验。

### B2. 技能

**从哪来**
- 资源库技能的分发目标里加一个「Ofox 助手」，id 为 `ofox-assistant`。
  - 技能文件夹：`~/.ofox-desktop/assistant/skills/<name>/`。整台电脑共用，不分账户。
  - 这个目标永远显示为「已安装」。
- 资源库原本就会扫描 12 个工具的技能，所以已有的技能也能在资源库里一键勾给助手。

**三档开关**
- 在资源库技能的「Ofox 助手」目标上选：自动用 / 只能手动用。取消勾选这个目标就是「关」。
- 档位存在 `~/.ofox-desktop/assistant/` 下，不改 SKILL.md。

**模型怎么用**（不用 Pi 自带的技能加载：Pi 要靠 `read` 工具读技能全文，普通对话里没有 `read`）
- 主程序每次请求把技能列表传给运行时：名称、描述、文件夹路径、档位。
- 系统提示里列出「自动用」的技能：
  - 只有名称和描述，描述最多 200 字；
  - 整个列表最多 8000 字，超出的部分不列，并注明还有几个没列。
- 新增工具 `use_skill({name})`：
  - 返回技能全文，最多 60000 字，超出的截断并注明；
  - 同时返回技能文件夹里的文件清单。
- 新增工具 `read_skill_file({name, path})`：只读技能文件夹内的文本文件，最多 60000 字，不能读到文件夹外面。
- 技能文件夹里有脚本时（`*.py`、`*.sh`、`*.js`、`*.ts`，或 `scripts/` 目录）：
  - 没开本机配置：`use_skill` 的结果里注明「运行脚本需要本机配置」，模型照现在的做法给出 `[[enable_local_config]]` 按钮。
  - 开了本机配置：用现有的命令工具跑，审批照旧。要确认本机配置的权限规则允许读取和执行这个技能文件夹，但不允许写。

**手动选技能**
- 输入框里，消息开头打 `/` 会弹出技能列表（自动用和只能手动用的都列），选中后在输入框里显示成一个「技能：名称」标签。
- 发送时，主程序把这个技能的全文附在这条用户消息前面，作为一段标明来源的技能内容。

**对话里找技能、装技能**
- 新增工具 `find_skills({query})`：在资源库推荐目录和 skills.sh 搜索里找，最多返回 8 个结果。结果同时交给模型，并在对话里显示成卡片，卡片上有「安装」按钮。
- 只有用户点「安装」才会装：
  - 走资源库现有的安装流程；
  - 装完自动勾上「Ofox 助手」，档位为「自动用」。
- 主程序要校验：只能安装这次对话最近一次 `find_skills` 结果里的技能。模型自己没法安装。

**存成技能**
- 新增工具 `save_skill({name, description, body})`：在对话里显示确认卡片，可以预览名称、描述和正文，用户点「保存」才写入。
- 写入资源库，来源标为「助手创建」，自动勾上「Ofox 助手」，档位为「自动用」。
- 同名的技能，只有来源是「助手创建」的才能被 `save_skill` 覆盖，其他的一律拒绝。

**来源**
- 不在我们推荐目录里的技能，在资源库和输入框的技能列表里标「未审核」。
- 资源库如果已经有更新技能的功能：技能文件在安装后被改过的，更新前先问用户。

### B3. MCP

**从哪来**
- 资源库 MCP 的分发目标里加「Ofox 助手」，id 为 `ofox-assistant`。
- 配置写进 `~/.ofox-desktop/assistant/mcp.json`，格式是 Pi 能读的 `mcpServers`。
- 写入和移除沿用资源库给其他工具写 MCP 配置的做法（`src-tauri/src/services/library/mcp_targets.rs`）。
- 需要浏览器登录的服务，这个目标不能勾，并提示「助手暂不支持需要浏览器登录的服务」。

**运行时**
- 有 MCP 服务时，运行时把配置写进本次会话的 `runtime-settings/mcp.json`。所有服务都设成 `exposure: "deferred"`：模型平时只看到服务名和一句说明，要用时再加载工具。
- 打开 Pi 内置的 `mcp` 和 `tool_search` 扩展，`codemode` 保持关闭。
- 现有的搜索服务不受影响。

**权限**
- 默认模式和自动编辑模式：服务标明 `readOnlyHint` 的工具直接跑，其他工具走现有的审批卡片（本次 / 本会话 / 拒绝）。
- 全自动模式：直接跑。
- MCP 调用计入每次回答的工具次数上限。

**npx 和 uvx**
- 启动命令是 `npx` 时，用助手自带的 Node 里的 npx 去跑。Windows 上是 `npx.cmd`。要先确认自带的 Node 里有 npm 和 npx。没装 Node 的电脑也要能跑。
- 启动命令是 `uvx`、但电脑上没有 uv 时，这个服务显示「需要安装 uv」，并告诉模型。

**显示**
- 回答里每次 MCP 调用显示成一步，带服务的图标，用 A3 的图标规则。

**要量的**
- 一条消息里第一次调用 npx 启动的服务，等了多久：冷启动和第二次各量一次。
- 结果写进最终报告。这一轮不做常驻。

## 留给实现者决定

- 组件拆分、文件命名、数据文件放 TS 还是 JSON。
- 卡片网格的细节间距，按现有资源库的样式来。
- 首字母方块的配色算法。
- 「技能：名称」标签和技能卡片的具体样式。
- 测试的组织方式：沿用项目现有的测试位置和工具。

## 验收

- `pnpm typecheck`、`pnpm format:check`、`pnpm test:unit` 全过。
- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全过。
- `pnpm assistant:test` 全过。
- 用浏览器加载真实样式检查资源库三页：窄窗口和宽窗口、亮色和暗色。所有卡片都有图标，没有破图。
- 助手：用假的模型服务跑通以下流程，再由用户在本机用真实 key 试：
  - 技能：自动使用、`/` 手动选、找技能并点击安装、存成技能；
  - MCP：只读调用直接跑，写调用要审批。
