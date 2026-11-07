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
  columns,
  notesCommand,
  pageNumberCommand,
  slideCommand,
  textboxCommand,
  titleCommands,
} from "./layout.mjs";
import { fitSize, lineCount } from "./text.mjs";
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
const oneLineSize = (text, width, sizes) =>
  sizes.find((size) => lineCount(text, width, size) === 1) ?? sizes.at(-1);

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
// top and its body under it.
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
  const size = oneLineSize(title, COVER_TITLE.w, sizes.hero);
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

// A thin primary bar down the slide's left edge marks a statement page.
const STATEMENT_RULE = { x: 0, y: 0, w: 0.3, h: CANVAS.hCm };
// Spec: the note under a statement is 20pt.
const STATEMENT_NOTE_PT = 20;

export function renderStatement(ctx, slide) {
  const { n, theme } = ctx;
  const { colors, sizes } = theme;
  const main = fitBox(
    slide.statement,
    CONTENT.w,
    FOCUS_ROOM_CM,
    sizes.statement,
  );
  const note =
    slide.note === undefined
      ? undefined
      : fitBox(slide.note, CONTENT.w, Infinity, [STATEMENT_NOTE_PT]);
  const [y, noteY] = stackCentered([main.h, note?.h], FOCUS_GAP_CM);
  const commands = [
    slideCommand(n, { background: colors.background }),
    textboxCommand(n, `s${n}-rule`, undefined, {
      ...STATEMENT_RULE,
      fill: colors.primary,
    }),
    textBox(
      ctx,
      "statement",
      slide.statement,
      { x: CONTENT.x, y, w: CONTENT.w, h: main.h },
      {
        size: main.size,
        color: colors.primary,
        bold: true,
        align: "center",
        valign: "middle",
      },
    ),
  ];
  if (note)
    commands.push(
      textBox(
        ctx,
        "note",
        slide.note,
        { x: CONTENT.x, y: noteY, w: CONTENT.w, h: note.h },
        { size: note.size, color: colors.textSecondary, align: "center" },
      ),
    );
  return commands;
}

// Spec: a quote is 28–32pt, its source 16pt.
const QUOTE_SIZES = [32, 28];
const QUOTE_SOURCE_PT = 16;

export function renderQuote(ctx, slide) {
  const { n, theme } = ctx;
  const { colors } = theme;
  const main = fitBox(slide.quote, CONTENT.w, FOCUS_ROOM_CM, QUOTE_SIZES);
  const source =
    slide.source === undefined
      ? undefined
      : fitBox(slide.source, CONTENT.w, Infinity, [QUOTE_SOURCE_PT]);
  const [y, sourceY] = stackCentered([main.h, source?.h], FOCUS_GAP_CM);
  const commands = [
    slideCommand(n, { background: colors.background }),
    textBox(
      ctx,
      "quote",
      slide.quote,
      { x: CONTENT.x, y, w: CONTENT.w, h: main.h },
      {
        size: main.size,
        color: colors.primary,
        align: "center",
        valign: "middle",
      },
    ),
  ];
  if (source)
    commands.push(
      textBox(
        ctx,
        "source",
        slide.source,
        { x: CONTENT.x, y: sourceY, w: CONTENT.w, h: source.h },
        { size: source.size, color: colors.textMuted, align: "center" },
      ),
    );
  return commands;
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
// to a row; more items split into two rows, the first row the longer.
const GRID_HEADING_PT = 16;
const GRID_BODY_PT = 14;
const GRID_ROW_MAX = 4;

export function renderGrid(ctx, slide) {
  const { items } = slide;
  const perRow =
    items.length <= GRID_ROW_MAX ? items.length : Math.ceil(items.length / 2);
  const slots = columns(perRow);
  const fitted = fitCards(items, slots[0].w, {
    headingSizes: [GRID_HEADING_PT],
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

// --- The deck ---------------------------------------------------------------

const notImplemented = () => {
  throw new Error("not implemented");
};

export const RENDERERS = {
  hero: renderHero,
  statement: renderStatement,
  pillars: renderPillars,
  evidence: notImplemented,
  timeline: notImplemented,
  comparison: notImplemented,
  grid: renderGrid,
  quote: renderQuote,
  cta: renderCta,
  showcase: notImplemented,
};

// Covers and breathing pages carry no page number (nor a title rule).
const UNNUMBERED = new Set(["hero", "statement", "quote", "cta"]);

// The batch for a deck parseDeck has read, in `theme`: every page in order,
// each followed by its speaker notes and, on content pages, its number.
export function renderDeck(deck, theme) {
  const fonts = fontsFor(theme, deck.language);
  const total = deck.slides.length;
  const commands = deck.slides.flatMap((slide, index) => {
    const ctx = { n: index + 1, total, theme, fonts, language: deck.language };
    const page = RENDERERS[slide.type](ctx, slide);
    if (slide.notes !== undefined) page.push(notesCommand(ctx.n, slide.notes));
    if (theme.decor.pageNumber && !UNNUMBERED.has(slide.type))
      page.push(pageNumberCommand(ctx.n, total, theme, fonts));
    return page;
  });
  if (commands.length > MAX_COMMANDS)
    throw new Error(`deck too large: ${commands.length} commands`);
  return { commands, slides: total };
}
