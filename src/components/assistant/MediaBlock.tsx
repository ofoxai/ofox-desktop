import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type {
  AssistantMessage,
  AssistantAttachment,
  AssistantMediaStatus,
} from "@/lib/api/assistant";
import { assistantErrorKey, assistantMediaUrl } from "@/lib/api/assistant";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { ofoxActivityUrl } from "@/lib/ofoxUrls";
import { settingsApi } from "@/lib/api/settings";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogClose,
} from "@/components/ui/dialog";

export function MediaBlock({
  message,
  media: shown,
  sessionId,
  owner,
  generation,
  disabled,
  onSave,
  onUseInput,
  onRegenerate,
}: {
  message: AssistantMessage;
  /** A generation of the reply; its own media when left out. */
  media?: {
    status: AssistantMediaStatus;
    outputs: string[];
    cost: string | null;
  };
  sessionId: string;
  owner: string;
  generation: number;
  disabled?: boolean;
  onSave: (file: AssistantAttachment) => Promise<unknown>;
  onUseInput?: (file: AssistantAttachment) => void;
  onRegenerate?: () => void;
}) {
  const { t } = useTranslation();
  const [large, setLarge] = useState<AssistantAttachment | null>(null);
  const [saving, setSaving] = useState(false);
  const media = shown ?? message.media;
  if (!media) return null;
  const files = (message.attachments ?? []).filter((file) =>
    media.outputs.includes(file.id),
  );
  const url = (file: AssistantAttachment) =>
    assistantMediaUrl(sessionId, file.id, owner, generation);
  const save = async (file: AssistantAttachment) => {
    setSaving(true);
    try {
      await onSave(file);
    } catch (error) {
      toast.error(t(assistantErrorKey(error)));
    } finally {
      setSaving(false);
    }
  };
  return (
    <div className="space-y-2">
      {media.status === "submitting" && (
        <p role="status" className="text-note text-ink-muted">
          {t("assistant.media.generating")}
        </p>
      )}
      {media.status === "unknown" && (
        <p role="alert" className="text-note text-ink-muted">
          {t("assistant.media.unknown")} <ActivityLink />
        </p>
      )}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        {files.map((file) => (
          <div
            key={file.id}
            className="min-w-0 rounded-list border border-line p-2"
          >
            <button
              type="button"
              aria-label={t("assistant.media.view")}
              className="block w-full rounded-ctl focus-visible:outline-brand"
              onClick={() => setLarge(file)}
            >
              <img
                src={url(file)}
                alt={file.name}
                loading="lazy"
                className="max-h-80 w-full rounded-ctl object-contain"
              />
            </button>
            <div className="mt-2 flex flex-wrap gap-1">
              <Button
                variant="ghost"
                disabled={saving}
                onClick={() => void save(file)}
              >
                {t("assistant.media.save")}
              </Button>
              {onUseInput && (
                <Button
                  variant="ghost"
                  disabled={
                    disabled ||
                    !["image/png", "image/jpeg"].includes(file.mimeType)
                  }
                  onClick={() => onUseInput(file)}
                >
                  {t("assistant.media.useInput")}
                </Button>
              )}
            </div>
          </div>
        ))}
      </div>
      {media.cost != null && (
        <p className="text-note tabular-nums text-ink-muted">
          {t("assistant.media.cost", { cost: media.cost })}
        </p>
      )}
      {media.status === "completed" && onRegenerate && (
        <Button variant="ghost" disabled={disabled} onClick={onRegenerate}>
          {t("assistant.media.again")}
        </Button>
      )}
      <Dialog
        open={large !== null}
        onOpenChange={(open) => {
          if (!open) setLarge(null);
        }}
      >
        <DialogContent
          className="max-h-[95vh] max-w-[min(1000px,95vw)] gap-3 overflow-y-auto p-4"
          aria-describedby={undefined}
        >
          <DialogTitle>{t("assistant.media.view")}</DialogTitle>
          {large && (
            <img
              src={url(large)}
              alt={large.name}
              className="max-h-[75vh] w-full object-contain"
            />
          )}
          <DialogClose asChild>
            <Button variant="ghost">{t("common.close")}</Button>
          </DialogClose>
        </DialogContent>
      </Dialog>
    </div>
  );
}

export function ActivityLink() {
  const { t } = useTranslation();
  const { apex, isLoading } = useOfoxApex();
  return (
    <a
      className="underline"
      href={ofoxActivityUrl(apex)}
      aria-disabled={isLoading}
      onClick={(event) => {
        event.preventDefault();
        if (!isLoading)
          void settingsApi
            .openExternal(ofoxActivityUrl(apex))
            .catch(() => toast.error(t("shell.openFailed")));
      }}
    >
      {t("assistant.media.activity")}
    </a>
  );
}
