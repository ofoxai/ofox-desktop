import type { Options } from "react-markdown";
import { codeSpanEnd, endsParagraph, rewriteProse } from "@/lib/markdownSource";

type Plugins = NonNullable<Options["remarkPlugins"]>;

let plugins: { remark: Plugins; rehype: Plugins } | null = null;
let loading: Promise<void> | null = null;

/**
 * Loads KaTeX, its plugins and its fonts once, in their own chunk, the first
 * time a message that holds math is shown, so none of it weighs on the app's
 * first paint. A failed load leaves the formulas as text and is retried on the
 * next message.
 */
export function loadMarkdownMath(): Promise<void> {
  loading ??= Promise.all([
    import("remark-math"),
    import("rehype-katex"),
    // Bundled with its fonts, so every font loads from the app itself.
    import("katex/dist/katex.min.css"),
  ])
    .then(([{ default: remarkMath }, { default: rehypeKatex }]) => {
      plugins = {
        // remark-math reads `$$ … $$` only: a `$ … $` formula is rewritten
        // to it first, by Pandoc's rule, so that money stays text.
        remark: [[remarkMath, { singleDollarTextMath: false }]],
        // `trust: false` leaves out \href, \url, \includegraphics and \htmlData;
        // a formula that will not parse is shown as its text, never thrown,
        // in the muted ink rather than KaTeX's red.
        rehype: [
          [
            rehypeKatex,
            {
              trust: false,
              maxExpand: 1000,
              maxSize: 20,
              strict: "ignore",
              errorColor: "rgb(var(--ink-muted))",
            },
          ],
        ],
      };
    })
    .catch((error) => {
      loading = null;
      console.warn("[markdownMath] KaTeX failed to load", error);
    });
  return loading;
}

export const markdownMathReady = () => plugins !== null;

/** The remark and rehype plugins that draw math, once they have loaded. */
export const markdownMathPlugins = () => plugins;

/** Only text that can hold a formula is worth loading KaTeX for. */
export function hasMathMarkers(content: string): boolean {
  return (
    content.includes("$$") ||
    content.includes("\\(") ||
    content.includes("\\[") ||
    hasDollarMath(content)
  );
}

/** A `$ … $` formula somewhere in `content`. */
function hasDollarMath(content: string): boolean {
  for (let index = content.indexOf("$"); index >= 0; ) {
    const lone =
      content[index - 1] !== "$" &&
      content[index - 1] !== "\\" &&
      content[index + 1] !== "$";
    if (lone && dollarClose(content, index) > 0) return true;
    index = content.indexOf("$", index + 1);
  }
  return false;
}

/**
 * The end of the formula that opens at `from` (after `\(` or `\[`), or -1. It
 * never reaches `limit` (the end of a table row).
 */
function closing(
  text: string,
  from: number,
  closer: string,
  limit = text.length,
): number {
  for (let index = from; index < limit; ) {
    if (text[index] === "\\") {
      if (text[index + 1] === closer) return index;
      index += 2;
    } else if (text[index] === "\n" && endsParagraph(text, index)) {
      return -1;
    } else {
      index += 1;
    }
  }
  return -1;
}

// Chinese, Japanese or Korean text, and full-width punctuation.
const CJK = /[　-〿぀-ヿ㐀-鿿가-힯＀-￯]/;
// The commands a formula writes words in.
const TEXT_COMMANDS =
  /\\(?:text[a-z]*|mbox|math(?:rm|bf|sf|tt|it)|boldsymbol|operatorname)\s*\{[^{}]*\}/g;

/**
 * A price, not a formula: what lies between two dollars starts with a digit
 * and holds Chinese words outside `\text{…}`, as between the two prices in
 * "$0.412/M。…补偿$待核". A formula may hold Chinese (`$F_{合}=ma$`), but it
 * does not start with an amount and run on into the sentence.
 */
const priceSpan = (span: string) =>
  /^\d/.test(span) && CJK.test(span.replace(TEXT_COMMANDS, ""));

/**
 * The `$` that closes the formula a lone `$` at `from` opens, or -1, as Pandoc
 * reads one: a non-space right after the opener, a non-space right before the
 * closer and no digit right after it; the first `$` that follows decides. So
 * `$5 and $10` and `$0.014–$0.028` stay money, and so does a price that runs
 * on into a Chinese sentence (see `priceSpan`). It never crosses a paragraph
 * break, code, `$$` or `limit` (the end of a table row).
 */
function dollarClose(text: string, from: number, limit = text.length): number {
  const first = text[from + 1];
  if (first === undefined || first === "$" || /\s/.test(first)) return -1;
  for (let index = from + 1; index < limit; ) {
    const char = text[index];
    if (char === "\\") {
      index += 2;
    } else if (char === "`") {
      return -1;
    } else if (char === "\n" && endsParagraph(text, index)) {
      return -1;
    } else if (char === "$") {
      const after = text[index + 1] ?? "";
      return after !== "$" &&
        !/\s/.test(text[index - 1]) &&
        !/\d/.test(after) &&
        !priceSpan(text.slice(from + 1, index))
        ? index
        : -1;
    } else {
      index += 1;
    }
  }
  return -1;
}

/**
 * `$$ … $$`, padded where a dollar at the edge of the formula, or right before
 * or after it (`before`, `after`: the characters around), would join its fence.
 */
function textMath(formula: string, before: string, after: string): string {
  const padded = /^\$|\$$/.test(formula) ? ` ${formula} ` : formula;
  return `${before === "$" ? " " : ""}$$${padded}$$${after === "$" ? " " : ""}`;
}

// What may stand before a fence on its line: a quote's `>`, an indent, and a
// list item's marker.
const CONTAINER = /^[ \t>]*(?:(?:[-*+]|\d{1,9}[.)])[ \t]+)?$/;
const LEADING_CONTAINER = /^[ \t>]*(?:(?:[-*+]|\d{1,9}[.)])[ \t]+)?/;
const LIST_MARKER = /(?:[-*+]|\d{1,9}[.)])[ \t]+$/;

/**
 * The text before `at` on its line when it is nothing but containers (see
 * CONTAINER), or null. It looks back a short way only, so a long line of
 * formulas is read in one pass.
 */
function containersBefore(text: string, at: number): string | null {
  let start = at;
  while (start > 0 && text[start - 1] !== "\n") {
    start -= 1;
    if (at - start > 32) return null;
  }
  const prefix = text.slice(start, at);
  return CONTAINER.test(prefix) ? prefix : null;
}

/** Containers as the indent of the lines under them: a list marker turns to spaces. */
const indentOf = (containers: string) =>
  containers.replace(LIST_MARKER, (marker) => " ".repeat(marker.length));

/** The indent of the lines under the line that holds `at`. */
function lineIndent(text: string, at: number): string {
  const start = text.lastIndexOf("\n", at - 1) + 1;
  return indentOf(LEADING_CONTAINER.exec(text.slice(start, at))?.[0] ?? "");
}

/** True when only spaces lie between `from` and the end of its line. */
function blankToLineEnd(text: string, from: number): boolean {
  let index = from;
  while (text[index] === " " || text[index] === "\t" || text[index] === "\r")
    index += 1;
  return index >= text.length || text[index] === "\n";
}

/**
 * The `$$` that closes the one before `from`, or -1. An escaped dollar is not
 * one. It never reaches `limit` (the end of a table row) and never crosses a
 * paragraph break, unless the opening `$$` stood on a line of its own
 * (`block`), as a fence may.
 */
function doubleDollarClose(
  text: string,
  from: number,
  block: boolean,
  limit: number,
): number {
  for (let index = from; index < limit; ) {
    const char = text[index];
    if (char === "\\") index += 2;
    else if (char === "$" && text[index + 1] === "$") return index;
    else if (char === "\n" && !block && endsParagraph(text, index)) return -1;
    else index += 1;
  }
  return -1;
}

/**
 * A display formula as a `$$` block whose fences stand on lines of their own,
 * the only shape remark-math reads as one: a fence with the formula on its
 * line is read as an opening fence that never closes, and takes the rest of
 * the message. Each line made here starts with `indent` (a quote's `>`, the
 * indent of a list item's lines).
 */
function displayBlock(formula: string, indent: string): string {
  const onNextLine = /^[ \t]*\n/.test(formula);
  const lines = formula
    .replace(/^[ \t]*\n/, "")
    .replace(/\n[ \t>]*$/, "")
    .split("\n")
    .map((line, at) =>
      at === 0 && !onNextLine ? indent + line : underIndent(line, indent),
    );
  return `$$\n${lines.join("\n")}\n${indent}$$`;
}

// A line's quote markers, each `>` with the space after it.
const QUOTE_MARKERS = /^(?:[ \t]*>[ \t]?)*/;

/**
 * A formula line under `indent`. Markdown lets a line leave out a quote's
 * `>` or a list item's indent (a lazy line), but in a fence such a line ends
 * the quote or the list item, and the fence with it: it gets them back.
 */
function underIndent(line: string, indent: string): string {
  if (!indent || line.trim() === "") return line;
  const quote = QUOTE_MARKERS.exec(indent)?.[0] ?? "";
  const list = indent.slice(quote.length);
  const depth = quote.split(">").length - 1;
  // Only as many `>` as the quote holds are markers; one more is the formula's.
  let rest = line;
  let markers = 0;
  for (; markers < depth; markers += 1) {
    const marker = /^[ \t]*>[ \t]?/.exec(rest);
    if (!marker) break;
    rest = rest.slice(marker[0].length);
  }
  return markers === depth && rest.startsWith(list)
    ? line
    : quote + list + rest.trimStart();
}

// The start of words that would open a list, a quote, a heading or a rule.
const BLOCK_START =
  /(?:[-+*](?=[ \t\r\n]|$)|#{1,6}(?=[ \t\r\n]|$)|>|(?:[-*_][ \t]*){3,}(?=\r?\n|$))/y;
const ORDERED_START = /\d{1,9}(?=[.)](?:[ \t\r\n]|$))/y;

// A table's delimiter row, such as `| --- | :-: |` or a one-column `|---`.
const DELIMITER_ROW =
  /^[ \t>]*(?:\|?(?:[ \t]*:?-+:?[ \t]*\|)+[ \t]*(?::?-+:?[ \t]*)?|\|[ \t]*:?-+:?[ \t]*)$/;
// A line that ends a table: a `$$` fence (whose rest holds no `$`), a
// heading, a quote, a list item or a rule starts a block of its own.
const ENDS_TABLE =
  /^[ \t]{0,3}(?:\$\$[^$]*$|#{1,6}(?:[ \t]|$)|>|[-*+][ \t]|\d{1,9}[.)][ \t]|(?:[-*_][ \t]*){3,}$)/;

/**
 * The [start, end) offsets of the lines of `text` that are a table's: a line
 * with a `|` whose next line is a delimiter row, and the lines after it up to
 * a blank one or one that starts another block.
 */
function tableRows(text: string): [number, number][] {
  const lines = text.split("\n");
  const starts: number[] = [];
  let offset = 0;
  for (const line of lines) {
    starts.push(offset);
    offset += line.length + 1;
  }
  const rows: [number, number][] = [];
  for (let row = 0; row < lines.length; row += 1) {
    if (!lines[row].includes("|") || !DELIMITER_ROW.test(lines[row + 1] ?? ""))
      continue;
    rows.push([starts[row], starts[row] + lines[row].length]);
    for (
      row += 1;
      row < lines.length &&
      lines[row].trim() !== "" &&
      !ENDS_TABLE.test(lines[row]);
      row += 1
    )
      rows.push([starts[row], starts[row] + lines[row].length]);
  }
  return rows;
}

/**
 * For offsets asked in increasing order, the end of the table row that holds
 * each, or -1 outside a table.
 */
function rowEnds(text: string): (at: number) => number {
  const rows = tableRows(text);
  let next = 0;
  return (at) => {
    while (next < rows.length && rows[next][1] <= at) next += 1;
    const row = rows[next];
    return row && at >= row[0] ? row[1] : -1;
  };
}

/**
 * A formula's bars as TeX commands. A table row is split into cells at every
 * `|` before its formulas are read, so `$|x|$` in a cell would cut it apart.
 */
const withoutBars = (formula: string) =>
  formula.replace(/\\[\s\S]|\|/g, (token) =>
    token === "\\|" ? "\\Vert " : token === "|" ? "\\vert " : token,
  );

function rewriteMath(text: string): string {
  const rowEnd = rowEnds(text);
  let out = "";
  // The last character written: reading it back from `out` would flatten the
  // whole string each time, once per formula.
  let last = "";
  const emit = (part: string) => {
    if (!part) return;
    out += part;
    last = part[part.length - 1];
  };
  let index = 0;
  while (index < text.length) {
    const char = text[index];
    if (char === "`") {
      const end = codeSpanEnd(text, index);
      emit(text.slice(index, end));
      index = end;
      continue;
    }
    const next = text[index + 1];
    if (char === "$" && next === "$") {
      const row = rowEnd(index);
      const containers = row < 0 ? containersBefore(text, index) : null;
      const alone = containers !== null && blankToLineEnd(text, index + 2);
      const end = doubleDollarClose(
        text,
        index + 2,
        alone,
        row < 0 ? text.length : row,
      );
      if (end < 0) {
        // Never closed: as text, rather than a fence that takes the rest.
        emit("\\$\\$");
        index += 2;
        continue;
      }
      const formula = text.slice(index + 2, end);
      const multiline = formula.includes("\n");
      const restBlank = blankToLineEnd(text, end + 2);
      let indent: string | null = null;
      if (containers !== null && (multiline || restBlank)) {
        indent = indentOf(containers);
      } else if (multiline && containersBefore(text, end) !== null) {
        // Opened in a sentence, closed at the start of a line: that line
        // would open a fence of its own, so the formula starts a new line.
        indent = lineIndent(text, index);
        out = out.replace(/[ \t]+$/, "");
        emit(`\n${indent}`);
      }
      index = end + 2;
      if (indent === null) {
        emit(`$$${row < 0 ? formula : withoutBars(formula)}$$`);
        continue;
      }
      emit(displayBlock(formula, indent));
      if (!restBlank) {
        // The words after the closing fence go on a line of their own, as
        // words: what would open a list, quote or heading there is escaped.
        emit(`\n${indent}`);
        while (text[index] === " " || text[index] === "\t") index += 1;
        BLOCK_START.lastIndex = index;
        ORDERED_START.lastIndex = index;
        const number = ORDERED_START.exec(text)?.[0];
        if (BLOCK_START.test(text)) emit("\\");
        else if (number) {
          emit(`${number}\\`);
          index += number.length;
        }
      }
      continue;
    }
    if (char === "$") {
      // A lone `$` may open a formula.
      const row = rowEnd(index);
      const end = dollarClose(text, index, row < 0 ? text.length : row);
      if (end < 0) {
        emit(char);
        index += 1;
        continue;
      }
      const formula = text.slice(index + 1, end);
      emit(
        textMath(
          row < 0 ? formula : withoutBars(formula),
          last,
          text[end + 1] ?? "",
        ),
      );
      index = end + 1;
      continue;
    }
    if (char !== "\\" || (next !== "(" && next !== "[")) {
      // A backslash and what it escapes go together, so `\\(` is not an opener.
      const length = char === "\\" ? 2 : 1;
      emit(text.slice(index, index + length));
      index += length;
      continue;
    }
    const row = rowEnd(index);
    const closer = next === "(" ? ")" : "]";
    const end = closing(text, index + 2, closer, row < 0 ? text.length : row);
    const formula = end < 0 ? "" : text.slice(index + 2, end);
    if (end < 0 || formula.includes("$$") || formula.trim() === "") {
      const stop = end < 0 ? index + 2 : end + 2;
      emit(text.slice(index, stop));
      index = stop;
      continue;
    }
    const containers = row < 0 ? containersBefore(text, index) : null;
    emit(
      next === "[" && containers !== null && blankToLineEnd(text, end + 2)
        ? displayBlock(formula, indentOf(containers))
        : textMath(
            row < 0 ? formula : withoutBars(formula),
            last,
            text[end + 2] ?? "",
          ),
    );
    index = end + 2;
  }
  return out;
}

/**
 * Writes math as remark-math reads it, outside code: `$…$`, `\(…\)` and
 * `\[…\]` as `$$` math (markdown reads `\(` as an escaped bracket and drops
 * the backslash, so this runs on the text before it is parsed), and a `$$`
 * formula that stands on its own lines as a block whose fences have lines of
 * their own. A `\[` on its own lines becomes a block too; every other formula
 * becomes text math. A `$$` that never closes is shown as text, and a
 * formula in a table row stays in its row and has its bars written as
 * `\vert`. A message above the size cap is left as written.
 */
export function rewriteMathDelimiters(content: string): string {
  return rewriteProse(content, rewriteMath);
}
