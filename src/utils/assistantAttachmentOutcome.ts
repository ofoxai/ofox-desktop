import type { TFunction } from "i18next";
import type { AssistantAttachment } from "@/lib/api/assistant";

const integers = (value: unknown): number[] =>
  Array.isArray(value)
    ? value.filter((item): item is number => Number.isInteger(item))
    : [];
const integer = (value: unknown) =>
  Number.isInteger(value) ? (value as number) : undefined;

/** Page numbers as ranges: "2–4, 7". */
function pageList(pages: number[]) {
  const sorted = [...new Set(pages)].sort((a, b) => a - b);
  const ranges: string[] = [];
  for (let start = 0; start < sorted.length; ) {
    let end = start;
    while (sorted[end + 1] === sorted[end] + 1) end += 1;
    ranges.push(
      end > start ? `${sorted[start]}–${sorted[end]}` : `${sorted[start]}`,
    );
    start = end + 1;
  }
  return ranges.join(", ");
}

/** The runtime's limits are binary megabytes: 25 MiB reads as "25 MB". */
const megabytes = (bytes: number) =>
  `${Number((bytes / 2 ** 20).toFixed(1))} MB`;

/**
 * How much a note matters: `info` is what to know about a file that was read
 * (an Office file read as text), `warn` a part that was not read or a model
 * that can't read it (the file can still be sent), `error` a file that can't
 * be used at all.
 */
export type AttachmentNoteTone = "info" | "warn" | "error";

export interface AttachmentNote {
  text: string;
  tone: AttachmentNoteTone;
}

/**
 * What the last read of a file means for the user and what to do about it,
 * from its status and the runtime's details. Never a raw code or reason.
 */
export function describeAttachmentOutcome(
  file: AssistantAttachment,
  t: TFunction,
  language: string,
): AttachmentNote[] {
  const details = file.details ?? {};
  const note = (
    tone: AttachmentNoteTone,
    name: string,
    values?: Record<string, unknown>,
  ): AttachmentNote => ({
    text: t(`assistant.attachmentOutcome.${name}`, values),
    tone,
  });
  const info = (name: string) => note("info", name);
  const warn = (name: string, values?: Record<string, unknown>) =>
    note("warn", name, values);
  const error = (name: string, values?: Record<string, unknown>) =>
    note("error", name, values);
  switch (file.status) {
    case "ready":
      return [];
    case "partial": {
      const notes: AttachmentNote[] = [];
      if (details.textOnly === true) notes.push(info("officeTextOnly"));
      if (details.format === "docx" && details.textOnly === true)
        notes.push(info("officeWordScope"));
      if (details.structureCoverage === "partial")
        notes.push(info("officeStructure"));
      if (details.format === "xlsx" && details.textOnly === true)
        notes.push(info("officeCachedValues"));
      // Without image input there is no `pagesAttachedAsImages`.
      const vision = Array.isArray(details.pagesAttachedAsImages);
      const shown = integers(details.pagesAttachedAsImages);
      const unread = integers(details.pagesWithoutText).filter(
        (page) => !shown.includes(page),
      );
      if (unread.length) {
        const one = unread.length === 1;
        const name = vision
          ? one
            ? "pageNotAttached"
            : "pagesNotAttached"
          : one
            ? "pageNeedsVision"
            : "pagesNeedVision";
        notes.push(warn(name, { pages: pageList(unread) }));
      }
      const limit = integer(details.extractionLimit);
      if (details.truncated === true && limit !== undefined)
        notes.push(
          warn("truncated", {
            characters: new Intl.NumberFormat(language).format(limit),
          }),
        );
      return notes.length ? notes : [warn("partial")];
    }
    case "model_unsupported": {
      // The runtime's own reading of the format wins over the file name's.
      const pdf =
        typeof details.format === "string"
          ? details.format === "pdf"
          : file.mimeType === "application/pdf";
      return [warn(pdf ? "pdfNeedsVision" : "imageNeedsVision")];
    }
    case "too_large": {
      if (details.limit === "office_expansion")
        return [error("officeTooLarge")];
      const maxBytes = integer(details.maxBytes);
      const page = integer(details.page);
      const maxPages = integer(details.maxPages);
      const pageCount = integer(details.pageCount);
      const maxImages = integer(details.maxImages);
      if (details.limit === "file_size" && maxBytes)
        return [error("tooLargeFile", { size: megabytes(maxBytes) })];
      if (details.limit === "image_size" && maxBytes)
        return [error("tooLargeImage", { size: megabytes(maxBytes) })];
      if (details.limit === "pdf_pages" && maxPages && pageCount)
        return [error("tooManyPages", { max: maxPages, pages: pageCount })];
      if (details.limit === "message_images" && maxImages)
        return [error("tooManyImages", { max: maxImages })];
      if (
        (details.limit === "rendered_page" || details.limit === "pdf_image") &&
        page
      )
        return [error("pageTooLarge", { page })];
      return [error("tooLarge")];
    }
    case "unreadable": {
      if (
        file.reason === "assistant_attachment_not_found" ||
        details.reason === "missing" ||
        details.reason === "unknown_id"
      )
        return [error("missing")];
      const reasons: Record<string, string> = {
        password_protected: "passwordProtected",
        invalid_pdf: "invalidPdf",
        invalid_utf8: "invalidText",
        invalid_image: "invalidImage",
        encrypted_office: "encryptedOffice",
        invalid_office: "invalidOffice",
        empty_office: "emptyOffice",
      };
      const reason =
        typeof details.reason === "string" &&
        Object.prototype.hasOwnProperty.call(reasons, details.reason)
          ? reasons[details.reason]
          : "unreadable";
      return [error(reason)];
    }
    case "unsupported":
      return [
        error(details.extension === ".doc" ? "legacyDoc" : "unsupported"),
      ];
    default:
      return [error("unavailable")];
  }
}
