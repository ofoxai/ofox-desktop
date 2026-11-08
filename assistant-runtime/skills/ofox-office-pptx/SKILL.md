---
name: ofox-office-pptx
description: Read before making a PowerPoint deck (.pptx) with create_document. Page types, limits and a worked deck object; raw batch commands for experts.
---

`create_document({ name: "deck.pptx", format: "pptx", content: { "kind": "deck", … } })` builds a finished 16:9 deck from a deck object. You choose a page type for each slide and write its text; the host sets fonts, sizes, colors and positions, draws the slides and checks them. A deck that breaks a rule is rejected with one message per problem, naming the slide, the field and the limit, and nothing is saved: fix exactly those and resend the whole deck (at most 2 retries). The result says how many slides were rendered and reports any issues it found; mention them.

## Ground rules

- In a deck object you write content only: never coordinates, font sizes, colors or shapes.
- One deck = one `create_document` call with every slide. Plan the whole deck first; never patch it with a second call.
- Titles first: list the slide titles before writing anything; read in order, they should tell the story.
- One idea per slide. A list goes to `pillars` (2–4 items) or `grid` (3–8 items); there is no `bullets` page type.
- Adjacent slides use different page types.
- Every content slide (everything but the cover and the closing) carries `notes`: what the speaker says.
- All text and notes in the user's language, with `language` set to match. No placeholders (`TODO`, `xxxx`) and no invented statistics: use the user's numbers, or leave `evidence` out.
- Over a limit means rejection, never truncation. Split the slide or shorten the wording; do not abbreviate until the meaning is lost.

## Narrative

Pick the structure that fits the occasion, then adapt it to the content.

- Fundraising, sales, bids (problem_solution): hero → statement → pillars → evidence → cta
- Reports, analysis (insight_driven): hero → statement → evidence → pillars → cta
- Promotion, speeches (vision_driven): hero → quote → pillars → evidence → cta
- Teaching, training (educational): hero → statement → pillars → grid → showcase → cta

Length: 3–5 slides for a short pitch, 5–8 for a normal talk, 10–15 for a long one; 3–30 is the limit, so avoid more than 20.

## Page types

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

## Deck fields

| Field | Value |
|---|---|
| `kind` | `"deck"`, required |
| `title` | The deck's name, at most 22, required |
| `language` | `zh`, `en` or `ja`, the language of the text; it picks the fonts, and a missing one means `zh` |
| `theme` | `navy`; leave it out |
| `slides` | 3–30 slide objects: `{ "type": …, the page's fields, "notes": … }` |

## Worked example

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
