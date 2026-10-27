import { useLayoutEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { FileImage, FileText, X } from "lucide-react";
import type { AssistantAttachment } from "@/lib/api/assistant";
import { fmtBinaryBytes } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Outcomes that still let a file be sent, shown as warnings. */
const WARNINGS = new Set<string>(["partial", "model_unsupported"]);

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
 * What the last read of a file means for the user and what to do about it,
 * from its status and the runtime's details. Never a raw code or reason.
 */
function describeOutcome(
  file: AssistantAttachment,
  t: TFunction,
  language: string,
): string[] {
  const details = file.details ?? {};
  const key = (name: string, values?: Record<string, unknown>) =>
    t(`assistant.attachmentOutcome.${name}`, values);
  switch (file.status) {
    case "ready":
      return [];
    case "partial": {
      const notes: string[] = [];
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
        notes.push(key(name, { pages: pageList(unread) }));
      }
      const limit = integer(details.extractionLimit);
      if (details.truncated === true && limit !== undefined)
        notes.push(
          key("truncated", {
            characters: new Intl.NumberFormat(language).format(limit),
          }),
        );
      return notes.length ? notes : [key("partial")];
    }
    case "model_unsupported": {
      // The runtime's own reading of the format wins over the file name's.
      const pdf =
        typeof details.format === "string"
          ? details.format === "pdf"
          : file.mimeType === "application/pdf";
      return [key(pdf ? "pdfNeedsVision" : "imageNeedsVision")];
    }
    case "too_large": {
      const maxBytes = integer(details.maxBytes);
      const page = integer(details.page);
      const maxPages = integer(details.maxPages);
      const pageCount = integer(details.pageCount);
      const maxImages = integer(details.maxImages);
      if (details.limit === "file_size" && maxBytes)
        return [key("tooLargeFile", { size: megabytes(maxBytes) })];
      if (details.limit === "image_size" && maxBytes)
        return [key("tooLargeImage", { size: megabytes(maxBytes) })];
      if (details.limit === "pdf_pages" && maxPages && pageCount)
        return [key("tooManyPages", { max: maxPages, pages: pageCount })];
      if (details.limit === "message_images" && maxImages)
        return [key("tooManyImages", { max: maxImages })];
      if (
        (details.limit === "rendered_page" || details.limit === "pdf_image") &&
        page
      )
        return [key("pageTooLarge", { page })];
      return [key("tooLarge")];
    }
    case "unreadable": {
      if (
        file.reason === "assistant_attachment_not_found" ||
        details.reason === "missing" ||
        details.reason === "unknown_id"
      )
        return [key("missing")];
      const reasons: Record<string, string> = {
        password_protected: "passwordProtected",
        invalid_pdf: "invalidPdf",
        invalid_utf8: "invalidText",
        invalid_image: "invalidImage",
      };
      const reason =
        typeof details.reason === "string" &&
        Object.prototype.hasOwnProperty.call(reasons, details.reason)
          ? reasons[details.reason]
          : "unreadable";
      return [key(reason)];
    }
    case "unsupported":
      return [key("unsupported")];
    default:
      return [key("unavailable")];
  }
}

function AttachmentChip({
  file,
  onRemove,
  removeDisabled,
}: {
  file: AssistantAttachment;
  onRemove?: (id: string) => void;
  removeDisabled?: boolean;
}) {
  const { t, i18n } = useTranslation();
  const notes = describeOutcome(file, t, i18n.language);
  const Icon = file.mimeType.startsWith("image/") ? FileImage : FileText;
  return (
    <li className="flex min-w-0 max-w-full flex-col rounded-ctl border border-line bg-card-2 px-2 py-1 text-ctl">
      <div className="flex min-w-0 items-center gap-1.5">
        <Icon aria-hidden className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
        <span className="min-w-0 truncate text-ink" title={file.name}>
          {file.name}
        </span>
        <span className="shrink-0 text-note text-ink-muted">
          {fmtBinaryBytes(file.sizeBytes)}
        </span>
        {onRemove && (
          <Button
            variant="icon"
            data-remove-file
            className="-my-1 -mr-1.5 h-6 w-6"
            aria-label={t("assistant.removeAttachment", { name: file.name })}
            title={t("assistant.removeAttachment", { name: file.name })}
            disabled={removeDisabled}
            onClick={() => onRemove(file.id)}
          >
            <X aria-hidden className="h-3 w-3" />
          </Button>
        )}
      </div>
      {notes.length > 0 && (
        <p
          className={cn(
            "mt-0.5 max-w-[360px] text-note leading-snug",
            WARNINGS.has(file.status) ? "text-warn" : "text-danger",
          )}
        >
          {notes.join(" ")}
        </p>
      )}
    </li>
  );
}

/**
 * Files in the composer, on a sent question or in an edit: name, type, size
 * and, once a run has read them, what that read means. No previews. Removing
 * a file moves focus to the next file's remove button (or the one before);
 * `onAllRemoved` takes it once no file is left.
 */
export function AttachmentList({
  files,
  onRemove,
  onAllRemoved,
  removeDisabled,
  className,
}: {
  files: AssistantAttachment[];
  onRemove?: (id: string) => void;
  onAllRemoved?: () => void;
  removeDisabled?: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const list = useRef<HTMLUListElement>(null);
  // Where the removed file was, until the list without it is on screen.
  const removedAt = useRef<number | null>(null);
  useLayoutEffect(() => {
    const index = removedAt.current;
    if (index === null) return;
    removedAt.current = null;
    const buttons =
      list.current?.querySelectorAll<HTMLButtonElement>(
        "button[data-remove-file]",
      ) ?? [];
    const next = buttons[Math.min(index, buttons.length - 1)];
    if (next) next.focus();
    else onAllRemoved?.();
  }, [files, onAllRemoved]);
  const remove = onRemove
    ? (id: string) => {
        removedAt.current = files.findIndex((file) => file.id === id);
        onRemove(id);
      }
    : undefined;
  if (!files.length) return null;
  return (
    // role="list": a list without bullets keeps its role in WebKit.
    <ul
      ref={list}
      role="list"
      aria-label={t("assistant.attachedFiles")}
      className={cn("flex flex-wrap items-start gap-2", className)}
    >
      {files.map((file) => (
        <AttachmentChip
          key={file.id}
          file={file}
          onRemove={remove}
          removeDisabled={removeDisabled}
        />
      ))}
    </ul>
  );
}
