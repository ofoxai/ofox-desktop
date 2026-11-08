# 调研：PPT 质量差距与优化路线（M3a-1 真机验收后）

日期：2026-11-08。起因：M3a-1 能稳定生成，但用户评价“质量太差”。本稿对照一份同题的对照样例，
汇总参考 Agent 与热门 skill 的做法，给出适合 Ofox Desktop 的路线。设计稿：
`docs/superpowers/specs/2026-11-07-assistant-office-polish-and-edit-design.md`；
前一份调研：`2026-11-07-slide-types-and-themes.md`。

## 结论

1. **差距不在字号，在每页的信息量、构图和图。** 对照样例正文只有 11.5–12pt，比我们的 16–20pt 还小，
   但每页约 334 字、2–4 个信息区、8 页里有 3 张为内容画的示意图；我们每页约 90 字、一个信息区、没有图。
2. **固定页型是稀疏的根因。** 我们的十种页型每页只有“标题 + 一块内容”，上限又把内容压得很薄
   （网格、时间线每卡 ≤ 30 字，三栏 ≤ 60 字，没有副标题字段，没有画图的途径）。居中和最小卡高只能让空白分布得均匀些。
   一个按模板填空的同类引擎做出来的页面和我们一样空；对照样例是强模型自由写版面代码的产物。
3. **业界的做法收敛在五件事上：** 每页论点句、按内容组合多个信息区、每页一个承载信息的视觉、版式节奏（不连续重复）、
   确定性检查 + 渲染图自查。这些都能在“模型写内容、代码排版”的架构里做，不必退回让模型写坐标。
4. **推荐路线：保留 deck 对象，把“页型”换成“页头 + 版式网格 + 组件”，加画图的一等路径（原生流程图、原生图表、
   模型写的 SVG 示意图由运行时栅格化），再加节奏 lint 与缩略图自查。** 全部用已打包的 Node 运行时、officecli 和
   @napi-rs/canvas，不需要 Python 和浏览器。

## 对照：同题“讲解 Transformer”

| 项 | 对照样例（8 页） | 我们（21 / 29 页，gpt-6.1-terra） |
|---|---|---|
| 平均每页字数 | 约 334 | 约 90 |
| 正文字号 | 11.5–13pt，标题 25pt | 16–20pt，标题 36–44pt |
| 每页信息区 | 2–4 个：论点副标题、主视觉、解释卡片、结论条 | 1 个：一排卡片 |
| 图 | 3 张示意图（架构流程、注意力弧线、位置编码热力图）+ 2 组原生形状图（多头串联、时间轴） | 无 |
| 构图 | 8 页各不相同 | 四五种页型轮流出现 |
| 强调 | 一个暖色只高亮“最该看的那张卡” | 无 |
| 每页形状数 | 10–38 | 5–15 |

对照样例也有缺点，不能照抄：没有标题占位符（大纲视图里每页无标题）、页脚和页码逐页手打、
字体只设了 macOS 专有的 PingFang SC、项目符号是手打的“·”、示意图是位图（用户改不了）、
图里有标签压线。

## 常见的优化方式

| 方式 | 谁在用 | 对质量的作用 | 适配我们（Node + officecli，无 Python / 浏览器） |
|---|---|---|---|
| 页头三件套：标题 + 一句论点副标题 + 细线 | 对照样例、ppt-master 规划字段（Core message）、PPTAgent | 每页有结论，层次立刻出来 | 适合，渲染器加字段 |
| 一页多个信息区，按网格组合（左图右卡、上公式下要点、三卡 + 结论条） | 对照样例、pptwise（语义 kind → 约 160 个版式） | 填满内容区，构图有变化 | 适合，是本次改动的核心 |
| 密度档位：讲解 / 阅读型密，演讲型疏 | pptwise（dense / balanced / spacious）、Gamma（brief → extensive） | 讲解稿可以每页多讲，演讲稿保持大字 | 适合，deck 级字段 |
| 每页一个承载信息的视觉：原生图表、大数字、形状流程图、时间轴、示意图 | Anthropic pptx、OfficeCLI、对照样例 | 抽象机制讲得清楚，页面不再是纯文字 | 适合：officecli 原生图表和 `diagram`（mermaid → 可编辑形状，不需要浏览器，已实测）；自定义示意图用 SVG → canvas 栅格化 |
| 版式节奏：相邻页不同版式、深浅交替、密页后接留白页 | Anthropic、pptwise、OfficeCLI、guizang | 不单调 | 适合，渲染器自动选变体 + lint |
| 克制的配色：一个主色 + 一个强调色 + 三档灰，强调色只给重点 | 对照样例、Anthropic（主色 60–70%） | 统一、专业 | 适合，主题角色已有 |
| 反模式清单写成检查：纯文字页、连续三页同版式、空白 > 40%、标题下装饰线、emoji 图标 | Anthropic、OfficeCLI、pptwise、ppt-agent-skills | 去掉“AI 味” | 适合，确定性 lint |
| 先规划再填充：每页论点、视觉计划、版式 | pptwise、ppt-master、Kimi、Genspark | 页型由论点决定，不是凑格子 | 适合，技能规则；大纲先行仍待第 1 项调研 |
| 渲染图自查：先确定性检查，再看缩略图总览，最多 2–3 轮 | Anthropic、OfficeCLI、pptwise | 兜底硬伤 | 部分适合：每页 SVG + 有字的缩略图已有，可拼总览；改页要等 M3a-2 的 revises |
| 图标库 | pptwise（lucide 转原生矢量） | 卡片更有识别度 | 可做：lucide（ISC）预先栅格化成 PNG；不要直接插 SVG（officecli 写的 PNG 回退是 1×1 透明图） |
| AI 生图做封面 / 章节背景 + 遮罩，文字保持原生 | codex-ppt-skill、banana-slides（AGPL，不可抄）、Manus Image 模式 | 视觉上限最高 | 以后可选：Ofox 自带生图 API，要处理耗时、成本、失败回退 |

不适合我们的：HTML → pptx（要浏览器，Anthropic 已退出 html2pptx）、SVG 整页转形状（ppt-master 依赖 Python + Chromium）、
LibreOffice 渲染（160–200 MB / 平台）、整页出图（不可编辑）、Morph 动画与 3D（截图验证不了）、
让模型直接写版面代码或坐标（对照样例的上限靠强模型，换弱模型会掉得很快；M3a-1 之前我们就是这条路）。

## 推荐路线

按“先拿到最大差距、每步都能验”的顺序，每步都用同一个对照提示（讲解 Transformer）和 `pnpm assistant:real-eval` 验收，
并把成品与对照样例并排看。

1. **页面结构换代（最大收益）。** 内容页 = 页头（title + subtitle 论点句）+ 一个版式网格（单栏、左右 6:4 / 4:6、三栏、上下）
   + 每格一个组件。组件：cards（每卡 2–4 条要点，可 highlight 一张）、bullets、kpi、formula（公式 / 代码框）、table、chart、
   flow（原生形状 + 箭头）、timeline、figure、callout（结论条）、quote。组件撑满所在格子，文字顶对齐。
   上限按组件和格子几何推，不再按页型一刀切；deck 级 `density: "detailed" | "presenting"`，讲解稿正文 12–14pt，演讲稿 18–24pt。
   旧的十种页型保留为常用组合的简写，现有 deck 仍可渲染。
2. **画图的一等路径。** flow / timeline 用主题色的原生形状（可编辑）；任意关系图走 officecli `diagram render=native` 并改成主题色；
   chart 用原生图表 + 洞察卡；`figure` 让模型写受限 SVG（viewBox、基本图形与文字、只用主题色名、无外链与脚本），
   运行时用已打包的 @napi-rs/canvas 栅格化成 2× PNG 插入，替代文字取 caption。技能里要求每页声明“靠什么视觉讲清楚”。
3. **节奏与检查。** 渲染器按相邻页自动选版式变体；lint：连续三页同版式、纯文字页、内容区覆盖率过低、标题超过两行、字号低于档位下限；
   结果作为 warning 回给模型。技能加规划规则（每页论点 + 视觉计划 + 版式）。
4. **自查闭环（随 M3a-2）。** 用已有的每页 SVG 与带字缩略图拼一张总览给模型看，最多两轮，改页走 revises。
5. **以后：** 图标（lucide 栅格化）、forest / warm 主题（M3a-3）、封面与章节页 AI 背景图（可选，带回退）。

## 待用户决定

- 路线：上面的“组件化 deck 对象”，还是放开让模型写版面代码（上限高、依赖模型、安全面更大）。
- 默认密度：用户说“讲解 / 详细”时默认 detailed，其余默认 presenting？
- `figure`（模型写 SVG 示意图）要不要在第一步就做。
- 这一轮放在 M3a-2 之前（推荐），还是并入 M3a-3。

## 来源

- 参考 Agent：`~/code/agent-refs` 下 AionCore / AionUi（Apache-2.0，OfficeCLI 技能与提示规则）、deepseek-harness（MIT，python-pptx）；
  opencode 只有查看器；aionrs、claw-code、pi 无 PPT 能力。
- 热门 skill（star 为 2026-11-08 GitHub API 实数）：anthropics/skills 189k（pptx skill，pptxgenjs）、hugohe3/ppt-master 74.7k（MIT）、
  iOfficeAI/OfficeCLI 33.1k（Apache-2.0）、zarazhangrui/frontend-slides 32.1k（MIT，HTML 路线）、op7418/guizang-ppt-skill 29.5k（AGPL）、
  Anionex/banana-slides 16.0k（AGPL）、MiniMax-AI/skills 13.8k（MIT）、presenton/presenton 11.8k（Apache-2.0）、
  ningzimu/codex-ppt-skill 7.7k（MIT）、icip-cas/PPTAgent 5.2k（MIT）、liustack/pptwise 27（MIT，Node-only，离我们的架构最近）。
- 对照样例：同题 8 页 PPT，python-pptx 默认模板、全部空白版式、配图为 Matplotlib 生成；数据由解包 XML 量得。

## 补充：gpt-image-2.6 的 SVG 输出（2026-11-08 实测）

Ofox 已支持：`POST /v1/images/generations`，`model: "openai/gpt-image-2.6-flare"`，`output_format: "svg"`，
并且必须带 `svg: { "model": "gpt-6-luna" }`（不带会 400）。模型目录的参数表里还没写 `output_format`。
机制是先出一张参考图，再由写作模型照图写 SVG（响应里另有 `svg_usage`）。

| 用例 | 耗时 | 估算费用 | 结果 |
|---|---|---|---|
| 带文字的 Transformer 架构图 | 62 秒 | 约 $0.04 | 结构对，文字不行：没居中、长标签被截断、两块深蓝底上的字也是深蓝（Chrome 里同样如此，是 SVG 本身的问题） |
| 无文字的封面插图（词块 + 注意力弧线） | 48 秒 | 约 $0.10 | 干净、配色按要求、2.8 KB；运行时的 @napi-rs/canvas 直接转 PNG 无误 |

结论：它适合封面、章节页和空白区的无字插图与图标，可选、异步、失败时不放图；
讲解用的示意图和数据图要准确的文字和数据，仍由代码画（原生形状与图表，原生做不了的由代码生成 SVG 再栅格化）。
