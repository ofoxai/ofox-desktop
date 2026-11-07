import test from "node:test";
import assert from "node:assert/strict";
import { THEME_NAMES, THEMES, fontsFor, themeFor } from "../src/deck/theme.mjs";
import { heightCm, lineCount } from "../src/deck/text.mjs";
import {
  CANVAS,
  CONTENT,
  GAP_CM,
  MARGIN_CM,
  TITLE_BOX,
  columns,
  notesCommand,
  pageNumberCommand,
  slideCommand,
  textboxCommand,
  titleCommands,
} from "../src/deck/layout.mjs";

const HEX = /^[0-9A-F]{6}$/;
const navy = themeFor("navy");

// The brightness officecli's contrast check uses.
const brightness = (hex) => {
  const [r, g, b] = [0, 2, 4].map((at) => parseInt(hex.slice(at, at + 2), 16));
  return 0.299 * r + 0.587 * g + 0.114 * b;
};

test("navy has every color role and six chart colors", () => {
  const roles = [
    "background",
    "card",
    "primary",
    "secondary",
    "accent",
    "text",
    "textSecondary",
    "textMuted",
    "dark",
    "onDark",
    "onDarkMuted",
  ];
  for (const role of roles)
    assert.match(navy.colors[role], HEX, `colors.${role}`);
  assert.equal(navy.colors.chart.length, 6);
  for (const color of navy.colors.chart) assert.match(color, HEX);
});

test("the theme names are the keys of THEMES", () => {
  assert.deepEqual(THEME_NAMES, Object.keys(THEMES));
  assert.deepEqual(THEME_NAMES, ["navy"]);
});

test("themeFor falls back to navy for a name it does not know", () => {
  assert.equal(themeFor("neon"), navy);
  assert.equal(themeFor(undefined), navy);
  assert.equal(themeFor("toString"), navy);
});

test("every size list runs from large to small, as fitSize expects", () => {
  for (const [role, sizes] of Object.entries(navy.sizes))
    assert.deepEqual(
      sizes,
      [...sizes].sort((a, b) => b - a),
      `sizes.${role}`,
    );
});

test("fontsFor picks CJK fonts by language and keeps Latin in font.latin", () => {
  const zh = fontsFor(navy, "zh");
  assert.equal(zh.title, "Microsoft YaHei");
  assert.equal(zh.body, "Microsoft YaHei");
  assert.equal(zh.data, "Arial");
  assert.equal(zh.latinTitle, "Arial Black");
  assert.equal(zh.latinBody, "Arial");

  const ja = fontsFor(navy, "ja");
  assert.equal(ja.title, "Yu Gothic");
  assert.equal(ja.body, "Yu Gothic");
  assert.equal(ja.latinTitle, "Arial Black");

  const en = fontsFor(navy, "en");
  assert.equal(en.title, "Arial Black");
  assert.equal(en.body, "Arial");
  assert.equal(en.data, "Arial");
  assert.equal(en.latinTitle, undefined);
  assert.equal(en.latinBody, undefined);
});

test("dark fills get light text", () => {
  assert.equal(navy.colors.onDark, "FFFFFF");
  assert.ok(brightness(navy.colors.dark) < 128);
  assert.ok(brightness(navy.colors.onDark) >= 204);
  assert.ok(brightness(navy.colors.onDarkMuted) >= 204);
});

test("the content area sits inside the margins of the canvas", () => {
  assert.equal(CONTENT.x, MARGIN_CM);
  assert.ok(Math.abs(CONTENT.x + CONTENT.w - (CANVAS.wCm - MARGIN_CM)) < 0.01);
  assert.equal(TITLE_BOX.x, CONTENT.x);
  assert.equal(TITLE_BOX.w, CONTENT.w);
  assert.ok(TITLE_BOX.y + TITLE_BOX.h < CONTENT.top);
  assert.ok(CONTENT.bottom < CANVAS.hCm);
});

test("columns divides the content width with 0.76 cm gaps", () => {
  assert.equal(GAP_CM, 0.76);

  const three = columns(3);
  assert.equal(three.length, 3);
  assert.equal(three[0].x, 2.33);
  assert.ok(Math.abs(three[0].w - 9.23) < 0.005, `w was ${three[0].w}`);
  assert.ok(Math.abs(three[1].x - (2.33 + 9.23 + 0.76)) < 0.005);

  const four = columns(4);
  assert.ok(Math.abs(four[0].w - 6.73) < 0.01, `w was ${four[0].w}`);

  for (const count of [1, 2, 3, 4, 6]) {
    const last = columns(count).at(-1);
    assert.ok(last.x + last.w <= 31.54 + 1e-9, `${count} columns overrun`);
  }
});

test("columns follows the box it is given", () => {
  const [left, right] = columns(2, { x: 1, w: 20, gap: 2 });
  assert.deepEqual(left, { x: 1, w: 9 });
  assert.deepEqual(right, { x: 12, w: 9 });
});

test("columns refuses a count that is not a positive whole number", () => {
  for (const count of [0, -1, 1.5, NaN, "3"])
    assert.throws(() => columns(count), RangeError);
});

test("slideCommand adds a slide with its background and title", () => {
  assert.deepEqual(slideCommand(1, { background: "1E3A5F", title: "封面" }), {
    command: "add",
    parent: "/",
    type: "slide",
    props: { background: "1E3A5F", title: "封面" },
  });
});

test("slideCommand leaves an unset background out", () => {
  const { props } = slideCommand(2, { title: "Plan" });
  assert.deepEqual(props, { title: "Plan" });
});

test("titleCommands styles the title placeholder", () => {
  const zh = fontsFor(navy, "zh");
  assert.deepEqual(
    titleCommands(3, "标题", { theme: navy, fonts: zh, size: 40 }),
    [
      {
        command: "set",
        path: "/slide[3]/placeholder[title]",
        props: {
          font: "Microsoft YaHei",
          "font.latin": "Arial Black",
          size: "40",
          bold: "true",
          color: "1E3A5F",
          align: "left",
        },
      },
    ],
  );
});

test("titleCommands uses light text on a dark slide", () => {
  const [command] = titleCommands(1, "Hello", {
    theme: navy,
    fonts: fontsFor(navy, "en"),
    dark: true,
    size: 60,
  });
  assert.equal(command.props.color, "FFFFFF");
  assert.equal(command.props["font.latin"], undefined);
  assert.equal(command.props.font, "Arial Black");
});

test("titleCommands takes an alignment and a box, for the cover", () => {
  const [command] = titleCommands(1, "封面", {
    theme: navy,
    fonts: fontsFor(navy, "zh"),
    dark: true,
    size: 54,
    align: "center",
    box: { x: 2.33, y: 6, w: 29.21, h: 4.5 },
  });
  assert.equal(command.props.align, "center");
  assert.equal(command.props.x, "2.33cm");
  assert.equal(command.props.y, "6cm");
  assert.equal(command.props.width, "29.21cm");
  assert.equal(command.props.height, "4.5cm");
});

test("titleCommands picks the largest title size that fits one line", () => {
  const options = { theme: navy, fonts: fontsFor(navy, "zh") };
  const sizeOf = (text) => titleCommands(1, text, options)[0].props.size;
  // A full-width line holds 18.5 characters at 44pt, 20.3 at 40, 22.6 at 36.
  assert.equal(sizeOf("字".repeat(10)), "44");
  assert.equal(sizeOf("字".repeat(20)), "40");
  assert.equal(sizeOf("字".repeat(22)), "36");
  // Nothing fits: the smallest size, never a smaller one.
  assert.equal(sizeOf("字".repeat(30)), "36");
});

test("textboxCommand writes units and names", () => {
  const command = textboxCommand(2, "s2-body", "第一条\n第二条", {
    x: 2.33,
    y: 5,
    w: 29.21,
    h: 3,
    size: 20,
    color: "333333",
    font: "Microsoft YaHei",
    latin: "Arial",
    bold: true,
    align: "left",
    valign: "top",
    list: "bullet",
    spaceAfter: "6pt",
    fill: "E8EEF4",
    radius: 0.3,
  });
  assert.deepEqual(command, {
    command: "add",
    parent: "/slide[2]",
    type: "shape",
    props: {
      name: "s2-body",
      text: "第一条\n第二条",
      x: "2.33cm",
      y: "5cm",
      width: "29.21cm",
      height: "3cm",
      font: "Microsoft YaHei",
      "font.latin": "Arial",
      size: "20",
      bold: "true",
      color: "333333",
      align: "left",
      valign: "top",
      list: "bullet",
      spaceAfter: "6pt",
      fill: "E8EEF4",
      preset: "roundRect",
      adj: "adj:val 16667",
    },
  });
});

test("textboxCommand leaves out what is not set", () => {
  const { props } = textboxCommand(1, "s1-x", "Hi", { x: 1, y: 2, w: 3, h: 4 });
  assert.deepEqual(Object.keys(props).sort(), [
    "height",
    "name",
    "text",
    "width",
    "x",
    "y",
  ]);
});

test("textboxCommand writes lengths with at most three decimals", () => {
  const { props } = textboxCommand(1, "s1-x", "Hi", {
    x: 6.73333333,
    y: 0.0001,
    w: 9.2300000001,
    h: 1e-7,
  });
  assert.equal(props.x, "6.733cm");
  assert.equal(props.y, "0cm");
  assert.equal(props.width, "9.23cm");
  assert.equal(props.height, "0cm");
});

test("textboxCommand draws a bar or a circle from a preset, without text", () => {
  const bar = textboxCommand(4, "s4-rule", undefined, {
    x: 2.33,
    y: 4.4,
    w: 2,
    h: 0.12,
    fill: "1E3A5F",
    preset: "rect",
  });
  assert.equal(bar.props.preset, "rect");
  assert.equal(bar.props.adj, undefined);
  assert.equal("text" in bar.props, false);

  const dot = textboxCommand(4, "s4-dot", "1", {
    x: 1,
    y: 1,
    w: 1,
    h: 1,
    preset: "ellipse",
    radius: 0.3,
  });
  assert.equal(dot.props.preset, "ellipse");
  assert.equal(dot.props.adj, undefined);
});

test("textboxCommand turns a point spacing into a length with a unit", () => {
  const { props } = textboxCommand(1, "s1-x", "a\nb", {
    x: 1,
    y: 1,
    w: 1,
    h: 1,
    spaceAfter: 6,
  });
  assert.equal(props.spaceAfter, "6pt");
});

test("textboxCommand refuses a length that is not a finite number", () => {
  assert.throws(
    () => textboxCommand(1, "s1-x", "Hi", { x: NaN, y: 0, w: 1, h: 1 }),
    RangeError,
  );
});

test("notesCommand adds speaker notes to the slide", () => {
  assert.deepEqual(notesCommand(3, "先讲结论。"), {
    command: "add",
    parent: "/slide[3]",
    type: "notes",
    props: { text: "先讲结论。" },
  });
});

test("pageNumberCommand puts a muted 13pt number at the bottom right", () => {
  const command = pageNumberCommand(4, 10, navy, fontsFor(navy, "zh"));
  const { props } = command;
  assert.equal(command.parent, "/slide[4]");
  assert.equal(props.name, "s4-page");
  assert.equal(props.text, "4 / 10");
  assert.equal(props.size, "13");
  assert.equal(props.color, "999999");
  assert.equal(props.align, "right");
  assert.equal(props.font, "Microsoft YaHei");
  assert.equal(props["font.latin"], "Arial");

  const right = parseFloat(props.x) + parseFloat(props.width);
  const bottom = parseFloat(props.y) + parseFloat(props.height);
  assert.ok(Math.abs(right - (CONTENT.x + CONTENT.w)) < 0.001);
  assert.ok(parseFloat(props.y) >= CONTENT.bottom);
  assert.ok(bottom <= CANVAS.hCm);
  // The box holds the number on one line.
  assert.equal(lineCount(props.text, parseFloat(props.width), 13), 1);
  assert.ok(heightCm(1, 13) <= parseFloat(props.height));
});

test("commands refuse a slide number that is not a position", () => {
  for (const n of [0, -1, 1.5, "2", undefined]) {
    assert.throws(() => slideCommand(n, { title: "x" }), RangeError);
    assert.throws(() => notesCommand(n, "x"), RangeError);
    assert.throws(
      () => textboxCommand(n, "s-x", "x", { x: 0, y: 0, w: 1, h: 1 }),
      RangeError,
    );
  }
});
