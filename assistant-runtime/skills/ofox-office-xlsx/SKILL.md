---
name: ofox-office-xlsx
description: Read before building an Excel workbook (.xlsx) with create_document. Sheet layout rules and batch commands for cells, formulas, formats, charts.
---

# Excel workbooks

`create_document({ name: "sales.xlsx", format: "xlsx", content: [...] })` starts from an empty workbook with one sheet, `Sheet1` (Calibri 11pt). `content` is a JSON array of commands run in order as one atomic batch: if one fails, nothing is saved and the result names its `index` and error; fix it and resend the whole array (at most 2 retries).

## Ground rules

- One workbook = one `create_document` call with the whole batch. Lay out every sheet first; never patch it with a second call.
- Stable paths use cell addresses: `/Sheet1/B2`, ranges `/Sheet1/A1:C1`, columns `/Sheet1/col[B]`, the sheet `/Sheet1`. Rename or add sheets first, then use the new names everywhere (paths, formulas, `dataRange`). Prefer sheet names without spaces.
- Prop values are strings: `"value":"42000"` (numbers are detected), `"true"`, colors as hex without `#`.
- Text color is `font.color`, background is `fill`; a bare `color` on a cell is ignored. Use the dotted names shown below.
- Labels and headers in the user's language. No placeholders (`TODO`, `xxxx`, `{var}`) in cells, chart titles or series names.
- Use only the commands and props below; unknown props are ignored or rejected.
- `issues` in the result list formula errors such as `#DIV/0!` and `#REF!`; fix every one and recreate.

## Layout rules

**Formulas, not typed results.** Anything computed from other cells is a formula (`SUM(B2:B13)`, `B5*(1+B2)`), so the workbook stays live. Put assumptions in their own labeled cells and reference them; `B5*1.05` hides a number.

**Zero formula errors.** Guard every division: `IFERROR(B5/C5,0)` or `IF(C5=0,0,B5/C5)`. Check that ranges cover every row (`SUM(B2:B13)` when data ends at row 13).

**Header row.** Row 1 (or the row under a merged title) holds the column headers: bold, fill in one accent color with white text, centered; freeze the panes below it so it stays visible.

**Explicit widths.** Columns do not auto-fit: set `width` on every column the reader uses — labels 20–25, numbers 12–15, dates 12, short codes 8–10. CJK text needs about twice the Latin width. A column too narrow shows `###`.

**Number formats.** Thousands separators `#,##0`; money `#,##0.00` with the currency in the header ("Revenue (USD)", "金额（元）"), not in every cell; percentages `0.0%`; negatives in parentheses for finance `#,##0;(#,##0)`. Years and codes are text: `"type":"string"`, so 2026 never shows as 2,026.

**One font, quiet color.** Keep the default font. Color carries meaning, not decoration: in financial models blue text (0000FF) marks typed inputs, black marks formulas, green (008000) marks links to other sheets.

**Charts.** Column (or bar for long labels, 6+ categories) compares categories; line shows a trend over time; pie only for 2–5 parts of a whole. More than 3 series or 8 categories: split the chart or use a table. Give each chart a real title.

## Commands

**Cells and formulas.** `value` for data, `formula` without a leading `=`. Formatting props: `font.bold`, `font.italic`, `font.color`, `font.size`, `fill`, `alignment.horizontal` (left, center, right), `alignment.wrapText`, `numberformat`, `type` (`"string"`).

```json
[
  {"command":"set","path":"/Sheet1/A1","props":{"value":"Month"}},
  {"command":"set","path":"/Sheet1/B1","props":{"value":"Revenue"}},
  {"command":"set","path":"/Sheet1/A2","props":{"value":"Jan"}},
  {"command":"set","path":"/Sheet1/B2","props":{"value":"42000","numberformat":"#,##0"}},
  {"command":"set","path":"/Sheet1/A3","props":{"value":"Feb"}},
  {"command":"set","path":"/Sheet1/B3","props":{"value":"45000","numberformat":"#,##0"}},
  {"command":"set","path":"/Sheet1/A4","props":{"value":"Total","font.bold":"true"}},
  {"command":"set","path":"/Sheet1/B4","props":{"formula":"SUM(B2:B3)","font.bold":"true","numberformat":"#,##0"}}
]
```

**Format a range.** A range path applies `font.bold`, `font.color`, `fill`, `alignment.horizontal` and `numberformat` to every cell in it.

```json
[{"command":"set","path":"/Sheet1/A1:B1","props":{"font.bold":"true","font.color":"FFFFFF","fill":"1F4E79","alignment.horizontal":"center"}}]
```

**Column width, freeze, merge.** `freeze` names the first cell below and right of the frozen area: `"A2"` freezes row 1. `merge` joins a range; its text lives in the first cell.

```json
[
  {"command":"set","path":"/Sheet1/col[A]","props":{"width":"14"}},
  {"command":"set","path":"/Sheet1/col[B]","props":{"width":"14"}},
  {"command":"set","path":"/Sheet1","props":{"freeze":"A2"}},
  {"command":"set","path":"/Sheet1/D1","props":{"value":"Notes"}},
  {"command":"set","path":"/Sheet1/D1:F1","props":{"merge":"true","alignment.horizontal":"center"}}
]
```

**Sheets.** Rename with `name` on the sheet; add more with `add`.

```json
[
  {"command":"set","path":"/Sheet1","props":{"name":"Data"}},
  {"command":"add","parent":"/","type":"sheet","props":{"name":"Summary"}},
  {"command":"set","path":"/Summary/A1","props":{"formula":"Data!B4"}}
]
```

**Chart.** `chartType`: column, bar, line or pie. `dataRange` always names the sheet and is 2-D: first column = categories, first row = series names. `anchor` is the cell box the chart covers; 5–6 categories need about 8 columns × 16 rows. Optional: `colors` (comma-separated hex per series), `legend` (`"bottom"`, `"none"`), `dataLabels` (`"value"`, `"percent"`).

```json
[{"command":"add","parent":"/Data","type":"chart","props":{"chartType":"column","dataRange":"Data!A1:B3","title":"Revenue by month","anchor":"H2:O18","colors":"1F4E79","legend":"none"}}]
```

## Worked example: a 3-column sheet

「做一个上半年销售表，算出合计和环比，再配一张折线图」

```json
[
  {"command":"set","path":"/Sheet1","props":{"name":"销售"}},
  {"command":"set","path":"/销售/A1","props":{"value":"2026 年上半年销售"}},
  {"command":"set","path":"/销售/A1:C1","props":{"merge":"true","font.bold":"true","alignment.horizontal":"center"}},
  {"command":"set","path":"/销售/A1","props":{"font.size":"14"}},
  {"command":"set","path":"/销售/A2","props":{"value":"月份"}},
  {"command":"set","path":"/销售/B2","props":{"value":"销售额（万元）"}},
  {"command":"set","path":"/销售/C2","props":{"value":"环比增长"}},
  {"command":"set","path":"/销售/A2:C2","props":{"font.bold":"true","font.color":"FFFFFF","fill":"1F4E79","alignment.horizontal":"center"}},
  {"command":"set","path":"/销售/A3","props":{"value":"1月"}},
  {"command":"set","path":"/销售/B3","props":{"value":"120"}},
  {"command":"set","path":"/销售/A4","props":{"value":"2月"}},
  {"command":"set","path":"/销售/B4","props":{"value":"98"}},
  {"command":"set","path":"/销售/C4","props":{"formula":"IFERROR(B4/B3-1,0)"}},
  {"command":"set","path":"/销售/A5","props":{"value":"3月"}},
  {"command":"set","path":"/销售/B5","props":{"value":"135"}},
  {"command":"set","path":"/销售/C5","props":{"formula":"IFERROR(B5/B4-1,0)"}},
  {"command":"set","path":"/销售/A6","props":{"value":"4月"}},
  {"command":"set","path":"/销售/B6","props":{"value":"142"}},
  {"command":"set","path":"/销售/C6","props":{"formula":"IFERROR(B6/B5-1,0)"}},
  {"command":"set","path":"/销售/A7","props":{"value":"5月"}},
  {"command":"set","path":"/销售/B7","props":{"value":"150"}},
  {"command":"set","path":"/销售/C7","props":{"formula":"IFERROR(B7/B6-1,0)"}},
  {"command":"set","path":"/销售/A8","props":{"value":"6月"}},
  {"command":"set","path":"/销售/B8","props":{"value":"161"}},
  {"command":"set","path":"/销售/C8","props":{"formula":"IFERROR(B8/B7-1,0)"}},
  {"command":"set","path":"/销售/A9","props":{"value":"合计","font.bold":"true"}},
  {"command":"set","path":"/销售/B9","props":{"formula":"SUM(B3:B8)","font.bold":"true"}},
  {"command":"set","path":"/销售/B3:B9","props":{"numberformat":"#,##0"}},
  {"command":"set","path":"/销售/C4:C8","props":{"numberformat":"0.0%"}},
  {"command":"set","path":"/销售/col[A]","props":{"width":"12"}},
  {"command":"set","path":"/销售/col[B]","props":{"width":"18"}},
  {"command":"set","path":"/销售/col[C]","props":{"width":"14"}},
  {"command":"set","path":"/销售","props":{"freeze":"A3"}},
  {"command":"add","parent":"/销售","type":"chart","props":{"chartType":"line","dataRange":"销售!A2:B8","title":"月度销售额（万元）","anchor":"E2:L18","colors":"1F4E79","legend":"none"}}
]
```

Rows 1–2 stay frozen; the chart reads months 1–6 only, not the total row.

## Common mistakes

| Mistake | Instead |
|---|---|
| A computed total typed as a number | `"formula":"SUM(B3:B8)"` |
| `"color":"FF0000"` on a cell | `"font.color"` or `"fill"` |
| No column widths | `width` on every used column |
| Year typed as a number (2,026) | `"type":"string"` |
| `dataRange` of a single column, or without the sheet | `"Data!A1:B13"` |
| The total row inside a chart's range | Range over the data rows only |
| A sheet renamed after formulas used the old name | Rename first |
| Division without a guard | `IFERROR(…,0)` |

<!-- Adapted with changes from OfficeCLI (skills/officecli-xlsx, v1.0.160, Apache-2.0); see notices/officecli-skills.txt -->
