---
name: ofox-office-pdf
description: Read before writing a PDF with create_document. How to write the Typst body — headings, lists, tables, emphasis, page breaks, CJK text.
---

# PDF documents

`create_document({ name: "article.pdf", format: "pdf", content: "<Typst body>" })` compiles Typst markup into a PDF. `content` is one string: the body only. The host puts a prelude in front of it that already sets A4 pages with margins and page numbers, an 11pt font chain that covers Chinese, Japanese and Latin text, justified paragraphs, heading styles (level 1 20pt, level 2 15pt, level 3 12.5pt, all bold, no numbers), the document language, and a light table style.

## Ground rules

- One document = one `create_document` call with the complete text. Never send a partial draft.
- Write the body only. Never write `#set page`, `#set text`, `#import`, `#include` or package names (`@preview/…`); do not pick fonts or redefine headings. The one allowed `#set` is the paragraph indent below.
- Do not use images in this version: the body cannot reference your files. Use text and tables.
- All text in the user's language; no placeholders (`TODO`, `xxxx`).
- A compile error returns the message and quotes the offending source line (line numbers also count the prelude). Fix that spot and resend the whole body (at most 2 retries).

## Structure

- Start with a title block (below), not a level-1 heading: `=` headings are the sections.
- `=` for sections, `==` for subsections, `===` rarely. Never skip a level. Want "一、" or "1." in a heading? Type it into the heading text; automatic numbering is off.
- Paragraphs are separated by a blank line. Keep each paragraph on one source line.
- Short paragraphs (3–6 sentences), lists for parallel points, tables for comparisons. A two-page article usually has 2–4 sections.

## Markup

**Title block.** Center the title, then a muted line for author or date:

```typst
#align(center)[
  #text(size: 22pt, weight: "bold")[Document title]

  #text(fill: luma(110))[Author · November 2026]
]
```

**Headings.**

```typst
= Section
== Subsection
```

**Emphasis.** `*bold*` and `_italic_` work at word boundaries and right next to CJK characters. Inside a Latin word use `#strong[...]` or `#emph[...]`. CJK fonts have no italics: emphasize Chinese and Japanese with bold.

**Lists.** `-` for bullets, `+` for auto-numbered steps; indent two spaces to nest.

```typst
- First point
- Second point
  - Detail
+ Step one
+ Step two
```

**Line break and spacing.** A backslash at the end of a line breaks it inside the paragraph; `#v(1em)` adds vertical space; `#line(length: 100%, stroke: 0.5pt + luma(180))` draws a divider.

**Table.** `columns` gives the column sizes (`auto` = fit, `1fr` = share the rest); every cell is `[...]`, filled row by row, so the cell count must be a multiple of the column count. `table.header` marks the header row (it repeats on page breaks); `fill` shades it; `align` takes one alignment per column.

```typst
#table(
  columns: (auto, 1fr, auto),
  align: (left, left, right),
  fill: (x, y) => if y == 0 { luma(235) },
  table.header([*Item*], [*Notes*], [*Cost*]),
  [Hosting], [Two regions], [1,200],
  [Support], [Business hours], [800],
)
```

**Quote, link, footnote, code.**

```typst
#quote(block: true, attribution: [Lao Tzu])[A journey of a thousand miles begins with a single step.]

See #link("https://typst.app/docs")[the Typst docs] for details.
A sentence with a note.#footnote[The note text.]
```

Code goes in a fenced raw block: three backticks with a language name, the code, three backticks.

**Page break.** `#pagebreak()` on its own line in the main text; inside `#block[...]`, `#box[...]` or a table cell it is a compile error.

**Special characters.** `#` `$` `*` `_` `@` `<` `` ` `` `\` `~` have meaning in markup; write them literally with a backslash: `\$15`, `user\@example.com`, `\#1`, `a \* b`. A line that starts with `=`, `-`, `+` or `1.` becomes a heading or list item; escape the first character (`\-`) when you mean plain text. Inside `[...]` brackets must balance; write a lone bracket as `\]`. `--` prints an en dash and `---` an em dash.

## Chinese and Japanese text

- Full-width punctuation（，。：；！？“”《》）. No spaces between CJK and Latin letters or digits: Typst adds the small gap itself.
- Chinese essays and letters indent every paragraph by two characters. Put this once, right after the title block (rules apply to what follows, so the title stays centered):

```typst
#set par(first-line-indent: (amount: 2em, all: true))
```

- Bold, not italic, for emphasis; headings like `= 一、历史` are fine.

## Worked example: a two-section article

「写一个 pdf，介绍我的家乡」 — the whole `content` string:

```typst
#align(center)[
  #text(size: 22pt, weight: "bold")[我的家乡：杭州]

  #text(fill: luma(110))[2026年11月]
]

#set par(first-line-indent: (amount: 2em, all: true))

杭州位于浙江省北部、钱塘江下游，是一座有两千多年建城史的城市。这里既有*西湖*的山水，也有繁忙的数字经济，古老与现代在同一条街上并存。

= 一、历史与人文

杭州在五代吴越国和南宋时两度成为都城，南宋时更是当时世界上人口最多的城市之一。白居易和苏轼都曾在这里做官，留下了白堤和苏堤。

今天还能看到的文化印记包括：

- 灵隐寺：始建于东晋，已有一千七百多年历史
- 京杭大运河：杭州是它的南端终点
- 龙井茶：采茶、炒茶的手艺代代相传

= 二、风景与美食

西湖是杭州的名片，四季各有风景：春天看苏堤桃柳，夏天赏曲院荷花，秋天闻满觉陇桂香，冬天等断桥残雪。

#table(
  columns: (auto, 1fr, auto),
  align: (left, left, center),
  fill: (x, y) => if y == 0 { luma(235) },
  table.header([*去处*], [*看点*], [*建议时长*]),
  [西湖], [湖光山色，环湖骑行], [半天],
  [灵隐寺], [古刹与飞来峰石刻], [2小时],
  [河坊街], [老字号与小吃], [1小时],
)

说到吃，*西湖醋鱼*酸甜鲜嫩，*东坡肉*肥而不腻，再配一杯明前龙井，就是最地道的杭州味道。

#quote(block: true, attribution: [苏轼])[欲把西湖比西子，淡妆浓抹总相宜。]

欢迎你来杭州，走一走湖边，尝一尝家乡的味道。
```

## Common mistakes

| Mistake | Instead |
|---|---|
| `#set page(...)`, `#set text(font: ...)`, `#import` | Leave layout and fonts to the prelude |
| The title written as `= Title` | The centered title block |
| `$100` or `a@b.com` unescaped | `\$100`, `a\@b.com` |
| 7 cells in a 3-column table (ragged last row) | Fill every row, `[]` for an empty cell |
| `#pagebreak()` inside `#block[...]` or a table cell | On its own line in the main text |
| `#image("photo.jpg")` | Describe it, or use a table |
| Typed numbers plus heading numbering | Numbers go into the heading text |
