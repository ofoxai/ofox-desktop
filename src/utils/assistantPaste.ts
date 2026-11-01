/** The most files one paste adds: as many as one pick (the backend's limit). */
export const MAX_PASTED_FILES = 10;

/** What the composer reads of a paste. */
type PastedData = Pick<DataTransfer, "files" | "getData">;

export interface PastedFiles {
  /** The files to import, in clipboard order, at most MAX_PASTED_FILES. */
  files: File[];
  /** How many more the clipboard held: never imported, the user is told. */
  skipped: number;
}

const LINE_BREAK = /\r\n|\r|\n/;

/**
 * A name, or a line of text, as compared: trimmed, and in one Unicode form
 * (macOS writes a file's name decomposed, the text may be composed).
 */
const nameKey = (value: string) => value.trim().normalize("NFC");

/**
 * Whether the plain text of a paste says nothing but which files it holds:
 * empty, blank, or lines that are each the name of one of the files. A file
 * copied in the file manager comes with its name as text.
 */
function onlyNamesTheFiles(text: string, files: File[]) {
  const lines = text.split(LINE_BREAK).map(nameKey).filter(Boolean);
  if (lines.length === 0) return true;
  const names = new Set(files.map((file) => nameKey(file.name)));
  return lines.every((line) => names.has(line));
}

/**
 * The files of a paste the composer takes over, or null when the text field
 * keeps it. A clipboard with files is a file paste (a screenshot, an image
 * copied from a page, a file copied in the file manager) when its plain text
 * is empty, blank or only the files' names. Any other plain text makes it a
 * text paste, files or not: copying text from a page puts files beside it,
 * and the words are what was meant. A file's type is never read here: the
 * backend reads it from the bytes.
 */
export function pastedFiles(clipboard: PastedData | null): PastedFiles | null {
  if (!clipboard || clipboard.files.length === 0) return null;
  const all = Array.from(clipboard.files);
  if (!onlyNamesTheFiles(clipboard.getData("text/plain"), all)) return null;
  return {
    files: all.slice(0, MAX_PASTED_FILES),
    skipped: Math.max(0, all.length - MAX_PASTED_FILES),
  };
}
