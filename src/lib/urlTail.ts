/**
 * Full-width punctuation: ideographic space and marks (U+3000-303F), the
 * full-width forms of ASCII punctuation (U+FF01-FF0F, FF1A-FF20, FF3B-FF40,
 * FF5B-FF65) and the ellipsis. Letters and numerals inside those blocks (々,
 * 〆, 〇) are part of a name, like ideographs and kana, so an IRI such as
 * /wiki/佐々木 stays whole.
 */
const FULL_WIDTH_PUNCTUATION = /(?![\p{L}\p{N}])[…　-〿！-／：-＠［-｀｛-･]/u;
const ASCII_SENTENCE_END = /[.,;:!?]+$/;

/**
 * A URL as found in running text, split from what follows it. Text without
 * spaces (Chinese, Japanese) runs a sentence's closing mark or the next
 * words straight into the URL: it ends at the first full-width punctuation
 * mark, and ASCII sentence punctuation right before it goes with it.
 */
export function splitUrlTail(found: string): { url: string; tail: string } {
  const cut = found.search(FULL_WIDTH_PUNCTUATION);
  const url = (cut < 0 ? found : found.slice(0, cut)).replace(
    ASCII_SENTENCE_END,
    "",
  );
  return { url, tail: found.slice(url.length) };
}
