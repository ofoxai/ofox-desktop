// Slide geometry and the officecli commands that put things on a slide. The
// canvas is officecli's empty 16:9 deck; the content box is the one the
// ofox-office-pptx skill teaches (x 2.33 cm, 29.21 cm wide, content from
// y 5 cm). Builders write what they are given: choosing a size or a box that
// fits is the renderer's job (see text.mjs), except for the title's default.
import { lineCount } from "./text.mjs";

export const CANVAS = { wCm: 33.867, hCm: 19.05 };
export const MARGIN_CM = 2.33;
export const CONTENT = { x: 2.33, w: 29.21, top: 5.0, bottom: 17.78 };
export const TITLE_BOX = { x: 2.33, y: 1.0, w: 29.21, h: 3.67 };
export const GAP_CM = 0.76;

// roundRect's own default corner, one sixth of the short side.
const ROUND_CORNER = "adj:val 16667";

// The page number sits just under the content area, flush with its right
// edge: one line of 13pt needs 0.86 cm.
const PAGE_NUMBER = { w: 3, h: 0.9, y: 17.9, size: 13 };

// n equal columns side by side, `gap` apart, filling the box [x, x + w].
export function columns(
  n,
  { x = CONTENT.x, w = CONTENT.w, gap = GAP_CM } = {},
) {
  if (!Number.isInteger(n) || n < 1)
    throw new RangeError(`columns needs a whole number of 1 or more, got ${n}`);
  const width = (w - (n - 1) * gap) / n;
  return Array.from({ length: n }, (_, index) => ({
    x: x + index * (width + gap),
    w: width,
  }));
}

// officecli numbers slides in the order they are added; a slide is addressed
// as /slide[N].
export function slidePath(n) {
  if (!Number.isInteger(n) || n < 1)
    throw new RangeError(
      `a slide number is a whole number of 1 or more, got ${n}`,
    );
  return `/slide[${n}]`;
}

// A length as officecli reads it: cm, at most three decimals, never in
// scientific notation.
export function cm(value) {
  if (!Number.isFinite(value))
    throw new RangeError(`a length is a finite number of cm, got ${value}`);
  return `${Number(value.toFixed(3))}cm`;
}

const length = (value) => (value === undefined ? undefined : cm(value));
const asString = (value) => (value === undefined ? undefined : String(value));

// Space after a paragraph: 6 means 6 pt; a string such as "6pt" is kept.
const spacing = (value) => (typeof value === "number" ? `${value}pt` : value);

// Props that are not set stay out of the command.
const compact = (props) =>
  Object.fromEntries(
    Object.entries(props).filter(
      ([, value]) => value !== undefined && value !== null,
    ),
  );

export function slideCommand(n, { background, title }) {
  slidePath(n);
  return {
    command: "add",
    parent: "/",
    type: "slide",
    props: compact({ background, title }),
  };
}

// The largest title size that keeps `title` on one line, else the smallest.
function titleSize(title, width, sizes) {
  return (
    sizes.find((size) => lineCount(title, width, size) === 1) ?? sizes.at(-1)
  );
}

// Styles the slide's title placeholder (font and size are not inherited, so
// every title needs this). The text itself is the slide's `title` prop. A
// `size` wins; without one the title takes the largest theme title size that
// fits one line. `align` and `box` ({ x, y, w, h }) move a title that is not
// a plain left-aligned content title, such as a cover's.
export function titleCommands(
  n,
  title,
  { theme, fonts, dark = false, size, align = "left", box },
) {
  const { colors, sizes } = theme;
  return [
    {
      command: "set",
      path: `${slidePath(n)}/placeholder[title]`,
      props: compact({
        x: length(box?.x),
        y: length(box?.y),
        width: length(box?.w),
        height: length(box?.h),
        font: fonts.title,
        "font.latin": fonts.latinTitle,
        size: asString(
          size ?? titleSize(title, box?.w ?? TITLE_BOX.w, sizes.title),
        ),
        bold: "true",
        color: dark ? colors.onDark : colors.primary,
        align,
      }),
    },
  ];
}

// A shape with text on it, or without: a card, a bar, a circle, a text box.
// x, y, w and h are cm; size is pt; `latin` is the Latin-script font for CJK
// text. `radius` rounds the corners (roundRect at its default corner, not by
// the number given); `preset` picks another shape, such as "rect" or
// "ellipse". Names should be unique on the slide, as paths address by name.
export function textboxCommand(
  n,
  name,
  body,
  {
    x,
    y,
    w,
    h,
    size,
    color,
    font,
    latin,
    bold,
    align,
    valign,
    list,
    spaceAfter,
    fill,
    radius,
    preset,
  } = {},
) {
  const shape = preset ?? (radius ? "roundRect" : undefined);
  return {
    command: "add",
    parent: slidePath(n),
    type: "shape",
    props: compact({
      name,
      text: body,
      x: length(x),
      y: length(y),
      width: length(w),
      height: length(h),
      font,
      "font.latin": latin,
      size: asString(size),
      bold: asString(bold),
      color,
      align,
      valign,
      list,
      spaceAfter: spacing(spaceAfter),
      fill,
      preset: shape,
      adj: radius && shape === "roundRect" ? ROUND_CORNER : undefined,
    }),
  };
}

export function notesCommand(n, notes) {
  return {
    command: "add",
    parent: slidePath(n),
    type: "notes",
    props: { text: notes },
  };
}

// "n / total", small and muted, bottom right. Whether a deck shows page
// numbers is theme.decor.pageNumber, which the renderer reads.
export function pageNumberCommand(n, total, theme, fonts) {
  return textboxCommand(n, `s${n}-page`, `${n} / ${total}`, {
    x: CONTENT.x + CONTENT.w - PAGE_NUMBER.w,
    y: PAGE_NUMBER.y,
    w: PAGE_NUMBER.w,
    h: PAGE_NUMBER.h,
    size: PAGE_NUMBER.size,
    color: theme.colors.textMuted,
    font: fonts.body,
    latin: fonts.latinBody,
    align: "right",
    valign: "middle",
  });
}
