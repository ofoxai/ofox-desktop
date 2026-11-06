---
name: ofox-office-pptx
description: Read before making a PowerPoint deck (.pptx) with create_document. Slide design rules and the batch commands for slides, text, shapes, tables.
---

# PowerPoint decks

`create_document({ name: "deck.pptx", format: "pptx", content: [...] })` starts from an empty 16:9 deck (33.87 × 19.05 cm). `content` is a JSON array of commands run in order as one atomic batch: if one fails, nothing is saved and the result names its `index` and error; fix it and resend the whole array (at most 2 retries).

## Ground rules

- One deck = one `create_document` call with the whole batch. Plan every slide first; never patch it with a second call.
- Stable paths: slides are numbered in the order you add them (the first `add slide` is `/slide[1]`). Give each shape and table a `name` unique on its slide and address it as `/slide[2]/shape[@name=Card1]` or `/slide[2]/table[@name=Plan]`, never `shape[N]`.
- Prop values are strings: sizes in points (`"20"`), lengths with a unit (`"2cm"`), `"true"`, colors as hex without `#`. A newline in `text` (`\n` in JSON) starts a new paragraph.
- All slide text and notes in the user's language. No placeholders (`TODO`, `xxxx`).
- Use only the commands and props below (unknown props are dropped). If `issues` flag overflow or shapes past the edge, fix and recreate the deck.

## Design rules

**Titles first.** List the titles before building; they alone should tell the story, in one style.

**One idea per slide.** 3–5 one-line bullets (≈ 12 words or 20 CJK characters); more means another slide. Notes on every content slide.

**Type scale**: title 36–44pt bold (at least 1.5× body); subtitle and card heading 20–24pt; body 18–24pt, sentences never below 18; captions and labels 10–14pt, muted; KPI numbers 60–72pt bold, ≤ 5 characters.

Box height ≥ lines × size × 0.05 cm + 0.4 cm; a line holds about width_cm ÷ (size × 0.035) CJK characters, twice as many Latin ones. Cut or split text that does not fit; never shrink below the scale.

**Grid.** Align content to the title edge (x 2.33 cm, width 29.21 cm); one 0.76 cm gap between blocks; ≥ 20% empty space. N cards in a row: width = (29.21 − (N − 1) × 0.76) ÷ N. Left-align body; center only cover titles and big numbers.

**Fonts.** Two at most, set with `font` on every title and box: Georgia + Calibri (formal), Arial Black + Arial (launch), Calibri Light + Calibri (clean). Chinese: "Microsoft YaHei"; Japanese: "Yu Gothic".

**Color.** One palette — primary (60–70% of the color), secondary, sparing accent, text, muted: the user's brand colors, a seed below, or a blend.

| Theme | Primary | Secondary | Accent | Text | Muted |
|---|---|---|---|---|---|
| Navy | 1E2761 | CADCFC | F96167 | 333333 | 8899BB |
| Forest | 2C5F2D | 97BC62 | D4A843 | 2D2D2D | 6B8E6B |

On dark fills use FFFFFF or a light tint; mid-gray vanishes on a projector.

**Structure.** Cover (title, subtitle, organization or presenter, date), agenda, content, closing with takeaways. Vary layouts (columns, card row, big number, table); most slides carry an informative non-text element. Avoid lines under titles, cards with a colored left stripe, emoji icons, hype words, invented statistics.

## Commands

**Slide, title, notes.** Slides are blank; `background` is a color. `title` makes the slide's real title; style it right after on `/slide[N]/placeholder[title]` (font and size are not inherited). Its box is x 2.33, y 1, 29.21 × 3.67 cm (content starts at y ≈ 5 cm); `x`, `y`, `width`, `height` move it.

```json
[
  {"command":"add","parent":"/","type":"slide","props":{"background":"FFFFFF","title":"Revenue grew 18%"}},
  {"command":"set","path":"/slide[1]/placeholder[title]","props":{"font":"Georgia","size":"36","bold":"true","color":"1E2761"}},
  {"command":"add","parent":"/slide[1]","type":"notes","props":{"text":"Lead with the 18% beat; EMEA explains most of it."}}
]
```

**Text box and bullets.** `list`: `"bullet"` or `"numbered"`, one item per paragraph. Props: `text`, `x`, `y`, `width`, `height`, `font`, `size`, `bold`, `italic`, `color`, `align` (left/center/right), `valign` (top/middle/bottom), `fill`, `spaceAfter`.

```json
[{"command":"add","parent":"/slide[1]","type":"shape","props":{"name":"Points","text":"Renewals up 12%\nEMEA launched in March\nNRR held at 118%","list":"bullet","x":"2.33cm","y":"5.2cm","width":"18cm","height":"5cm","font":"Calibri","size":"20","color":"333333","spaceAfter":"6pt"}}]
```

**Shapes.** `preset`: rect, roundRect, ellipse, triangle, diamond, rightArrow, chevron; `fill` colors it, `line` adds an outline color; it can hold text. Later shapes draw on top.

```json
[{"command":"add","parent":"/slide[1]","type":"shape","props":{"name":"Kpi","preset":"roundRect","fill":"1E2761","x":"21.09cm","y":"5.2cm","width":"10.45cm","height":"5cm","text":"118%","font":"Georgia","size":"60","bold":"true","color":"FFFFFF","align":"center","valign":"middle"}}]
```

**Table.** `data`: rows split by `;`, cells by `,` (quote a cell as `\"…\"` to keep a comma); then style the body on the table and the header on `tr[1]`. At most about 6 × 5 cells, body 14–18pt.

```json
[
  {"command":"add","parent":"/slide[1]","type":"table","props":{"name":"Plan","data":"Phase,Owner,Date;Pilot,Operations,Q1;Rollout,Sales,Q2","x":"2.33cm","y":"10.96cm","width":"29.21cm","height":"4cm","colWidths":"12cm,10cm,7.21cm","headerFill":"1E2761","bodyFill":"F5F7FA"}},
  {"command":"set","path":"/slide[1]/table[@name=Plan]","props":{"font":"Calibri","size":"16","color":"333333","valign":"middle"}},
  {"command":"set","path":"/slide[1]/table[@name=Plan]/tr[1]","props":{"bold":"true","color":"FFFFFF"}}
]
```

**Picture.** `src` is a base64 `data:` URI of image data you really have (PNG, JPEG, or a small SVG you wrote; SVG needs a recent PowerPoint). Attachments cannot be referenced; web URLs are often refused. Set `alt`, or use shapes.

```json
[{"command":"add","parent":"/slide[1]","type":"picture","props":{"name":"Dot","src":"data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMCAxMCI+PGNpcmNsZSBjeD0iNSIgY3k9IjUiIHI9IjUiIGZpbGw9IiMxRTI3NjEiLz48L3N2Zz4=","x":"30.54cm","y":"16.3cm","width":"1cm","height":"1cm","alt":"Brand dot"}}]
```

## Worked example: 8-slide deck

「做一个 Transformer 介绍的 PPT」, Navy palette.

```json
[
  {"command":"add","parent":"/","type":"slide","props":{"background":"1E2761","title":"Transformer 入门"}},
  {"command":"set","path":"/slide[1]/placeholder[title]","props":{"font":"Microsoft YaHei","x":"2.33cm","y":"6cm","width":"29.21cm","height":"3cm","size":"44","bold":"true","color":"FFFFFF","align":"center"}},
  {"command":"add","parent":"/slide[1]","type":"shape","props":{"font":"Microsoft YaHei","name":"Sub","text":"注意力机制如何改变了深度学习\n团队技术分享 · 2026 年 11 月","x":"2.33cm","y":"9.5cm","width":"29.21cm","height":"2.6cm","size":"20","color":"CADCFC","align":"center"}},
  {"command":"add","parent":"/","type":"slide","props":{"title":"今天讲什么"}},
  {"command":"set","path":"/slide[2]/placeholder[title]","props":{"font":"Microsoft YaHei","size":"36","bold":"true","color":"1E2761"}},
  {"command":"add","parent":"/slide[2]","type":"shape","props":{"font":"Microsoft YaHei","name":"Agenda","text":"为什么需要 Transformer\n自注意力机制\n整体结构\n应用与局限","list":"numbered","x":"2.33cm","y":"5.2cm","width":"20cm","height":"7cm","size":"24","color":"333333"}},
  {"command":"add","parent":"/","type":"slide","props":{"title":"为什么需要 Transformer"}},
  {"command":"set","path":"/slide[3]/placeholder[title]","props":{"font":"Microsoft YaHei","size":"36","bold":"true","color":"1E2761"}},
  {"command":"add","parent":"/slide[3]","type":"shape","props":{"font":"Microsoft YaHei","name":"Why","text":"RNN 逐词计算，难以并行\n长句里前后信息容易衰减\n2017 年：注意力取代循环","list":"bullet","x":"2.33cm","y":"5.2cm","width":"18cm","height":"7cm","size":"22","color":"333333"}},
  {"command":"add","parent":"/slide[3]","type":"shape","props":{"font":"Microsoft YaHei","name":"Year","preset":"roundRect","fill":"1E2761","x":"21.09cm","y":"5.2cm","width":"10.45cm","height":"6cm","text":"2017","size":"60","bold":"true","color":"FFFFFF","align":"center","valign":"middle"}},
  {"command":"add","parent":"/slide[3]","type":"notes","props":{"text":"瓶颈在顺序计算。"}},
  {"command":"add","parent":"/","type":"slide","props":{"title":"自注意力：每个词都看全句"}},
  {"command":"set","path":"/slide[4]/placeholder[title]","props":{"font":"Microsoft YaHei","size":"36","bold":"true","color":"1E2761"}},
  {"command":"add","parent":"/slide[4]","type":"shape","props":{"font":"Microsoft YaHei","name":"Q","preset":"roundRect","fill":"CADCFC","x":"2.33cm","y":"5.5cm","width":"9.23cm","height":"6cm","text":"Query\n想找什么","size":"24","color":"1E2761","align":"center","valign":"middle"}},
  {"command":"add","parent":"/slide[4]","type":"shape","props":{"font":"Microsoft YaHei","name":"K","preset":"roundRect","fill":"CADCFC","x":"12.32cm","y":"5.5cm","width":"9.23cm","height":"6cm","text":"Key\n能被谁找到","size":"24","color":"1E2761","align":"center","valign":"middle"}},
  {"command":"add","parent":"/slide[4]","type":"shape","props":{"font":"Microsoft YaHei","name":"V","preset":"roundRect","fill":"1E2761","x":"22.31cm","y":"5.5cm","width":"9.23cm","height":"6cm","text":"Value\n被取走的内容","size":"24","color":"FFFFFF","align":"center","valign":"middle"}},
  {"command":"add","parent":"/slide[4]","type":"notes","props":{"text":"Query 与每个 Key 比相似度，再汇总 Value。"}},
  {"command":"add","parent":"/","type":"slide","props":{"title":"整体结构"}},
  {"command":"set","path":"/slide[5]/placeholder[title]","props":{"font":"Microsoft YaHei","size":"36","bold":"true","color":"1E2761"}},
  {"command":"add","parent":"/slide[5]","type":"table","props":{"name":"Parts","data":"模块,作用;位置编码,标记词的先后顺序;多头注意力,从多个角度关注上下文;编码器 / 解码器,理解输入 / 逐词生成","x":"2.33cm","y":"5.2cm","width":"29.21cm","height":"7.2cm","colWidths":"9cm,20.21cm","headerFill":"1E2761","bodyFill":"F5F7FA"}},
  {"command":"set","path":"/slide[5]/table[@name=Parts]","props":{"font":"Microsoft YaHei","size":"18","color":"333333","valign":"middle"}},
  {"command":"set","path":"/slide[5]/table[@name=Parts]/tr[1]","props":{"bold":"true","color":"FFFFFF"}},
  {"command":"add","parent":"/slide[5]","type":"notes","props":{"text":"编码器读懂输入，解码器逐词写出结果。"}},
  {"command":"add","parent":"/","type":"slide","props":{"title":"代价也不小"}},
  {"command":"set","path":"/slide[6]/placeholder[title]","props":{"font":"Microsoft YaHei","size":"36","bold":"true","color":"1E2761"}},
  {"command":"add","parent":"/slide[6]","type":"shape","props":{"font":"Microsoft YaHei","name":"Costs","fill":"1E2761","text":"计算量随句长平方增长\n依赖海量训练数据\n内部决策难以解释","list":"bullet","x":"2.33cm","y":"5.2cm","width":"29.21cm","height":"6cm","size":"24","color":"FFFFFF"}},
  {"command":"add","parent":"/slide[6]","type":"notes","props":{"text":"长文本的成本是主要短板。"}},
  {"command":"add","parent":"/","type":"slide","props":{"title":"要点回顾"}},
  {"command":"set","path":"/slide[7]/placeholder[title]","props":{"font":"Microsoft YaHei","size":"36","bold":"true","color":"1E2761"}},
  {"command":"add","parent":"/slide[7]","type":"shape","props":{"font":"Microsoft YaHei","name":"Recap","text":"每个词直接看到全句\n并行计算，训练更快\n文本、图像、语音通用","list":"bullet","x":"2.33cm","y":"5.2cm","width":"29.21cm","height":"5cm","size":"24","color":"333333"}},
  {"command":"add","parent":"/slide[7]","type":"notes","props":{"text":"三句话带走。"}},
  {"command":"add","parent":"/","type":"slide","props":{"background":"1E2761","title":"谢谢！欢迎提问"}},
  {"command":"set","path":"/slide[8]/placeholder[title]","props":{"font":"Microsoft YaHei","x":"2.33cm","y":"7cm","width":"29.21cm","height":"3cm","size":"40","bold":"true","color":"FFFFFF","align":"center"}}
]
```

## Common mistakes

| Mistake | Instead |
|---|---|
| "•" or "1." typed into text | `"list":"bullet"` / `"numbered"` |
| Title without `font`/`size` | Style the title placeholder |

<!-- Adapted with changes from OfficeCLI (skills/officecli-pptx, v1.0.160, Apache-2.0); see notices/officecli-skills.txt -->
