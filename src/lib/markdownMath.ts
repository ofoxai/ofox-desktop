import type { Options } from "react-markdown";

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
        // a formula that will not parse is shown as its text, never thrown.
        rehype: [
          [
            rehypeKatex,
            { trust: false, maxExpand: 1000, maxSize: 20, strict: "ignore" },
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

// A backtick fence's info string holds no backtick, or it is inline code.
const FENCE_OPEN =
  /^[\s>]*(?:(?:[-*+]|\d{1,9}[.)])\s+)?(?:(`{3,})[^`]*|(~{3,})[\s\S]*)$/;
const FENCE_CLOSE = /^[\s>]*(`{3,}|~{3,})\s*$/;
const LIST_ITEM = /^ {0,3}(?:[-*+]|\d{1,9}[.)])(?:\s|$)/;
const INDENTED = /^(?: {4}|\t)/;

/**
 * Splits text into runs of prose and runs of code (fenced or indented), by
 * line. A `run` is its lines joined, so a run of prose can be scanned whole.
 */
function splitCode(content: string): { code: boolean; text: string }[] {
  const runs: { code: boolean; lines: string[] }[] = [];
  const add = (code: boolean, line: string) => {
    const last = runs[runs.length - 1];
    if (last?.code === code) last.lines.push(line);
    else runs.push({ code, lines: [line] });
  };
  let fence: { char: string; length: number } | null = null;
  let afterBlank = true;
  let afterCode = false;
  let inList = false;
  for (const line of content.split("\n")) {
    const blank = line.trim() === "";
    let code = false;
    if (fence) {
      code = true;
      const close = FENCE_CLOSE.exec(line)?.[1];
      if (close?.[0] === fence.char && close.length >= fence.length)
        fence = null;
    } else {
      const match = FENCE_OPEN.exec(line);
      const open = match?.[1] ?? match?.[2];
      if (open) {
        code = true;
        fence = { char: open[0], length: open.length };
      } else if (!blank && INDENTED.test(line)) {
        code = (afterBlank || afterCode) && !inList;
      } else if (!blank) {
        inList = LIST_ITEM.test(line) || (inList && !afterBlank);
      }
    }
    add(code, line);
    afterCode = code && !blank;
    afterBlank = blank;
  }
  return runs.map(({ code, lines }) => ({ code, text: lines.join("\n") }));
}

/** True when the line break at `newline` is followed by a blank line or the end. */
function endsParagraph(text: string, newline: number): boolean {
  let index = newline + 1;
  while (text[index] === " " || text[index] === "\t" || text[index] === "\r")
    index += 1;
  return index >= text.length || text[index] === "\n";
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

/**
 * The `$` that closes the formula a lone `$` at `from` opens, or -1, as Pandoc
 * reads one: a non-space right after the opener, a non-space right before the
 * closer and no digit right after it; the first `$` that follows decides. So
 * `$5 and $10` and `$0.014–$0.028` stay money. It never crosses a paragraph
 * break, code or `$$`.
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
      return after !== "$" && !/\s/.test(text[index - 1]) && !/\d/.test(after)
        ? index
        : -1;
    } else {
      index += 1;
    }
  }
  return -1;
}

/** The end of the code span whose backtick run starts at `from`, or the run's end. */
function codeSpanEnd(text: string, from: number): number {
  let run = from;
  while (text[run] === "`") run += 1;
  const ticks = text.slice(from, run);
  for (let index = run; index < text.length; ) {
    if (text[index] === "\n" && endsParagraph(text, index)) break;
    if (text[index] !== "`") {
      index += 1;
      continue;
    }
    let end = index;
    while (text[end] === "`") end += 1;
    if (end - index === ticks.length) return end;
    index = end;
  }
  return run;
}

/**
 * `$$ … $$`, padded where a dollar at the edge of the formula, or right before
 * or after it (`before`, `after`: the characters around), would join its fence.
 */
function textMath(formula: string, before: string, after: string): string {
  const padded = /^\$|\$$/.test(formula) ? ` ${formula} ` : formula;
  return `${before === "$" ? " " : ""}$$${padded}$$${after === "$" ? " " : ""}`;
}

function rewriteProse(text: string): string {
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
    if (char === "$") {
      // `$$ … $$` is remark-math's own; a lone `$` may open a formula.
      if (next === "$") {
        const end = text.indexOf("$$", index + 2);
        const stop = end < 0 ? index + 2 : end + 2;
        out += text.slice(index, stop);
        index = stop;
        continue;
      }
      const end = dollarClose(text, index);
      if (end < 0) {
        out += char;
        index += 1;
        continue;
      }
      out += textMath(
        text.slice(index + 1, end),
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
    const lineStart = text.lastIndexOf("\n", index - 1) + 1;
    const prefix = text.slice(lineStart, index);
    const lineEnd = text.indexOf("\n", end);
    const rest = text.slice(end + 2, lineEnd < 0 ? undefined : lineEnd);
    const block =
      next === "[" && /^[ \t>]*$/.test(prefix) && /^[ \t\r]*$/.test(rest);
    out += block
      ? `$$\n${formula.replace(/^[ \t]*\n/, "").replace(/\n[ \t>]*$/, "")}\n${prefix}$$`
      : textMath(formula, out.slice(-1), text[end + 2] ?? "");
    index = end + 2;
  }
  return out;
}

// The same bound markdownSyntax puts on the text it will work through.
const MAX_REWRITE = 64 * 1024;

/**
 * Writes `$…$`, `\(…\)` and `\[…\]` as the `$$` math remark-math reads, outside code.
 * Markdown reads `\(` as an escaped bracket and drops the backslash, so this
 * must run on the text before it is parsed. A `\[` that stands on its own
 * lines becomes a `$$` block; every other formula becomes text math. A message
 * above the size cap is left as written (its `$$` math still draws).
 */
export function rewriteMathDelimiters(content: string): string {
  if (content.length > MAX_REWRITE) return content;
  return splitCode(content)
    .map(({ code, text }) => (code ? text : rewriteProse(text)))
    .join("\n");
}
