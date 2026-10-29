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

/**
 * The files of a paste the composer takes over, or null when the text field
 * keeps it. A clipboard with files and no plain text (a screenshot, an image
 * copied from a page, a file copied in the file manager) is a file paste. One
 * with plain text is a text paste, files or not: copying text from a page
 * puts files beside it, and the words are what was meant. A file's type is
 * never read here: the backend reads it from the bytes.
 */
export function pastedFiles(clipboard: PastedData | null): PastedFiles | null {
  if (!clipboard || clipboard.files.length === 0) return null;
  if (clipboard.getData("text/plain") !== "") return null;
  const all = Array.from(clipboard.files);
  return {
    files: all.slice(0, MAX_PASTED_FILES),
    skipped: Math.max(0, all.length - MAX_PASTED_FILES),
  };
}
