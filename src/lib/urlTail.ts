/**
 * Full-width punctuation: ideographic space and marks (U+3000-303F), the
 * full-width forms of ASCII punctuation (U+FF01-FF0F, FF1A-FF20, FF3B-FF40,
 * FF5B-FF65), curly quotes (U+2018-201F) and the ellipsis. Letters and numerals inside those blocks (々,
 * 〆, 〇) are part of a name, like ideographs and kana, so an IRI such as
 * /wiki/佐々木 stays whole.
 */
export const FULL_WIDTH_PUNCTUATION =
  /(?![\p{L}\p{N}])[…‘-‟　-〿！-／：-＠［-｀｛-･]/u;
const ASCII_SENTENCE_END = /[.,;:!?]+$/;
/** What closes a quote, bracket or emphasis around a URL in running text. */
const ASCII_CLOSERS = /[.,;:!?"'*_~\]]+$/;

const count = (text: string, char: string) => text.split(char).length - 1;

/** Drops closers from the end until the URL ends in none. A `)` goes only when no `(` in the URL opens it. */
function trimClosers(url: string): string {
  let kept = url;
  for (;;) {
    const bare = kept.replace(ASCII_CLOSERS, "");
    const next =
      bare.endsWith(")") && count(bare, ")") > count(bare, "(")
        ? bare.slice(0, -1)
        : bare;
    if (next === kept) return kept;
    kept = next;
  }
}

/**
 * A URL as found in running text, split from what follows it. Text without
 * spaces (Chinese, Japanese) runs a sentence's closing mark or the next
 * words straight into the URL: it ends at the first full-width punctuation
 * mark. What closed a bracket, quote or emphasis around the URL, and ASCII
 * sentence punctuation, right before that mark goes with it, as markdown's
 * own link detection would have done had the mark not followed. Without a
 * full-width mark only sentence punctuation is dropped.
 */
export function splitUrlTail(found: string): { url: string; tail: string } {
  const cut = found.search(FULL_WIDTH_PUNCTUATION);
  const url =
    cut < 0
      ? found.replace(ASCII_SENTENCE_END, "")
      : trimClosers(found.slice(0, cut));
  return { url, tail: found.slice(url.length) };
}
