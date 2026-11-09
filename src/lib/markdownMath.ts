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

/** The end of the formula that opens at `from` (after `\(` or `\[`), or -1. */
function closing(text: string, from: number, closer: string): number {
  for (let index = from; index < text.length; ) {
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
const CJK =
  /[\u3000-\u303f\u3040-\u30ff\u3400-\u9fff\uac00-\ud7af\uff00-\uffef]/;
// The commands a formula writes words in.
const TEXT_COMMANDS =
  /\\(?:text|textrm|textbf|textit|mbox|mathrm|operatorname)\s*\{[^{}]*\}/g;

/**
 * Words outside `\text{…}` and its kin: a formula holds none, so what lies
 * between two dollars that has some, as between two prices in a Chinese
 * sentence ("$0.412/M。…补偿$待核"), is text.
 */
const proseInFormula = (formula: string) =>
  CJK.test(formula.replace(TEXT_COMMANDS, ""));

/**
 * The `$` that closes the formula a lone `$` at `from` opens, or -1, as Pandoc
 * reads one: a non-space right after the opener, a non-space right before the
 * closer and no digit right after it; the first `$` that follows decides. So
 * `$5 and $10` and `$0.014–$0.028` stay money, and so does a span with Chinese
 * words outside `\text`. It never crosses a paragraph break, code or `$$`.
 */
function dollarClose(text: string, from: number): number {
  const first = text[from + 1];
  if (first === undefined || first === "$" || /\s/.test(first)) return -1;
  for (let index = from + 1; index < text.length; ) {
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
        !proseInFormula(text.slice(from + 1, index))
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

/** The start and the end (its line break, or the text's end) of the line that holds `at`. */
function lineAround(text: string, at: number): { start: number; end: number } {
  const end = text.indexOf("\n", at);
  return {
    start: text.lastIndexOf("\n", at - 1) + 1,
    end: end < 0 ? text.length : end,
  };
}

/**
 * The `$$` that closes the one before `from`, or -1. An escaped dollar is not
 * one. It never crosses a paragraph break, unless the opening `$$` stood on a
 * line of its own (`block`), as a fence may.
 */
function doubleDollarClose(text: string, from: number, block: boolean): number {
  for (let index = from; index < text.length; ) {
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
 * the message. `prefix` is what stands before the opening fence on its line
 * (a quote's `>`, a list item's indent); each line made here starts with it.
 */
function displayBlock(formula: string, prefix: string): string {
  const onNextLine = /^[ \t]*\n/.test(formula);
  const lines = formula.replace(/^[ \t]*\n/, "").replace(/\n[ \t>]*$/, "");
  return `$$\n${onNextLine ? "" : prefix}${lines}\n${prefix}$$`;
}

// A table's delimiter row, such as `| --- | :-: |`.
const DELIMITER_ROW =
  /^[ \t>]*\|?(?:[ \t]*:?-+:?[ \t]*\|)+[ \t]*(?::?-+:?[ \t]*)?$/;

/**
 * The [start, end) offsets of the lines of `text` that are a table's: a line
 * with a `|` whose next line is a delimiter row, and the lines after it up to
 * a blank one.
 */
function tableLines(text: string): [number, number][] {
  const lines = text.split("\n");
  const starts: number[] = [];
  let offset = 0;
  for (const line of lines) {
    starts.push(offset);
    offset += line.length + 1;
  }
  const ranges: [number, number][] = [];
  for (let row = 0; row < lines.length; row += 1) {
    if (!lines[row].includes("|") || !DELIMITER_ROW.test(lines[row + 1] ?? ""))
      continue;
    for (; row < lines.length && lines[row].trim() !== ""; row += 1)
      ranges.push([starts[row], starts[row] + lines[row].length]);
  }
  return ranges;
}

/**
 * A formula's bars as TeX commands. A table row is split into cells at every
 * `|` before its formulas are read, so `$|x|$` in a cell would cut it apart.
 */
const withoutBars = (formula: string) =>
  formula.replace(/\\\|/g, "\\Vert ").replace(/\|/g, "\\vert ");

function rewriteMath(text: string): string {
  const tables = tableLines(text);
  const formulaAt = (formula: string, at: number) =>
    tables.some(([start, end]) => at >= start && at < end)
      ? withoutBars(formula)
      : formula;
  let out = "";
  let index = 0;
  while (index < text.length) {
    const char = text[index];
    if (char === "`") {
      const end = codeSpanEnd(text, index);
      out += text.slice(index, end);
      index = end;
      continue;
    }
    const next = text[index + 1];
    if (char === "$" && next === "$") {
      const line = lineAround(text, index);
      const prefix = text.slice(line.start, index);
      const atLineStart = /^[ \t>]*$/.test(prefix);
      const alone = /^[ \t\r]*$/.test(text.slice(index + 2, line.end));
      const end = doubleDollarClose(text, index + 2, atLineStart && alone);
      if (end < 0) {
        // Never closed: as text, rather than a fence that takes the rest.
        out += "\\$\\$";
        index += 2;
        continue;
      }
      const formula = text.slice(index + 2, end);
      const rest = text.slice(end + 2, lineAround(text, end + 2).end);
      const restBlank = /^[ \t\r]*$/.test(rest);
      index = end + 2;
      if (atLineStart && (formula.includes("\n") || restBlank)) {
        out += displayBlock(formula, prefix);
        if (!restBlank) {
          // The words after the closing fence go on a line of their own.
          out += `\n${prefix}`;
          while (text[index] === " " || text[index] === "\t") index += 1;
        }
      } else {
        out += `$$${formulaAt(formula, end)}$$`;
      }
      continue;
    }
    if (char === "$") {
      // A lone `$` may open a formula.
      const end = dollarClose(text, index);
      if (end < 0) {
        out += char;
        index += 1;
        continue;
      }
      out += textMath(
        formulaAt(text.slice(index + 1, end), index),
        out.slice(-1),
        text[end + 1] ?? "",
      );
      index = end + 1;
      continue;
    }
    if (char !== "\\" || (next !== "(" && next !== "[")) {
      // A backslash and what it escapes go together, so `\\(` is not an opener.
      const length = char === "\\" ? 2 : 1;
      out += text.slice(index, index + length);
      index += length;
      continue;
    }
    const closer = next === "(" ? ")" : "]";
    const end = closing(text, index + 2, closer);
    const formula = end < 0 ? "" : text.slice(index + 2, end);
    if (end < 0 || formula.includes("$$") || formula.trim() === "") {
      const stop = end < 0 ? index + 2 : end + 2;
      out += text.slice(index, stop);
      index = stop;
      continue;
    }
    const prefix = text.slice(lineAround(text, index).start, index);
    const rest = text.slice(end + 2, lineAround(text, end).end);
    const block =
      next === "[" && /^[ \t>]*$/.test(prefix) && /^[ \t\r]*$/.test(rest);
    out += block
      ? displayBlock(formula, prefix)
      : textMath(formulaAt(formula, index), out.slice(-1), text[end + 2] ?? "");
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
 * formula in a table row has its bars written as `\vert`. A message above
 * the size cap is left as written.
 */
export function rewriteMathDelimiters(content: string): string {
  return rewriteProse(content, rewriteMath);
}
