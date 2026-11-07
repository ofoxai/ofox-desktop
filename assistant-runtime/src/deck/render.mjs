// Lays a parsed deck out as an officecli batch. Each page type has one
// renderer that places its text in boxes sized with text.mjs, so the host's
// `view issues` check finds nothing to fix in a deck within the schema
// limits. Renderers place and size text; they never shorten, split or
// reword it. Sizes come from the theme's ladders, or are fixed below where
// the spec fixes them.
import {
  CANVAS,
  CONTENT,
  GAP_CM,
  TITLE_BOX,
  cm,
  columns,
  notesCommand,
  pageNumberCommand,
  slideCommand,
  slidePath,
  textboxCommand,
  titleCommands,
} from "./layout.mjs";
import { FIT, fitSize, heightCm, lineCount } from "./text.mjs";
import { imageSize } from "./image.mjs";
import { fontsFor } from "./theme.mjs";

// The host refuses a longer batch (create.rs MAX_COMMANDS).
const MAX_COMMANDS = 2000;

// A box holding several paragraphs puts this much space (pt) after each;
// every fit counts it the same way.
const SPACE_AFTER_PT = 6;
const FIT_OPTIONS = { spaceAfterPt: SPACE_AFTER_PT };

const paragraphs = (text) => text.split(/\r?\n/).length;

// A box's height: the estimate rounded up to 0.1 cm. The estimate holds at
// least one line, so a box is never shorter than one line.
const boxHeight = ({ height }) => Math.ceil(height * 10 - 1e-9) / 10;

// The largest size in `sizes` (large to small) at which `text` fits `width`
// within `room`, else the smallest, with the box height the text needs.
function fitBox(text, width, room, sizes) {
  const fit = fitSize(text, width, room, sizes, FIT_OPTIONS);
  return { size: fit.size, h: boxHeight(fit) };
}

// The largest size that keeps `text` on one line, else the smallest.
// `options` are lineCount's: TITLE_FONT for text in the title font.
const oneLineSize = (text, width, sizes, options) =>
  sizes.find((size) => lineCount(text, width, size, options) === 1) ??
  sizes.at(-1);

// Latin letters in the title font (Arial Black) are wider than the
// checker's estimate (see FIT.titleLatinRatio).
const TITLE_FONT = { latinRatio: FIT.titleLatinRatio };

// Boxes side by side share one size, the smallest any text needs (`sizeOf`
// picks a text's own size), so they read as a set, and one height, the
// tallest at that size, so they line up.
function fitRow(texts, width, sizeOf) {
  const size = Math.min(...texts.map(sizeOf));
  const h = Math.max(
    ...texts.map((text) => fitBox(text, width, Infinity, [size]).h),
  );
  return { size, h };
}

// A text box in the body font. Latin runs in CJK text take the Latin body
// font, so headings inside cards are body font, bold; only slide titles use
// the title fonts.
function textBox(ctx, role, text, box, style) {
  return textboxCommand(ctx.n, `s${ctx.n}-${role}`, text, {
    ...box,
    font: ctx.fonts.body,
    latin: ctx.fonts.latinBody,
    spaceAfter: paragraphs(text) > 1 ? SPACE_AFTER_PT : undefined,
    ...style,
  });
}

// The tops of boxes `heights` tall stacked `gap` apart and centered on the
// slide's height; an absent box (undefined) takes no room.
function stackCentered(heights, gap) {
  const present = heights.filter((h) => h !== undefined);
  const total =
    present.reduce((sum, h) => sum + h, 0) + gap * (present.length - 1);
  let y = (CANVAS.hCm - total) / 2;
  return heights.map((h) => {
    if (h === undefined) return undefined;
    const top = y;
    y += h + gap;
    return top;
  });
}

// --- Content pages: title, rule, cards -----------------------------------

// The short bar under a content page's title (decor.titleRule "bar").
const TITLE_RULE = { x: TITLE_BOX.x, y: 4.4, w: 2, h: 0.12 };

// A content page opens with its title, sized to one line, and the theme's
// rule under it. renderDeck adds the page number.
function contentPage(ctx, title) {
  const { n, theme, fonts } = ctx;
  const commands = [
    slideCommand(n, { background: theme.colors.background, title }),
    ...titleCommands(n, title, { theme, fonts }),
  ];
  if (theme.decor.titleRule === "bar")
    commands.push(
      textboxCommand(n, `s${n}-title-rule`, undefined, {
        ...TITLE_RULE,
        fill: theme.colors.primary,
      }),
    );
  return commands;
}

// Text inside a card sits this far in from the card's edge, on top of the
// text box's own inset.
const CARD_PAD_CM = 0.25;

// Sizes and heights for cards `w` wide holding `items` ({ heading, body }):
// headings take the largest size that keeps every one on one line, bodies
// the largest that fits every one within `bodyRoom`; all cards are as tall
// as the tallest.
function fitCards(items, w, { headingSizes, bodySizes, bodyRoom }) {
  const inner = w - 2 * CARD_PAD_CM;
  const heading = fitRow(
    items.map((item) => item.heading),
    inner,
    (text) => oneLineSize(text, inner, headingSizes),
  );
  const body = fitRow(
    items.map((item) => item.body),
    inner,
    (text) => fitBox(text, inner, bodyRoom, bodySizes).size,
  );
  return { heading, body, h: 2 * CARD_PAD_CM + heading.h + body.h };
}

// Card `i` at { x, y, w }: a rounded `card`-colored panel, its heading on
// top and its body under it. The panel is a roundRect at its default corner
// with no `adj`: the host refuses `adj:val 16667` as an address, so a deck
// with a card on it made no file at all.
function cardCommands(ctx, i, item, { x, y, w }, fitted) {
  const { colors } = ctx.theme;
  const inner = { x: x + CARD_PAD_CM, w: w - 2 * CARD_PAD_CM };
  const top = y + CARD_PAD_CM;
  return [
    textboxCommand(ctx.n, `s${ctx.n}-card-${i}`, undefined, {
      x,
      y,
      w,
      h: fitted.h,
      fill: colors.card,
      radius: true,
    }),
    textBox(
      ctx,
      `heading-${i}`,
      item.heading,
      { ...inner, y: top, h: fitted.heading.h },
      { size: fitted.heading.size, color: colors.primary, bold: true },
    ),
    textBox(
      ctx,
      `body-${i}`,
      item.body,
      { ...inner, y: top + fitted.heading.h, h: fitted.body.h },
      { size: fitted.body.size, color: colors.text },
    ),
  ];
}

// --- Covers: hero and cta -------------------------------------------------

// The cover title box (spec): one line up to 60pt, or two lines of 44pt.
const COVER_TITLE = { x: CONTENT.x, y: 6, w: CONTENT.w, h: 4.5 };
// Between the cover title box and the lines under it, and between those.
const COVER_GAP_CM = 0.3;
// Spec: the cover subtitle is 24pt, the meta or contact line 16pt.
const COVER_SUBTITLE_PT = 24;
const COVER_SMALL_PT = 16;

// A dark cover: the title centered at the foot of its box, so it sits on
// the subtitle, then the subtitle and a small line, all centered.
function cover(ctx, title, lines) {
  const { n, theme, fonts } = ctx;
  const { colors, sizes } = theme;
  const size = oneLineSize(title, COVER_TITLE.w, sizes.hero, TITLE_FONT);
  const [heading] = titleCommands(n, title, {
    theme,
    fonts,
    dark: true,
    size,
    align: "center",
    box: COVER_TITLE,
  });
  const commands = [
    slideCommand(n, { background: colors.dark, title }),
    { ...heading, props: { ...heading.props, valign: "bottom" } },
  ];
  let y = COVER_TITLE.y + COVER_TITLE.h + COVER_GAP_CM;
  for (const { role, text, size: pt } of lines) {
    if (text === undefined) continue;
    const { h } = fitBox(text, CONTENT.w, Infinity, [pt]);
    commands.push(
      textBox(
        ctx,
        role,
        text,
        { x: CONTENT.x, y, w: CONTENT.w, h },
        { size: pt, color: colors.onDarkMuted, align: "center" },
      ),
    );
    y += h + COVER_GAP_CM;
  }
  return commands;
}

export function renderHero(ctx, slide) {
  return cover(ctx, slide.title, [
    { role: "subtitle", text: slide.subtitle, size: COVER_SUBTITLE_PT },
    { role: "meta", text: slide.meta, size: COVER_SMALL_PT },
  ]);
}

export function renderCta(ctx, slide) {
  return cover(ctx, slide.title, [
    { role: "subtitle", text: slide.subtitle, size: COVER_SUBTITLE_PT },
    { role: "contact", text: slide.contact, size: COVER_SMALL_PT },
  ]);
}

// --- Breathing pages: statement and quote ---------------------------------

// A statement or quote takes at most this much of the slide's height (the
// spec's 29.21 × 6 cm block).
const FOCUS_ROOM_CM = 6;
// Between a statement or quote and the line under it.
const FOCUS_GAP_CM = 0.4;

// A breathing page: one block of text fitted to `sizes` within
// FOCUS_ROOM_CM and an optional smaller line under it (`line`: role, text,
// size, color), both centered across the content width and stacked in the
// middle of the slide. `style` colors and weights the block.
function focusPage(ctx, { role, text, sizes, style, line }) {
  const { n, theme } = ctx;
  const block = fitBox(text, CONTENT.w, FOCUS_ROOM_CM, sizes);
  const under =
    line.text === undefined
      ? undefined
      : fitBox(line.text, CONTENT.w, Infinity, [line.size]);
  const [y, lineY] = stackCentered([block.h, under?.h], FOCUS_GAP_CM);
  const commands = [
    slideCommand(n, { background: theme.colors.background }),
    textBox(
      ctx,
      role,
      text,
      { x: CONTENT.x, y, w: CONTENT.w, h: block.h },
      { size: block.size, align: "center", valign: "middle", ...style },
    ),
  ];
  if (under)
    commands.push(
      textBox(
        ctx,
        line.role,
        line.text,
        { x: CONTENT.x, y: lineY, w: CONTENT.w, h: under.h },
        { size: under.size, color: line.color, align: "center" },
      ),
    );
  return commands;
}

// A thin primary bar down the slide's left edge marks a statement page.
const STATEMENT_RULE = { x: 0, y: 0, w: 0.3, h: CANVAS.hCm };
// Spec: the note under a statement is 20pt.
const STATEMENT_NOTE_PT = 20;

export function renderStatement(ctx, slide) {
  const { n, theme } = ctx;
  const { colors, sizes } = theme;
  return [
    ...focusPage(ctx, {
      role: "statement",
      text: slide.statement,
      sizes: sizes.statement,
      style: { color: colors.primary, bold: true },
      line: {
        role: "note",
        text: slide.note,
        size: STATEMENT_NOTE_PT,
        color: colors.textSecondary,
      },
    }),
    textboxCommand(n, `s${n}-rule`, undefined, {
      ...STATEMENT_RULE,
      fill: colors.primary,
    }),
  ];
}

// Spec: a quote is 28–32pt, its source 16pt.
const QUOTE_SIZES = [32, 28];
const QUOTE_SOURCE_PT = 16;

export function renderQuote(ctx, slide) {
  const { colors } = ctx.theme;
  return focusPage(ctx, {
    role: "quote",
    text: slide.quote,
    sizes: QUOTE_SIZES,
    style: { color: colors.primary },
    line: {
      role: "source",
      text: slide.source,
      size: QUOTE_SOURCE_PT,
      color: colors.textMuted,
    },
  });
}

// --- Card pages: pillars and grid -----------------------------------------

// Room for a pillar's body: at the 60-character limit this lands on 16pt in
// three columns and 14pt in four, the sizes the spec gives pillars.
const PILLAR_BODY_ROOM_CM = 4.4;

export function renderPillars(ctx, slide) {
  const { sizes } = ctx.theme;
  const slots = columns(slide.pillars.length);
  const fitted = fitCards(slide.pillars, slots[0].w, {
    headingSizes: sizes.subtitle,
    bodySizes: sizes.card,
    bodyRoom: PILLAR_BODY_ROOM_CM,
  });
  return [
    ...contentPage(ctx, slide.title),
    ...slide.pillars.flatMap((item, index) =>
      cardCommands(
        ctx,
        index + 1,
        item,
        { ...slots[index], y: CONTENT.top },
        fitted,
      ),
    ),
  ];
}

// Spec: grid cells have 16pt bold headings over 14pt bodies, at most four
// to a row; more items split into two rows, the first row the longer. A
// heading that 16pt cannot keep on one line takes 14pt: at four to a row a
// 20-letter English heading wraps at 16pt, and two rows of such cards, with
// bodies that use both line breaks, would end past the content area.
const GRID_HEADING_SIZES = [16, 14];
const GRID_BODY_PT = 14;
const GRID_ROW_MAX = 4;

export function renderGrid(ctx, slide) {
  const { items } = slide;
  const perRow =
    items.length <= GRID_ROW_MAX ? items.length : Math.ceil(items.length / 2);
  const slots = columns(perRow);
  const fitted = fitCards(items, slots[0].w, {
    headingSizes: GRID_HEADING_SIZES,
    bodySizes: [GRID_BODY_PT],
    bodyRoom: Infinity,
  });
  return [
    ...contentPage(ctx, slide.title),
    ...items.flatMap((item, index) => {
      const row = Math.floor(index / perRow);
      const y = CONTENT.top + row * (fitted.h + GAP_CM);
      return cardCommands(
        ctx,
        index + 1,
        item,
        { ...slots[index % perRow], y },
        fitted,
      );
    }),
  ];
}

// --- Data pages: evidence, timeline, comparison, showcase -----------------

// Spec: a metric's label is 16pt bold over a 14pt note; the detail list
// under the metrics is 18pt.
const METRIC_LABEL_PT = 16;
const METRIC_NOTE_PT = 14;
const DETAIL_PT = 18;

// The KPI ladder from the size the spec gives `count` metrics down: the
// largest (72pt) for one or two, the next (60) for three, 48 for four.
const kpiSizes = (kpi, count) => kpi.slice(Math.max(0, count - 2));

// Figures are set in the theme's data font. In a CJK deck that font takes
// the Latin slot, so any CJK in a figure stays in the body font.
const dataFonts = (fonts) =>
  fonts.latinBody === undefined ? { font: fonts.data } : { latin: fonts.data };

export function renderEvidence(ctx, slide) {
  const { colors, sizes } = ctx.theme;
  const { metrics, detail } = slide;
  const slots = columns(metrics.length);
  const { w } = slots[0];
  const ladder = kpiSizes(sizes.kpi, metrics.length);
  const value = fitRow(
    metrics.map((metric) => metric.value),
    w,
    (text) => oneLineSize(text, w, ladder),
  );
  const label = fitRow(
    metrics.map((metric) => metric.label),
    w,
    () => METRIC_LABEL_PT,
  );
  const notes = metrics
    .map((metric) => metric.note)
    .filter((text) => text !== undefined);
  const note =
    notes.length > 0 ? fitRow(notes, w, () => METRIC_NOTE_PT) : undefined;
  // Each row (values, labels, notes) is as tall as its tallest text, so the
  // block is at least the tallest metric's stack and the rows line up.
  // Metrics have no card, so there is no padding to add.
  const h = value.h + label.h + (note?.h ?? 0);

  // The metrics, and the detail list a gap under them, sit together in the
  // middle of the content area.
  const list = detail?.join("\n");
  const listH =
    list === undefined
      ? undefined
      : fitBox(list, CONTENT.w, Infinity, [DETAIL_PT]).h;
  const total = h + (listH === undefined ? 0 : GAP_CM + listH);
  const top = CONTENT.top + (CONTENT.bottom - CONTENT.top - total) / 2;

  const commands = contentPage(ctx, slide.title);
  metrics.forEach((metric, index) => {
    const i = index + 1;
    const { x } = slots[index];
    commands.push(
      textBox(
        ctx,
        `value-${i}`,
        metric.value,
        { x, y: top, w, h: value.h },
        {
          size: value.size,
          color: colors.primary,
          bold: true,
          align: "center",
          ...dataFonts(ctx.fonts),
        },
      ),
      textBox(
        ctx,
        `label-${i}`,
        metric.label,
        { x, y: top + value.h, w, h: label.h },
        { size: label.size, color: colors.text, bold: true, align: "center" },
      ),
    );
    if (metric.note !== undefined)
      commands.push(
        textBox(
          ctx,
          `note-${i}`,
          metric.note,
          { x, y: top + value.h + label.h, w, h: note.h },
          { size: note.size, color: colors.textSecondary, align: "center" },
        ),
      );
  });
  if (list !== undefined)
    commands.push(
      textBox(
        ctx,
        "detail",
        list,
        { x: CONTENT.x, y: top + h + GAP_CM, w: CONTENT.w, h: listH },
        { size: DETAIL_PT, color: colors.text, list: "bullet" },
      ),
    );
  return commands;
}

// Spec: a step's number is white 20pt bold in a 1.5 cm circle over a card
// with a 16pt bold heading and a 14pt body; a 0.12 cm track joins the
// circles.
const STEP_MARK_CM = 1.5;
const STEP_NUMBER_PT = 20;
const STEP_HEADING_PT = 16;
const STEP_BODY_PT = 14;
const TRACK_CM = 0.12;
// Between a step's circle and its card.
const STEP_GAP_CM = 0.3;

export function renderTimeline(ctx, slide) {
  const { n, theme } = ctx;
  const { colors } = theme;
  const { steps } = slide;
  const slots = columns(steps.length);
  const fitted = fitCards(steps, slots[0].w, {
    headingSizes: [STEP_HEADING_PT],
    bodySizes: [STEP_BODY_PT],
    bodyRoom: Infinity,
  });
  const centers = slots.map(({ x, w }) => x + w / 2);
  const middle = CONTENT.top + STEP_MARK_CM / 2;
  const cardY = CONTENT.top + STEP_MARK_CM + STEP_GAP_CM;
  return [
    ...contentPage(ctx, slide.title),
    // Added before the circles, so they are drawn over it.
    textboxCommand(n, `s${n}-track`, undefined, {
      x: centers[0],
      y: middle - TRACK_CM / 2,
      w: centers.at(-1) - centers[0],
      h: TRACK_CM,
      fill: colors.secondary,
    }),
    ...steps.flatMap((step, index) => {
      const i = index + 1;
      return [
        textBox(
          ctx,
          `step-${i}-no`,
          String(i),
          {
            x: centers[index] - STEP_MARK_CM / 2,
            y: CONTENT.top,
            w: STEP_MARK_CM,
            h: STEP_MARK_CM,
          },
          {
            size: STEP_NUMBER_PT,
            color: colors.onDark,
            bold: true,
            align: "center",
            valign: "middle",
            fill: colors.primary,
            preset: "ellipse",
          },
        ),
        ...cardCommands(ctx, i, step, { ...slots[index], y: cardY }, fitted),
      ];
    }),
  ];
}

// Spec: each side's heading is 20pt bold on a 1.4 cm color bar, its points
// an 18pt list under it; a 2 cm circle with a 20pt bold "VS" sits between
// the bars.
const SIDE_BAR_CM = 1.4;
const SIDE_HEADING_PT = 20;
const POINTS_PT = 18;
const VS_CM = 2;
const VS_PT = 20;
// Between a side's bar and its points.
const SIDE_GAP_CM = 0.3;

export function renderComparison(ctx, slide) {
  const { colors } = ctx.theme;
  const slots = columns(2);
  const { w } = slots[0];
  const sides = [
    { role: "left", ...slide.left, fill: colors.primary, color: colors.onDark },
    {
      role: "right",
      ...slide.right,
      fill: colors.secondary,
      color: colors.text,
    },
  ];
  // A heading the bar cannot hold on one line makes both bars taller.
  const bar = Math.max(
    SIDE_BAR_CM,
    fitRow(
      sides.map((side) => side.heading),
      w,
      () => SIDE_HEADING_PT,
    ).h,
  );
  // The bars sit low enough that the VS circle centered on them starts
  // inside the content area.
  const barY = CONTENT.top + Math.max(0, (VS_CM - bar) / 2);
  const points = fitRow(
    sides.map((side) => side.points.join("\n")),
    w,
    () => POINTS_PT,
  );
  return [
    ...contentPage(ctx, slide.title),
    ...sides.flatMap((side, index) => {
      const { x } = slots[index];
      return [
        textBox(
          ctx,
          `${side.role}-heading`,
          side.heading,
          { x, y: barY, w, h: bar },
          {
            size: SIDE_HEADING_PT,
            color: side.color,
            bold: true,
            align: "center",
            valign: "middle",
            fill: side.fill,
          },
        ),
        textBox(
          ctx,
          `${side.role}-points`,
          side.points.join("\n"),
          { x, y: barY + bar + SIDE_GAP_CM, w, h: points.h },
          { size: POINTS_PT, color: colors.text, list: "bullet" },
        ),
      ];
    }),
    // Added after the bars, so it is drawn over their inner ends.
    textBox(
      ctx,
      "vs",
      "VS",
      {
        x: CONTENT.x + (CONTENT.w - VS_CM) / 2,
        y: barY + (bar - VS_CM) / 2,
        w: VS_CM,
        h: VS_CM,
      },
      {
        size: VS_PT,
        color: colors.primary,
        bold: true,
        align: "center",
        valign: "middle",
        fill: colors.card,
        preset: "ellipse",
      },
    ),
  ];
}

// Spec: the media fills 20 × 11 cm at the content area's top left, the
// body (18pt or larger) takes the rest of the content width beside it, and
// the 14pt caption sits under it. Without a body the media and caption
// move to the middle.
const MEDIA = { y: CONTENT.top, w: 20, h: 11 };
const SHOWCASE_BODY = {
  x: CONTENT.x + MEDIA.w + GAP_CM,
  w: CONTENT.w - MEDIA.w - GAP_CM,
};
const CAPTION = { y: 16.3, size: 14 };

// officecli's chart type for each chart kind.
const CHART_TYPES = { bar: "column", line: "line", pie: "pie" };

// Spec: table text is 16pt up to four columns, 14 at five and 13 (the
// floor for any text) at six.
const tableSize = (count) => (count <= 4 ? 16 : count === 5 ? 14 : 13);

// A cell as officecli's inline table data reads it: a cell that holds a
// separator or a quote is quoted, its own quotes doubled.
const tableCell = (text) =>
  /[,;"]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;

// A box as officecli's placement props.
const placement = ({ x, y, w, h }) => ({
  x: cm(x),
  y: cm(y),
  width: cm(w),
  height: cm(h),
});

function chartCommand(ctx, chart, box) {
  const { n, theme } = ctx;
  const pie = chart.kind === "pie";
  // A pie colors each slice; bars and lines color each series.
  const colored = pie ? chart.categories.length : chart.series.length;
  return {
    command: "add",
    parent: slidePath(n),
    type: "chart",
    props: {
      name: `s${n}-chart`,
      chartType: CHART_TYPES[chart.kind],
      categories: chart.categories.join(","),
      data: chart.series
        .map(({ name, values }) => `${name}:${values.join(",")}`)
        .join(";"),
      colors: theme.colors.chart.slice(0, colored).join(","),
      // A legend names a pie's slices, or tells two or more series apart.
      legend: pie || chart.series.length > 1 ? "bottom" : "none",
      title: "none",
      ...(pie && { dataLabels: "value" }),
      ...placement(box),
    },
  };
}

// The table, its text style, and its header row in white bold. Every row
// is as tall as the cell with the most lines needs, so the table is as
// tall as its rows.
function tableCommands(ctx, table, box) {
  const { n, theme, fonts } = ctx;
  const { colors } = theme;
  const name = `s${n}-table`;
  const path = `${slidePath(n)}/table[@name=${name}]`;
  const rows = [table.columns, ...table.rows];
  const count = table.columns.length;
  const size = tableSize(count);
  const width = box.w / count;
  const lines = Math.max(
    ...rows.flat().map((cell) => lineCount(cell, width, size)),
  );
  const rowH = boxHeight({ height: heightCm(lines, size) });
  return [
    {
      command: "add",
      parent: slidePath(n),
      type: "table",
      props: {
        name,
        data: rows.map((row) => row.map(tableCell).join(",")).join(";"),
        colWidths: Array(count).fill(cm(width)).join(","),
        headerFill: colors.primary,
        bodyFill: colors.card,
        ...placement({ ...box, h: rowH * rows.length }),
      },
    },
    {
      command: "set",
      path,
      props: {
        font: fonts.body,
        ...(fonts.latinBody && { "font.latin": fonts.latinBody }),
        size: String(size),
        color: colors.text,
        valign: "middle",
      },
    },
    {
      command: "set",
      path: `${path}/tr[1]`,
      props: { bold: "true", color: colors.onDark },
    },
  ];
}

// The largest box with the picture's proportions that fits `box`, centered
// in it, so the picture is not stretched; the whole box when its size
// cannot be read.
function pictureBox(image, box) {
  const size = imageSize(image);
  if (size === null) return box;
  const scale = Math.min(box.w / size.width, box.h / size.height);
  const w = size.width * scale;
  const h = size.height * scale;
  return { x: box.x + (box.w - w) / 2, y: box.y + (box.h - h) / 2, w, h };
}

function pictureCommand(ctx, slide, box) {
  const { n } = ctx;
  return {
    command: "add",
    parent: slidePath(n),
    type: "picture",
    props: {
      name: `s${n}-image`,
      src: slide.media.image,
      alt: slide.caption ?? slide.title,
      ...placement(pictureBox(slide.media.image, box)),
    },
  };
}

function mediaCommands(ctx, slide, box) {
  const { chart, table } = slide.media;
  if (chart !== undefined) return [chartCommand(ctx, chart, box)];
  if (table !== undefined) return tableCommands(ctx, table, box);
  return [pictureCommand(ctx, slide, box)];
}

export function renderShowcase(ctx, slide) {
  const { colors, sizes } = ctx.theme;
  const { caption, body } = slide;
  const x =
    body === undefined ? CONTENT.x + (CONTENT.w - MEDIA.w) / 2 : CONTENT.x;
  const commands = [
    ...contentPage(ctx, slide.title),
    ...mediaCommands(ctx, slide, { ...MEDIA, x }),
  ];
  if (body !== undefined) {
    const fit = fitBox(body, SHOWCASE_BODY.w, MEDIA.h, sizes.body);
    commands.push(
      textBox(
        ctx,
        "body",
        body,
        { ...SHOWCASE_BODY, y: MEDIA.y, h: fit.h },
        { size: fit.size, color: colors.text },
      ),
    );
  }
  if (caption !== undefined) {
    const { h } = fitBox(caption, MEDIA.w, Infinity, [CAPTION.size]);
    commands.push(
      textBox(
        ctx,
        "caption",
        caption,
        { x, y: CAPTION.y, w: MEDIA.w, h },
        { size: CAPTION.size, color: colors.textMuted, align: "center" },
      ),
    );
  }
  return commands;
}

// --- The deck ---------------------------------------------------------------

export const RENDERERS = {
  hero: renderHero,
  statement: renderStatement,
  pillars: renderPillars,
  evidence: renderEvidence,
  timeline: renderTimeline,
  comparison: renderComparison,
  grid: renderGrid,
  quote: renderQuote,
  cta: renderCta,
  showcase: renderShowcase,
};

// Covers and breathing pages carry no page number (nor a title rule).
const UNNUMBERED = new Set(["hero", "statement", "quote", "cta"]);

// Every box a page placed lies on the canvas. Schema limits keep in-limit
// content on the page; a deck that slips past them fails here rather than
// sending shapes off the slide.
function assertOnPage(ctx, type, page) {
  const off = page.some(({ props }) => {
    if (props?.y === undefined || props.height === undefined) return false;
    const [x, y, w, h] = [props.x, props.y, props.width, props.height].map(
      (length) => Number.parseFloat(length),
    );
    return x < 0 || y < 0 || x + w > CANVAS.wCm || y + h > CANVAS.hCm;
  });
  if (off)
    throw new Error(`slide ${ctx.n} (${type}): content does not fit the page`);
}

// The batch for a deck parseDeck has read, in `theme`: every page in order,
// each followed by its speaker notes and, on content pages, its number.
export function renderDeck(deck, theme) {
  const fonts = fontsFor(theme, deck.language);
  const total = deck.slides.length;
  const commands = deck.slides.flatMap((slide, index) => {
    const ctx = { n: index + 1, total, theme, fonts, language: deck.language };
    const page = RENDERERS[slide.type](ctx, slide);
    assertOnPage(ctx, slide.type, page);
    if (slide.notes !== undefined) page.push(notesCommand(ctx.n, slide.notes));
    if (theme.decor.pageNumber && !UNNUMBERED.has(slide.type))
      page.push(pageNumberCommand(ctx.n, total, theme, fonts));
    return page;
  });
  if (commands.length > MAX_COMMANDS)
    throw new Error(`deck too large: ${commands.length} commands`);
  return { commands, slides: total };
}
