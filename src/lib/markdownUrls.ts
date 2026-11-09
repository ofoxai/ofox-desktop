import { codeSpanEnd, rewriteProse } from "@/lib/markdownSource";
import { FULL_WIDTH_PUNCTUATION, splitUrlTail } from "@/lib/urlTail";

const SCHEME = /^https?:\/\//i;
const WWW = /^www\./i;
// A bare URL runs to a space or a `<`, as markdown reads one.
const URL_RUN = /^[^\s<]+/;

/**
 * Whether markdown starts a bare URL at `index`: `http(s)://` after anything
 * but an ASCII letter, `www.` after a space, a bracket or an emphasis mark.
 * Not inside a link's destination (`](…`) or its text (`[…`).
 */
function bareUrlAt(text: string, index: number): "scheme" | "www" | null {
  const before = text[index - 1] ?? "";
  if (before === "[" || (before === "(" && text[index - 2] === "]"))
    return null;
  const ahead = text.slice(index, index + 8);
  if (SCHEME.test(ahead)) return /[A-Za-z]/.test(before) ? null : "scheme";
  if (WWW.test(ahead)) return /^$|[\s(*_[\]~]/.test(before) ? "www" : null;
  return null;
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
    if (char === "<") {
      // An autolink or a tag goes as it is, up to its `>`.
      const end = text.indexOf(">", index);
      if (end > index && !/\s/.test(text.slice(index, end))) {
        out += text.slice(index, end + 1);
        index = end + 1;
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
