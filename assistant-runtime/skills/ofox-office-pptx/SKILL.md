---
name: ofox-office-pptx
description: Read before making a PowerPoint deck (.pptx) with create_document. Python deck code with a preview step; a deck object where Python is unavailable.
---

Prefer the offered `{"kind":"python","code":"…"}` route; otherwise use "When Python is unavailable".

## Choose a style and plan the pages

When offered, use `get_presentation_candidates` first, choose an eligible ID (manual wins), then read `get_presentation_style({id:"…"})`. Otherwise list styles. Match audience/density; adaptations are experimental.

Distinguish theme/fonts, composition, narrative, workflow and engine. Keep composition, not just colours. Partial native layouts need no HTML runtime/remote assets. Check font fallback; see [workflow](resources/design-workflow.md).

Plan each page's claim, evidence, relationship, visual, composition, density and editing. Check facts/exports; plans certify nothing. Core formulas use `equation()` even on diagram pages. Visual kinds: `native-chart` data, `native-math` formulas, `native-diagram` simple steps/relations, `code-figure` a richer diagram or curve drawn with `figure()`/`plot()`, `generated-image` a picture.

Use candidates for requirements (optional unknowns: null), then `freeze_presentation_plan` once for all pages. Explain inferences. Current plan references bind automatically; explicit ones must match. New plan, new turn.

Native editing is not prior round-trip certification: default to `unverifiedAllowed`; use `verifiedRequired` only for explicit certification demands. If blocked, follow `nextAction`, never retry unchanged or skip freezing.

The opening Host text says whether slide images are on and how many (up to N, or exactly N): plan `generated-image` pages to match; when off, plan none. It also lists pictures the conversation already holds: place one free as a `user-image` whose `assetId` is its attachment id, and keep an earlier deck's pictures that way when changing it. A plan grants no budget; without Host approval use supplied assets or the declared fallback. Planned `generate_image` calls carry `presentationAssetId`/`planId`/`revision`. Never expose private paths or asset bytes.

## Python deck code

`create_document({ name: "deck.pptx", format: "pptx", content: { "kind": "python", "code": "…" }, preview: true })`. The host runs plain Python source in a sandbox, saves and checks the deck, and renders its slides. A string's `\n` starts a paragraph.

- Already imported: the helpers, `T`, `G`, `Inches`, `Pt`, `Emu`, `RGBColor`, `MSO_SHAPE`, `PP_ALIGN`, `CategoryChartData`, `XyChartData`, `XL_CHART_TYPE`, `XL_LEGEND_POSITION`, `XL_LABEL_POSITION`, `plt`, `np`.
- For a new deck use `prs = new_deck(title="…", footer="…", style="cartesian", language="zh")` with the selected ID and language (`zh`/`en`/`ja`); omitted style is `navy`. Use helpers, not `prs.slide_layouts[...]`. A source edit uses `source_deck()` instead.
- Never call `prs.save` or `sys.exit`; never use files, network, subprocess or pip.
- One deck, one file: preview it, then deliver it; never make a second file to patch the first.
- Start the code with a plan comment: one line per slide, its point, its visual, its layout. Read in order, the points should tell the story.
- All text and notes in the user's language. No placeholders, no invented numbers: use the user's, or compute from them.

## Preview, then deliver

1. Send the code with `preview: true`. Nothing is delivered: you get a `previewId`, the issues and an overview image of the slides, each under its slide number.
2. Look at the image and read the issues: clipped or overlapping text, empty areas, small text, a plot or figure out of place. Fix the code.
3. Deliver under the same file name without `preview`, or use `fromPreview: "<previewId>"` without content for that exact preview. Preview twice per reply at most; the latest is delivered automatically if omitted.

On Python error, fix the traceback line (at most 2 retries). After delivery, describe the file and remaining issues.

## Helpers

Geometry uses inches; type uses pt. Canvas 13.333 × 7.5. `G`: `X=.72`, `R=12.613`, `CW=11.893`, `TOP=1.7`, `BOTTOM=6.85`, `GAP=.3`. `T` has hex colours without `#` and body/title/data font roles. Readonly aliases: `T.card=T.tint`, `T.textMuted=T.faint`.

- `new_deck(title="", footer="", style="navy", language="zh")`: returns `prs`; rejects unknown/disabled styles. Footer appears on content pages.
- `source_deck()`: Host-authorized original snapshot; source mode rejects `new_deck`.
- `target_slides(prs)`: authorized actual source pages in plan order. Edit these; preserve the remaining pages.
- `bring_title_to_front(slide)`: raise the same title; retain ID/rich text/fonts and extension order. Source edits need this, not `.text=` merely to raise it.
- `cover_slide(prs, title, subtitle="", kicker="", meta="", notes="", art=None)`: cover; optional right SVG art, legacy 4.6×5.5, adaptations 4.2×4.5.
- `section_slide(prs, title, number="", subtitle="", notes="")`: section break.
- `statement_slide(prs, statement, kicker="", attribution="", notes="")`: one large claim or attributed quote.
- `closing_slide(prs, title, subtitle="", meta="", notes="")`: sign-off page.
- `content_slide(prs, title, subtitle="", notes="")`: native title/header/footer; never redraw the header. Keep title ≤22 CJK characters, subtitle ≤40.
- `panel_slide(prs, title, subtitle="", notes="", side="auto")`: architectural 13cm panel, alternating left/right with numbered navigation. Respect its separate panel/main regions.
- `lesson_slide(prs, title, objective, subtitle="", notes="", objective_title=None, current=None)`: objective text/list (`current` index) rail.
- `slide_regions(slide)`: returns named `main`/`panel`/`objective` boxes (`x,y,w,h`) for these helpers; place content within its intended region.
- `text(slide, content, x, y, w, h, size=None, color=None, bold=False, align="left", valign="top", font=None, spacing=1.15, bullet=False, gap=0, role="body")`: string or list of strings/`{"text","bold","color"}`; `\n` starts a paragraph. Text fields must be strings; objects fail. Role sets body/title/data fonts; font overrides. No fit check.
- `bullets(slide, items, x, y, w, h, size=13, color=None, gap=6)`: native bullets from strings or `{"text"}`.
- `card(slide, x, y, w, h, title, body="", points=None, highlight=False, size=None)`: themed heading/body or bullets; highlight uses accent.
- `kpi(slide, x, y, w, h, value, label, note="", highlight=False)`: big number; h≥2 with a note.
- `steps(slide, items, x, y, w, h, highlight=-1, size=12)`: arrow-linked `{"title","body"}` items.
- `timeline(slide, items, x, y, w, h, highlight=-1, size=12)`: `{"label","title","body"}` items.
- `callout(slide, y, message, x=None, w=None, h=0.62)`: one-line takeaway; full content width by default.
- `table(slide, headers, rows, x, y, w, h, size=12)`: native table; 2–8 headers, 1–12 string rows.
- `pyramid(slide, levels, x, y, w, h, size=14)`: 2–6 native levels, narrowest first; label or `{"title","body"}`.
- `code(slide, source, x, y, w, h, size=12)`: ≤40 monospaced lines on a dark panel.
- `formula(slide, x, y, w, h, lines, size=15)`: plain text; warns; not academic math.
- `equation(slide, x, y, w, h, expression, size=22)`: Native OMML; LaTeX adds noneditable PNG. `{"latex":r"\sin(x)"}` or structures. LaTeX: scripts,hat/widehat,frac,Greek,braced sum/sqrt,top,dots/ldots/cdots,sin/cos,quad (em-space),mathrm/operatorname (1–64 ASCII alphanumerics). Brace signed/multi-digit scripts; other macros fail. Literal text. Verify WPS/PowerPoint display/editing.
- `relation(slide, items, x, y, w, h, kind="process", size=15)`: 2–6 string labels; process means sequence, comparison means independent items without arrows.
- `fork_join(slide, branches, x, y, w, h, source="", merge="")`: 2–6 parallel label strings; source→all→merge, no branch-to-branch arrows. For lesson/panel bodies use the `main` box, not full-slide `G`.
- `native_chart(slide, kind, categories, series, x, y, w, h, title="")`: native chart plus workbook; bar/line/pie; series `[{"name":"…","values":[…]}]`, at most 100 supplied finite values.
- `shape(slide, kind, x, y, w, h, fill=None, line=None, line_w=0.75, radius=None)`: `MSO_SHAPE.RECTANGLE`, `ROUNDED_RECTANGLE`, `OVAL`, etc.; radius only for rounded.
- `arrow(slide, x1, y1, x2, y2, color=None, width=1.5)`
- `image(slide, asset_id, x, y, w, h, fit="contain", alt="", behind=False, veil=None)`: authorized PNG/JPEG ID; contain or cover fit. Raster artwork, not editable chart/math. Unknown IDs fail. Covering the title or text fails: use a clear region, or `behind=True` for a background (full page, `fit="cover"`; the page colour veils it so text stays readable).
- `figure(slide, svg, x, y, w, h, alt="figure")`: an SVG figure drawn in code, for architecture and block diagrams with labelled boxes and arrows, maps and concept sketches.
- `plot(slide, fig, x, y, w, h, alt="chart")`: a matplotlib figure, for curves, heatmaps, scatter and distributions; it takes the page's colours. Both are pictures: editable chart data goes in `native_chart`, formulas in `equation`.
- Page canvases: page kinds may be dark, light or saturated. Helpers colour themselves for their page; `T.text`/`T.primary`/`T.muted`/`T.tint` hold the last-used page's colours, so finish one page before the next; no fixed hex or `background`. Vary page kinds per the style's `pageMapping`.
- `background(slide, color)`: whole-slide fill. On dark fills, recolour native title runs via `r.font.color.rgb = rgb(T.white)` and use light text/subtitles.
- Native chart: `slide.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(x), Inches(y), Inches(w), Inches(h), chart_data).chart`; set chart/axis/data-label fonts ≥12pt and distinct series colours.
- Fit: `text_height(text, pt, width_in, spacing=1.3)`, `lines_for(text, pt, width_in)`, `em_width(s)`, `bullets_height(items, pt, width_in, gap_pt=6)`, `need(what, needed, has, slide=None)` (overflow lint), `rgb(h)` (hex to colour).

## Design rules

1. Match the page to its relationship/style: visual with evidence, editorial area, lesson sidebar, split panel or statement. Allow whitespace; avoid forced card grids or stretched content. Stay inside `G` or the helper's named regions.
2. Every content slide has one visual that explains: a plot, an SVG figure, a native chart, steps or a timeline, a table, or a formula with a worked example. Architecture: block diagram. Relations or weights: heatmap with the numbers in the cells. Numbers: chart or kpis. Process: steps. History: timeline.
3. Vary the rhythm intentionally and avoid mechanical repetition. Repeat a layout for consecutive comparisons or a consistent scale when that helps reading. For 8+ slides, consider a section break and a number-forward page.
4. Colour: primary for structure, grays for text, the accent only for the one thing to notice on a slide. In figures and plots, primary against accent (and their tints) separates two groups.
5. Type follows the selected theme's title/body/data roles. Default content title is 26, subtitle 13, headings 14–16, body 12–14, captions 11; nothing below 10 pt, figures and plots included. Bullets only through `bullets()` or `card(points=…)`, never a typed "•".
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

The helpers report problems in the issues, each line starting with `slide N: ` (the slide's place in the deck). Fix each in the code and look again.

- `… text needs 1.20 in, box has 0.80 in` (title, subtitle, bullets, card, steps, timeline, kpi note, callout): the text does not fit. Shorten it, enlarge the box, or go down one size step; body text stays at 12 pt or more.
- `… the box is 4.00 in tall but its text needs 1.50 in; drawn at 1.75 in …` (card; steps say "boxes are …"): the box was more than half empty and is drawn at its content's height. Use the freed space for another zone, or add content.
- `figure …: viewBox … does not match the box …; it will be stretched`: set the viewBox to `{w*192} × {h*192}`.
- `figure …: smallest text is 8.0 pt on the slide; use at least 10 pt`: raise `font-size` to 28 px or more.
- `plot …: tick labels are 8 pt; use at least 10 (…)`: raise that group (title, axis labels or tick labels) as the line shows, to 11 or more.
- Other issues are the host's checks of the finished file (overflow, overlap): find the slide in the overview and fix its layout.
- `relation/fork_join body outside main region …`: use `slide_regions(s)["main"]`; explicit coordinates are not moved for you.

Check fit and exports; zero automated findings certify neither formulas, facts nor native editing.

Parallel branches use a fork/join, never a sequence of feature boxes:

```python
prs = new_deck(style="course-module", language="en")
s = lesson_slide(prs, "Parallel branches", objective="Independent branches share input and merge results.")
b = slide_regions(s)["main"]
fork_join(s, ["Branch A", "Branch B", "Branch C"], **b, source="Shared input", merge="Combined result")
```

## Worked example

「讲一下缓存命中率怎么影响接口速度。数据：命中 5 ms，未命中 100 ms。」: four layouts; other numbers derive from these times.

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
shape(s, MSO_SHAPE.RECTANGLE, 7.92, 1.7, 4.693, 2.4, fill=T.tint)
equation(s, 8.12, 1.9, 4.293, 0.65, {"latex": r"E[t]=100-95p"}, size=20)
equation(s, 8.12, 2.7, 4.293, 0.65, {"latex": r"E[t]_{p=0.9}=14.5"}, size=18)
text(s, "p 为命中率；平均耗时的单位是 ms", 8.12, 3.5, 4.293, 0.35, size=11, color=T.muted)
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

`create_document({ name: "deck.pptx", format: "pptx", content: { "kind": "deck", … } })` builds a 16:9 deck. Choose page types and text; the host sets layout and checks it. Rejection names each slide, field and limit; nothing is saved. Fix those problems and resend the whole deck (at most 2 retries). Report remaining issues.

### Ground rules

- In a deck object you write content only: never coordinates, font sizes, colors or shapes.
- One deck = one `create_document` call with every slide. Plan the whole deck first; never patch it with a second call.
- Titles first: list the slide titles before writing anything; read in order, they should tell the story.
- One idea per slide. A list goes to `pillars` (2–4 items) or `grid` (3–8 items); there is no `bullets` page type.
- Vary page types purposefully; consecutive comparisons or shared-scale evidence may repeat a layout for clarity.
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

- `hero` (cover): title≤18, subtitle?≤30, meta?≤30 (presenter/date).
- `statement`: statement≤40, note?≤60.
- `pillars`: title; 2–4 pillars `{heading,body}` (heading≤12, body≤60).
- `evidence`: title; 1–4 metrics `{value,label,note?}` (value≤3, e.g. `+18%`; label≤12, note≤30); detail? up to 3 lines, each≤40.
- `timeline`: title; 3–6 steps `{heading,body}` (heading≤10, body≤30).
- `comparison`: title; left/right `{heading,points}` (heading≤12, 1–4 points, each≤30).
- `grid`: title; 3–8 items `{heading,body}` (heading≤10, body≤30).
- `quote`: quote≤60, source?≤20.
- `cta` (closing): title≤18, subtitle?≤40, contact?≤30.
- `showcase`: title, media; caption?≤30, body?≤100.

Page `title`≤22; any slide may add `notes`≤300. `note` is visible text on a statement or metric; `notes` are speaker notes.

Line breaks: at most 2 in pillar/grid/step bodies, statement `note` and showcase `body`; unlimited in `notes`; none in other text fields.

`showcase.media` holds exactly one of:

- `"image"`: a `data:image/png|jpeg|svg+xml;base64,…` URI of image data you really have; prefer PNG or JPEG, since the check may report an SVG as distorted. Attachments and web URLs cannot be used.
- `"chart"`: `{ "kind": "bar", "categories": ["Q1", "Q2"], "series": [{ "name": "2026", "values": [142, 158] }] }`. Kinds: `bar`, `line`, `pie`; at most 8 categories and 3 series, one value per category. A pie takes exactly one series with 2–6 slices. Labels (categories, series names) are at most 12 and never contain `,` `;` or `:`.
- `"table"`: `{ "columns": ["Item", "Share"], "rows": [["A", "+18%"]] }`. At most 6 columns and 6 rows counting the header, so 5 body rows; header cells at most 8, body cells at most 10; every row has one cell per column.

A pillar, grid item or step may be a `"heading: body"` string, which splits at the first colon (`:` or `：`). Use the object form whenever a heading contains a colon.

### Deck fields

Required: `kind:"deck"`, title≤22, and 3–30 `slides` (`{type,…page fields,notes}`). `language`: zh/en/ja; default zh, selects fonts. `theme`: an enabled ID from `get_presentation_style`, default navy; controls cover/header/components as well as colour.

### Worked example

「介绍 Ofox：内测 3 个月，1200 位用户，绑定平均 30 秒，满意度 92%。」: eight varied slides using only supplied numbers; without numbers omit `evidence`.

```json
{
  "kind": "deck",
  "title": "认识 Ofox",
  "language": "zh",
  "slides": [
    {
      "type": "hero",
      "title": "认识 Ofox",
      "subtitle": "一个账号，用好你的 AI 工具",
      "meta": "产品介绍 · 团队分享"
    },
    {
      "type": "statement",
      "statement": "AI 工具越来越多，配置不该越来越难",
      "note": "Ofox 把账号、密钥和模型选择收进一个桌面应用",
      "notes": "先讲痛点：每多一个工具，就多一套账号、密钥和配置文件。"
    },
    {
      "type": "pillars",
      "title": "Ofox 能做什么",
      "pillars": [
        {
          "heading": "一键绑定",
          "body": "登录后为 Claude Code、Codex 等工具写好配置，不用手动复制密钥"
        },
        {
          "heading": "统一模型",
          "body": "一个账号接入多家模型，在每个工具里按需切换"
        },
        {
          "heading": "桌面助手",
          "body": "写文档、做 PPT、生成图片和视频，结果直接存成附件"
        }
      ],
      "notes": "先讲绑定，这是新用户最先感受到的；再讲模型和助手。"
    },
    {
      "type": "timeline",
      "title": "从安装到开始使用",
      "steps": [
        { "heading": "安装", "body": "下载桌面应用并打开" },
        { "heading": "登录", "body": "用 Ofox 账号登录" },
        { "heading": "绑定", "body": "选择要用的工具，一键写好配置" },
        { "heading": "使用", "body": "回到原来的工具，直接开始工作" }
      ],
      "notes": "强调不改变用户习惯：绑定之后仍在原来的工具里工作。"
    },
    {
      "type": "grid",
      "title": "可以绑定的工具",
      "items": [
        { "heading": "Claude Code", "body": "终端里的编程助手" },
        { "heading": "Codex", "body": "命令行编程代理" },
        { "heading": "Gemini CLI", "body": "命令行 AI 助手" },
        { "heading": "OpenCode", "body": "开源的终端编程工具" },
        { "heading": "OpenClaw", "body": "个人 AI 助手" },
        { "heading": "Hermes", "body": "开源 AI 代理" }
      ],
      "notes": "列出常用的几款，现场可以演示其中一款的绑定。"
    },
    {
      "type": "evidence",
      "title": "内测 3 个月的结果",
      "metrics": [
        { "value": "1200", "label": "内测用户", "note": "内测 3 个月" },
        { "value": "30 秒", "label": "完成一次绑定" },
        { "value": "92%", "label": "内测满意度" }
      ],
      "notes": "三个数字都来自内测资料：先讲用户规模，再讲绑定速度和满意度。"
    },
    {
      "type": "comparison",
      "title": "手动配置与 Ofox",
      "left": {
        "heading": "手动配置",
        "points": [
          "每个工具单独注册账号",
          "密钥复制粘贴到配置文件",
          "换模型要改多处设置",
          "出了问题自己排查"
        ]
      },
      "right": {
        "heading": "用 Ofox",
        "points": [
          "一个账号管理全部工具",
          "登录后自动写好配置",
          "在应用里切换模型",
          "解绑时还原原来的配置"
        ]
      },
      "notes": "左边是用户今天的做法，右边逐条对应 Ofox 省掉的步骤。"
    },
    {
      "type": "cta",
      "title": "现在就试试 Ofox",
      "subtitle": "下载桌面应用，登录后选择要绑定的工具"
    }
  ]
}
```

## Expert mode: raw commands

When no page type fits, send `content` as one atomic command array (33.87 × 19.05cm). Fix the reported index; ≤2 retries.

- Use `/slide[1]` and named shapes/tables: `/slide[2]/shape[@name=Card1]`, never `shape[N]`.
- Props: string pt sizes, lengths `"2cm"`, hex colors without `#`. Unknown props are dropped; newlines start paragraphs.
- Set title font/size via `/slide[1]/placeholder[title]`: 36–44pt bold, body≥18pt. Content x≥2.33cm, y≥5cm, width≤29.21cm. Use the user's CJK font; allow space for wrapping.

Create a slide with `add`, `parent:"/"`, `type:"slide"`, then add `notes` under its slide path.

Shapes: `name,text,x,y,width,height,font,size,color`; `list`: bullet/numbered. Use `preset:"roundRect"` and `fill` for backgrounds. Table `data`: `;` rows, `,` cells.

<!-- Adapted with changes from OfficeCLI (skills/officecli-pptx, v1.0.160; page types and palette from skills/morph-ppt; Apache-2.0); see notices/officecli-skills.txt -->
