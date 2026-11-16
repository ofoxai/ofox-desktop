# 调研：结构化页型与主题（路线图“待调研”第 3 项，M3a-1 前）

日期：2026-11-07。目的：校准 M3 设计稿里的页型、每页上限和主题定义。来源都是公开文档或仓库原文，
数字按原文摘录；写“未找到”的就是没有公开资料。设计稿：
`docs/superpowers/specs/2026-11-07-assistant-office-polish-and-edit-design.md`。

## 结论

1. **“模型写内容，代码排版”是行业常态。** Presenton 用 zod schema 定每个版式的字段上限，PPTAgent 用模板版式加字符建议，
   Gamma 5 用固定 16:9 网格版式，Beautiful.ai 的 Smart Slide 随内容自适应。只有 OfficeCLI 自带技能让模型写坐标
   （“no layout engine, you own the grid math”），也正是我们现状的来源。设计稿方向成立。
2. **页型 10 个上下。** Gamma 官方可数的块约 10 种，OfficeCLI morph 10 种，Presenton 通用组 12 种。我们设计稿写“八种”但列了九种
   （cover / section / bullets / cards / kpi / table / chart / image / closing），建议补 `steps`（流程 / 时间线）和
   `compare`（两栏对比）两种，共 11 种，都只是卡片页的变体，渲染成本低。
3. **上限要按页型和几何推，不能一刀切。** 用设计稿自己的公式核算：六条 × 60 个中文字只有 18pt 全宽放得下，分栏和卡片页都放不下。
   Presenton 的做法是每个版式单独定 min/max，Presenton V2 直接由版式几何算出 `max_length` / `max_items`。下面的上限表照此推。
4. **溢出策略与业界一致：校验拒绝 + 重发，渲染后不裁。** Presenton 按 JSON Schema 校验，最多 4 轮重试，提示词写明
   “不要截断，改写”；PPTAgent 让模型按字符上限改写；Gamma 导出时整页缩放，从不拒绝。我们“超限拒绝整个调用 + 质检闭环兜底”在这条线上。
5. **渲染器的估算必须和 officecli 的检查器用同一套常数**，否则质检闭环会反复发现溢出。检查器常数：字宽 = 字号（CJK）或 0.55 × 字号（拉丁），
   行高 = 字号 × 1.2（拉丁）/ 1.32（CJK），内边距左右 0.1 in、上下 0.05 in，越界容差 0.5 cm，低对比度 = 底色亮度 < 30% 且文字亮度 < 80%。
   设计稿的 0.035 cm/字和 0.05 cm/行与之接近，但没算内边距（左右共 0.51 cm），要补上。
6. **主题缺图表配色。** `chart` 页型需要 4–6 个系列色，设计稿的调色板没有；OfficeCLI 技能、Presenton V2（`graph_0…graph_9`）都有。

## 对比

| 来源 | 页型 | 每页上限 | 溢出策略 | 主题定义 | 我们取什么 |
|---|---|---|---|---|---|
| **OfficeCLI 技能**（pptx / pitch-deck / morph-ppt，Apache-2.0） | pptx：5 种版式 + 6 个配方；pitch：6 种版式 + 12 个配方；morph：hero、statement、pillars、evidence、timeline、comparison、grid、quote、cta、showcase 共 10 种；页数 3–5 / 5–8 / 10–15，避免 < 3 或 > 20 | 卡片 ≤ 30 词；要点 ≤ 14 词（8.8 cm 宽）；2×2 恰好 4 项，5+ 改 3×2；图表 > 3 系列或 > 8 类别就拆；饼图 2–5 片；对比表 5–7 行 × 4–6 列；60pt 在 9.78 cm 卡片里约 5 个字符；每页要点条数未写 | 永不裁内容：加高或缩短；放不下就减卡片不减字号；标题永不低于 36pt；`view issues` 必须为空，截图审稿最多 3 轮 | 10 套调色板 × 5 角色（primary / secondary / accent / text / muted），8 组字体对，“最多两种字体”，亮度公式判深底；morph 52 套风格 × 7 角色 + 中英标题 / 正文 / 数据字体 | 画布与网格常数（33.87 × 19.05，边距 1.27–1.5，间距 0.76，留白 ≥ 20%）、字号地板、调色板数值（navy = Midnight Executive，forest = Forest & Moss，warm 候选 Coral Energy / Terracotta）、检查器常数 |
| **Gamma**（Classic API + Gamma 5） | 无官方总表；可数的块：timeline、process、funnel、chart、diagram、column、gallery、comparison table、card（文 + 配图）、table 约 10 种；智能图示 12+ 种另算 | 不公布每卡字数；用 `textOptions.amount` = brief / medium / detailed / extensive 四档 + `numCards`（默认 10，付费 1–100，“软目标”） | Classic 卡片随内容长高（fluid），固定比例时卡片会变成 3 倍大，官方“不支持锁定卡片尺寸”；Gamma 5 改固定 16:9 网格；导出用“缩放适配”整页缩，从不拒绝 | 颜色（背景、按钮 / 链接、填充 / 描边、标题 / 正文色、渐变）、字体（标题 / 正文、行高、字距、基准字号、大写标题）、logo、形状（圆角、描边、阴影、块填充）、配图风格；从 PPTX 导入可抽主题名、主色、背景、字体、logo | 页型数量级（10）；主题字段清单（圆角 / 装饰 / logo 位置）；不学 fluid，固定画布 + 网格是对的；`amount` 四档可作为以后的 deck 级“密度”选项 |
| **Presenton**（开源，zod schema 版式） | 8–13 组共 120–228 个版式，通用组 12 个；LLM 为每页选一个版式序号，规则如“有表格数据必须选表格版式”“相邻两页版式不同” | 精确到字段：带图标要点 ≤ 3 条（标题 2–60，描述 10–100）；纯要点两栏 ≤ 8 条、每条 ≤ 100；卡片 1–6（标题 3–17，描述 18–80）；KPI 大数 2–3（值 ≤ 10，标签 ≤ 50）或紧凑卡 3–6（值 ≤ 5，正文 ≤ 16）；表格 ≤ 6 列 × 6 行，表头 ≤ 18，单元格 ≤ 24；团队 ≤ 4 人；标题 25–40 | 按 JSON Schema 校验，最多 4 轮 × 3 次解析重试，提示“不要截断改写”；组件再兜底 `slice(0,6)`、表头字号随列数降到 13；Smart 模式字号阶梯 48→14，不低于 14px，页数 ≤ 50 | V2：`primary / background / card / stroke / primary_text / background_text / graph_0…9` + 正文字体；上限由 1280×720 几何算出 | 每页型单独 min/max 的做法；具体数字作为我们上限的参照；图表系列色进主题；“相邻两页版式不同”可作技能建议 |
| **PPTAgent**（论文 + 代码） | 4 种结构页（开场、目录、章节、结尾）+ 从模板聚类出的内容版式，用视觉模型命名（如“图片加三个要点”） | 每个元素 `suggested_characters` = 模板原文最长值，可变长度元素 1 个且有 (min, max) | 超长文本让模型按字符上限改写；条数越界直接报错 | 继承模板 PPTX | 结构页 / 内容页二分；超限报错 + 模型改写，与我们一致；PPTEval 三维（内容、设计、连贯）可做回归集的人工评分表 |
| **Beautiful.ai** | 61 个 Smart Slide，8 类（Impact、Structural、Data & Chart、Comparison、Diagrams & Timeline、Essential、Product & Customer、People） | 未找到 | 版式随内容增减自动重排 | 未找到 | 分类名提示我们缺 Comparison 和 Timeline 两类 |
| **Manus / Genspark / Kimi** | 未公开页型 | 未找到 | 未找到 | 未找到 | 只对第 1 项调研有用：Genspark “先出大纲等确认”，Kimi “先生成大纲再做”，Gamma 5 默认 Plan 模式先问目标 / 受众再给计划等确认，Express 跳过 |
| **md2pptx / Marp** | 无页型 | md2pptx ≤ 10 个内容块；Marp 无 | md2pptx 只按元数据缩字号；Marp 只横向缩放标题，正文会溢出底部不裁不缩 | CSS | 反面教材：没有上限的 Markdown 转 PPT 必然溢出 |

## 建议的页型与上限（供 M3a-1 计划）

2026-11-08 用户决定：页型、配色和常数按 AionUi（即 OfficeCLI morph-ppt 技能）取，渲染器照做。最终的页型表、上限和三套主题数值
已写进设计稿“A. 内容模型”和“主题与排版规则”两节，以设计稿为准；下表是决定前的草案，保留作对照。

单位：中文字符数；拉丁字符按两倍计。上限按检查器常数（字宽 0.0353 cm × 字号，行高 1.32 倍，左右内边距共 0.51 cm）核过，
在“渲染要点”列的字号下都放得下，不需要再缩。内容页标题统一 ≤ 22（36pt 一行）。

| 页型 | 字段 | 上限 | 渲染要点 |
|---|---|---|---|
| `cover` | title、subtitle、meta（讲者 / 日期，可选） | 18 / 30 / 30 | 44pt 居中一行（18 字 = 44pt 一行的容量）；logo 右下 |
| `section` | title、subtitle（可选） | 18 / 30 | 深底，40pt |
| `bullets` | title、bullets[]、lead（可选引言） | 3–6 条，每条 40；引言 60 | 全宽 18–24pt；6 条 × 40 字在 20pt 刚好一行（40.7 字容量），渲染器按总行数选档 |
| `cards` | title、cards[{heading, body}] | 2–4 张；heading 12，body 60 | 3 张 16pt、4 张 14pt，body 4–5 行 |
| `kpi` | title、items[{value, label, note?}] | 1–4 个；value 6 字符、label 12、note 30 | 1–2 个 72pt，3 个 60pt，4 个 48pt（6 个拉丁字符在 60pt 宽 7.0 cm，放不进 6.2 cm 的四分卡） |
| `table` | title、columns[]、rows[][] | ≤ 6 列 × ≤ 6 行（含表头）；表头 8，单元格 12 | 16 / 14 / 12pt 按列数选 |
| `chart` | title、chart{kind: bar \| line \| pie, categories[], series[{name, values[]}]}、note（可选） | ≤ 8 类别、≤ 3 系列；饼图 2–6 片；note 60 | 图 20 × 11 cm 左，note 卡右 |
| `image` | title、image（`attachment:<id>` 或 data URI）、caption、body（可选） | caption 30；body 100 | 图左 16 cm，文右 18pt |
| `steps`（新增） | title、steps[{heading, body}] | 3–6 步；heading 10，body 30 | 横向编号卡片，等同 cards 的变体 |
| `compare`（新增） | title、columns[{heading, bullets[]}] | 恰好 2 栏；heading 12；每栏 ≤ 4 条 × 30 | 两栏 18pt |
| `closing` | title、subtitle | 18 / 40 | 深底居中 |
| deck | slides、notes | 3–30 页；每页 notes ≤ 300 | 超过 30 页拒绝并建议拆成两份 |

字号阶梯补一档“卡片正文 / 表格 14–16pt”，设计稿的“正文 18–24”只管 bullets 和 compare；13pt 以下一律不用（morph 与 Presenton 的地板一致）。

## 主题样式表字段（建议）

- 颜色 8 角色：`background`、`surface`（卡片底）、`primary`、`secondary`、`accent`、`text`、`textMuted`、`onDark`（深底文字）；
  加 `chart[]` 4–6 个系列色。偏好套件只覆盖 `primary` / `accent`，`chart[]` 由 primary 推导。
- 字体 4 项：拉丁标题 / 正文、CJK 标题 / 正文（morph 的 title_en / title_cn 做法）；每个形状显式写字体。
- 装饰 3 项：标题样式（下划线 / 色条 / 无）、圆角（0 / 0.3 cm）、页脚（页码、公司名、位置）。
- 数值来源：navy 取 OfficeCLI 的 Midnight Executive（1E2761 / CADCFC / F96167 / 333333 / 8899BB），forest 取 Forest & Moss
  （2C5F2D / 97BC62 / D4A843 / 2D2D2D / 6B8E6B），warm 从 Coral Energy / Terracotta 里挑，保留 Apache-2.0 来源说明。

## 顺带发现（归入其他调研项）

- 第 1 项“大纲先行”：Gamma 5 默认 Plan 模式先问再给计划等确认，Express 模式跳过；Genspark 先出大纲等确认；Kimi 先生成大纲。
  三家都把“先确认”做成默认，并留一个跳过入口。
- 第 5 项“偏好套件”：Gamma 从上传的 PPTX 里抽主题名、主色、背景、字体、logo，是 brand kit 的捷径。

## 来源

- OfficeCLI：`skills/officecli-pptx/SKILL.md`、`skills/officecli-pitch-deck/SKILL.md`、`skills/morph-ppt/reference/pptx-design.md`、
  `src/officecli/Handlers/Pptx/PowerPointHandler.ShapeProperties.cs`、`src/officecli/Core/IssueSubtypes.cs`（github.com/iOfficeAI/OfficeCLI）。
  AionUi 本身没有 pptx 技能，AionCore 内嵌的是 OfficeCLI 同一份（`crates/aionui-app/assets/builtin-skills/`）。
- Gamma：developers.gamma.app（generate、agent generations、limits）、help.gamma.app 11016396 / 11029150 / 15002203 / 15939201、ideas.gamma.app。
- Presenton：`servers/nextjs/app/presentation-templates/*`（v0.6.0–v0.8.9）、`servers/fastapi/utils/llm_calls/generate_presentation_structure.py`、`docs/template-v2.md`。
- PPTAgent：github.com/icip-cas/PPTAgent v0.2.0（`prompts/category_split.txt`、`roles/schema_extractor.yaml`、`induct.py`）、arXiv 2501.03936。
- Beautiful.ai：beautiful.ai/templates；Manus / Genspark / Kimi 官方帮助页；md2pptx user-guide；marp-core docs/markdown.md。
