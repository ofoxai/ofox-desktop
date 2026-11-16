# Ofox PPT 生成的设计系统调研与改进方案

日期：2026-11-09，按补充要求修订。后续执行状态见 [11 月 10 日实施记录](2026-11-10-ppt-implementation.md)；下文保留方案修订时的诊断与未执行状态。范围：本地 `code/agent-refs`、Ofox 1.5.2 工作区，以及固定提交的公开项目文件。本次只修改研究与设计方案；没有改造生成器、安装依赖、生成正式 PPTX、提交或推送代码。

## 1. 结论

**设计建议：采用「公开资源筛选 → 代表风格适配 → 同内容质量验证 → 批量扩充模板库」的路线。** 保留现有 Python 自由排版与 OfficeCLI 作为首选基础，直接复用许可明确的开源代码、模板、样式与资源，并为现有引擎做必要适配。无需把公开实现全部重新编写，也不预先决定替换引擎或接入整个 OpenDesign 应用。

首批以 **20–30 套成熟风格作为筛选目标**，覆盖学术、咨询、教学、产品发布、经营复盘、技术方案。这是建议目标，不是已实现、已适配或已通过验收的数量。先从候选中选 **3–5 套差异明显的代表风格** 打通引入、适配与验证流程，再逐批启用其他资源。「三个风格包」仅可作为技术验证样本，不再作为最终产品模板库目标。

优先筛选 beautiful-html-templates 的完整样例、OpenDesign `html-ppt` 的实际模板与样式，以及能复用现有 OfficeCLI 的公开构建样例。PPT Master 的通用布局、图表和转换代码可按模块评估；Presenton 适合后续模板解析能力。每页设计计划、选择策略与检查规则由 Ofox 独立设计，以公开资料为依据。质量提升、转换覆盖率与成本收益都须通过第 8 节的对照实验验证。

### 来源与许可边界

- **闭源 ofox-agent：只参考能力划分、设计思路和工作流程。** 不复制其代码、配置结构、私有模板、素材、提示词或样例内容；不将其列为可移植资产、实验内容或公开依据的来源。这是本次用户给定的边界，不从可访问文件或相似命名推断复制授权。
- **许可明确的开源来源：可以直接引入并修改。** 对实际采用的文件、目录、代码与资源核对许可范围，记录仓库、完整提交、原始路径、上游来源、许可证及归属、原文件校验值与修改说明。保留必要 LICENSE/NOTICE；不以仓库公开、技能可发现或 README 宣传代替许可核对。
- **第三方资产单独记账。** 字体文件、图片、图标、Logo、音效和示例附件逐项记录自身来源与许可。仓库根目录许可证不自动覆盖这些资产；字体名称或 Google Fonts 链接也不是字体文件的分发许可证明。未核清的部分不进入发布包，可保留为待核资源或替换为授权明确的资产。
- **本地参考不等于资产来源。** 第 2、3 节保留源码诊断和技术观察；正式引入资产优先追溯第 5 节固定版本的公开上游，不能把本地 checkout 中的未知资源直接搬入产品。

### 证据分级

全文区分：**已核实的现状**指实际文件、目录统计、许可文本或源码行为；**设计建议**指筛选目标、优先级、架构与验收规则；**待验证假设**指质量、兼容性、耗时、费用和可编辑性收益。本次未运行生成实验，目录数量、静态样例和上游声明均不是 Ofox 实际生成质量的证明。

## 2. 当前源码能确认的问题

**已核实的现状（方案修订时）。** 本节依据 `ofox-desktop-1.5.2` 工作树，分支 `feat/assistant-1.5.1`、HEAD `2ebe6491` 及工作区文件；本次只读复核的相关诊断仍成立。目录名称不等于已发布版本，不能据此认定用户当前安装包采用哪条路径。

| 观察                                                         | 源码证据                                                     | 对「模板单一」的影响                                                        |
| ------------------------------------------------------------ | ------------------------------------------------------------ | --------------------------------------------------------------------------- |
| Python 路径已有自由布局、SVG 图、matplotlib 和预览           | `assistant-runtime/skills/ofox-office-pptx/SKILL.md:6–23`    | 不能把问题归因为「只能填固定页型」；自由度已经存在，设计决策仍容易收敛      |
| Python 的主题颜色、字体固定在 `T`                            | `assistant-runtime/src/python/deck.py:23–27`                 | 不同主题仍默认蓝橙、相似字体与相似对比关系                                  |
| 内容页统一使用 Content 母版，并固定绘制标题与副标题          | `assistant-runtime/src/python/deck.py:161–170`；技能第 34 行 | 每页的视觉入口高度相似；深色内容页还需要额外处理标题颜色                    |
| Node deck 路径只注册 `navy`                                  | `assistant-runtime/src/deck/theme.mjs:45–47`                 | 该路径实际没有多套主题可选；新增 Python 风格时也要同步考虑此路径            |
| 技能同时要求内容区填到接近底部、2–4 个信息区、相邻页不同布局 | `assistant-runtime/skills/ofox-office-pptx/SKILL.md:53–60`   | 这些规则能防止空洞，但容易机械执行，妨碍强调页、留白页和连续比较页          |
| SVG 示意图转 PNG；`formula()` 是等宽文本                     | `assistant-runtime/src/python/deck.py:265–285`               | 外观可以丰富，但「可编辑的图形/公式」仍需单独建设，不能把预览图当作原生对象 |
| 预览机制已经存在，且回合结束可自动交付最新预览               | `assistant-runtime/src/document-tools.mjs:247–248`           | 改进应关注成品复核证据、问题修复和草稿标识，不能只再加一个预览按钮          |

三个母版 Cover / Section / Content 本身不是问题：内容页内部仍可有很多构图。真正需要松动的是固定的页头、字体层级、卡片样式和每页构图的决策方式。

另外，11 月 8 日的旧调研早于当前 Python 路径。它关于固定页型的诊断适用于当时的 deck 路径，不能当作全部现状；其中对 PPTAgent 主线和 PPT Master 浏览器依赖的描述也需按本次核对更新。

## 3. 本地参考项目的实际做法

以下保留前轮只读扫描的技术观察，不构成对其中所有代码、模板或素材的移植许可判断。

| 项目                    | 此次查到的实现                                                                        | 可借鉴的部分                                                         | 适用边界                                                                     |
| ----------------------- | ------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| AionCore                | 内置 `officecli-pptx`、pitch-deck、Morph 技能；风格目录同时提供描述和部分参考构建脚本 | 先读标题序列、按内容选图表、不同构图、字体与留白规则、目标查看器验收 | 风格名称和调色板不是自动布局引擎；Morph 是按需能力，不宜作为解决单调的第一步 |
| AionUi                  | 助手规则与技能衔接、文件预览等产品层能力                                              | 让生成过程和成品可见、可修改                                         | 不应把预览能力误认为丰富的 PPT 生成能力                                      |
| deepseek-harness        | `office-pptx` 技能以 python-pptx 建立可编辑文本、图表、表格，并分离渲染技能           | 工具准备、保留已有格式、原生对象、外观修改后的渲染检查               | 基础文档能力完整，不等于有内容驱动的风格选择                                 |
| OpenCode                | `OfficePresentation` 明确是只读预览；worker 按需绘制页面                              | 大文件预览、后台渲染、可见页面优先                                   | 本次命中的这部分代码不是生成器                                               |
| pi / claw-code / aionrs | 本次关键词扫描未找到可直接对照的 PPT 创作系统                                         | 作为通用 Agent 基础设施参考                                          | 不据此推断它们通过外部工具无法生成 PPT                                       |

关键本地来源：

- [AionCore PPTX 规则](../../../../agent-refs/AionCore/crates/aionui-app/assets/builtin-skills/officecli-pptx/SKILL.md:179)：构图、图表、素材与验收。
- [AionCore 风格索引](../../../../agent-refs/AionCore/crates/aionui-app/assets/builtin-skills/morph-ppt/reference/styles/INDEX.md:1)。其中 architectural-plan 用分区、左右交替、编号形成风格；minimal-corporate 用非对称网格、字号和蓝金关系形成风格，已超出换颜色。
- [deepseek-harness PPTX 技能](../../../../agent-refs/deepseek-harness/packages/skill/skill-office/assets/office-pptx/SKILL.md:1)。
- [OpenCode 只读预览器](../../../../agent-refs/opencode/packages/gui-plugins/src/microsoft-office/presentation.tsx:31)。

这些规则不能逐条混用：例如 AionCore 的部分风格参考使用标题下划线，通用技能又把装饰下划线列为反模式。Ofox 应保留「为什么使用这种表达」的原则，并建立自己的统一规范。

## 4. PPT Master 的核心设计

**已核实的现状。** 官方仓库 `hugohe3/ppt-master` 固定提交 `3c6ffd0` 的技能版本为 6.9.0；前轮检查的本地 `.claude/skills/ppt-master` 入口版本相同。该项目的公开代码与资源按第 5 节区分直接引入、适配和独立许可核对。

### 四种资源各管一件事

| 资源   | 负责                               | 在 Ofox 中的对应建议                 |
| ------ | ---------------------------------- | ------------------------------------ |
| Brand  | 品牌颜色、字体、Logo、语气         | 用户品牌资料；不把品牌等同于版式     |
| Style  | 表达方法、视觉语言、节奏、证据表达 | 学术答辩、咨询决策、教学解释等风格包 |
| Layout | 画布、母版、槽位、容量、对齐       | 可组合构图、可填充模板               |
| Deck   | 某种反复使用的整套演示应用         | 月度经营汇报、项目答辩等常用稿件骨架 |

这样，同一品牌可以做科研汇报或产品发布；同一种表达方法也能使用不同品牌，而不用维护大量互相复制的整套模板。[模板资源契约](https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/templates/README.md)

### 先决定表达关系，再画几何

每页规划包含受众认知变化、信息关系、核心结论和构图方向。顺序、隶属、对比、交叠等关系决定是否需要连线、分组、层级或矩阵，避免所有内容都变成平行卡片。[每页设计规范](https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/templates/design_spec_reference.md)

阅读型、平衡型、演讲型使用不同的信息密度；结构页、密集页和留白强调页形成整套节奏。重复的根源是表达任务没有变化却机械套用同一种承载方式，而不是卡片本身一律不能用。[页面执行链](https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/references/executor-base.md)

### SVG 是设计源稿，导出编译为原生对象

完整画面先表达在 SVG 源稿中，再将支持的内容转换为 DrawingML、母版、版式和原生对象。原生图表、表格、公式另有各自的受支持契约。这不等于把整页 SVG 当作一张图片插入，也不承诺任意 SVG/CSS 都能无损转换。[导出边界](https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/scripts/README.md)

**设计建议：** 可直接复用许可明确的通用 SVG 原型、图表/表格资源和合适的转换模块，并验证其与 Python / OfficeCLI 的对接；无需把这些全部重新实现。整体替换当前引擎另需证明转换覆盖率、字体与打包依赖适配的收益，现阶段不据目录丰富度作此决定。

### 区分静态检查与视觉审稿

PPT Master 默认流程有质量检查；单独的逐页视觉审稿阶段是用户显式启用的可选环节，使用 Playwright/Chromium。核心 SVG 转 PPTX 依赖不能与这个可选阶段混为一谈。[生成流程](https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/workflows/generate-pptx.md)、[视觉审稿](https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/workflows/stages/visual-review.md)、[依赖清单](https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/requirements.txt)

Ofox 可以借鉴分层检查，但用现有预览能力实现适合桌面端的默认检查，不必照搬全部流程和依赖。

## 5. 公开资源核对与引入判断

### 5.1 固定版本与数量口径

本节来源均按 2026-11-09 读取时的完整提交固定，表中短 SHA 仅供阅读，引用链接保留完整 SHA。**已核实**的是文件存在、目录统计和文档声明；**优先级及引入方式是设计建议**，尚无生成质量、运行兼容性或费用结果。

`nexu-io/open-design@07a86b1` 的 `design-templates/html-ppt` 已按未截断的 Git tree 重新计数，和该版本目录说明相符：

| 资源     | 实际统计口径                        | 数量 | 能证明的范围                                       |
| -------- | ----------------------------------- | ---: | -------------------------------------------------- |
| 主题样式 | `assets/themes/*.css`               |   36 | 颜色、字体与部分视觉规则；不等于 36 套完整稿件     |
| 单页布局 | `templates/single-page/*.html`      |   31 | 单页 HTML 示例；不等于完整风格或 PPTX 模板         |
| 完整演示 | `templates/full-decks/*/index.html` |   15 | 存在完整 HTML 演示入口；不证明原生 PPTX 转换和质量 |

[目录快照][od-html]、[主题目录][od-themes]、[单页布局目录][od-layouts]、[完整演示目录][od-decks]是计数依据。不能相加、相乘为「可用 PPTX 模板数」。目录内独立 [LICENSE][od-html-license] 为 MIT，版权人为 lewis；[技能元数据][od-html-skill] 声明上游为 `lewislulu/html-ppt-skill`。该 OpenDesign 快照未因此等同于上游的某个已知提交，引入时应同时记录本快照和上游地址。

`zarazhangrui/beautiful-html-templates@e5e204f` 的 [index.json][bht-index] 有 34 个条目，目录也实际有 34 个 `templates/*/template.html`。这些是 HTML 样例，不是已通过 Ofox 验收的 PPTX。每套还有 `template.json`、`design.md`，部分使用相邻的 `deck-stage.js`。例如 [Blue Professional][bht-blue] 用 Space Grotesk/Inter 对比，[Editorial Forest][bht-forest] 用 Source Serif 4/JetBrains Mono；读取和复制时要保留完整关联文件。

html-ppt 的 [fonts.css][od-fonts] 和上述 beautiful 样例引用外部 Google Fonts。静态 HTML 可直接采用，不代表其字体文件已随包具备离线分发依据；字体本体、字重、CJK 覆盖与替代方案仍需逐项核对。替代时保留衬线/无衬线、展示/正文等对比，不能全部退回同一种字体。

OpenDesign 的 Zhangzara 目录中，[Blue Professional 的来源声明][od-zara-source] 和 [MIT 文本][od-zara-license] 明确指向上述上游。frontend-slides 的 [bold-template-pack][fs-bold] 也来源于该库。三处不是三个独立模板库，应按上游资源身份去重；`design.md` 的参考配方和完整 `template.html` 也分开记录。

### 5.2 OpenDesign 其他目录与同名项目

- **其他 deck 确有实际文件，但形态不同。** `simple-deck` 有 `assets/template.html`、`example.html` 与布局说明；`replit-deck` 有种子和多份 HTML 示例；`kami-deck` 有 `SKILL.md`、README 和 `example.html`。[simple-deck][od-simple]、[replit-deck][od-replit]、[kami-deck][od-kami] 都不是可直接计数的原生 PPTX 生成器。前者依赖项目设计系统；kami 还注明 `tw93/kami` 为 MIT 上游；replit 说明视觉来自产品图库快照。它们没有各自独立 LICENSE 文件，仓库原创文件的 Apache-2.0 与第三方来源需要分别记录，不能把所有品牌、截图或字体都视作已获分发许可。
- **design-systems 是设计系统包。** [README][od-systems] 定义 `manifest.json`、`DESIGN.md`、`tokens.css`，并有组件、预览、来源证据等扩展。已抽查 [corporate manifest][od-corporate] 与 [evidence][od-corporate-evidence]，来源为 curated bundled fixture；evidence 明确不声称重新抓取上游。目录说明还列出 awesome-design-md、awesome-design-skills、kami 等多种来源。仓库根为 [Apache-2.0][od-license]；本次树检查未发现 design-systems 子目录独立 LICENSE，不能据此省略各包 `source`、资产和上游核对。品牌启发式说明不等于官方品牌资产，也不是 PPT 页面模板。
- **目录入口不等于完整实现。** `skills/frontend-slides` 和 `skills/pptx-generator` 各只有 `SKILL.md`，内容明确说它们用于发现上游，完整资源和脚本需到上游获取。[frontend-slides 入口][od-fs-entry]、[pptx-generator 入口][od-pptx-entry]不能作为已内置生成能力的证据。本次只读取这些说明，没有执行其中安装指令。
- **`manalkaff/opendesign` 是另一项目。** 固定 `cecd9bb`，根 [MIT 许可][mk-license]；[make-a-deck][mk-deck] 实际是创作技能文本，覆盖固定画布、标题顺序、字号、密度、构图和检查。该目录没有完整模板库或 PPTX 转换器。可复用公开工作流文字并按 Ofox 场景改写，不能把它和 nexu-io 的模板及应用混称。

### 5.3 资源比较表

「可直接引入」限定为许可已核对的项目自有文件；不包含尚未逐项核清的第三方资产。P0 表示优先筛选与小样验证，P1 表示后续资源/模块适配，P2 表示依赖或来源核清后的条件评估，均不代表已准入产品。

| 来源及版本                                             | 资源类型                                         | 许可依据                                                               | 可直接引入内容                                                         | 适配工作                                                          | 运行依赖                                                                                                      | 可编辑性                                          | 优先级                      |
| ------------------------------------------------------ | ------------------------------------------------ | ---------------------------------------------------------------------- | ---------------------------------------------------------------------- | ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- | --------------------------- |
| [beautiful-html-templates][bht] `e5e204f`              | 完整 HTML、元数据、设计说明                      | 根 [MIT][bht-license]；所选目录资产另核                                | 选定 HTML/CSS/JS、元数据与公开设计说明，可复制修改                     | 替换样例内容；保留构图与字体对比；映射 CJK 字体；为 PPTX 适配对象 | HTML 用浏览器；样例含 Google Fonts 链接，部分需相邻 JS；静态资源抽取不需整个应用                              | HTML 源码可编辑；原生 PPTX 未验证                 | P0，完整风格主来源          |
| OpenDesign [html-ppt][od-html] `07a86b1`               | 主题、单页、完整 HTML、导航运行时                | 包内 [MIT][od-html-license]，声明 lewislulu 上游                       | 通过筛选的 CSS、布局、完整稿件及必要 JS，可复制修改                    | 修复相对路径；绑定真实内容；保留整套视觉语言；分别适配 HTML/PPTX  | HTML 浏览器＋字体；其 [PNG 脚本][od-render] 使用 Node/Playwright/managed Chromium，并含安装动作，本次未运行   | HTML 可编辑；PNG 是图片；无本包原生 PPTX 转换证明 | P0，场景与组件来源          |
| OpenDesign 其他 [deck 目录][od-templates] `07a86b1`    | simple/replit/kami 等 HTML 种子与示例            | 根 [Apache-2.0][od-license]；kami 上游与 replit 视觉来源按各自说明另核 | 已核清的原创种子和布局代码；不默认打包来源未清资产                     | 设计系统绑定、上游归属与资产核对、字体和导航适配                  | 按所选 HTML 文件；不需默认引入 OpenDesign daemon                                                              | HTML 源码；无统一原生 PPTX 保证                   | P1；未核部分暂缓            |
| OpenDesign [design-systems][od-systems] `07a86b1`      | tokens、字体角色、设计说明、组件样例             | 根 Apache-2.0＋逐包 manifest/source；第三方独立核对                    | 核清后的通用 tokens/说明/组件代码                                      | 选择适合 PPT 的视觉角色；保留字号与字体层级；不照搬网页控件       | 静态资料不需应用；组件预览按文件依赖                                                                          | 设计资料；不是完整 deck，也不是 PPTX              | P1，主题系统补充            |
| [OfficeCLI][office-styles] `c56512c`，发布提交 1.0.160 | style.md、部分 build.sh/PPTX、原生对象工具       | [Apache-2.0][office-license]＋[NOTICE][office-notice]；其他资产另核    | 公开风格说明、具备脚本的生成代码及核清的原型，可复制修改               | 对接现有命令/批处理；改写 shell 与路径；字体与 macOS/Windows 验证 | 现有 OfficeCLI；执行 shell 示例另有 Bash 要求，不自动升级或安装                                               | 脚本可生成原生对象；按所用功能及成品实测          | P0，现有引擎适配对照        |
| [PPT Master][pm-templates] `3c6ffd0`，技能 6.9.0       | Brand/Style/Layout/Deck、SVG、图表表格与转换模块 | [MIT][pm-license]；[图标独立声明][pm-icons]                            | 通用无品牌原型、公开样式和合适转换代码，可复制修改；不默认引入品牌素材 | 选择子集；映射现有引擎；验证 SVG/原生对象覆盖范围                 | 静态资源无新运行时；执行转换另需 python-pptx/XlsxWriter 等；路径合并/字体塑形模块依赖另核；视觉审稿可选浏览器 | 支持范围内转原生对象；不是任意 SVG 无损保证       | P1，原型/组件优先于全链迁移 |
| [frontend-slides][fs] `9906a34`                        | HTML 工作流、CSS/JS、风格配方                    | 根 [MIT][fs-license]；bold pack 追溯 beautiful 上游                    | 公开 CSS/JS、选择与预览配方，可修改                                    | Ofox 自定交互与检查；配方与完整样例去重；PPTX 另适配              | HTML 浏览器；字体与脚本按文件；截图流程单独评估                                                               | HTML 可编辑；不能直接当作原生 PPTX                | P1，工作流与 HTML 辅助      |
| [Presenton][presenton-v2] `9af2b47`                    | Template V2 分析、schema、布局与生成代码         | 根 [Apache-2.0][presenton-license]；静态资产/字体另核                  | 公开解析、schema 与容量分析代码可按模块复用                            | 对接现有引擎与数据；模板建库与生成分开；不直接搬整套服务          | 完整项目需 Python/FastAPI、Node/Next.js 与导出服务；取子模块时另核闭包                                        | 文档声明可编辑导出，具体类型与效果待实验          | P2，后续模板导入能力        |
| [manalkaff/opendesign][mk-deck] `cecd9bb`              | make-a-deck 设计技能文本                         | 根 [MIT][mk-license]                                                   | 可复用并改写公开工作流文字                                             | 独立制定 Ofox 每页计划和检查，过滤不适用的硬规则                  | 技能本身没有生成引擎；HTML 创作另需宿主工具                                                                   | 不提供 PPTX 转换器                                | P1，工作流参考与改写        |

进一步核查：OfficeCLI 固定版本下 `skills/morph-ppt/reference/styles` 实际有 51 份 `style.md`、27 份 `build.sh`，并非每个风格目录都有可执行构建样例。Presenton `templates/general` 确有 `template.json` 和 `static/`，但本次没有逐项批准其静态素材。这些目录不能仅凭名称或根许可证整包纳入。

PPT Master 的 [依赖说明][pm-deps] 将 SVG 转换、PDF 输入和其他功能分开，且明确标注 PDF 路径的 PyMuPDF 许可不同于主项目 MIT；模块执行依赖也要独立核对，不把整份 requirements 默认引入。图标声明另列 MIT、CC BY、CC0 及品牌限制，同样不能统一按根 MIT 处理。

首轮可从以下公开资源选 3–5 套做适配样本：beautiful 的 Blue Professional（咨询/经营）、Cartesian（学术/研究）；OpenDesign `html-ppt` 的 course-module（教学）、product-launch（发布）；OfficeCLI dark--architectural-plan（技术方案）。这是候选组合，需按统一内容检验差异，并不宣布已可用。后续可加入 Editorial Forest 等，向经过去重的 20–30 套筛选目标扩充。

### 5.4 保留的技术判断

下面保留工具路线对照，和资产准入分开看。PPTAgent 固定 `9dacac5` 的 [转换约束][pptagent-contract] 仍要求检查导出后的字体与布局；Anthropic 技能固定 `2c5958e` 作为技术对照，不将未经单独核定许可范围的文件列为首批移植资产。

| 项目                                                                                                                                                                          | 核实到的路线                                                                                             | 值得采用                                                        | 对 Ofox 的成本或限制                                             |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- | ---------------------------------------------------------------- |
| [Presenton Template V2](https://github.com/presenton/presenton/blob/9af2b47dda31060d3d5070e020a0825b15c94b32/docs/template-v2.md)                                             | 对已有 PPTX 结合预览与原始元素做语义分析，区分装饰和可编辑内容，分析重复区域与文字容量，再按 schema 填充 | 后续做「上传公司模板，生成新稿」时很有价值                      | 模板建库需要视觉模型和多阶段分析；不宜在每次生成时重做           |
| [Presenton 版式选择](https://github.com/presenton/presenton/blob/9af2b47dda31060d3d5070e020a0825b15c94b32/servers/fastapi/utils/llm_calls/generate_presentation_structure.py) | 按内容用途选择版式；用户指定图表和布局优先于多样性                                                       | 内容匹配、视觉变化作为软目标                                    | 重复版式有时是正确选择，不能只按重复次数判错                     |
| [PPTAgent 当前主线](https://github.com/icip-cas/PPTAgent/blob/9dacac53ac9ef39346461063df729396551d929e/skills/pptagent/SKILL.md)                                              | HTML 源稿→逐页渲染审查→构建 PPTX→导出稿审查→finalize                                                     | 修改后使旧检查失效；源稿与成品都要看；复核结果绑定当前版本      | 目前技能依赖 Python 和 Playwright；HTML 好看不代表 PPTX 一定一致 |
| [PPTAgent 2025 论文](https://arxiv.org/abs/2501.03936)                                                                                                                        | 两阶段、编辑式生成，分析参考稿的结构与版面后生成                                                         | 借鉴优秀参考稿的信息组织，评价内容、设计和连贯性                | 研究版应按官方固定版本复现，不能与当前 main 混称                 |
| [Anthropic PPTX](https://github.com/anthropics/skills/blob/2c5958ecac7cee2b583967a2c4981d0f4a75c579/skills/pptx/SKILL.md)                                                     | 新稿使用 PptxGenJS；已有稿件优先保留模板与 OOXML 结构，强调原生图表、版式变化与渲染检查                  | 工具只提供能力，设计仍需规则与素材；编辑已有 PPT 应尽量保留原稿 | 换一个写文件库本身不会产生风格多样性                             |
| [frontend-slides](https://github.com/zarazhangrui/frontend-slides/blob/9906a34d640d2111f724544cbc50f7f130569ae1/SKILL.md)                                                     | HTML 演示，先用真实内容展示三种风格方向；阅读稿和演讲稿分密度                                            | 用样张表达审美，比只给模板名称有效                              | 原生交付是 HTML，不能直接视为可编辑 PPTX 方案                    |

PPTAgent 当前 README 明确区分研究版本与当前开发主线，旧的「模板聚类」介绍不能覆盖今天的全部实现。[官方版本说明](https://github.com/icip-cas/PPTAgent/tree/9dacac53ac9ef39346461063df729396551d929e)

## 6. 架构分层与设计方向

以下均为**设计建议**，不是已实现能力或外部资源数量统计。资源接入先区分五层，避免把主题、布局、工作流与引擎混成「模板数」。

| 层                  | 负责的内容                                           | 计数与对接方式                                                                |
| ------------------- | ---------------------------------------------------- | ----------------------------------------------------------------------------- |
| 视觉主题与字体系统  | 颜色角色、字体对比、字阶、线条、装饰语汇             | 统计主题/字体方案；从公开 CSS、tokens、说明提取，不能把一次换色当完整风格适配 |
| 页面构图与组件      | 网格、图文比例、层级、图片裁切、流程、矩阵、图表位置 | 统计单页原型和组件；保留来源的完整构图，投射到 Python / OfficeCLI 对象        |
| 场景叙事结构        | 论证、教学、复盘、发布等整稿任务和页面角色           | 统计场景骨架/完整稿件；与视觉风格建立适用关系，不覆盖用户事实与结论           |
| 创作工作流          | 素材筛选、每页计划、风格选择、创作、复核、局部修订   | 统计流程能力，不算模板；按公开依据独立制定 Ofox 策略                          |
| PPTX 生成与渲染工具 | 文件写入、原生对象、字体、预览、导出与验证           | 统计引擎及能力覆盖，不算风格；先复用已有 Python / OfficeCLI                   |

资源台账分别记录「候选、许可核清、已适配、已验证、已启用」，按上游身份去重。20–30 是成熟风格候选的筛选目标，不能用 36 个 CSS、31 个布局或它们的组合凑数。

### 对接现有引擎

HTML 交付可直接复制并修改许可明确的完整模板及必要静态运行时，补齐相对路径、字体与授权素材；只交付 HTML 不应强制安装 OpenDesign 应用或另一个浏览器运行时。

可编辑 PPTX 优先采用已有引擎：复用公开原型中的构图、样式与合适的代码，再映射为原生文本、形状、连线、表格、Chart/工作簿及受支持公式。OfficeCLI 的公开构建代码可直接适配，PPT Master 的转换模块可按需复用，不要求所有原型人工从零重画。Python 的主题与 Node 的主题模型可以共享来源和语义角色，但不能把所有风格压成同一套字阶、页头和卡片。

HTML 到原生 PPTX 是单独的兼容性实验：普通文本、富文本、SVG、裁切、渐变、阴影、网格、字体回退、图表和公式分别记录覆盖范围、损失与适配工时。CSS/Canvas 动画、滤镜和浏览器布局不默认有原生等价物。整页截图只能标为图片型导出，不计入原生可编辑 PPTX；可编辑 HTML 也不算已完成 PPTX 适配。只有现有工具确实无法满足要求，且实验显示收益与依赖成本可接受，才另行评估浏览器转换器或引擎替换。

### 独立制定每页计划和检查

每页计划由 Ofox 按公开的 [PPT Master 每页规范][pm-spec]、[Presenton 模板语义][presenton-v2]、[manalkaff 标题与密度规则][mk-deck]及用户要求独立设计。建议包含：核心论点、必须保留的事实/来源、信息关系、页面角色、所选原型及版本、主视觉、字体与密度约束、可编辑性要求、素材来源和说明/备注去向。这些是本方案要求，不是从闭源 ofox-agent 复制的配置结构。

风格选择结合场景、正式程度、语言/字体、密度、内容关系和已验证能力，先查元数据再加载所选资源。检查分为事实/文件完整性、视觉可读性、风格特征保留、成品对象可编辑性；允许按场景有例外，不把上游技能的所有强制问答、固定字号和装饰禁令直接移植。公开技能文字可以复用改写，实际执行策略仍以 Ofox 和用户要求为准。

### 风格包同时改变信息组织和视觉语言

| 风格方向 | 信息组织              | 视觉表达                                           | 首选场景           |
| -------- | --------------------- | -------------------------------------------------- | ------------------ |
| 学术答辩 | 问题→方法→证据→边界   | 中性浅底、清晰编号、原生公式、图注、证据主视觉     | 当前能源电力答辩   |
| 咨询决策 | 结论→依据→选项→建议   | 强弱对比、比较矩阵、差距图、判断框；重点只强调一处 | 战略研究、经营分析 |
| 叙事讲解 | 提问→直觉→机制→算例   | 大字引导、逐步放大局部、同一示意图延续             | 课程、算法解释     |
| 产品发布 | 场景→痛点→体验→能力   | 大图、真实截图、局部特写、少量大数字               | 产品介绍           |
| 经营复盘 | 目标→差距→原因→行动   | 原生图表、瀑布图、趋势、负责人和下一步             | 月报、季度复盘     |
| 工程蓝图 | 全局→模块→数据流→约束 | 层级结构、泳道、依赖连线、局部高亮                 | 系统设计、技术方案 |

这些是场景分类，不是限定产品只做六套模板。每套资源保留自己的构图、字体对比和视觉特征；同一套稿件内部保持字体与间距一致。3–5 套代表样本从成熟公开资源中选择和适配，不再默认自建前三个完整风格包。

### 构图由关系与数据决定

| 内容关系     | 优先考虑的表达                 | 避免的机械替代           |
| ------------ | ------------------------------ | ------------------------ |
| 先后、因果   | 流程、泳道、闭环               | 三张互不连接的卡片       |
| 层级、归属   | 分层架构、树、嵌套分组         | 平行列表                 |
| 选项比较     | 对照矩阵、共同基线、差异高亮   | 左右两大段正文           |
| 构成、增量   | 堆叠、瀑布、分配流             | 无来源的大数字装饰       |
| 门槛、敏感性 | 阈值曲线、区间、局部算例       | 只有公式没有解释         |
| 多情景       | 同尺度小图、热力矩阵、稳健区间 | 每个情景各占一张相同卡片 |
| 单一关键结论 | 一个主视觉或大字留白页         | 强行填满 2–4 个信息区    |

把图作为论证的一部分：每一张图都应能回答「它让读者更容易理解什么」。不把 AI 生图当默认装饰；架构、数值、公式优先原生对象，照片和概念插画按内容选择。

### 用当前答辩的页面检验方案

基于此前交付记录中的第 26–43 页主题，以下可作第二批、来源和使用授权明确的业务验收材料；具体数值仍以原始报告为准。第一批对照采用第 8 节自编合成样张，避免将真实资料或闭源项目内容混入公共样本。

| 页面主题           | 建议构图                                           |
| ------------------ | -------------------------------------------------- |
| 五层决策架构       | 层级堆叠＋跨层数据流；右侧只解释一条完整路径       |
| 三道决策门         | 顺序门槛＋通过/等待/退出的分支                     |
| 等待期权与进入门槛 | 阈值图＋一个按原始数据计算的算例；公式留为原生公式 |
| 节点组合与三池配置 | 比较矩阵＋资源分配图，区分选择与分配两个问题       |
| 多 Agent 协同      | 泳道展示职责、交接和反馈，避免头像卡片堆叠         |
| 六情景稳健性       | 同尺度情景对比＋关键稳健区间，结论与证据紧邻       |

### 放松过于机械的设计规则

- 相邻页不同构图是建议，不是硬门禁；连续对照页可以保持布局以便比较。结构重复与内容关系重复应一起判断。
- 留白不按统一覆盖率判错。强调页可以稀疏，阅读型证据页可以密集；先确定页面任务，再判断是否空洞。
- 一套稿件可以有不同密度，但须保持字号角色和信息层次稳定。投影演讲与离线阅读不应共用一套固定正文尺寸。
- 把统一标题下划线、侧边色条等从默认必选改为由风格决定；来源风格中有表达作用的线条和装饰可保留，不用一套「反 AI 味」规则抹平所有资源。
- 数据不足时删减或明确缺口，不靠装饰性指标、虚构曲线或重复总结填空间。

## 7. 推荐实现顺序

以下为后续实施建议，本次不执行。

### 第一阶段 公开资源筛选

以 20–30 套成熟风格为首批筛选目标，按六类场景覆盖，从第 5 节的实际文件出发建候选清单。核对许可证、上游、字体与其他资产，合并 OpenDesign、beautiful 与 frontend-slides 中的同源项。记录每套资源的代表构图、字体层级、密度、可复用文件、依赖与缺口，不以截图宣传或同色变体凑数。

产出是候选与来源台账，不是 20–30 套已启用 PPTX 模板。来源未清的资产保留待核状态，不阻止先采用已核清的开源部分。

### 第二阶段 代表风格适配

选 3–5 套差异明显的代表风格，覆盖明暗、衬线/无衬线、阅读/演讲密度、图文与技术图解等差别。优先直接引入开源文件，再为 Python / OfficeCLI 适配；完整 HTML 另可直接作为 HTML 交付候选。保留源资源的构图、字阶对比、视觉重心和组件语法，避免适配后统一成默认页头加卡片。

明确每套资源的 HTML 结果和 PPTX 结果、原生对象覆盖范围与降级项。普通图表优先原生 Chart 和嵌入工作簿；流程和矩阵优先原生形状/连线；公式验证受支持的原生数学对象。复杂科学图可作为标明图片属性的资源保留生成源，但不能因此把原生公式或图表验收标为通过。

转换模块或新运行时只作必要性与成本评估，不由资源目录存在直接推导整套引入。每页计划与新增视觉复核作为独立变量接入后续实验，不与风格资源一次混入，以便分清收益来源。

### 第三阶段 同内容质量验证

按第 8 节做当前 Desktop、公开风格资源、每页计划、成品视觉复核的逐层对照。保持生成基础一致，报告原始结果、失败和成本；没有测得收益就不写「质量已提升」。特别检查 CJK 字体对比是否保留、预览与实际 PPTX 是否一致，以及已知公式/图片限制是否阻断验收。

### 第四阶段 批量扩充与启用

代表样本通过后，按场景逐批适配和启用其他资源。每批仍做许可核对、风格特征核对、同内容回归和对象可编辑性检查。分别报告候选数、许可核清数、适配数、验证数和启用数。首批筛选目标不是产品最终容量上限，3–5 套验证样本也不是产品库规模目标；未通过的资源不得被计入已可用 PPTX 库。

按需加载元数据和选定资源，避免把全部模板塞入模型上下文。普通任务可自动选择合适风格并成稿，用户需要探索时提供实际内容样张；展示几个备选与库内资源数量是两回事，不增加每次生成的强制确认。

### 后续 公司模板与能力扩展

验证资源路线后，再按实际需求复用 Presenton 的公开解析、容量与重复区分析模块，建设用户模板导入。对已有稿件修改，保留其母版、版式和未修改页面。全引擎替换、浏览器打包、Morph 和 3D 另设实验，不作为本轮路线的先决条件。

以后若接入 OFox 图片生成、模型目录或认证，继续遵循 `ofoxApex` 动态 URL 规则，覆盖 `.ai` / `.io`、首次启动、切区和重启。公开项目中的固定 API 地址不直接成为 Ofox 运行配置。

## 8. 可验证的实验与验收设计

### 8.1 自编统一样张

以下是本稿独立编写的固定测试内容草案，**全部为合成演示，不代表实际业务或系统性能**。不使用闭源 ofox-agent 的内容，也不使用模板厂商样例中的人物、品牌或指标填充效果。第一次实验前固化文字、数据、必需事实、语言、画布、密度与版本；本次没有生成样张文件。

主题为「将离线文档整理为可检索资料」，固定中文、16:9、六页、平衡阅读密度。各组正文使用相同可读性下限，建议不低于 12pt，图注和数据标签不低于 10pt；允许来源风格拥有不同字体与字阶，不强制相同信息区数量。

| 页  | 必须保留的内容                                                                                              | 对照任务                           |
| --- | ----------------------------------------------------------------------------------------------------------- | ---------------------------------- |
| 1   | 主题、合成演示声明、目标为比较资料整理方案                                                                  | 封面与论点                         |
| 2   | 采集→去重→索引→检索四步，去重在索引之前                                                                     | 关系与流程表达                     |
| 3   | 定义 A 为每批全量重建；B 为仅处理变更。A 不维护增量状态，B 需维护变更记录；本例尚未测量两者性能             | 比较与限定条件                     |
| 4   | 批次甲/乙/丙：输入条目 100/200/300，有效条目 90/180/270，索引耗时 12/24/36 秒；注明合成数据                 | 表格与原生图表，所有组保留相同数据 |
| 5   | 有效率＝有效条目÷输入条目×100%；乙批次为 180÷200×100%＝90%                                                  | 公式、算例和可编辑性               |
| 6   | B 是待测试候选；尚未验证真实更新负载，不能据这些批次数据宣称 B 优于 A；下一步比较相同负载下的耗时与维护成本 | 论证闭合与限制                     |

允许调整措辞与构图，禁止删减必需事实、改变数值、增加页数、降低密度要求或缩小字号以制造优势。已有答辩、产品介绍和经营复盘可在来源明确后补为不同题材的回归集，所有实验条件都必须使用同一版本内容包。

### 8.2 四组逐层对照

| 条件               | 增量                              | 固定与比较方式                                                                                |
| ------------------ | --------------------------------- | --------------------------------------------------------------------------------------------- |
| A 当前基线         | 当前 Desktop 完整生成路径         | 记录代码/安装包版本、工作区差异、引擎路线、现有技能和预览行为；不移除已有预览或检查来弱化基线 |
| B 加入公开风格资源 | A＋选定且许可核清的资源及必要适配 | 暂不增加新每页计划或新视觉审稿；比较资源本身的贡献                                            |
| C 加入每页设计计划 | B＋第 6 节独立制定的每页计划      | 与 B 使用同一资源和适配版本，比较计划的贡献                                                   |
| D 加入成品视觉复核 | C＋对当前导出成品的视觉检查与修复 | 保留同一生成模型；记录审稿模型、调用与修复轮数，建议额外修复最多两轮；比较缺陷减少与额外成本  |

固定内容包、页数、语言、密度、生成模型版本与参数、工具版本和平台环境；B/C/D 的同一配对使用同一风格版本。先固定风格比较，再单独测试选择策略，避免「选了不同模板」混入每页计划的收益。HTML 对 HTML、原生 PPTX 对原生 PPTX 分别比较，不能把 HTML 浏览器效果与基线 PPTX 截图混评。

优先在同一 Python / OfficeCLI 路线上隔离变量；Node 回退路线单列验证，不混入总体平均。建议每条件至少重复三次，报告样本数、中位数和范围，保留失败；小样本不作统计显著性或普遍质量保证。先用 3–5 套代表样本验证，再决定扩大范围。

### 8.3 评价和记录

| 维度               | 验证方法与证据                                                                                                                                                        |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 内容完整性         | 对照必需事实清单，检查遗漏、改写失真、数据/单位与 90% 算例；记录逐页文本和检查结果                                                                                    |
| 论证顺序           | 只读标题是否连贯，图证是否支持结论，是否保留「未测 B/A 性能」的限制；避免用模板故事覆盖内容                                                                           |
| 构图差异与风格保留 | 比较整稿和同页图，检查信息分区、视觉主次、字体对比、图文比例与来源特征；换色不单独算构图提升，重复布局也不自动算缺陷                                                  |
| 字体与溢出         | 记录实际字体、CJK 回退、字号，检查缺字、换行、遮挡、裁切与对比度；查看目标 PowerPoint 中的导出文件                                                                    |
| 实际可编辑性       | 按核心对象清单检查文本、形状/连线、表格、Chart/嵌入数据与原生数学对象；在 PowerPoint 修改文字、图表数据、公式后保存并重开。只有可拖动图片或可改源代码不算原生对象通过 |
| 耗时               | 记录提交到初稿、提交到最终成品的端到端时间、渲染/复核时间及重试；首次加载与热运行分开，首次适配工时单列                                                               |
| 费用               | 逐次记录输入/输出 token、图片/审稿调用、失败重试、使用的计价版本与日期；分别列实际账单与估算。一次性适配和依赖维护成本不混作单次模型费用                              |

内容、论证、构图和可读性可采用预先固定的 1–5 分量表，由不知道实验组别的评审按相同规则评分；这是建议的评审设计，尚无得分。对象编辑测试、缺陷截图和数据检查是独立证据，不用综合观感代替。至少复核 macOS 与 Windows 的共享生成路径及目标 PowerPoint，单平台结果不代表另一平台已通过。

### 8.4 扩库门槛与停止条件

建议启用门槛为：必需事实与数据无错误、无阻断阅读的溢出/缺字、核心可编辑对象通过、许可与来源台账齐全；同时保留来源风格的结构特征。若已有引擎不支持样张要求的原生公式等，应如实列为阻断项，不用截图降级把验收标为通过。

按 B−A、C−B、D−C 分别报告收益和成本。只有代表样本中的可复核改进成立、内容与论证不退化、额外时间与费用可接受，才逐批扩充。结果持平、失败或只有配色改变时，先修复资源适配/选择/构图问题，不靠增加目录数量宣布优化成功。没有足够样本时保留待验证结论。

## 9. 当前状态与仍需验证的事项

本次已核对：第 2 节源码诊断，新增来源的固定提交、关键目录和许可文本，OpenDesign 36/31/15 的分项计数，beautiful 的 34 份完整 HTML 与索引，以及入口技能和实际实现的区别。许可观察仅覆盖所指文件，不是对全部第三方素材的统一批准。

尚待验证：代表资源的 CJK 字体及资产许可闭包、现有引擎适配工时、HTML 到原生 PPTX 的逐项覆盖与视觉保真、跨平台可编辑性、每页计划与成品复核的真实质量增益、耗时及费用。以上均未通过生成实验验证；20–30 为筛选目标，3–5 为技术样本规模。

本次只读访问公开来源并更新本文件，没有运行来源项目脚本、安装依赖、改生成器、调用生成/审稿 API、提交或推送。完整提交见下列不可变链接；目录统计来自相应提交的 Git tree，不能据 README 文案外推可用 PPTX 数或实际生成质量。

[od-html]: https://github.com/nexu-io/open-design/tree/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/html-ppt
[od-themes]: https://github.com/nexu-io/open-design/tree/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/html-ppt/assets/themes
[od-layouts]: https://github.com/nexu-io/open-design/tree/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/html-ppt/templates/single-page
[od-decks]: https://github.com/nexu-io/open-design/tree/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/html-ppt/templates/full-decks
[od-html-license]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/html-ppt/LICENSE
[od-html-skill]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/html-ppt/SKILL.md
[od-license]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/LICENSE
[od-render]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/html-ppt/scripts/render.sh
[od-fonts]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/html-ppt/assets/fonts.css
[od-templates]: https://github.com/nexu-io/open-design/tree/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates
[od-simple]: https://github.com/nexu-io/open-design/tree/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/simple-deck
[od-replit]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/replit-deck/SKILL.md
[od-kami]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/kami-deck/README.md
[od-systems]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-systems/README.md
[od-corporate]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-systems/corporate/manifest.json
[od-corporate-evidence]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-systems/corporate/source/evidence.md
[od-fs-entry]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/skills/frontend-slides/SKILL.md
[od-pptx-entry]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/skills/pptx-generator/SKILL.md
[od-zara-source]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/html-ppt-zhangzara-blue-professional/SKILL.md
[od-zara-license]: https://github.com/nexu-io/open-design/blob/07a86b1041531bfa0bec1006d9c0993985fee560/design-templates/html-ppt-zhangzara-blue-professional/LICENSE
[bht]: https://github.com/zarazhangrui/beautiful-html-templates/tree/e5e204fb1f3b06290846e7dcd7aceddabeceec8c
[bht-index]: https://github.com/zarazhangrui/beautiful-html-templates/blob/e5e204fb1f3b06290846e7dcd7aceddabeceec8c/index.json
[bht-license]: https://github.com/zarazhangrui/beautiful-html-templates/blob/e5e204fb1f3b06290846e7dcd7aceddabeceec8c/LICENSE
[bht-blue]: https://github.com/zarazhangrui/beautiful-html-templates/tree/e5e204fb1f3b06290846e7dcd7aceddabeceec8c/templates/blue-professional
[bht-forest]: https://github.com/zarazhangrui/beautiful-html-templates/tree/e5e204fb1f3b06290846e7dcd7aceddabeceec8c/templates/editorial-forest
[office-styles]: https://github.com/iOfficeAI/OfficeCLI/tree/c56512c8c7b76fe284f09940e4cd492470099d3e/skills/morph-ppt/reference/styles
[office-license]: https://github.com/iOfficeAI/OfficeCLI/blob/c56512c8c7b76fe284f09940e4cd492470099d3e/LICENSE
[office-notice]: https://github.com/iOfficeAI/OfficeCLI/blob/c56512c8c7b76fe284f09940e4cd492470099d3e/NOTICE
[pm-templates]: https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/templates/README.md
[pm-license]: https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/LICENSE
[pm-icons]: https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/templates/icons/THIRD_PARTY_NOTICES.md
[pm-spec]: https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/templates/design_spec_reference.md
[pm-deps]: https://github.com/hugohe3/ppt-master/blob/3c6ffd0ee6d6e31aa9ca316bd157173802e3526f/skills/ppt-master/requirements.txt
[fs]: https://github.com/zarazhangrui/frontend-slides/tree/9906a34d640d2111f724544cbc50f7f130569ae1
[fs-bold]: https://github.com/zarazhangrui/frontend-slides/blob/9906a34d640d2111f724544cbc50f7f130569ae1/bold-template-pack/README.md
[fs-license]: https://github.com/zarazhangrui/frontend-slides/blob/9906a34d640d2111f724544cbc50f7f130569ae1/LICENSE
[presenton-v2]: https://github.com/presenton/presenton/blob/9af2b47dda31060d3d5070e020a0825b15c94b32/docs/template-v2.md
[presenton-license]: https://github.com/presenton/presenton/blob/9af2b47dda31060d3d5070e020a0825b15c94b32/LICENSE
[mk-deck]: https://github.com/manalkaff/opendesign/blob/cecd9bb6b59408cb96a3974449b8e6ef9f5b17bb/skills/make-a-deck/SKILL.md
[mk-license]: https://github.com/manalkaff/opendesign/blob/cecd9bb6b59408cb96a3974449b8e6ef9f5b17bb/LICENSE
[pptagent-contract]: https://github.com/icip-cas/PPTAgent/blob/9dacac53ac9ef39346461063df729396551d929e/skills/pptagent/references/task-contract.md
