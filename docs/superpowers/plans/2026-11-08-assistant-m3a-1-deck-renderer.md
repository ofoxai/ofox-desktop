# 助手 M3a-1：结构化 PPT 生成骨架 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 模型只写 deck 对象（十种页型 + 文字），运行时的渲染器按 `navy` 主题把它变成 officecli 批处理，宿主生成后自动修正溢出、出界、低对比度，导出每一页 SVG；三个真实模型跑同一提示得到观感一致、零警告的一份 PPT。

**Architecture:**
- 运行时（Node sidecar）新增 `src/deck/`：`schema.mjs` 宽容解析与上限校验，`text.mjs` 文字适配估算（常数与 officecli 检查器一致），`theme.mjs` 主题样式表，`layout.mjs` 画布常数与排版原语，`render.mjs` 十种页型的渲染。`create_document` 收到 deck 对象时在运行时内渲染成批处理，再按现有路径交给宿主；批处理数组（专家模式）原样不变。
- 宿主（Rust）在 `batch → validate → view issues` 之后新增质检闭环：解析 issues，对溢出 / 出界 / 低对比度生成 `set` 修正批处理（过现有校验器），最多两轮；再把每一页导出为 SVG，随附件保存到 `<sid>/attachments/<id>.pages/<n>.svg`。
- 真实模型回归最小版：一个 `#[ignore]` 的 cargo 集成测试用真实网关跑 4 个提示 × N 个模型，脚本负责取 key、挑模型、汇总成 Markdown。

**Tech Stack:** Node 22 + typebox + node:test（运行时）；Rust / Tauri 2，inline `#[cfg(test)]` + `src-tauri/tests/assistant_host.rs` e2e（宿主）；officecli 1.0.160（已随包，`assistant-runtime/resources/engines/officecli/officecli`）。

**Spec:** `docs/superpowers/specs/2026-11-07-assistant-office-polish-and-edit-design.md`（“A. 内容模型”“主题与排版规则”“质检闭环”“里程碑”第 1 行、已决定 1–19）。调研依据：`docs/superpowers/research/2026-11-07-slide-types-and-themes.md`。路线图：`docs/assistant-roadmap.md` 第 1 项切片 1、第 3 项。

## Global Constraints

- 只在 `/Users/yunshen/code/ofox-desktop-1.5.2` 工作，分支 `feat/assistant-deliverables`。不 checkout、不修改、不推送 `release/1.5.0`、`feat/assistant-1.5.1`、`feat/assistant-media-agent`。不打 tag。每个任务完成后本地提交；不推送、不开 PR。
- 磁盘：每次 Rust 全量编译前 `df -h /System/Volumes/Data`，可用 < 20 GB 时只能删 `/Users/yunshen/code/ofox-desktop/src-tauri/target`、`/Users/yunshen/code/ofox-desktop-assistant-media/src-tauri/target`、本工作区 `src-tauri/target/debug/incremental`。重任务（`cargo build/test/clippy`、`pnpm test:unit`、`pnpm assistant:test`）一次只跑一个。不在 `/Applications` 安装新版 app。
- 开源边界：页型词汇、调色板数值、字号地板取自 OfficeCLI 的 morph-ppt 技能（Apache-2.0），在 `theme.mjs` 顶部和 `assistant-runtime/notices/officecli-skills.txt` 写明来源；不拿 Anthropic 的 pptx 技能；提交信息和注释不提闭源产品。
- 沙箱与限制不变：批处理 ≤ 2000 条（`create.rs:40 MAX_COMMANDS`）、内容 ≤ 2 MiB（`create.rs:38`）、引擎 stdout ≤ 256 KiB（`runner.rs:11 OUTPUT_LIMIT`）、超时 60 s；修正批处理同样经 `check_batch`（`create.rs:373`）和 `sandbox-exec`。
- 安全边界：渲染器只产出 `add` / `set` 命令，图片只收 `data:` URI（`attachment:<id>` 在 M3a-3）；工具结果是纯文本；新文案无需 i18n（本切片不改前端）。
- 项目规范：pnpm；Conventional Commits，不带生成信息；Rust 过 `cargo fmt --check --manifest-path src-tauri/Cargo.toml` 和 `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`；运行时 `src` 改动后先 `pnpm assistant:prepare` 再 `pnpm assistant:test`；任何 Cargo 测试前先 `pnpm assistant:prepare`。
- 反目标：渲染器绝不截断或改写用户文字；超限一律拒绝整个调用；13pt 以下不用；用户文件（M3b）不在本切片。

## Review Focus

1. **估算和真实字体不一致**（中英混排的标题、全角标点）：Task 9 的 e2e 用含 “Transformer 入门”“Q1 收入 +18%” 的 deck 真跑 officecli，断言 `issues == []` 且 `fixed == 0`。
2. **Pi 把单个 deck 对象包成一元数组**：Task 6 的 `document-tools.test.mjs` 断言 `[deck]` 和 `deck` 都走渲染器。
3. **DeepSeek 风格的松散输入**（数字写成字符串、数组写成带换行的字符串、多余字段、页型大小写）：Task 1 测试逐项覆盖，宽容转换但不删内容。
4. **修正循环不终止或改到别的形状**：Task 7 断言两轮后停止、修正命令的 `path` 等于 issue 的 `path`、修正批处理过 `check_batch`。
5. **30 页满额 deck 撑爆限制**：Task 5 断言 30 页每页满额内容的批处理 ≤ 2000 条、序列化 ≤ 2 MiB；Task 8 断言某页 SVG 超过 256 KiB 时只丢那一页并记 warning。

---

## 文件结构

运行时（`assistant-runtime/`）：
- `src/deck/schema.mjs`：页型表、上限表、`textLength`、`parseDeck`、`deckOf`。
- `src/deck/text.mjs`：适配常数、行容量、行数、框高、选字号。
- `src/deck/theme.mjs`：`THEMES.navy`、`themeFor`、`fontsFor`。
- `src/deck/layout.mjs`：画布常数、`columns`、`slideCommands`、`titleCommands`、`textboxCommand`、`cardCommands`。
- `src/deck/render.mjs`：`renderDeck` 与十个页型渲染函数。
- `src/document-tools.mjs`：deck 检测 → 渲染 → 现有 hostTool；描述与指引文案。
- `skills/ofox-office-pptx/SKILL.md`：改写为 deck 优先。
- `test/deck-schema.test.mjs`、`test/deck-text.test.mjs`、`test/deck-theme.test.mjs`、`test/deck-render.test.mjs`；`test/document-tools.test.mjs`、`test/bundled-skills.test.mjs` 增改。

宿主（`src-tauri/`）：
- `src/services/assistant/documents/fix.rs`（新）：issue 解析、几何解析、修正命令生成（纯函数，不碰引擎）。
- `src/services/assistant/documents/create.rs`：`make_office` 接入修正循环和全页 SVG；`Made` / 结果 JSON 增 `fixed`、`pages`。
- `src/services/assistant/attachments.rs`：生成附件后保存 `<id>.pages/`；删除附件时一并删除。
- `tests/assistant_host.rs`：`mod documents` 新增 deck e2e；`mod real_eval` 新增 `#[ignore]` 真实回归。
- `tests/fixtures/assistant-documents/deck-object.json`、`deck-showcase.json`、`deck-unknown-type.json`、`deck-too-long.json`。

脚本与文档：
- `scripts/assistant-real-eval.mjs`（新）；`docs/assistant-engines.md`、`docs/assistant-roadmap.md` 更新。

---

### Task 0: 准备

- [ ] **Step 1: 确认分支、磁盘、依赖**

Run: `cd /Users/yunshen/code/ofox-desktop-1.5.2 && git branch --show-current && git log --oneline -1 && df -h /System/Volumes/Data | tail -1`
Expected: `feat/assistant-deliverables`，最新提交 `7245359e docs(assistant): adopt the morph-ppt page types…` 或更新，可用空间 > 20 GB。

Run: `pnpm install --frozen-lockfile && pnpm assistant:prepare`
Expected: 无报错；`assistant-runtime/resources/engines/officecli/officecli --version` 输出 `1.0.160`。

- [ ] **Step 2: 基线测试**

Run: `pnpm assistant:test 2>&1 | tail -4`
Expected: 全部通过。

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test assistant_host documents 2>&1 | grep "test result"`
Expected: `test result: ok`（8 个文档 e2e 通过）。

---

### Task 1: deck 内容模型与宽容解析

**Files:**
- Create: `assistant-runtime/src/deck/schema.mjs`
- Test: `assistant-runtime/test/deck-schema.test.mjs`

**Interfaces:**
- Produces: `export const PAGE_TYPES = ["hero","statement","pillars","evidence","timeline","comparison","grid","quote","cta","showcase"]`。
- Produces: `export const LIMITS`，中文字符为单位，与设计稿页型表一致：

```js
{ deck: { slides: [3, 30], notes: 300, title: 22 },
  hero: { title: 18, subtitle: 30, meta: 30 }, statement: { statement: 40, note: 60 },
  pillars: { count: [2, 4], heading: 12, body: 60 }, evidence: { count: [1, 4], value: 6, label: 12, note: 30, detail: [3, 40] },
  timeline: { count: [3, 6], heading: 10, body: 30 }, comparison: { heading: 12, points: [4, 30] },
  grid: { count: [3, 8], heading: 10, body: 30 }, quote: { quote: 60, source: 20 }, cta: { title: 18, subtitle: 40, contact: 30 },
  showcase: { caption: 30, body: 100, chart: { categories: 8, series: 3, pie: [2, 6], label: 12 }, table: { columns: 6, rows: 6, header: 8, cell: 12 } } }
```

  `evidence.value` 的 6 按字符数算（不按中文单位）；`detail: [3, 40]` 表示 ≤ 3 条、每条 ≤ 40。
- Produces: `export function textLength(text: string): number` — CJK（`⺀-鿿`、`가-힯`、`豈-﫿`、`＀-￯`）每字 1，其余每字 0.5，向上取整；换行不计。
- Produces: `export function deckOf(content: unknown): object | null` — `content` 是带 `kind === "deck"` 的对象，或恰好一个元素且该元素 `kind === "deck"` 的数组时返回该对象；否则 `null`。
- Produces: `export function parseDeck(value: object, { fallbackLanguage = "zh" } = {}): { deck, warnings: string[] } | { problems: string[] }`。
  - 宽容：`language` 不在 zh/en/ja 时取 `fallbackLanguage`；`theme` 缺省为 `"navy"`，未知主题记 warning 并用 navy；页型大小写不敏感；数组字段给成字符串按换行拆；`points`/`detail` 里的元素是对象时取其 `text`；`metrics[].value` 是数字时转字符串；`chart.series[].values` 是字符串时按逗号转数字；未知字段丢弃并记 warning `slide 3: ignored field "color"`。
  - 拒绝（`problems`，一条一句，点名页序号、页型、字段、实际值、上限）：缺 `slides` 或页数不在 `[3, 30]`；未知页型 → `slide 2: unknown type "bullets"; use one of hero, statement, pillars, evidence, timeline, comparison, grid, quote, cta, showcase`；缺必填字段；条数越界 → `slide 4 (pillars): 5 pillars; the limit is 2–4`；字数越界 → `slide 3 (pillars): pillars[1].body is 72 characters; the limit is 60`；`showcase.media` 不是恰好 image / chart / table 之一；chart 的 `categories`、`series[].name` 含 `,` `;` `:`（officecli 用它们分隔）；`chart.kind` 不是 bar / line / pie；table 行列不齐；`media.image` 不是 `data:image/(png|jpeg|svg+xml);base64,` 开头。
  - 输出的 `deck` 是规范化副本：每页 `{ type, ...fields, notes? }`，字符串已 `trim`，不改写任何文字。

- [ ] **Step 1: 写失败的测试**

`test/deck-schema.test.mjs`（node:test，仿 `test/document-tools.test.mjs` 的写法）：
- `textLength counts CJK as one and Latin as half`：`textLength("Transformer 入门")` 等于 `ceil(11×0.5 + 1 + 2) = 9`；`textLength("")` 为 0。
- `deckOf finds a deck object and its one-element wrapper`：对象、`[对象]` 都返回对象；`[cmd]`、字符串、两元素数组返回 `null`。
- `parseDeck accepts every page type at its limits`：十种页型各一页、每个字段填到上限（用 `"字".repeat(n)`），断言返回 `deck.slides.length === 10`、`warnings` 为空。
- `parseDeck tolerates loose input`：`type: "Pillars"`、`pillars` 给成 `"感知: 多模态\n推理: 思维链"`（拆成两条，`heading` 为冒号前、`body` 为冒号后）、`metrics[0].value: 118`、`series[0].values: "12, 15"`、多余字段 `color` → 解析成功，`warnings` 含 `ignored field "color"`。
- `parseDeck rejects an unknown page type and lists the types`：断言 `problems[0]` 等于上面 Interfaces 里的那句。
- `parseDeck rejects overlong text and names the slide, field and limit`：`pillars[1].body` 61 个汉字 → `problems` 含 `slide 1 (pillars): pillars[1].body is 61 characters; the limit is 60`。
- `parseDeck rejects too many or too few items`：5 个 pillars、2 个 timeline steps、31 页、2 页。
- `parseDeck rejects chart labels with separators` 与 `rejects a table whose rows do not match the columns`。
- `parseDeck never rewrites text`：带前后空格和内部双空格的标题，输出等于 `trim` 后原文。

- [ ] **Step 2: 运行，确认失败**

Run: `cd assistant-runtime && node --test test/deck-schema.test.mjs 2>&1 | tail -3`
Expected: 全部失败（模块不存在）。

- [ ] **Step 3: 实现 `schema.mjs`**

按 Interfaces 写 `PAGE_TYPES`、`LIMITS`、`textLength`、`deckOf`、`parseDeck`。每个页型一个 `normalize<Type>(slide, index, problems, warnings)`，公共的 `str(value, { field, limit })`、`list(value, { field, count, each })` 做转换与越界检查。

- [ ] **Step 4: 运行，确认通过**

Run: `node --test test/deck-schema.test.mjs 2>&1 | tail -3`
Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add assistant-runtime/src/deck/schema.mjs assistant-runtime/test/deck-schema.test.mjs
git commit -m "feat(assistant): parse deck objects for pptx with per-type limits"
```

---

### Task 2: 文字适配估算

**Files:**
- Create: `assistant-runtime/src/deck/text.mjs`
- Test: `assistant-runtime/test/deck-text.test.mjs`

**Interfaces:**
- Produces: `export const FIT = { cjkWidthCm: 0.0353, latinRatio: 0.55, lineHeightCm: 0.0466, insetXCm: 0.254, insetYCm: 0.127 }`（officecli 检查器：字宽 = 字号 pt（CJK）/ 0.55 × 字号（拉丁），行高 1.32 × 字号，内边距 0.1 in 左右、0.05 in 上下）。
- Produces: `export function lineCapacity(widthCm, sizePt): number` = `(widthCm − 2·insetXCm) / (sizePt · cjkWidthCm)`，以中文单位计。
- Produces: `export function lineCount(text, widthCm, sizePt): number` = 各段 `max(1, ceil(textLength(段) / lineCapacity))` 之和。
- Produces: `export function heightCm(lines, sizePt, { paragraphs = 1, spaceAfterPt = 0 } = {}): number` = `lines · sizePt · lineHeightCm + 2·insetYCm + (paragraphs − 1) · spaceAfterPt · 0.0353`。
- Produces: `export function fitSize(text, widthCm, maxHeightCm, sizes: number[], options?): { size, lines, height, fits }` — `sizes` 从大到小，取第一个 `height ≤ maxHeightCm` 的；都放不下返回最小字号且 `fits: false`。

- [ ] **Step 1: 写失败的测试**

- `a full-width line at 20pt holds about 40 CJK characters`：`lineCapacity(29.21, 20)` 在 40.5–41 之间。
- `six one-line bullets at 20pt need about 6.5 cm`：`heightCm(6, 20, { paragraphs: 6, spaceAfterPt: 6 })` 在 6.4–6.6。
- `mixed text counts Latin as half`：`lineCount("Transformer 入门", 10, 36)` 为 1；`lineCount("字".repeat(41), 29.21, 20)` 为 2。
- `fitSize steps down and reports when nothing fits`：`fitSize("字".repeat(240), 14.22, 11.7, [24, 22, 20, 18])` 返回 `size: 18, fits: true`；`maxHeightCm: 3` 时 `fits: false, size: 18`。

- [ ] **Step 2: 运行，确认失败** — `node --test test/deck-text.test.mjs`，模块不存在。
- [ ] **Step 3: 实现 `text.mjs`**（从 `schema.mjs` 导入 `textLength`）。
- [ ] **Step 4: 运行，确认通过**。
- [ ] **Step 5: Commit** — `feat(assistant): estimate slide text fit with officecli's checker constants`

---

### Task 3: navy 主题与排版原语

**Files:**
- Create: `assistant-runtime/src/deck/theme.mjs`、`assistant-runtime/src/deck/layout.mjs`
- Test: `assistant-runtime/test/deck-theme.test.mjs`

**Interfaces:**
- Produces `theme.mjs`：`export const THEMES = { navy }`，`export function themeFor(name): theme`（未知名回退 navy），`export function fontsFor(theme, language): { title, body, data, latinTitle, latinBody }`。主题对象：

```js
{ name: "navy", colors: { background: "FFFFFF", card: "E8EEF4", primary: "1E3A5F", secondary: "D4A84B", accent: "3498DB",
    text: "333333", textSecondary: "666666", textMuted: "999999", dark: "1E3A5F", onDark: "FFFFFF", onDarkMuted: "CADCFC",
    chart: ["1E3A5F", "D4A84B", "3498DB", "7F9CC0", "E5C87A", "8FC1E8"] },
  fonts: { latin: { title: "Arial Black", body: "Arial", data: "Arial" }, zh: { title: "Microsoft YaHei", body: "Microsoft YaHei" }, ja: { title: "Yu Gothic", body: "Yu Gothic" } },
  sizes: { hero: [60, 54, 48, 44], statement: [40, 36, 32], title: [44, 40, 36], subtitle: [24, 22, 20], body: [24, 22, 20, 18], card: [18, 16, 14], caption: [16, 14, 13], kpi: [72, 60, 48] },
  decor: { titleRule: "bar", radiusCm: 0.3, pageNumber: true } }
```

  `fontsFor`：`en` → title/body/data 都是拉丁字体；`zh`/`ja` → `title`/`body` 为 CJK 字体，`latinTitle`/`latinBody` 为拉丁字体（写进 `font.latin`），`data` 为 `Arial`。文件顶部注释写明数值来源 `OfficeCLI skills/morph-ppt/reference/styles/light--minimal-corporate + light--project-proposal (Apache-2.0)`。
- Produces `layout.mjs`：`export const CANVAS = { wCm: 33.867, hCm: 19.05 }`、`MARGIN_CM = 2.33`、`CONTENT = { x: 2.33, w: 29.21, top: 5.0, bottom: 17.78 }`、`TITLE_BOX = { x: 2.33, y: 1.0, w: 29.21, h: 3.67 }`、`GAP_CM = 0.76`；`export function columns(n, { x = CONTENT.x, w = CONTENT.w, gap = GAP_CM } = {}): { x, w }[]`；`export function slideCommand(n, { background, title }): command`（`add slide`，`title` 为 `props.title`）；`export function titleCommands(n, text, { theme, fonts, dark, size }): command[]`（`set /slide[n]/placeholder[title]` 带 `font`、`font.latin`、`size`、`bold`、`color`、`align`；`dark` 时颜色为 `onDark`）；`export function textboxCommand(n, name, text, { x, y, w, h, size, color, font, latin, bold, align, valign, list, spaceAfter, fill, radius }): command`（`add shape`，`name` 原样，长度统一写成 `"<n>cm"`，字号写成 `"<n>"`，`radius` 为 `preset: "roundRect"` + `adj: "adj:val 16667"`）；`export function notesCommand(n, text)`；`export function pageNumberCommand(n, total, theme, fonts)`（右下 13pt `textMuted`，名 `s<n>-page`）。所有命令只用 `add` / `set`，props 只用现有技能已列出的键加 `font.latin`、`adj`、`opacity`。

- [ ] **Step 1: 写失败的测试**

- `navy has every color role and six chart colors`：逐键断言存在且为 6 位十六进制；`chart.length === 6`。
- `fontsFor picks CJK fonts by language and keeps Latin in font.latin`：`zh` → `title === "Microsoft YaHei"`、`latinTitle === "Arial Black"`；`ja` → `Yu Gothic`；`en` → `title === "Arial Black"`、`latinTitle` 为 undefined。
- `dark fills get light text`：`themeFor("navy").colors.onDark === "FFFFFF"`；用 OfficeCLI 亮度公式断言 `dark` 亮度 < 128、`onDark` 亮度 ≥ 204。
- `columns divides the content width with 0.76 cm gaps`：`columns(3)` 第一列 `x 2.33`、`w ≈ 9.23`；`columns(4)` 的 `w ≈ 6.73`；最后一列右边 ≤ 31.54。
- `textboxCommand writes units and names`：返回 `{ command: "add", parent: "/slide[2]", type: "shape", props: { name: "s2-body", x: "2.33cm", size: "20", font: "Microsoft YaHei", "font.latin": "Arial", ... } }`。

- [ ] **Step 2: 运行，确认失败**。
- [ ] **Step 3: 实现 `theme.mjs` 与 `layout.mjs`**。
- [ ] **Step 4: 运行，确认通过**。
- [ ] **Step 5: Commit** — `feat(assistant): add the navy deck theme and slide layout primitives`

---

### Task 4: 渲染器——文字页型（hero / statement / pillars / grid / quote / cta）

**Files:**
- Create: `assistant-runtime/src/deck/render.mjs`
- Test: `assistant-runtime/test/deck-render.test.mjs`

**Interfaces:**
- Produces: `export function renderDeck(deck, theme): { commands: command[], slides: number }` — `deck` 是 `parseDeck` 的输出；按页序调用 `RENDERERS[slide.type](ctx, slide)`，`ctx = { n, total, theme, fonts, language }`；每页后追加 `notesCommand`（有 `notes` 时）和内容页的页码。
- Produces（本任务）：`renderHero`、`renderStatement`、`renderPillars`、`renderGrid`、`renderQuote`、`renderCta`，各返回 `command[]`。约定：
  - hero / cta：`background` 为 `colors.dark`，标题用 `sizes.hero` 中 `fitSize` 能一行放下的最大字号（`TITLE_BOX` 改为 `y 6cm h 4.5cm`），副标题 `subtitle` 24pt `onDarkMuted`，`meta` 16pt；居中。
  - statement：浅底；`statement` 用 `sizes.statement` 在 `w 29.21 h 6` 内 `fitSize`，居中垂直居中；`note` 20pt `textSecondary`。左侧 0.3 cm 宽 `primary` 色条（名 `s<n>-rule`）。
  - pillars：`columns(count)`；每栏一张 `card` 底（`roundRect`，`card` 色），`heading` 用 `sizes.subtitle`，`body` 用 `sizes.card` 的 `fitSize`，框高按估算；3 栏起始 16pt、4 栏 14pt（由 `fitSize` 自然得出）。
  - grid：≤ 4 一行，> 4 两行；每格 `heading` 16pt bold + `body` 14pt，格底 `card` 色。
  - quote：居中 `quote` 用 `[32, 28]` `fitSize`，`primary` 色；`source` 16pt `textMuted`；无其他装饰。
  - 标题（内容页）：`titleCommands` 用 `sizes.title` 里一行放得下的最大字号；`decor.titleRule === "bar"` 时标题下方 `y 4.4cm` 加 `primary` 色条 `s<n>-title-rule`（w 2cm h 0.12cm）。
  - 每个形状 `name` 为 `s<n>-<role>[-<i>]`，在一页内唯一。

- [ ] **Step 1: 写失败的测试**

- `renderDeck makes one slide per page in order`：hero + statement + pillars + grid + quote + cta 六页 → `slides === 6`，`add slide` 命令恰好 6 条且 `parent === "/"` 顺序对应。
- `every text shape carries explicit font, size and color`：遍历 `type === "shape"` 且有 `text` 的命令，断言 `props.font`、`props.size`、`props.color` 存在；`language: "zh"` 时 `props["font.latin"] === "Arial"`。
- `every shape stays inside the canvas`：解析 `x/y/width/height` 的 `cm`，断言 `x ≥ 0`、`x + w ≤ 33.867`、`y + h ≤ 19.05`。
- `names are unique within a slide`。
- `dark pages use light text`：hero 和 cta 的标题 `color === "FFFFFF"`，`add slide` 的 `background === "1E3A5F"`。
- `pillars at the limit fit without shrinking below 14pt`：4 栏、heading 12 字、body 60 字 → 正文 `size` ≥ 14 且每栏估算高度 ≤ `CONTENT.bottom − CONTENT.top`（用 `text.mjs` 复算）。
- `grid with eight items uses two rows`：第二行格子的 `y` 大于第一行。
- `notes become speaker notes`：`add notes` 的 `parent === "/slide[3]"`、`props.text` 原样。

- [ ] **Step 2: 运行，确认失败**。
- [ ] **Step 3: 实现 `render.mjs` 的框架与六个页型**（`RENDERERS` 表里另外四个先抛 `Error("not implemented")`，Task 5 补）。
- [ ] **Step 4: 运行，确认通过**。
- [ ] **Step 5: Commit** — `feat(assistant): render hero, statement, pillars, grid, quote and cta slides`

---

### Task 5: 渲染器——数据页型（evidence / timeline / comparison / showcase）

**Files:**
- Modify: `assistant-runtime/src/deck/render.mjs`
- Test: `assistant-runtime/test/deck-render.test.mjs`

**Interfaces:**
- Produces：`renderEvidence`、`renderTimeline`、`renderComparison`、`renderShowcase`。
  - evidence：`metrics` 横排 `columns(count)`，`value` 用 `sizes.kpi` 按 `count`（1–2 → 72，3 → 60，4 → 48）且 `fitSize` 校验一行放下，`label` 16pt，`note` 14pt `textSecondary`；有 `detail` 时指标占上半（h 6 cm），detail 在下方 18pt 列表（`list: "bullet"`，`spaceAfter: "6pt"`）。
  - timeline：`columns(count)` 编号卡片：圆形数字（`ellipse`，`primary` 底，白字 20pt bold，名 `s<n>-step-<i>-no`）+ `heading` 16pt bold + `body` 14pt；卡片之间 0.12 cm 高的 `secondary` 连线 `s<n>-track`（一条横线横贯内容区，y 为圆心高度，zorder 在前）。
  - comparison：`columns(2)`；每侧 `heading` 20pt bold 放在色条上（左 `primary` 底白字，右 `secondary` 底 `text` 字），下面 `points` 18pt 列表；中间 `s<n>-vs` 圆形 “VS”（`card` 底，`primary` 字）。
  - showcase：媒体区 `x 2.33 y 5 w 20 h 11`，右侧 `body` 区 `x 23.09 w 8.45` 18pt；`caption` 14pt `textMuted` 在媒体下方（y 16.3）。`media.image` → `add picture` `props.src` 为 data URI，`alt` 为 caption 或标题；`media.chart` → `add chart`：`chartType` = `bar`→`column`、`line`→`line`、`pie`→`pie`，`categories` 逗号连接，`data` = `name:values;...`，`colors` = `theme.colors.chart` 前 N 个逗号连接，`legend` = 系列 > 1 时 `bottom` 否则 `none`，`title: "none"`，`x/y/width/height`；`media.table` → `add table`：`data` 行 `;` 列 `,`（单元格含 `,` 或 `;` 时用 `"…"` 包住，`"` 翻倍）、`colWidths` 均分、`headerFill: primary`、`bodyFill: card`，再 `set` 表体字号（≤ 4 列 16、5 列 14、6 列 12）与 `tr[1]` 白字加粗。
- Produces：`renderDeck` 结束时断言 `commands.length ≤ 2000`，超出抛 `Error("deck too large: N commands")`（不应发生，30 页满额约 600 条）。

- [ ] **Step 1: 写失败的测试**

- `evidence picks the KPI size by count and fits the value`：4 个 `value: "118.5%"` → `size === "48"`；2 个 → `"72"`。
- `timeline draws one numbered card per step on a track`：6 步 → 6 个 `ellipse`、6 个 heading、1 条 `s<n>-track`，卡片 x 递增。
- `comparison puts left and right headings on contrasting bars`：左 `fill === primary`、右 `fill === secondary`。
- `showcase chart maps kinds, colors and data`：bar → `chartType: "column"`，`categories: "Q1,Q2"`，`data: "收入:12,15"`，`colors: "1E3A5F"`，`legend: "none"`；两个系列 → `legend: "bottom"`。
- `showcase table quotes cells with separators and sizes by column count`：6 列 → 表体 `size === "12"`；单元格 `a,b` 写成 `"a,b"`。
- `showcase image passes the data URI and an alt`。
- `thirty full slides stay within the batch limits`：30 页（十种页型循环，字段填到上限）→ `commands.length ≤ 2000`、`JSON.stringify(commands).length ≤ 2 × 1024 × 1024`。
- `every renderer fits its content at the limits`：十种页型各一页满额内容 → 对每个带 `text` 的 shape，用 `lineCount` / `heightCm` 复算高度 ≤ 框高（框高由渲染器写出）。

- [ ] **Step 2: 运行，确认失败**。
- [ ] **Step 3: 实现四个页型**。
- [ ] **Step 4: 运行，确认通过**；再跑 `node --test test/*.test.mjs | tail -3` 确认无回归。
- [ ] **Step 5: Commit** — `feat(assistant): render evidence, timeline, comparison and showcase slides`

---

### Task 6: 接入 `create_document`

**Files:**
- Modify: `assistant-runtime/src/document-tools.mjs`（`description()` :50-62、`guidelines()` :33-48、`contentProblem()` :65-74、执行体 :171-184）
- Test: `assistant-runtime/test/document-tools.test.mjs`、`assistant-runtime/test/tool-schema.test.mjs`（不改 schema，只确认仍通过）

**Interfaces:**
- Consumes: `deckOf`、`parseDeck`（Task 1），`themeFor`（Task 3），`renderDeck`（Task 4/5）。
- Produces: 执行体顺序：`contentProblem` 之前先 `const deck = args.format === "pptx" ? deckOf(args.content) : null`；有 deck 时 `parseDeck(deck)`（语言只看 deck 自己的 `language`，缺省 `zh`）：
  - `problems` → 返回 `{ content: [{ type: "text", text: "The deck was not rendered; fix these and send the whole deck again:\n- " + problems.join("\n- ") }], details: { code: "assistant_deck_invalid" }, isError: true }`，不调宿主。
  - 否则 `const { commands, slides } = renderDeck(parsed.deck, themeFor(parsed.deck.theme))`，以 `{ ...args, content: commands }` 调 `hostTool`；结果文本前面加一行 `Rendered ${slides} slides from the deck with theme ${theme}.`，有 `warnings` 时再加 `Ignored: …`。
  - 批处理数组（非 deck）路径不变。
- Produces: `description()` 的 pptx 一句改为 `pptx: content is a deck object ({"kind":"deck",…}, see the ofox-office-pptx skill) or, for experts, an array of officecli batch commands`；`guidelines()` 增一条 `For a pptx, send the deck object the skill describes; do not write slide coordinates yourself.`

- [ ] **Step 1: 写失败的测试**

在 `document-tools.test.mjs`：
- `a deck object is rendered before it reaches the host`：`execute` 传 `content: DECK_OBJECT` → `calls[0].args.content` 是数组且 `calls[0].args.content[0].type === "slide"`；结果文本以 `Rendered 3 slides` 开头。
- `a one-element array holding the deck is a deck too`：`content: [DECK_OBJECT]` 同上。
- `an invalid deck is refused without calling the host`：`type: "bullets"` → `isError`，`calls.length === 0`，文本含 `unknown type "bullets"`。
- `a batch array still goes through unchanged`：现有 `DECK` 批处理 → `calls[0].args.content` 与输入深等。
- `the description and guidelines teach the deck object`：`tool.description` 含 `deck object`；`promptGuidelines` 含 `do not write slide coordinates`。

- [ ] **Step 2: 运行，确认失败** — `node --test test/document-tools.test.mjs`。
- [ ] **Step 3: 实现**。
- [ ] **Step 4: 运行，确认通过** — `node --test test/document-tools.test.mjs test/tool-schema.test.mjs test/runtime.test.mjs | tail -3`。
- [ ] **Step 5: Commit** — `feat(assistant): render deck objects inside create_document`

---

### Task 7: 宿主质检闭环

**Files:**
- Create: `src-tauri/src/services/assistant/documents/fix.rs`
- Modify: `src-tauri/src/services/assistant/documents/mod.rs`（`mod fix;`）、`documents/create.rs`（`make_office` :2172-2224、`Made` :1967、`make_document` 结果 :2765-2821、测试夹具 `officecli()` :3057）
- Test: `fix.rs` 内 `#[cfg(test)]`；`create.rs` 的 `mod tests`

**Interfaces:**
- Produces `fix.rs`（纯函数）：
  - `pub(super) const SLIDE_W_CM: f64 = 33.867; SLIDE_H_CM = 19.05; EDGE_CM = 1.27; MIN_SIZE_PT = 13.0; SIZE_STEP_PT = 2.0; MAX_ROUNDS = 2`。
  - `pub(super) enum Edge { Left, Right, Top, Bottom }`
  - `pub(super) enum Kind { Overflow { size_pt: f64, suggest_cm: f64 }, OffSlide { edge: Edge, by_cm: f64 }, LowContrast, Other }`
  - `pub(super) struct ViewIssue { pub path: String, pub kind: Kind }`
  - `pub(super) fn parse_issue(issue: &Value) -> Option<ViewIssue>`：`subtype == "low_contrast"` → `LowContrast`；消息匹配 `text overflow: (\d+) lines at ([\d.]+)pt need (\d+)pt, usable (\d+)pt. suggest.height=([\d.]+)cm` → `Overflow`；匹配 `extends ([\d.]+)cm past slide (left|right|top|bottom) edge` → `OffSlide`；其余 `Other`；无 `path` 返回 `None`。
  - `pub(super) struct Geometry { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }`（cm）；`pub(super) fn length_cm(text: &str) -> Option<f64>`（`cm`、`pt`（÷ 28.3465）、`in`（× 2.54）、纯数字按 EMU ÷ 360000）；`pub(super) fn geometry(get_report: &Value) -> Option<Geometry>`（读 `data.results[0].format` 的 `x/y/width/height`）。
  - `pub(super) struct Fix { pub path: String, pub props: serde_json::Map<String, Value> }`；`pub(super) fn fix_for(issue: &ViewIssue, geometry: Option<&Geometry>) -> Option<Fix>`：
    - `Overflow`：`geometry.y + suggest_cm ≤ SLIDE_H_CM − EDGE_CM` → `height: "<suggest>cm"`；否则 `size_pt − SIZE_STEP_PT ≥ MIN_SIZE_PT` → `size: "<size − 2>"`；否则 `None`。
    - `OffSlide`：`Right` → `x: max(0, SLIDE_W − EDGE − width)`；`Left` → `x: EDGE`；`Bottom` → `y: max(0, SLIDE_H − EDGE − height)`；`Top` → `y: EDGE`；无 geometry 时 `None`。
    - `LowContrast` → `color: "FFFFFF"`。
    - `Other` → `None`。
  - `pub(super) fn fix_batch(fixes: &[Fix]) -> Vec<Value>`：每条 `{"command":"set","path":…,"props":…}`。
- Produces `create.rs`：
  - `Made` 增 `fixed: usize`。
  - `async fn fix_layout(work: &Work<'_>, file: &str, format: DocumentFormat, first: &Value) -> Result<(Vec<Issue>, usize), String>`：只对 `DocumentFormat::Pptx`，其余直接 `(view_issues(work, first), 0)`。循环最多 `MAX_ROUNDS` 轮：解析 issues → 需要几何的先 `work.report(&["get", file, path, "--json"])` → `fix_for` → 无可修的就停 → `check_batch` 校验修正批处理（复用模型批处理同一函数）→ 写 `fix-<round>.json`（`write_new`）→ `work.run(&["batch", file, "--input", "fix-<round>.json", "--json"])` → 再 `view … issues --json`；累计 `fixed` = 应用过的修正条数。结束返回最后一次 issues 的 warnings。
  - `make_office` 用 `fix_layout` 替换原来的单次 `view_issues` 调用（`validate` 的 errors 照旧在前）。
  - 结果 JSON 与 `MediaRecord.params` 增 `"fixed": n`；运行时 `describe()` 在 `fixed > 0` 时加一句 `Auto-fixed N layout issues.`（`document-tools.mjs:105-120`）。
- 测试夹具：`officecli()`（create.rs:3057）增参数或新函数 `officecli_rounds(rounds: &[&str], stats: &str, geometry: &str)`：stub 用 `$dir/issues.calls` 计数，第 k 次 `view … issues` 打印 `rounds[k]`（越界打印最后一个）；`get` 打印 `geometry`；每次 `batch --input <f>` 把 `<f>` 追加到 `$dir/batches.log`。

- [ ] **Step 1: 写失败的单元测试（fix.rs）**

- `parses_an_overflow_with_its_suggested_height`：用探测到的原文 `text overflow: 7 lines at 20.0pt need 168pt, usable 49pt. suggest.height=6.2cm` → `Overflow { size_pt: 20.0, suggest_cm: 6.2 }`，`path == "/slide[1]/shape[@id=100000]"`。
- `parses_off_slide_and_low_contrast`。
- `length_cm_reads_pt_cm_in_and_emu`：`"66pt"` ≈ 2.328、`"29.21cm"`、`"1in"` = 2.54、`"360000"` = 1。
- `overflow_grows_the_box_when_it_fits_the_page`：y 5.2、suggest 6.2 → `height: "6.2cm"`。
- `overflow_past_the_bottom_steps_the_size_down_to_13`：y 12、suggest 7 → `size: "18"`；size 13 → `None`。
- `off_slide_clamps_inside_the_edge`：右越 2.1、width 6 → `x` 为 `26.597`（四舍五入到 3 位）。
- `low_contrast_turns_the_text_white`。

- [ ] **Step 2: 写失败的集成测试（create.rs `mod tests`）**

- `overflow_is_fixed_in_one_round_and_reported_as_fixed`：rounds = [一条 overflow, 空] → 结果 `issues == []`、`fixed == 1`，`batches.log` 第二行是 `fix-1.json`，其内容是 `[{"command":"set","path":"/slide[1]/shape[@id=100000]","props":{"height":"6.2cm"}}]`。
- `fixing_stops_after_two_rounds_and_keeps_the_rest`：rounds = [overflow, overflow, overflow] → `fixed == 2`，`issues` 含那条 warning，`batches.log` 恰好 3 行（原批 + 2 轮）。
- `fix_commands_target_the_reported_path_only`：两条 issues 不同 path → 修正批处理的 path 集合等于 issues 的 path 集合。
- `non_pptx_formats_are_not_fixed`：docx 同样 rounds → `fixed == 0`，`batches.log` 1 行。
- `fix_batches_pass_the_validator`（fix.rs 单元测试）：四种修正各一条经 `fix_batch` 后调用 `check_batch(&commands, DocumentFormat::Pptx)` 都返回 `Ok`；`fix_layout` 在 `check_batch` 返回 `Err` 时不运行修正、把该轮 issues 原样留作 warning（用 create.rs 测试 `a_refused_fix_batch_leaves_the_issues_as_warnings`：stub 的 issue path 为空字符串 → `parse_issue` 返回 `None` → 无修正、`fixed == 0`、warning 保留）。

- [ ] **Step 3: 运行，确认失败**

Run: `pnpm assistant:prepare && cargo test --manifest-path src-tauri/Cargo.toml documents:: 2>&1 | grep -E "FAILED|error\[" | head`
Expected: 新测试失败（符号未定义）。

- [ ] **Step 4: 实现 `fix.rs` 与 `fix_layout`，接入 `make_office`、结果 JSON、运行时 `describe()`**。
- [ ] **Step 5: 运行，确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml documents:: 2>&1 | grep "test result"` 和 `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings && cargo fmt --check --manifest-path src-tauri/Cargo.toml`；`pnpm assistant:test | tail -3`。
Expected: 全部通过，无警告。

- [ ] **Step 6: Commit** — `feat(assistant): auto-fix slide overflow, off-slide shapes and low contrast after officecli checks`

---

### Task 8: 全页 SVG 并随附件保存

**Files:**
- Modify: `src-tauri/src/services/assistant/documents/create.rs`（`svg_preview` :2417-2440 → `svg_pages`；`Made.preview` → `pages: Vec<PathBuf>`；`make_document` :2765-2821）
- Modify: `src-tauri/src/services/assistant/attachments.rs`（`generated_file` :636-718 之后保存 pages；删除附件处一并删 `<id>.pages`）
- Test: `create.rs` `mod tests`；`attachments.rs` 测试

**Interfaces:**
- Produces: `async fn svg_pages(work: &Work<'_>, file: &str, count: u64) -> (Vec<PathBuf>, Vec<Issue>)`：对 `n in 1..=count` 运行 `view <file> svg --start n --end n`，按 `svg_preview` 的完整性判定写 `work.dir/pages/<n>.svg`；某页不完整就跳过并记一条 warning `slide n has no preview`（不中断后续页）。`Made.preview` 保留为 `pages/1.svg` 的路径（运行时缩略图不变），新增 `Made.pages: Vec<PathBuf>`。
- Produces: `attachments::keep_pages(store, sid, id, pages: &[PathBuf]) -> Result<usize, String>`：复制到 `store.attachment_dir(sid)?.join(format!("{id}.pages"))/<n>.svg`（目录 0700、文件 0600，用 `FileTxn`），返回份数；`make_document` 在 `generated_file` 之后调用，`params["svgPages"] = n` 写回记录（通过 `store.mutate` 更新该 generation 的 params，或在 `generated_file` 前就把 `svgPages` 放进 `record.params`，实现者选后者更简单：先导出、再导入）。
- Produces: 删除附件的函数（搜 `attachment_dir(` 在 `attachments.rs` / `store.rs` 里的删除调用）同时 `remove_dir_all(<id>.pages)`，不存在不报错。

- [ ] **Step 1: 写失败的测试**

- create.rs：`every_slide_gets_an_svg_page`：stub 的 `view … svg --start n --end n` 打印 `<svg>n</svg>`，stats 报 3 页 → `pages` 3 个文件，内容分别含 1/2/3。
- `an_oversized_page_is_skipped_with_a_warning`：第 2 页打印 300 KiB 的 SVG → `pages` 为 2 份（1、3），`issues` 含 `slide 2 has no preview`。
- attachments.rs：`pages_are_kept_beside_the_attachment_and_removed_with_it`：`keep_pages` 后 `<id>.pages/1.svg` 存在且 mode 0600；删除附件后目录不存在。

- [ ] **Step 2: 运行，确认失败**。
- [ ] **Step 3: 实现**。
- [ ] **Step 4: 运行，确认通过** — `cargo test --manifest-path src-tauri/Cargo.toml assistant:: 2>&1 | grep "test result"`，clippy、fmt。
- [ ] **Step 5: Commit** — `feat(assistant): export every slide as SVG and keep the pages beside the attachment`

---

### Task 9: 技能改写与真实引擎 e2e

**Files:**
- Modify: `assistant-runtime/skills/ofox-office-pptx/SKILL.md`（整体改写，≤ 12000 字符，description ≤ 150）
- Modify: `assistant-runtime/test/bundled-skills.test.mjs`（:96-107 的 json 块检查）
- Create: `src-tauri/tests/fixtures/assistant-documents/deck-object.json`、`deck-showcase.json`、`deck-unknown-type.json`、`deck-too-long.json`
- Modify: `src-tauri/tests/assistant_host.rs`（`mod documents` :6989 起）

**Interfaces:**
- Produces SKILL.md 结构（顺序固定）：
  1. 开头一行仍以 `` `create_document({ name: "deck.pptx", format: "pptx", content: { "kind": "deck", … } })` `` 开始（e2e 断言技能文本含 `create_document({ name: `）。
  2. **Ground rules**：模型只写内容，不写坐标、字号、颜色；一次一个调用；标题先行；一页一个论点；列表用 `pillars`（2–4）或 `grid`（3–8），没有 bullets 页型；相邻两页页型不同；内容页写 `notes`；语言跟用户；超限会被拒绝，拆页而不是精简到失真。
  3. **Narrative**：四种结构各一行（problem_solution：hero → statement → pillars → evidence → cta；insight_driven：hero → statement → evidence → pillars → cta；vision_driven：hero → quote → pillars → evidence → cta；educational：hero → statement → pillars → pillars → showcase → cta）；页数 3–5 / 5–8 / 10–15，最多 30。
  4. **Page types**：一张表（页型 | 用途 | 字段 | 上限），数值与 `LIMITS` 一致。
  5. **Deck fields**：`kind`、`title`、`language`（zh / en / ja）、`theme`（`navy`；省略即可）、`slides[]`、`notes`。
  6. **Worked example**：8 页 Transformer（hero、statement、pillars、timeline、comparison、evidence、grid、cta），一个 ```json 块，内容就是 `deck-object.json`。
  7. **Expert mode**：压缩的批处理说明（现有 Commands 一节删到只剩 slide/title/shape/table 四个最小例子），说明只在 deck 做不到时使用。
- Produces `bundled-skills.test.mjs`：json 块若是对象且 `kind === "deck"`，用 `parseDeck` 校验 `problems` 为空；否则沿用数组 + `command` 检查。
- Produces fixtures：`deck-object.json` 8 页，标题含 “Transformer 入门”“Q1 收入 +18%” 这类中英混排，每页 `notes`；`deck-showcase.json` 3 页 showcase（chart bar 两系列、table 4×5、image 为 1×1 像素 PNG 的 data URI）加 hero 和 cta；`deck-unknown-type.json`（`type: "bullets"`）；`deck-too-long.json`（pillars body 61 字）。
- Produces e2e（`mod documents`，复用 `create_document(bundle, model, skill, args)` :7137 与 `Outcome::made/refused`）：
  - `renders_a_deck_into_a_pptx_with_no_issues`：`made(PPTX)` 后断言 generation `params.slides == 8`、`params.issues == []`、`params.fixed == 0`、`params.svgPages == 8`；`stored().join(format!("{id}.pages"))` 下 `1.svg`…`8.svg` 都以 `<svg` 开头；`sent()` 含 `Rendered 8 slides from the deck with theme navy`。
  - `renders_showcase_media_without_issues`：`deck-showcase.json` → `slides == 5`、`issues == []`。
  - `rejects_a_deck_with_an_unknown_page_type`：`refused()` 文本含 `unknown type "bullets"` 和 `hero, statement`。
  - `rejects_an_overlong_deck_without_a_file`：文本含 `is 61 characters; the limit is 60`。

- [ ] **Step 1: 改 `bundled-skills.test.mjs`，运行确认现有技能仍通过**。
- [ ] **Step 2: 改写 SKILL.md；运行 `node --test test/bundled-skills.test.mjs test/skills.test.mjs`**，Expected: 通过，body 长度 ≤ 12000（测试输出里看不到长度时用 `wc -m`）。
- [ ] **Step 3: 写 fixtures 与四个 e2e，运行确认前两个失败**（在 Task 6–8 已合入的前提下它们应直接通过；若失败，先修渲染器而不是放宽断言——`issues == []` 和 `fixed == 0` 是本切片的验收）。

Run: `pnpm assistant:prepare && cargo test --manifest-path src-tauri/Cargo.toml --test assistant_host documents -- --nocapture 2>&1 | grep -E "^test |test result"`
Expected: 12 个测试 `ok`。

- [ ] **Step 4: 如 `issues` 非空**：读失败输出里的 issue 原文，调整 `theme.sizes` / `text.mjs` 系数或该页型的框高，重跑；每次调整都把原因写进 `render.mjs` 的注释。
- [ ] **Step 5: 全量门禁**

Run: `pnpm assistant:test | tail -3 && pnpm typecheck && pnpm format:check && cargo fmt --check --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings && cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | sort | uniq -c`
Expected: 全部 ok。

- [ ] **Step 6: Commit** — `feat(assistant): teach the pptx skill the deck object and prove it end to end`（技能、测试、fixtures 一起）

---

### Task 10: 真实模型回归最小版

**Files:**
- Modify: `src-tauri/tests/assistant_host.rs`（新 `mod real_eval`；`TestEnv` 增 `set_key(&self, key: String)`）
- Create: `scripts/assistant-real-eval.mjs`
- Modify: `package.json`（`"assistant:real-eval": "node scripts/assistant-real-eval.mjs"`）、`.gitignore`（`.real-eval/`）

**Interfaces:**
- Produces `mod real_eval`：`#[test] #[ignore] fn four_prompts_on_the_listed_models()`。读环境变量：`OFOX_REAL_EVAL_KEY`（缺则 `eprintln!` 后 `return`）、`OFOX_REAL_EVAL_APEX`（默认 `ofox.ai`）、`OFOX_REAL_EVAL_MODELS`（JSON 数组，每项是 `test_model()` 同形的模型对象）、`OFOX_REAL_EVAL_OUT`（目录）。对每个模型 × 四个提示（`写一个 pdf，介绍四川` / `做一个 8 页的 Transformer 介绍 ppt` / `做一张三科成绩表` / `写一份 Word 版的周报`）：新 `TestEnv`（`gateways` 为 `[("ofox.ai", &Gateway::start())]` 占位，随后 `env.route(apex, format!("https://api.{apex}"))`，`env.set_key(key)`，`*env.catalog = vec![model]`），`create(&host)`，`turn(&host, &env, request(&sid, prompt, json!({})))`，取 `detail`。每个用例打印一行 JSON：`{"case","model","ok","files","warnings","fixed","calls","seconds","tokens":{"input","output"},"note"}`：`files` = `detail.attachments.len()`；`warnings` = 该 generation `params.issues` 长度；`fixed` = `params.fixed`；`calls` = `tool_end` 里 `name == "create_document"` 的次数；`tokens` 从 `detail.usage["requests"]` 各项的输入 / 输出 token 字段累加（字段名见 `ordinary_queue_sends_keep_distinct_runs_context_and_usage_on_both_apexes` 里的断言）；`ok` = `files == 1 && warnings == 0 && calls == 1 && 文件无字面 \n`（Office 文件用 `engines_at(bundle).officecli` 跑 `view <file> text` 断言不含 `\\n`；PDF 跳过这一项）。附件与 `.pages` 复制到 `OUT/<model slug>/<case>/`。任何 panic 都换成 `ok: false` 加 `note`，让 12 个用例跑完。
- Produces `scripts/assistant-real-eval.mjs`：选项 `--apex`（默认 ofox.ai）、`--account`（默认 `assistant-key`）、`--models`（逗号分隔的 id 模式，默认 `terra,luna,deepseek-v4.1-flash`）、`--out`（默认 `.real-eval/<ISO 时间>`）。步骤：1）key：`OFOX_API_KEY` 环境变量，否则 `security find-generic-password -s ai.ofox.desktop.apikey -a <account> -w`（`execFileSync`，不打印）；2）`GET https://api.<apex>/v1/models` 用 `Authorization: Bearer` 取目录，每个模式取 id 含该模式的第一个模型，缺一个就退出码 2 并列出候选；3）`spawn("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--test", "assistant_host", "real_eval", "--", "--ignored", "--nocapture", "--test-threads=1"], { env: { ...process.env, OFOX_REAL_EVAL_KEY, OFOX_REAL_EVAL_APEX, OFOX_REAL_EVAL_MODELS, OFOX_REAL_EVAL_OUT } })`；4）收集 stdout 里的 JSON 行，写 `OUT/report.md`：表头 `| 日期 | 模型 | 用例 | 文件数 | 警告 | 自动修正 | 调用次数 | 秒 | tokens | 结果 |`，末尾 `通过 N / 12`；5）退出码：失败 ≤ 1 → 0，否则 1。key 不进日志、不进 report。

- [ ] **Step 1: 写 `set_key` 与 `mod real_eval`**，`cargo test --test assistant_host real_eval -- --ignored` 在无 key 时打印 skipping 并通过。
- [ ] **Step 2: 写脚本**；`node scripts/assistant-real-eval.mjs --help` 列出选项；无 key 时退出码 2 并说明。
- [ ] **Step 3: 本机实跑**（key 在钥匙串时由实现者跑，否则由用户跑）：`pnpm assistant:real-eval`，Expected: `report.md` 12 行，通过 ≥ 11。失败的行把 `note` 和 OUT 里的文件交给用户看，不改断言。
- [ ] **Step 4: Commit** — `test(assistant): add the real-model regression script for the four acceptance prompts`（report 不提交）

---

### Task 11: 文档与路线图

**Files:**
- Modify: `docs/assistant-engines.md`（“Formats and content” :214 加 deck 对象；“Previews” :339 写 `.pages`；“Acceptance on a real key” :354 增 `pnpm assistant:real-eval` 的用法和报告粘贴位置）
- Modify: `docs/assistant-roadmap.md`（切片表 M3a-1 → `done`，第 3 项最小版 → `done`，日期）
- Modify: `assistant-runtime/notices/officecli-skills.txt`（加 morph-ppt 的来源行）

- [ ] **Step 1: 改三份文档**。
- [ ] **Step 2: `pnpm assistant:test | tail -3`**（packaging 测试检查 notices）。
- [ ] **Step 3: Commit** — `docs(assistant): document the deck object, slide pages and the real-model eval`

---

## 验收（本切片）

- 门禁全绿：`pnpm typecheck`、`format:check`、`assistant:prepare`、`assistant:test`、`assistant:smoke`、`cargo fmt --check`、`clippy -D warnings`、`cargo test`。
- 假模型 e2e：deck → 8 页 PPTX、`validate` 通过、issues 为零、`fixed` 为零、`.pages/1..8.svg` 存在；showcase 三种媒体零 issues；非法页型、超限内容被拒绝且无文件。
- 真实脚本 12 次：每次一份、零警告、无字面 `\n`，最多失败 1 次；报告贴进 `docs/assistant-engines.md`。
- 用户本机：`做一个 8 页的 Transformer 介绍 ppt` 在三个模型上各出一份，“保存并打开”无告警，观感一致。
