---
name: ofox-office-docx
description: Read before writing a Word document (.docx) with create_document. Report layout rules and batch commands for headings, lists, tables, footers.
---

# Word documents

`create_document({ name: "report.docx", format: "docx", content: [...] })` starts from an empty A4 document (standard margins, Calibri 11pt). `content` is a JSON array of commands run in order as one atomic batch: if one fails, nothing is saved and the result names its `index` and error; fix it and resend the whole array (at most 2 retries).

## Ground rules

- One document = one `create_document` call with the whole batch. Outline it first; never patch it with a second call.
- Build top to bottom: every `add` with `"parent":"/body"` appends. To change what you just added use `/body/p[last()]` or `/body/tbl[last()]`, and cells as `/body/tbl[last()]/tr[R]/tc[C]` (1-based). Never count paragraphs (`p[7]`): tables, pictures and breaks shift the count.
- One paragraph per command. Prop values are strings: sizes with `pt` (`"11pt"`), `"true"`, colors as hex without `#`.
- All text in the user's language. No placeholders (`TODO`, `xxxx`, `{name}`); write real content or leave the section out.
- Use only the commands and props below; unknown props are ignored or rejected.
- `issues` in the result flag empty paragraphs and similar problems; fix real ones and recreate. "Missing first-line indent" matters only for Chinese body text.

## Layout rules

**Hierarchy.** Title → Heading1 → Heading2 → Heading3 → body. Never skip a level. Headings are real heading styles (`"style":"Heading1"`), never bold Normal text — Word's navigation pane and outline read them.

**Sizes**, set explicitly because template defaults drift:

| Element | Size |
|---|---|
| Title | 26–32pt bold |
| Heading1 | 18pt bold (20pt in long reports) |
| Heading2 | 14pt bold |
| Heading3 | 12pt bold |
| Body | 11–12pt, line spacing 1.15–1.5× |
| Caption, header, footer | 9pt, gray |

**One body font, one accent.** Set the body font once with `docDefaults.font` (Calibri, Cambria, Georgia or Times New Roman) and give headings `font` explicitly. One accent color (for example 1F4E79) for headings and table headers; no rainbow text.

**Chinese documents.** Font "Microsoft YaHei" (or "SimSun" for formal print). Body paragraphs get `"firstLineChars":"200"` (two-character indent), `"align":"justify"` and line spacing 1.5×. Use full-width punctuation; the issue "Mixed CJK/Latin punctuation" points at half-width marks (decimals like 2.1 are fine).

**Spacing** comes from `spaceBefore` and `spaceAfter`, never from empty paragraphs.

**Pages.** Start a major section on a new page with `pageBreakBefore` on its heading, or a `pagebreak` command — never both for the same break. Reports longer than about three pages get a cover: title, subtitle, prepared for or by, date, one line of key themes. Any document longer than a page gets a footer with a live page number; never type "Page 1".

**Tables.** Header row in the accent color with white bold text, marked `header` so it repeats on every page; light gray borders; numbers right-aligned; totals bold. At most about 6 columns on a portrait page.

**Lists.** 3–7 items with parallel phrasing; bullets for unordered points, numbers for steps or ranking.

**Typography.** Curly quotes (“ ” ‘ ’), en dash for ranges (2024–2026), em dash for breaks.

## Commands

**Defaults** — once, first:

```json
[{"command":"set","path":"/","props":{"docDefaults.font":"Calibri","docDefaults.fontSize":"11pt"}}]
```

**Headings and paragraphs.** `style`: Title, Heading1, Heading2, Heading3, or omit for body. Props: `text`, `font`, `size`, `bold`, `italic`, `color`, `align` (left, center, right, justify), `spaceBefore`, `spaceAfter`, `lineSpacing` (`"1.15x"`), `firstLineChars`, `pageBreakBefore`.

```json
[
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Summary","style":"Heading1","font":"Cambria","size":"18pt","bold":"true","color":"1F4E79","spaceBefore":"18pt","spaceAfter":"8pt"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Revenue grew 18% year over year.","lineSpacing":"1.15x","spaceAfter":"8pt"}}
]
```

**Bold, italic, color inside a paragraph** — append a run to the paragraph just added:

```json
[{"command":"add","parent":"/body/p[last()]","type":"run","props":{"text":" Ahead of plan.","bold":"true","italic":"true","color":"C00000"}}]
```

**Lists.** One paragraph per item with `listStyle` `"bullet"` or `"numbered"`; consecutive numbered items count 1, 2, 3, and a list after other content restarts at 1.

```json
[
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Enterprise renewals","listStyle":"bullet"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Hire two agents","listStyle":"numbered"}}
]
```

**Table.** `data`: rows split by `;`, cells by `,` (quote a cell as `\"…\"` to keep a comma). Then mark row 1 as header and style each header cell; cell props: `fill`, `bold`, `color`, `align`.

```json
[
  {"command":"add","parent":"/body","type":"table","props":{"data":"Item,Cost;Hosting,1200;Support,800","width":"100%","border.all":"single;4;BFBFBF"}},
  {"command":"set","path":"/body/tbl[last()]/tr[1]","props":{"header":"true"}},
  {"command":"set","path":"/body/tbl[last()]/tr[1]/tc[1]","props":{"fill":"1F4E79","bold":"true","color":"FFFFFF"}},
  {"command":"set","path":"/body/tbl[last()]/tr[1]/tc[2]","props":{"fill":"1F4E79","bold":"true","color":"FFFFFF","align":"right"}},
  {"command":"set","path":"/body/tbl[last()]/tr[2]/tc[2]","props":{"align":"right"}},
  {"command":"set","path":"/body/tbl[last()]/tr[3]/tc[2]","props":{"align":"right"}}
]
```

**Picture.** `src` is a base64 `data:` URI of image data you really have (PNG, JPEG, or a small SVG you wrote; SVG needs a recent Word). Attachments cannot be referenced and web URLs are often refused. Always set `alt`, `width` and `height`; the picture gets its own paragraph, which you can center.

```json
[
  {"command":"add","parent":"/body","type":"picture","props":{"src":"data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMCAxMCI+PGNpcmNsZSBjeD0iNSIgY3k9IjUiIHI9IjUiIGZpbGw9IiMxRjRFNzkiLz48L3N2Zz4=","width":"1.5cm","height":"1.5cm","alt":"Blue dot"}},
  {"command":"set","path":"/body/p[last()]","props":{"align":"center"}}
]
```

**Page break.** Prefer `"pageBreakBefore":"true"` on the next heading; otherwise:

```json
[{"command":"add","parent":"/body","type":"pagebreak","props":{}}]
```

**Header and footer.** `"field":"page"` inserts a live page number after `text`; append a run for text after it.

```json
[
  {"command":"add","parent":"/","type":"header","props":{"type":"default","text":"Acme · Q3 Review","align":"right","size":"9pt","color":"7F7F7F"}},
  {"command":"add","parent":"/","type":"footer","props":{"type":"default","text":"Page ","field":"page","align":"center","size":"9pt"}}
]
```

## Worked example: a short report

"Write a short report on our Q3 support results."

```json
[
  {"command":"set","path":"/","props":{"docDefaults.font":"Calibri","docDefaults.fontSize":"11pt"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Q3 Customer Support Review","style":"Title","font":"Cambria","size":"28pt","bold":"true","color":"1F4E79","spaceAfter":"4pt"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Support team · October 2026","size":"10pt","color":"7F7F7F","spaceAfter":"18pt"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Summary","style":"Heading1","font":"Cambria","size":"18pt","bold":"true","color":"1F4E79","spaceBefore":"12pt","spaceAfter":"6pt"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Ticket volume rose 12% while first-response time fell to 2.1 hours.","lineSpacing":"1.15x","spaceAfter":"8pt"}},
  {"command":"add","parent":"/body/p[last()]","type":"run","props":{"text":" Satisfaction held at 94%.","bold":"true"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Results","style":"Heading1","font":"Cambria","size":"18pt","bold":"true","color":"1F4E79","spaceBefore":"12pt","spaceAfter":"6pt"}},
  {"command":"add","parent":"/body","type":"table","props":{"data":"Metric,Q2,Q3;Tickets,4180,4690;First response (h),3.4,2.1;Satisfaction,93%,94%","width":"100%","border.all":"single;4;BFBFBF"}},
  {"command":"set","path":"/body/tbl[last()]/tr[1]","props":{"header":"true"}},
  {"command":"set","path":"/body/tbl[last()]/tr[1]/tc[1]","props":{"fill":"1F4E79","bold":"true","color":"FFFFFF"}},
  {"command":"set","path":"/body/tbl[last()]/tr[1]/tc[2]","props":{"fill":"1F4E79","bold":"true","color":"FFFFFF","align":"right"}},
  {"command":"set","path":"/body/tbl[last()]/tr[1]/tc[3]","props":{"fill":"1F4E79","bold":"true","color":"FFFFFF","align":"right"}},
  {"command":"set","path":"/body/tbl[last()]/tr[2]/tc[2]","props":{"align":"right"}},
  {"command":"set","path":"/body/tbl[last()]/tr[2]/tc[3]","props":{"align":"right"}},
  {"command":"set","path":"/body/tbl[last()]/tr[3]/tc[2]","props":{"align":"right"}},
  {"command":"set","path":"/body/tbl[last()]/tr[3]/tc[3]","props":{"align":"right"}},
  {"command":"set","path":"/body/tbl[last()]/tr[4]/tc[2]","props":{"align":"right"}},
  {"command":"set","path":"/body/tbl[last()]/tr[4]/tc[3]","props":{"align":"right"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"What drove the change","style":"Heading2","font":"Cambria","size":"14pt","bold":"true","color":"1F4E79","spaceBefore":"14pt","spaceAfter":"4pt"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"A new triage queue for billing questions","listStyle":"bullet"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Help-center articles for the five most common issues","listStyle":"bullet"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Weekend coverage from August","listStyle":"bullet"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Next steps","style":"Heading1","font":"Cambria","size":"18pt","bold":"true","color":"1F4E79","spaceBefore":"12pt","spaceAfter":"6pt"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Hire two agents before the holiday peak","listStyle":"numbered"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Extend triage to account questions","listStyle":"numbered"}},
  {"command":"add","parent":"/body","type":"paragraph","props":{"text":"Review satisfaction by channel in January","listStyle":"numbered"}},
  {"command":"add","parent":"/","type":"header","props":{"type":"default","text":"Q3 Customer Support Review","align":"right","size":"9pt","color":"7F7F7F"}},
  {"command":"add","parent":"/","type":"footer","props":{"type":"default","text":"Page ","field":"page","align":"center","size":"9pt","color":"7F7F7F"}}
]
```

For a Chinese report, set `docDefaults.font` and every heading `font` to "Microsoft YaHei", give body paragraphs `"firstLineChars":"200"`, `"align":"justify"`, `"lineSpacing":"1.5x"`, and write the footer as `"text":"第 "` with `"field":"page"`, then a run `" 页"` on `/footer[1]/p[1]`.

## Common mistakes

| Mistake | Instead |
|---|---|
| Bold Normal text as a heading | `"style":"Heading1"` plus explicit size |
| Heading1 followed directly by Heading3 | Keep levels consecutive |
| Empty paragraphs for spacing | `spaceBefore` / `spaceAfter` |
| `listStyle` on a run | It is a paragraph prop |
| `/body/p[12]` counted by hand | `/body/p[last()]` right after the add |
| Fill or bold set on a table row | Set it on each cell `tr[R]/tc[C]` |
| `pageBreakBefore` plus a `pagebreak` | One of them |
| Typed "Page 1" in the footer | `"field":"page"` |

<!-- Adapted with changes from OfficeCLI v1.0.160 skills/officecli-docx (Apache-2.0); see notices/officecli-skills.txt -->
