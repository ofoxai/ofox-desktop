# 助手待办清单（按优先级）

记录日期：2026-11-07，同日按设计稿 M3–M5 和参考调研补齐缺口；2026-11-09 增补第 1a 项（用 Python 写 PPT）。基线：分支
`feat/assistant-deliverables`（M1/M2 已完成：PDF/PPTX/DOCX/XLSX 可稳定生成，引擎随包分发并签名，
批处理沙箱与公式白名单）。设计稿：`docs/superpowers/specs/2026-11-07-assistant-deliverables-and-media-design.md`。
产品定位：助手直接产出**一份**用户要的、精美的 Office 文件，首轮即可用，并且能继续改。

已定决策：
- 2026-11-07：M3 第一版就能改文件，包括用户上传的文件，不只是一次性生成。
- 2026-11-07：M3 拆成小切片，顺序 M3a-1 → M3a-2 → M3b → M3a-3 → M3c，每段单独计划、单独验收，
  验收过了再开下一段；最小真实模型回归脚本随第一段交付，之后每段都用它做门槛。
- 2026-11-07：内容超限拒绝整个调用，渲染器不截断、不改写用户文字。
- 2026-11-07：实现方式先调研其他产品再讨论（见文末“待调研”），调研项没定之前不进切片计划。
- 子 agent、钩子与定时任务、通用记忆、通用模型路由与“轻”的定位冲突，现阶段无用户场景，
  列入文末“暂缓”，出现触发条件时再议。

状态：`todo` 未开始 / `doing` 进行中 / `done` 完成 / `deferred` 暂缓（不排期，触发条件出现再议）。

## P0 主线：产出效果与可改

### 1. Office 产出精美化并可修改（M3；先 PPT，再 Word，最后 Excel） — todo

现状：版式由模型按坐标写，效果随模型波动；officecli 只能查溢出和出界，查到后回传模型重做，
一次请求出两份；生成后无法只改一页，用户上传的文件完全不能改。
设计稿：`docs/superpowers/specs/2026-11-07-assistant-office-polish-and-edit-design.md`。

切片（2026-11-07 定；每段开工时单独写实施计划，验收通过再开下一段）：

| 顺序 | 切片 | 交付 | 状态 |
|---|---|---|---|
| 1 | M3a-1 生成骨架 | deck 内容模型 + `navy` 主题 + 渲染器 + 质检闭环 + 全页 SVG + 技能页型表 + 最小真实回归脚本（4 提示 × 3 模型） | done（2026-11-08 代码与假模型 e2e 完成；待用户真机验收与 `pnpm assistant:real-eval`） |
| 2 | M3a-2 一次只出一份 | `revises` 卡片替换 + `content.json` + 大纲规则（待调研） | todo |
| 3 | M3b 修改 | `inspect_document` + `edit_document`（docx / pptx / xlsx）+ 源字节不变 | todo |
| 4 | M3a-3 像用户的 | `forest` / `warm` 主题 + 偏好套件 + PDF 主题 + `attachment:<id>` 图片 | todo |
| 5 | M3c 度量 | 回归脚本补修改用例 + token/时间统计 + 验收表，18 次最多失败 1 次 | todo |
| 6 | M3d 后半段 | Word / Excel 渲染器与主题、基于内容重做精美版 | todo，另立计划 |

2026-11-08：M3a-1 的运行时在 `feat/assistant-deliverables`，宿主部分已合并到 `feat/assistant-deliverables`；
待用户验收与真实模型回归（`pnpm assistant:real-eval`，结果填进
`docs/assistant-engines.md`）；验收通过后再改上表状态。

2026-11-09：M3a-1 的 deck 对象真机验收认为质量太差，用户决定统一改用 Python 写文档（macOS 先行），
见下面第 1a 项；M3d 的 Word / Excel 也按这个决定在第 1a 项之后的切片里做，不另写 JS 渲染器。

下面的“生成 / 修改 / 验收”是整段 M3 的总目标，不是单个切片的范围。

生成：
- 结构化中间格式：十种页型取自 OfficeCLI morph-ppt（hero / statement / pillars / evidence / timeline / comparison / grid / quote / cta / showcase），
  模型只写内容，没有 bullets 页型（2026-11-08 定，按 AionUi 的做法）。
- 两到三套设计主题，用母版、版式和占位符承载字体、配色、间距；代码把中间格式渲染成 officecli 批处理。
- 生成前先给大纲（页数、每页标题、要点）让用户确认一次；用户说“直接出”就跳过。
- 偏好套件：公司名、品牌色、logo、常用主题、语言，存在用户设置里，生成时自动套用。
- 出卡片前自动质检：渲染全部页面，溢出、出界按建议直接修正，不再绕回模型。
- 一次请求只出一份文件；模型直接写原始批处理保留为专家模式和兜底。
- 生成的图片可以直接嵌进文档（图片工具 → 文档工具）；PDF 支持内嵌图片（设计稿 M3）。

修改：
- 助手生成的文件保留其内容模型（`content.json`），“把第 3 页改成…”重发整份 deck 并整份重渲染；
  渲染器确定性，没改的页字节一致，用户感知为只改了那一页（设计稿决定 3）。
- 用户上传的 docx / pptx / xlsx：`inspect_document` 读结构，`edit_document` 用 officecli 的
  `set` / `add` / `remove` 产出新文件，源文件字节不变（设计稿 M3 验收第四条）。
- 修改后同样过质检，并替换卡片而不是新增一张。

验收：
- 四个验收提示（PDF 四川、8 页 Transformer PPT、三科成绩表、Word 周报）在 terra、Luna、
  DeepSeek Flash 上各跑一次，一次出一份、零警告、三个模型观感一致。
- “把第 2 页标题改成 X”只改一页；上传一份 docx 改标题，原文件不变。
- Word：样式模板（标题、正文、表格、页眉页脚）；Excel：表格样式、数字格式、图表样式；PDF 只补主题。

### 1a. P1：用 Python 写 PPT（macOS 先行） — done（2026-11-09 代码与假模型 e2e 完成；待用户验收）

设计稿：`docs/superpowers/specs/2026-11-09-assistant-python-documents-design.md`，实施计划：
`docs/superpowers/plans/2026-11-09-assistant-python-decks.md`，调研：`docs/superpowers/research/2026-11-08-ppt-quality-gap.md`、
`2026-11-08-python-runtime.md`。这里的 P1 是这一轮在设计稿里的名字，与下面“P1：图像与视频”的优先级分组无关。

起因：deck 对象能稳定出文件，但每页一排小卡片、平均 90 字、没有图，版式单调；对照样例是强模型自由写 python-pptx、
用 matplotlib 画图的产物。

已交付（macOS）：
- Python 引擎：Pyodide 314.0.7，随包的 wheel（numpy、matplotlib 等、lxml、Pillow、python-pptx、python-docx、openpyxl、
  XlsxWriter）和思源黑体，由 `scripts/python-downloads.json` 锁定版本与 SHA-256，`scripts/assistant-prepare.mjs` 只在 macOS
  放进包、其他平台剪掉，声明写在 `assistant-runtime/notices/python.txt`。
- 运行与辅助：`assistant-runtime/src/python/runner.mjs`、辅助库 `deck.py`、16:9 模板 `template.pptx`（由
  `scripts/assistant-deck-template.mjs` 生成）。
- 宿主：Python 在独立子进程里、用自己的 `sandbox-exec` 配置、在文档目录下自己的子目录里运行（无网络、不能启动其他程序、
  只写自己的目录、不读用户的其他文件和临时目录）；Python 与 officecli 两个沙箱探测都没失败时提供（探测随第一份需要它的
  文档运行），某次调用发现沙箱缺失就拒绝该调用，Python 自己的探测失败一次就到重启前都不再提供，失败即关闭；deck 作为
  普通文件拷出，由 officecli 校验、查问题、修溢出、出每页 SVG；Python 运行串行。
- 预览：`preview: true` 只出总览图（最多 24 页）和检查结果，不出附件；之后带新内容或 `fromPreview` 交付；每个文件名每次回复
  最多预览 2 次；回复结束时最近一次预览自动交付（用户停止的回复不交付）。工具说明只在宿主提供 Python 时提 python 和预览。
- 技能：`ofox-office-pptx` 先教 Python 路线；deck 对象作兜底（Windows / Linux、沙箱探测失败时）。
- 回归：`pnpm assistant:real-eval` 的报告多 `route`（python / deck）和 `previews` 两列，pptx 用例要求最终只有一份附件、
  零警告。

待用户验收：`pnpm assistant:real-eval`（四个提示 × terra / Luna / DeepSeek Flash，结果填进 `docs/assistant-engines.md`）；
真机上做“讲解 Transformer”，与对照样例并排看。通过后改本项为“验收通过”，再开下面的切片。

后续切片（各自单独立设计、单独验收）：

| 顺序 | 切片 | 交付 | 状态 |
|---|---|---|---|
| P1-2 | Word / Excel 用 Python | 复用 P1 的引擎、隔离与预览；docx / xlsx 辅助库与模板；再往后改用户上传的文件也走它（M3b） | todo |
| P1-3 | Windows 沙箱 | Windows 上与 macOS `sandbox-exec` 同等的系统级隔离；做完才能在 Windows 开 Python 引擎，到那时 Windows 才不再只走 deck 对象 | todo |
| P1-4 | officecli / Typst 沙箱加固 | P1 评审时记下：两个引擎的 macOS 沙箱配置比 Python 的松（Typst 只禁了网络）；把 Python 那份的收紧移植过来，并加测试 | todo |

### 2. 在线预览，只生成一份 — todo

参考 ChatGPT 画布和 Codex 右侧预览：生成的文档在右侧面板直接翻页查看，不必先保存再打开。

- PPT 每页 SVG（宿主已产出全部页面的 SVG（`<id>.pages/`），预览面板只需读取）；PDF 用现有 pdf.js 渲染；
  Word、Excel 用 officecli `view html` 快照转图，或严格隔离后展示。
- 安全约束沿用：app 本身永不加载文档文件（媒体协议拒绝提供文档），预览只用宿主在沙箱里渲染出的图片或 SVG。
- 被新版本替代的旧卡片折叠显示为“已重做”；与第 1 项配合后同一请求只留一份。
- 预览面板内提供“保存并打开”“保存…”，与卡片一致；zh / en / ja 文案。

### 3. 真实模型回归集与首轮成功率 — todo

现状：所有自动测试都走假网关；2026-11-07 的字面 `\n` bug 正是假模型测不出的那类问题。
2026-11-07 定：最小版（4 个生成提示 × 3 模型，断言一份、零警告、无字面 `\n`）随 M3a-1 交付，
之后每个切片都用它做门槛；修改用例、token 统计和磁带回放留到 M3c。
参考：pi 的 evals、deepseek-harness 的真实 API e2e、opencode / aionrs 的流量录制回放。

- 可选脚本，用钥匙串里的 key（脚本自己读，不进上下文），跑四个验收提示 × 三个模型。
- 自动检查：文件有效、零警告、只出一份、文本里无字面 `\n`、中文字体正常（抽样渲染）。
- 统计首轮成功率、重做次数、每份文件的 token 成本，作为第 1 项的进度指标。
- 把真实流量录成磁带，让 CI 不花钱也能回放一遍。

## P1：图像与视频（设计稿 M4）

### 4. 图像：模型看得见结果、提示词策略、多参考图、GPT Image 2.6 — todo

现状：对话里的图片生成只能选模型和张数；参考图上限 1；模型看不到自己生成的图。
网关侧进展见 Fizzy 卡片 https://app.fizzy.do/6179973/cards/1468（2026-11-06：除 markup
待复核外参数已透传，SVG 已支持且额外计费）。

- B1 缩略图回传：支持视觉的模型在工具结果里看到长边 768px 的缩略图，自查再改。
- B2 提示词策略：保主体、补技术描述、缺关键参数问一句；composer 开了生成开关就不问。
- B3 参考图上限按模型族放到 4；费用按张数折算。
- B4 接上“用作输入”和“重新生成换参考图”。
- 模型：`openai/gpt-image-2.6-flare`、`openai/gpt-image-2.6-sunburst`，按发布时间排序展示。
- 参数：尺寸/比例、quality、background、output_format（png / jpeg / webp / gif / svg）、complexity；
  GIF 的 `frame_count` 2–16（默认 9）和 `frame_duration_ms`，最小分辨率要提示；
  SVG 会额外调用文本模型产生两份费用，成本确认里单独列出。
- 改图：`/v1/images/edits` 支持参考图和 markup 标注图；对话里支持拖入参考图。
- 验收：与 ofox.design 网页版同一提示词、同一参数出图一致；成本显示与用量页一致。

### 5. 视频：首帧流程与随包媒体技能 — todo

现状（2026-11-06 用户反馈）：图像、视频生成质量差，主要是提示词和参数选择问题。

- B5 首帧流程：先出首帧（可选尾帧）确认，再用 `material_use: "frames"` 生成视频；费用确认沿用。
- B6 随包媒体技能：`ofox-media-core` 加六个场景技能（广告片、产品视频、短剧、产品图、改图、首尾帧动画），
  改编自 ofoxai/skills（MIT），CLI 流程换成工具调用。
- 先定质量标准：五个场景各留一组参考样片，作为验收基线。
- 参数：首尾帧、参考图、比例、时长、分辨率；生成中进度与失败原因可见，失败不重复扣费。

## P2：agent 侧加固与扩展

### 6. FullAuto 模式的围栏 — todo

只影响 FullAuto；Default 和 AutoEdit 下现有“审批 + 路径范围 + 加密日志还原”已够用。
参考实现：deepseek-harness `packages/sandbox`（macOS Seatbelt、Linux bwrap + Landlock、Windows 受限令牌）。

- bash / PowerShell 的 OS 级沙箱（macOS 可复用文档引擎的 `sandbox-exec` 方案）。
- 危险命令预检（`rm -rf`、`sudo`、`chmod 777` 等）直接转为询问，作为体验层而非安全层。
- 不持有 sudo；需要提权的动作走系统原生提权框，逐个确认。
- 无人值守或定时运行时，所有询问一律视为拒绝（deepseek 的 `never` 策略）。
- 技能脚本的独立沙盒与自带 Python（1.5.1 和本轮都推迟；文档引擎的 Python 见第 1a 项，只跑 pptx 代码）。

### 7. MCP：OAuth 登录与常驻服务 — todo

2026-11-06 的第三个目标，设计稿本轮不做。pi 原生支持 OAuth 和会话级常驻连接，我们当前关闭
（`mcp.mjs` 报“MCP sign-in unsupported”，每次运行都重启服务）。

### 8. 工具结果纯文本审计 — todo

凡是模型会照抄的内容（技能正文、参考文件、文档结构）一律以纯文本返回，不包在 JSON 里。
`use_skill` 已改（2026-11-07），其余工具逐个核对并加测试。

### 9. 文档任务的模型路由 — todo

生成文件时自动选一个强模型，不跟随聊天模型；用户明确指定时以用户为准。失败时无备用模型，暂不做。

## 发布门槛（设计稿 M5，不随功能排期，发版前必须完成）

- Windows：文档引擎运行器的 Windows 分支从未编译，需先编译通过，再手动验收四种格式和中文字体。
- 公证：`APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` 由发布负责人提供，公证后从安装包里实跑两个引擎。
- Python 引擎（macOS）：公证后从安装包里实跑一次 Python 路线（预览 + 交付）；`python.txt` 补上 `pyodide.asm.wasm`
  内含的 C 库（libffi、expat、libmpdec、bzip2 / xz）的声明；在较老的 macOS 上确认 SVG 图里的中文不退成方块
  （沙箱不让读每用户的字体缓存，未验证）。
- 网关并行工具调用 bug（`delta.tool_calls[].index` 不递增）重新探测；仍在就按设计稿只对
  chat-completions 请求加 `parallel_tool_calls: false`，并写明移除条件。
- `docs/assistant-engines.md` 的真实 key 验收表填写；新增文案的 zh / en / ja 齐全。
- DMG 打包在 2026-11-07 本机构建中失败（`bundle_dmg.sh`，疑与磁盘空间有关），需在干净环境复现。

## M1/M2 遗留（随手做）

- `documents/create.rs` 拆分；`'x|y'!A1` 带引号工作表名的 `|`；`/Sheet1/cf[1]` 规范路径；
  `Book.xml!A1` 联合成员；PDF 页数伪造；崩溃残留输出清扫；一条偶发不稳定的运行时测试。
- `assistant-prepare.mjs` 的“是否最新”检查不认签名变化：签名构建后 `pnpm assistant:test` 会失败，
  目前靠 `rm -rf assistant-runtime/resources/node && pnpm assistant:prepare` 恢复。

## 待调研（动手前先看其他产品怎么做，调研完再讨论实现）

2026-11-07 用户要求：先建任务，实现方式等调研后再定。每项调研产出一页对比（谁、怎么做、我们取什么），
放进对应切片的实施计划。

1. **大纲先行要不要做、怎么做**（M3a-2 前）：ChatGPT、Gamma、Manus、Genspark 这类产品生成 PPT 前是否让用户
   确认大纲、阈值是什么、确认交互长什么样（对话里一问、还是可编辑的大纲卡片）。设计稿决定 6 暂挂。
2. **一次只出一份的版本显示**（M3a-2 前）：ChatGPT 画布、Codex 预览、Manus 的文件版本是怎么折叠旧版、
   展示历史版本的；我们“旧卡片折叠成一行”是否够用。
3. **结构化页型与主题**（M3a-1 前）— done 2026-11-08：调研见 `docs/superpowers/research/2026-11-07-slide-types-and-themes.md`；
   用户决定按 AionUi（OfficeCLI morph-ppt）的页型、配色和常数做，结果已写进设计稿“A. 内容模型”和“主题与排版规则”。
4. **改用户上传文件的交互**（M3b 前）：deepseek-harness、AionUi 怎么让模型“先 inspect 再改、只改要求的”，
   改完怎么向用户展示改了哪里。
5. **偏好套件**（M3a-3 前）：Gamma / Canva 的 brand kit 收哪些字段、在哪里设置、怎么套用到模板。

## 暂缓（deferred）

参考调研里其他 agent 有而我们没有的能力。现阶段与“轻量助手”的定位冲突，也没有用户场景，不排期；
写明触发条件，出现了再议。

- **子 agent 与并行工具调用** — deferred。aionrs、deepseek-harness 都有并行子 agent；我们每个工具
  顺序执行，一个 sidecar 一次只跑一个运行。触发条件：出现单次请求要同时产出多份文件或多路媒体的场景，
  或网关并行调用 bug 修复后需要恢复 pi 默认的并行工具调用。
- **钩子与定时任务** — deferred。AionUi 有会话绑定的 cron，aionrs 有工具前后 shell 钩子。
  触发条件：用户要求“每周自动生成周报”这类无人值守场景；届时必须先做第 6 项里的“无人值守一律拒绝”。
- **通用记忆系统** — deferred。aionrs 有分类型的项目记忆自动加载；我们只有 4000 字的全局指令。
  第 1 项的“偏好套件”（品牌色、公司名、常用主题）已覆盖产出所需，通用记忆等有跨会话上下文需求再议。
- **通用模型路由与备用模型** — deferred。pi 的虚拟模型可按任务和成本选模型。我们只做第 9 项的
  “文档任务选强模型”；按成本自动降级、失败切换备用模型，等有稳定的多模型供给和成本压力再议。

