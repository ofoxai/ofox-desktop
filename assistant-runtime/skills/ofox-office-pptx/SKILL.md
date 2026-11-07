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

「做一个 Transformer 介绍的 PPT」: eight slides, none of the same type next to another. Its evidence figures are made up for the example; never copy them into a real deck.

```json
{
  "kind": "deck",
  "title": "Transformer 入门",
  "language": "zh",
  "slides": [
    {"type": "hero", "title": "Transformer 入门", "subtitle": "注意力机制如何改变深度学习", "meta": "团队技术分享 · 2026 年 11 月"},
    {"type": "statement", "statement": "让每个词都能直接看到整句话", "note": "2017 年 Transformer 取代 RNN，成为今天大模型的共同基础", "notes": "先抛结论：注意力让模型一步看到全句，后面几页解释为什么这很重要。"},
    {"type": "pillars", "title": "为什么需要 Transformer", "pillars": [
      {"heading": "RNN 逐词计算", "body": "必须按顺序处理每个词，GPU 无法并行，训练很慢"},
      {"heading": "长句信息衰减", "body": "距离越远的词，信息在逐步传递中越容易丢失"},
      {"heading": "注意力取代循环", "body": "每个词一步直接关注全句，既能并行，也不怕距离"}
    ], "notes": "三个痛点对应三个收益：并行、长距离依赖、训练规模。"},
    {"type": "timeline", "title": "从注意力到大模型", "steps": [
      {"heading": "2014 年", "body": "注意力首次用于 RNN 翻译"},
      {"heading": "2017 年", "body": "Transformer 论文发表"},
      {"heading": "2018 年", "body": "BERT 与 GPT 相继出现"},
      {"heading": "2022 年", "body": "大模型走向公众"}
    ], "notes": "四个节点只讲一句话，重点放在 2017 年。"},
    {"type": "comparison", "title": "RNN 与 Transformer", "left": {"heading": "RNN", "points": ["逐词顺序计算", "长距离依赖容易丢失", "训练难以并行", "结构简单，参数较少"]}, "right": {"heading": "Transformer", "points": ["整句并行计算", "任意两词直接关联", "充分利用 GPU", "计算量随句长平方增长"]}, "notes": "先讲 RNN 的局限，再讲 Transformer 的优势，最后坦率提到它的计算成本。"},
    {"type": "evidence", "title": "落地效果：Q1 收入 +18%", "metrics": [
      {"value": "+18%", "label": "Q1 收入", "note": "相对上季度"},
      {"value": "3×", "label": "GPU 训练速度", "note": "对比 RNN 基线"},
      {"value": "-40%", "label": "推理延迟", "note": "同等精度下"}
    ], "detail": ["数据来自内部 A/B 测试，样本为 2026 年 Q1"], "notes": "三个指标都来自内部 A/B 测试：先讲收入，再讲训练速度和延迟。"},
    {"type": "grid", "title": "Transformer 的应用", "items": [
      {"heading": "文本", "body": "翻译、摘要与对话"},
      {"heading": "图像", "body": "ViT 把图片切成小块处理"},
      {"heading": "语音", "body": "识别与合成同样适用"},
      {"heading": "代码", "body": "补全、解释与审查"},
      {"heading": "视频", "body": "按时空小块建模与生成"},
      {"heading": "多模态", "body": "图文音统一在一个模型里"}
    ], "notes": "六个场景共用同一套结构，这正是它流行的原因。"},
    {"type": "cta", "title": "谢谢，欢迎提问", "subtitle": "下一步：用 PyTorch 实现一个小型 Transformer", "contact": "ai-club@example.com"}
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

<!-- Adapted with changes from OfficeCLI (skills/officecli-pptx, v1.0.160, Apache-2.0); see notices/officecli-skills.txt -->
