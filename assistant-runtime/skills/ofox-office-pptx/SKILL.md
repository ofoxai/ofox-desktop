---
name: ofox-office-pptx
description: Read before making a PowerPoint deck (.pptx) with create_document. Python deck code with a preview step; a deck object where Python is unavailable.
---

If create_document offers `{"kind":"python","code":"…"}` content, make the deck as Python code: everything down to "When Python is unavailable". If it does not (no Python, no `preview`), send a deck object: go straight to "When Python is unavailable" and ignore the code, helper and preview parts.

## Python deck code

`create_document({ name: "deck.pptx", format: "pptx", content: { "kind": "python", "code": "…" }, preview: true })`. The host runs `code` in a sandbox with python-pptx, matplotlib and numpy plus the helpers below, saves the deck it builds, checks it and draws its slides. `code` is plain source, not a Markdown block; `\n` inside a string starts a new paragraph.

- Already imported: the helpers, `T`, `G`, `Inches`, `Pt`, `Emu`, `RGBColor`, `MSO_SHAPE`, `PP_ALIGN`, `CategoryChartData`, `XyChartData`, `XL_CHART_TYPE`, `XL_LEGEND_POSITION`, `XL_LABEL_POSITION`, `plt`, `np`.
- Start with `prs = new_deck(title="…", footer="…")`. Make every slide with `cover_slide`, `section_slide` or `content_slide`: the template has only those three layouts, so never index `prs.slide_layouts[...]`.
- Never call `prs.save` or `sys.exit`; never use files, network, subprocess or pip.
- One deck, one file: preview it, then deliver it; never make a second file to patch the first.
- Start the code with a plan comment: one line per slide, its point, its visual, its layout. Read in order, the points should tell the story.
- All text and notes in the user's language. No placeholders, no invented numbers: use the user's, or compute from them.

## Preview, then deliver

1. Send the code with `preview: true`. Nothing is delivered: you get a `previewId`, the issues and an overview image of the slides, each under its slide number.
2. Look at the image and read the issues: clipped or overlapping text, empty areas, small text, a plot or figure out of place. Fix the code.
3. Deliver under the same file name by calling again without `preview` (the code may have changed), or with `fromPreview: "<previewId>"` and no content to deliver that preview as it is. A file may be previewed twice per reply; if the reply ends without delivering, the latest preview is delivered automatically.

A Python error comes back as the end of the traceback and nothing is made: fix that line and resend (at most 2 retries). After delivery, tell the user what the deck contains and mention any issue left.

## Helpers

Positions and sizes in inches, type in pt. 16:9, 13.333 × 7.5. `G`: content area x `G.X` 0.72 to `G.R` 12.613 (width `G.CW` 11.893), y `G.TOP` 1.7 to `G.BOTTOM` 6.85, gap between blocks `G.GAP` 0.3. `T`: colours as hex strings without "#": `primary`, `primaryDark`, `accent`, `text`, `muted`, `faint`, `line`, `tint`, `accentTint`, `white`; `T.font`, `T.mono`.

- `new_deck(title="", footer="")` returns `prs`; `footer` is the text on every content slide.
- `cover_slide(prs, title, subtitle="", kicker="", meta="", notes="", art=None)`: dark cover; `art` is an optional SVG for the right side (a 4.6 × 5.5 box, viewBox 883 × 1056).
- `section_slide(prs, title, number="", subtitle="", notes="")`: dark section break or one key statement.
- `content_slide(prs, title, subtitle="", notes="")` returns the slide and draws the header (accent tick, title, hairline at y 1.42, footer, page number): never redraw it. Title at most 22 CJK characters; subtitle the slide's point in one sentence, at most 40.
- `text(slide, content, x, y, w, h, size=13, color=None, bold=False, align="left", valign="top", font=None, spacing=1.15, bullet=False, gap=0)`: `content` is a string (`\n` = new paragraph) or a list of strings or `{"text", "bold", "color"}`; `align` left, center or right; `valign` top, middle or bottom; `color` hex. No fit check.
- `bullets(slide, items, x, y, w, h, size=13, color=None, gap=6)`: real bullets from strings or `{"text"}`.
- `card(slide, x, y, w, h, title, body="", points=None, highlight=False, size=13)`: tinted box with a heading and a body or bullet points; `highlight` uses the accent.
- `kpi(slide, x, y, w, h, value, label, note="", highlight=False)`: a big number; with a note, h at least 2.0.
- `steps(slide, items, x, y, w, h, highlight=-1, size=12)`: items `{"title", "body"}`; boxes with arrows.
- `timeline(slide, items, x, y, w, h, highlight=-1, size=12)`: items `{"label", "title", "body"}`.
- `callout(slide, y, message, x=None, w=None, h=0.62)`: a one-line takeaway bar, full content width by default.
- `formula(slide, x, y, w, h, lines, size=15)`: monospace lines, for a formula with a worked example. No fit check.
- `shape(slide, kind, x, y, w, h, fill=None, line=None, line_w=0.75, radius=None)`: `kind` is `MSO_SHAPE.RECTANGLE`, `ROUNDED_RECTANGLE`, `OVAL` …; `radius` only for rounded.
- `arrow(slide, x1, y1, x2, y2, color=None, width=1.5)`
- `figure(slide, svg, x, y, w, h, alt="figure")`: an SVG figure, for diagrams with labelled boxes and arrows.
- `plot(slide, fig, x, y, w, h, alt="chart")`: a matplotlib figure resized to the box, for curves, heatmaps, scatter, distributions.
- `background(slide, color)`: colours the whole slide; cover and section slides already use `T.primaryDark`.
  On a content slide with a dark background the layout's navy title disappears and its accent tick and hairline are covered: recolour the title with `for r in s.shapes.title.text_frame.paragraphs[0].runs: r.font.color.rgb = rgb(T.white)`, leave `subtitle=""` and write the subtitle with the `text` helper in a light colour such as "DCE6F0", and give all other text a light colour.
- Native chart: `chart = slide.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(x), Inches(y), Inches(w), Inches(h), chart_data).chart`; set `chart.font`, the axis tick label fonts and the data label font to `Pt(12)`, and colour the series from `T`.
- Native table: `table = slide.shapes.add_table(rows, cols, Inches(x), Inches(y), Inches(w), Inches(h)).table`; fill each `table.cell(r, c)` from `T` (header `T.primary` with white text) and set every run's `font.size` to `Pt(12)` or more and `font.name` to `T.font`.
- Fit estimates (CJK 1 em, Latin 0.55 em): `text_height(text, pt, width_in, spacing=1.3)`, `lines_for(text, pt, width_in)`, `em_width(s)`, `bullets_height(items, pt, width_in, gap_pt=6)`, `need(what, needed, has)` (adds a lint line when `needed` exceeds `has`), `rgb(h)` (hex to colour).

## Design rules

1. Every content slide has a title, a subtitle that states its point, and 2–4 information zones (figure + cards, formula + plot + bullets, kpis + chart, steps + cards + callout …) that fill the content area: blocks span the full width and reach y 6.3–6.85. No small row of cards over empty space; no figure floating in a large empty box.
2. Every content slide has one visual that explains: a plot, an SVG figure, a native chart, steps or a timeline, a table, or a formula with a worked example. Architecture: block diagram. Relations or weights: heatmap with the numbers in the cells. Numbers: chart or kpis. Process: steps. History: timeline.
3. No two adjacent slides share a layout. A deck of 8 or more slides has at least one `section_slide` and one number-forward slide (kpis or a chart).
4. Colour: primary for structure, grays for text, the accent only for the one thing to notice on a slide. In figures and plots, primary against accent (and their tints) separates two groups.
5. Type: title 26 (fixed), subtitle 13, headings 14–16, body 12–14, captions 11; nothing below 10 pt, figures and plots included. Bullets only through `bullets()` or `card(points=…)`, never a typed "•".
6. Fit: a CJK character is about 1.0 × the font size wide, a Latin one 0.55 ×; a line is 1.3 × tall; 1 pt = 1/72 in. Keep 10% slack. Shorten the text rather than shrink it.
7. Be concrete: real numbers, names and short worked examples.
8. Speaker notes: 1–3 sentences per slide, in `notes=`.

## SVG figures

- `viewBox` = the box at 192 px per inch: `0 0 {w*192} {h*192}`, rounded (another ratio is stretched). 10 pt = 27 px, 12 pt = 32 px, 14 pt = 37 px: the smallest text is 28 px.
- Fill the viewBox, at most 5% margin. Set `font-family="Microsoft YaHei, PingFang SC, sans-serif"` on the root `<svg>` and `font-size`, `fill`, `text-anchor` on every `<text>`. Colours are "#" + `T.x` (`#{T.primary}` in an f-string).
- Allowed: `rect`, `circle`, `ellipse`, `line`, `polyline`, `polygon`, `path`, `text`, `tspan`, `g`, `defs`, `linearGradient`. No images, `foreignObject`, CSS or scripts. A `marker` is not drawn: make arrowheads with a `polygon`. Escape `&` and `<` in text.

## matplotlib

- Make the figure at the box's size, `fig, ax = plt.subplots(figsize=(w, h), layout="constrained")`; `plot()` resizes it to the box and closes it, so never `plt.show()` or `savefig`.
- Colours "#" + `T.x`; `fontsize` 11 or more on everything (title, labels, `ax.tick_params(labelsize=11)`, annotations, legend); light grid; no top or right spines. The CJK font is already set.

## Lint lines

The helpers report problems in the issues. Fix each in the code and look again.

- `… text needs 1.20 in, box has 0.80 in` (title, subtitle, bullets, card, steps, timeline, kpi note, callout): the text does not fit. Shorten it, enlarge the box, or go down one size step; body text stays at 12 pt or more.
- `… the box is 4.00 in tall but its text needs 1.50 in; lower h or add content` (a card; steps say "boxes are … lower h or add a body line"): the box is more than half empty. Lower `h` and use the space for another zone, or add content.
- `figure …: viewBox … does not match the box …; it will be stretched`: set the viewBox to `{w*192} × {h*192}`.
- `figure …: smallest text is 8.0 pt on the slide; use at least 10 pt`: raise `font-size` to 28 px or more.
- `plot …: tick labels are 8 pt; use at least 10 (…)`: raise that group (title, axis labels or tick labels) as the line shows, to 11 or more.
- Other issues are the host's checks of the finished file (overflow, overlap): find the slide in the overview and fix its layout.

`text()`, `formula()` and native charts are not checked: estimate their fit yourself (rule 6).

## Worked example

「讲一下缓存命中率怎么影响接口速度。数据：命中 5 ms，未命中 100 ms。」: four slides, each with its own layout and one visual. The user gave the two times; every other number is computed from them. A deck of 8 or more slides also needs a `section_slide`; slide 4 is the number-forward one.

```python
# 计划（用户给的数据：缓存命中 5 ms，未命中 100 ms）
# 1 封面 | 论点：缓存命中率与接口速度 | 视觉：深色封面 | 版式：cover
# 2 请求怎么走 | 命中直接返回，未命中才查数据库 | SVG 流程图 | 图 + 两张卡片
# 3 命中率的影响 | 平均耗时随命中率直线下降 | 曲线 + 带代入的公式 | 图 + 公式 + 卡片 + 提示条
# 4 三个数字 | 命中率 90% 到 99%，耗时 14.5 ms 到 5.95 ms | 数字卡 + 原生柱状图 | 三卡 + 图 + 卡片
prs = new_deck(title="缓存命中率与接口速度", footer="后端团队分享")
cover_slide(prs, "缓存命中率与接口速度", subtitle="两个耗时，算出平均耗时", kicker="技术分享", meta="后端团队分享",
            notes="先说结论：接口快不快，主要看缓存命中率。")

s = content_slide(prs, "请求怎么走", subtitle="命中直接返回，未命中才去查数据库",
                  notes="命中只要 5 ms，未命中要 100 ms，差了 20 倍。")
svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1267 941" font-family="Microsoft YaHei, PingFang SC, sans-serif">
<rect x="40" y="360" width="250" height="220" rx="18" fill="#{T.tint}" stroke="#{T.primary}" stroke-width="3"/>
<text x="165" y="484" font-size="44" fill="#{T.primary}" text-anchor="middle" font-weight="bold">请求</text>
<rect x="430" y="360" width="260" height="220" rx="18" fill="#{T.primary}"/>
<text x="560" y="484" font-size="44" fill="#{T.white}" text-anchor="middle" font-weight="bold">缓存</text>
<rect x="880" y="40" width="347" height="220" rx="18" fill="#{T.accentTint}" stroke="#{T.accent}" stroke-width="3"/>
<text x="1053" y="164" font-size="40" fill="#{T.accent}" text-anchor="middle" font-weight="bold">返回 5 ms</text>
<rect x="880" y="680" width="347" height="220" rx="18" fill="#{T.tint}" stroke="#{T.primary}" stroke-width="3"/>
<text x="1053" y="804" font-size="40" fill="#{T.primary}" text-anchor="middle" font-weight="bold">数据库 100 ms</text>
<line x1="295" y1="470" x2="412" y2="470" stroke="#{T.primary}" stroke-width="5"/><polygon points="430,470 405,457 405,483" fill="#{T.primary}"/>
<path d="M690,470 H780 V150 H862" fill="none" stroke="#{T.accent}" stroke-width="5"/><polygon points="880,150 855,137 855,163" fill="#{T.accent}"/>
<path d="M690,470 H780 V790 H862" fill="none" stroke="#{T.primary}" stroke-width="5"/><polygon points="880,790 855,777 855,803" fill="#{T.primary}"/>
<text x="800" y="330" font-size="34" fill="#{T.accent}" text-anchor="start">命中</text>
<text x="800" y="640" font-size="34" fill="#{T.muted}" text-anchor="start">未命中</text>
</svg>'''
figure(s, svg, 0.72, 1.7, 6.6, 4.9, alt="请求经过缓存的流程")
card(s, 7.62, 1.7, 4.993, 2.3, "命中", points=["缓存里已有结果", "5 ms 返回", "不碰数据库"], highlight=True)
card(s, 7.62, 4.3, 4.993, 2.3, "未命中", points=["缓存里没有", "去数据库查，100 ms", "结果写回缓存"])

s = content_slide(prs, "命中率的影响", subtitle="平均耗时随命中率直线下降",
                  notes="用公式代一个数：命中率 90% 时平均 14.5 ms。")
fig, ax = plt.subplots(figsize=(6.9, 4.2), layout="constrained")  # the plot box's size
rate = np.linspace(0, 1, 101)
ax.plot(rate * 100, 100 - 95 * rate, color="#" + T.primary, linewidth=2.5)
ax.scatter([90], [14.5], color="#" + T.accent, s=90, zorder=3)
ax.annotate("90%: 14.5 ms", (90, 14.5), (45, 35), color="#" + T.accent, fontsize=13,
            arrowprops=dict(arrowstyle="->", color="#" + T.accent))
ax.set_xlabel("缓存命中率 (%)", fontsize=12); ax.set_ylabel("平均耗时 (ms)", fontsize=12)
ax.tick_params(labelsize=11); ax.grid(color="#" + T.line)
for side in ("top", "right"): ax.spines[side].set_visible(False)
plot(s, fig, 0.72, 1.7, 6.9, 4.2, alt="平均耗时与命中率的关系")
formula(s, 7.92, 1.7, 4.693, 2.4, ["平均 = 命中率 × 5", "     + (1 - 命中率) × 100", "90%: 0.9×5 + 0.1×100", "     = 14.5 ms"])
card(s, 7.92, 4.4, 4.693, 1.5, "怎么读", body="曲线是公式的图；橙点是 90% 命中率。")
callout(s, 6.15, "命中率每提高 1 个百分点，平均耗时少 0.95 ms")

s = content_slide(prs, "三个数字", subtitle="命中率从 90% 到 99%，平均耗时从 14.5 ms 到 5.95 ms",
                  notes="最后一个数字最值得记：99% 命中率只要 5.95 ms。")
for i, (value, label, note) in enumerate([("52.5 ms", "命中率 50%", "一半请求要查库"), ("14.5 ms", "命中率 90%", "多数请求走缓存"), ("5.95 ms", "命中率 99%", "几乎全部走缓存")]):
    kpi(s, 0.72 + i * 4.064, 1.7, 3.764, 2.1, value, label, note=note, highlight=i == 2)  # the accent: the one to notice
data = CategoryChartData(); data.categories = ["50%", "80%", "90%", "99%"]; data.add_series("平均耗时 (ms)", (52.5, 24, 14.5, 5.95))
chart = s.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(0.72), Inches(4.1), Inches(7.6), Inches(2.7), data).chart
pl = chart.plots[0]; pl.has_data_labels = True; chart.has_legend = False; chart.has_title = False
for f in (chart.font, chart.category_axis.tick_labels.font, chart.value_axis.tick_labels.font, pl.data_labels.font): f.size = Pt(12)
pl.data_labels.number_format = "0.0#"; pl.data_labels.number_format_is_linked = False
pl.series[0].format.fill.solid(); pl.series[0].format.fill.fore_color.rgb = rgb(T.primary)
pl.series[0].points[3].format.fill.solid(); pl.series[0].points[3].format.fill.fore_color.rgb = rgb(T.accent)
card(s, 8.62, 4.1, 3.993, 2.7, "怎么用", points=["先量命中率", "再决定要不要优化数据库"])
```

## When Python is unavailable

When create_document's description does not offer Python content (no `"kind":"python"`, no `preview`), send a deck object; the code, helper and preview parts above do not apply.

`create_document({ name: "deck.pptx", format: "pptx", content: { "kind": "deck", … } })` builds a finished 16:9 deck from a deck object. You choose a page type for each slide and write its text; the host sets fonts, sizes, colors and positions, draws the slides and checks them. A deck that breaks a rule is rejected with one message per problem, naming the slide, the field and the limit, and nothing is saved: fix exactly those and resend the whole deck (at most 2 retries). The result says how many slides were rendered and reports any issues it found; mention them.

### Ground rules

- In a deck object you write content only: never coordinates, font sizes, colors or shapes.
- One deck = one `create_document` call with every slide. Plan the whole deck first; never patch it with a second call.
- Titles first: list the slide titles before writing anything; read in order, they should tell the story.
- One idea per slide. A list goes to `pillars` (2–4 items) or `grid` (3–8 items); there is no `bullets` page type.
- Adjacent slides use different page types.
- Every content slide (everything but the cover and the closing) carries `notes`: what the speaker says.
- All text and notes in the user's language, with `language` set to match. No placeholders (`TODO`, `xxxx`) and no invented statistics: use the user's numbers, or leave `evidence` out.
- Over a limit means rejection, never truncation. Split the slide or shorten the wording; do not abbreviate until the meaning is lost.

### Narrative

Pick the structure that fits the occasion, then adapt it to the content.

- Fundraising, sales, bids (problem_solution): hero → statement → pillars → evidence → cta
- Reports, analysis (insight_driven): hero → statement → evidence → pillars → cta
- Promotion, speeches (vision_driven): hero → quote → pillars → evidence → cta
- Teaching, training (educational): hero → statement → pillars → grid → showcase → cta

Length: 3–5 slides for a short pitch, 5–8 for a normal talk, 10–15 for a long one; 3–30 is the limit, so avoid more than 20.

### Page types

Limits count Chinese characters; a Latin letter, digit or space counts half, rounded up (6 Latin characters = 3). `?` marks an optional field.

| Type | Use for | Fields | Limits |
|---|---|---|---|
| `hero` | Cover | `title`, `subtitle?`, `meta?` (presenter, date) | title 18, subtitle 30, meta 30 |
| `statement` | One sentence to remember | `statement`, `note?` | statement 40, note 60 |
| `pillars` | 2–4 parallel points | `title`, `pillars`: 2–4 × `{ heading, body }` | heading 12, body 60 |
| `evidence` | Key numbers | `title`, `metrics`: 1–4 × `{ value, label, note? }`, `detail?`: up to 3 lines | value 3 (about 6 Latin characters, like `+18%`), label 12, note 30, detail line 40 |
| `timeline` | Steps in order | `title`, `steps`: 3–6 × `{ heading, body }` | heading 10, body 30 |
| `comparison` | Two sides | `title`, `left`, `right`: each `{ heading, points }` with 1–4 points | heading 12, point 30 |
| `grid` | 3–8 short parallel items | `title`, `items`: 3–8 × `{ heading, body }` | heading 10, body 30 |
| `quote` | A quotation | `quote`, `source?` | quote 60, source 20 |
| `cta` | Closing: thanks, next step | `title`, `subtitle?`, `contact?` | title 18, subtitle 40, contact 30 |
| `showcase` | One picture, chart or table | `title`, `media`, `caption?`, `body?` | caption 30, body 100 |

The `title` of a pillars, evidence, timeline, comparison, grid or showcase page is at most 22. Any slide may add `notes`, at most 300. Do not mix them up: `note` is a line of text on the statement page (and under a metric), while `notes` are the speaker notes, which the slide never shows.

Line breaks: titles, subtitles, meta, headings, labels, values, captions, the statement, the quote, the source, the contact, list items, table cells, chart labels and metric notes allow none. Pillar, grid and step bodies, a statement's `note` and a showcase `body` allow at most 2. `notes` are free.

`showcase.media` holds exactly one of:

- `"image"`: a `data:image/png|jpeg|svg+xml;base64,…` URI of image data you really have; prefer PNG or JPEG, since the check may report an SVG as distorted. Attachments and web URLs cannot be used.
- `"chart"`: `{ "kind": "bar", "categories": ["Q1", "Q2"], "series": [{ "name": "2026", "values": [142, 158] }] }`. Kinds: `bar`, `line`, `pie`; at most 8 categories and 3 series, one value per category. A pie takes exactly one series with 2–6 slices. Labels (categories, series names) are at most 12 and never contain `,` `;` or `:`.
- `"table"`: `{ "columns": ["Item", "Share"], "rows": [["A", "+18%"]] }`. At most 6 columns and 6 rows counting the header, so 5 body rows; header cells at most 8, body cells at most 10; every row has one cell per column.

A pillar, grid item or step may be a `"heading: body"` string, which splits at the first colon (`:` or `：`). Use the object form whenever a heading contains a colon.

### Deck fields

| Field | Value |
|---|---|
| `kind` | `"deck"`, required |
| `title` | The deck's name, at most 22, required |
| `language` | `zh`, `en` or `ja`, the language of the text; it picks the fonts, and a missing one means `zh` |
| `theme` | `navy`; leave it out |
| `slides` | 3–30 slide objects: `{ "type": …, the page's fields, "notes": … }` |

### Worked example

「做一个介绍 Ofox 的 PPT。数据：内测 3 个月，1200 位用户，绑定一次平均 30 秒，满意度 92%。」: eight slides, none of the same type next to another. The user's brief gave these numbers, so the evidence page uses them and no others; without numbers from the user, leave `evidence` out.

```json
{
  "kind": "deck",
  "title": "认识 Ofox",
  "language": "zh",
  "slides": [
    {"type": "hero", "title": "认识 Ofox", "subtitle": "一个账号，用好你的 AI 工具", "meta": "产品介绍 · 团队分享"},
    {"type": "statement", "statement": "AI 工具越来越多，配置不该越来越难", "note": "Ofox 把账号、密钥和模型选择收进一个桌面应用", "notes": "先讲痛点：每多一个工具，就多一套账号、密钥和配置文件。"},
    {"type": "pillars", "title": "Ofox 能做什么", "pillars": [
      {"heading": "一键绑定", "body": "登录后为 Claude Code、Codex 等工具写好配置，不用手动复制密钥"},
      {"heading": "统一模型", "body": "一个账号接入多家模型，在每个工具里按需切换"},
      {"heading": "桌面助手", "body": "写文档、做 PPT、生成图片和视频，结果直接存成附件"}
    ], "notes": "先讲绑定，这是新用户最先感受到的；再讲模型和助手。"},
    {"type": "timeline", "title": "从安装到开始使用", "steps": [
      {"heading": "安装", "body": "下载桌面应用并打开"},
      {"heading": "登录", "body": "用 Ofox 账号登录"},
      {"heading": "绑定", "body": "选择要用的工具，一键写好配置"},
      {"heading": "使用", "body": "回到原来的工具，直接开始工作"}
    ], "notes": "强调不改变用户习惯：绑定之后仍在原来的工具里工作。"},
    {"type": "grid", "title": "可以绑定的工具", "items": [
      {"heading": "Claude Code", "body": "终端里的编程助手"},
      {"heading": "Codex", "body": "命令行编程代理"},
      {"heading": "Gemini CLI", "body": "命令行 AI 助手"},
      {"heading": "OpenCode", "body": "开源的终端编程工具"},
      {"heading": "OpenClaw", "body": "个人 AI 助手"},
      {"heading": "Hermes", "body": "开源 AI 代理"}
    ], "notes": "列出常用的几款，现场可以演示其中一款的绑定。"},
    {"type": "evidence", "title": "内测 3 个月的结果", "metrics": [
      {"value": "1200", "label": "内测用户", "note": "内测 3 个月"},
      {"value": "30 秒", "label": "完成一次绑定"},
      {"value": "92%", "label": "内测满意度"}
    ], "notes": "三个数字都来自内测资料：先讲用户规模，再讲绑定速度和满意度。"},
    {"type": "comparison", "title": "手动配置与 Ofox", "left": {"heading": "手动配置", "points": ["每个工具单独注册账号", "密钥复制粘贴到配置文件", "换模型要改多处设置", "出了问题自己排查"]}, "right": {"heading": "用 Ofox", "points": ["一个账号管理全部工具", "登录后自动写好配置", "在应用里切换模型", "解绑时还原原来的配置"]}, "notes": "左边是用户今天的做法，右边逐条对应 Ofox 省掉的步骤。"},
    {"type": "cta", "title": "现在就试试 Ofox", "subtitle": "下载桌面应用，登录后选择要绑定的工具"}
  ]
}
```

## Expert mode: raw commands

Use this only when no page type can do the job (a custom layout, a picture at a chosen spot); a deck object is the first choice. `content` is then a JSON array of commands run in order as one atomic batch on an empty 33.87 × 19.05 cm deck. If one fails, nothing is saved and the result names its `index` and error; fix it and resend the whole array (at most 2 retries).

- Slides are numbered in the order added (the first `add slide` is `/slide[1]`). Give each shape and table a `name` unique on its slide and address it as `/slide[2]/shape[@name=Card1]` or `/slide[2]/table[@name=Plan]`, never `shape[N]`.
- Prop values are strings: sizes in points (`"20"`), lengths with a unit (`"2cm"`), colors as hex without `#`. A newline in `text` starts a new paragraph; unknown props are dropped.
- Title 36–44pt bold, body at least 18pt, content from x 2.33 cm, width 29.21 cm, below y 5 cm, at least 20% empty. Chinese font "Microsoft YaHei", Japanese "Yu Gothic". A box is at least lines × size × 0.05 cm + 0.4 cm tall; a line holds about width_cm ÷ (size × 0.035) CJK characters, twice as many Latin ones.

Slide with a background and a real title, plus notes:

```json
[
  {"command":"add","parent":"/","type":"slide","props":{"background":"FFFFFF","title":"Revenue grew 18%"}},
  {"command":"add","parent":"/slide[1]","type":"notes","props":{"text":"Lead with the 18% beat."}}
]
```

Title style (font and size are not inherited, so set them right after):

```json
[{"command":"set","path":"/slide[1]/placeholder[title]","props":{"font":"Georgia","size":"36","bold":"true","color":"1E2761"}}]
```

Text box, one item per paragraph (`list`: `"bullet"` or `"numbered"`; `preset` such as `roundRect` with `fill` makes it a shape):

```json
[{"command":"add","parent":"/slide[1]","type":"shape","props":{"name":"Points","text":"Renewals up 12%\nEMEA launched in March","list":"bullet","x":"2.33cm","y":"5.2cm","width":"18cm","height":"5cm","font":"Calibri","size":"20","color":"333333"}}]
```

Table (`data`: rows split by `;`, cells by `,`):

```json
[{"command":"add","parent":"/slide[1]","type":"table","props":{"name":"Plan","data":"Phase,Owner;Pilot,Operations;Rollout,Sales","x":"2.33cm","y":"10.96cm","width":"29.21cm","height":"4cm","headerFill":"1E2761","bodyFill":"F5F7FA"}}]
```

<!-- Adapted with changes from OfficeCLI (skills/officecli-pptx, v1.0.160; page types and palette from skills/morph-ppt; Apache-2.0); see notices/officecli-skills.txt -->
