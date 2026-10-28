import { useLayoutEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { FileImage, FileText, X } from "lucide-react";
import type { AssistantAttachment } from "@/lib/api/assistant";
import { describeAttachmentOutcome } from "@/utils/assistantAttachmentOutcome";
import { fmtBinaryBytes } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Outcomes that still let a file be sent, shown as warnings. */
const WARNINGS = new Set<string>(["partial", "model_unsupported"]);

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
  const notes = describeAttachmentOutcome(file, t, i18n.language);
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
