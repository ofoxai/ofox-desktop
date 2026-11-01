/** Quote at the composer's last caret/selection, preserving surrounding text. */
export function insertAssistantQuote(
  draft: string,
  start: number,
  end: number,
  selected: string,
) {
  const before = draft.slice(0, start);
  const quote = selected
    .trim()
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => `> ${line}`)
    .join("\n");
  const insertion = `${before && !before.endsWith("\n") ? "\n" : ""}${quote}\n\n`;
  return {
    text: before + insertion + draft.slice(end),
    cursor: before.length + insertion.length,
  };
}
