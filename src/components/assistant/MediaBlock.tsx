import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Download, ImagePlus, Loader2, RefreshCw } from "lucide-react";
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
import { MEDIA_PLACEHOLDER, MediaToolButton, MediaTools } from "./MediaTools";

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
  tools,
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
  /** More of the result's actions; the result's details are among them. */
  tools?: ReactNode;
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
        <div role="status" className={`${MEDIA_PLACEHOLDER} h-48 w-48`}>
          <Loader2 aria-hidden className="h-4 w-4 animate-spin" />
          {t("assistant.media.generating")}
        </div>
      )}
      {media.status === "unknown" && (
        <p
          role="alert"
          className="max-w-[420px] rounded-list border border-line bg-card-2 px-3 py-2.5 text-note text-ink-muted"
        >
          {t("assistant.media.unknown")} <ActivityLink />
        </p>
      )}
      {files.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {files.map((file) => (
            <div key={file.id} className="group/media relative max-w-full">
              <button
                type="button"
                aria-label={t("assistant.media.view")}
                className="block max-w-full rounded-list focus-visible:outline-brand"
                onClick={() => setLarge(file)}
              >
                <img
                  src={url(file)}
                  alt={file.name}
                  loading="lazy"
                  className="block h-auto max-h-64 w-auto max-w-full rounded-list object-contain"
                />
              </button>
              <MediaTools>
                <MediaToolButton
                  label={t("assistant.media.save")}
                  disabled={saving}
                  onClick={() => void save(file)}
                >
                  <Download aria-hidden />
                </MediaToolButton>
                {onUseInput && (
                  <MediaToolButton
                    label={t("assistant.media.useInput")}
                    disabled={
                      disabled ||
                      !["image/png", "image/jpeg"].includes(file.mimeType)
                    }
                    onClick={() => onUseInput(file)}
                  >
                    <ImagePlus aria-hidden />
                  </MediaToolButton>
                )}
                {media.status === "completed" && onRegenerate && (
                  <MediaToolButton
                    label={t("assistant.media.again")}
                    disabled={disabled}
                    onClick={onRegenerate}
                  >
                    <RefreshCw aria-hidden />
                  </MediaToolButton>
                )}
                {tools}
              </MediaTools>
            </div>
          ))}
        </div>
      )}
      {/* A generation's cost is in its details; an older result shows it. */}
      {!tools && media.cost != null && (
        <p className="text-note tabular-nums text-ink-muted">
          {t("assistant.media.cost", { cost: media.cost })}
        </p>
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
