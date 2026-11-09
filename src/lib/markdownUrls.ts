import { codeSpanEnd, rewriteProse } from "@/lib/markdownSource";
import { FULL_WIDTH_PUNCTUATION, splitUrlTail } from "@/lib/urlTail";

const SCHEME = /^https?:\/\//i;
const WWW = /^www\./i;
// A bare URL runs to a space or a `<`, as markdown reads one, or to the `|`
// that ends its table cell.
const URL_RUN = /^[^\s<|]+/;
// A reference definition, `[1]: https://…`, whose URL is the link's own.
const DEFINITION = / {0,3}\[[^\]\n]+\]:[^\n]*/y;
// An HTML tag, whose attributes may hold URLs, as CommonMark reads one, or an
// autolink (`<https://…>`); `x<y 时 … z>0` is neither.
const TAG =
  /<(?:[A-Za-z][A-Za-z0-9-]*(?:\s+[A-Za-z_:][\w.:-]*(?:\s*=\s*(?:"[^"\n]*"|'[^'\n]*'|[^\s"'=<>`]+))?)*\s*\/?|\/[A-Za-z][A-Za-z0-9-]*\s*|[A-Za-z][A-Za-z0-9+.-]{1,31}:[^\s<>]*)>/y;

/**
 * True when `index` is inside a link's text: an unclosed `[` before it on its
 * line, and a `](` or `][` after it that closes that text. A lone `[`, as in
 * `区间 [0, 1)`, is not one.
 */
function inLinkText(text: string, index: number): boolean {
  let open = false;
  for (let at = index - 1; at >= 0 && index - at <= 200; at -= 1) {
    if (text[at] === "]" || text[at] === "\n") return false;
    if (text[at] === "[") {
      open = true;
      break;
    }
  }
  if (!open) return false;
  for (let at = index; at < text.length && at - index <= 500; at += 1) {
    if (text[at] === "[" || text[at] === "\n") return false;
    if (text[at] === "]") return text[at + 1] === "(" || text[at + 1] === "[";
  }
  return false;
}

/**
 * Whether markdown starts a bare URL at `index`: `http(s)://` after anything
 * but an ASCII letter, `www.` after a space, a bracket or an emphasis mark.
 * Not inside a link's destination (`](…`) or its text (`[…`).
 */
function bareUrlAt(text: string, index: number): "scheme" | "www" | null {
  const ahead = text.slice(index, index + 8);
  const before = text[index - 1] ?? "";
  const kind =
    SCHEME.test(ahead) && !/[A-Za-z]/.test(before)
      ? "scheme"
      : WWW.test(ahead) && /^$|[\s(*_[\]~]/.test(before)
        ? "www"
        : null;
  if (!kind || (before === "(" && text[index - 2] === "]")) return null;
  return inLinkText(text, index) ? null : kind;
}

function rewriteUrls(text: string): string {
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
    if (char === "\\") {
      out += text.slice(index, index + 2);
      index += 2;
      continue;
    }
    if (index === 0 || text[index - 1] === "\n") {
      DEFINITION.lastIndex = index;
      if (DEFINITION.test(text)) {
        out += text.slice(index, DEFINITION.lastIndex);
        index = DEFINITION.lastIndex;
        continue;
      }
    }
    if (char === "<") {
      // A tag goes as it is, attributes and all; so does an autolink.
      TAG.lastIndex = index;
      if (TAG.test(text)) {
        out += text.slice(index, TAG.lastIndex);
        index = TAG.lastIndex;
        continue;
      }
    }
    const kind = bareUrlAt(text, index);
    if (!kind) {
      out += char;
      index += 1;
      continue;
    }
    const run = URL_RUN.exec(text.slice(index))?.[0] ?? "";
    const { url } = splitUrlTail(run);
    const host = url.replace(SCHEME, "");
    // Markdown ends the URL right itself unless full-width punctuation
    // follows it with no space: then it runs on into the sentence and takes
    // the bold or brackets around it with it.
    if (!FULL_WIDTH_PUNCTUATION.test(run) || !host || url.includes(">")) {
      out += run;
      index += run.length;
      continue;
    }
    out += kind === "scheme" ? `<${url}>` : `[${url}](http://${url})`;
    index += url.length;
  }
  return out;
}

/**
 * Ends each bare URL (outside code and links) that full-width punctuation
 * follows, as `<url>`: markdown's link detection runs a URL on to the next
 * space, so in `**http://localhost:3000**，打开` it takes `**，打开` along
 * and the bold never closes. What closes a bracket, quote or emphasis right
 * before the punctuation stays outside the URL (see `splitUrlTail`).
 */
export function rewriteBareUrls(content: string): string {
  if (!FULL_WIDTH_PUNCTUATION.test(content)) return content;
  return rewriteProse(content, rewriteUrls);
}
