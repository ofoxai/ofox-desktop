// Reading markdown source before it is parsed: which lines are code, where
// a paragraph ends and where an inline code span closes. The rewrites that
// run on a message's text (formulas, bare URLs) leave code as it was.

// A backtick fence's info string holds no backtick, or it is inline code.
const FENCE_OPEN =
  /^[\s>]*(?:(?:[-*+]|\d{1,9}[.)])\s+)?(?:(`{3,})[^`]*|(~{3,})[\s\S]*)$/;
const FENCE_CLOSE = /^[\s>]*(`{3,}|~{3,})\s*$/;
const LIST_ITEM = /^ {0,3}(?:[-*+]|\d{1,9}[.)])(?:\s|$)/;
const INDENTED = /^(?: {4}|\t)/;
const MATH_FENCE = /^[\s>]*\$\$\s*$/;

/** The most text a rewrite works through; a longer message is left as written. */
export const MAX_REWRITE = 64 * 1024;

/**
 * Splits text into runs of prose and runs of code (fenced or indented), by
 * line. A `run` is its lines joined, so a run of prose can be scanned whole.
 * An indented line inside a `$$` block is the formula's, not code.
 */
export function splitCode(content: string): { code: boolean; text: string }[] {
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
  let inMath = false;
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
        code = (afterBlank || afterCode) && !inList && !inMath;
        if (!code && MATH_FENCE.test(line)) inMath = !inMath;
      } else if (!blank) {
        if (MATH_FENCE.test(line)) inMath = !inMath;
        inList = LIST_ITEM.test(line) || (inList && !afterBlank);
      }
    }
    add(code, line);
    afterCode = code && !blank;
    afterBlank = blank;
  }
  return runs.map(({ code, lines }) => ({ code, text: lines.join("\n") }));
}

/** Rewrites the prose of `content` with `rewrite`, leaving its code as it was. */
export function rewriteProse(
  content: string,
  rewrite: (prose: string) => string,
): string {
  if (content.length > MAX_REWRITE) return content;
  return splitCode(content)
    .map(({ code, text }) => (code ? text : rewrite(text)))
    .join("\n");
}

/** True when the line break at `newline` is followed by a blank line or the end. */
export function endsParagraph(text: string, newline: number): boolean {
  let index = newline + 1;
  while (text[index] === " " || text[index] === "\t" || text[index] === "\r")
    index += 1;
  return index >= text.length || text[index] === "\n";
}

/** The end of the code span whose backtick run starts at `from`, or the run's end. */
export function codeSpanEnd(text: string, from: number): number {
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
