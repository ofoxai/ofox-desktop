import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type {
  AssistantMessage,
  AssistantAttachment,
} from "@/lib/api/assistant";
import { assistantErrorKey, assistantMediaUrl } from "@/lib/api/assistant";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { Button } from "@/components/ui/button";
import { ActivityLink } from "./MediaBlock";

export function VideoBlock({
  message,
  sessionId,
  owner,
  generation,
  disabled,
  onSave,
  onCancel,
  onResume,
}: {
  message: AssistantMessage;
  sessionId: string;
  owner: string;
  generation: number;
  disabled?: boolean;
  onSave: (file: AssistantAttachment) => Promise<unknown>;
  onCancel: () => Promise<unknown>;
  onResume: () => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const { apex, isLoading } = useOfoxApex();
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(Date.now);
  const media = message.media;
  const pending = Boolean(
    media &&
      ["submitting", "queued", "in_progress", "downloading"].includes(
        media.status,
      ),
  );
  useEffect(() => {
    if (!pending) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [pending]);
  if (!media || media.kind !== "video") return null;
  const terminal = [
    "completed",
    "cancelled",
    "failed",
    "expired",
    "error",
  ].includes(media.status);
  const paused =
    !terminal && (isLoading || Boolean(media.apex && media.apex !== apex));
  const perform = async (action: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await action();
    } catch (error) {
      toast.error(t(assistantErrorKey(error)));
    } finally {
      setBusy(false);
    }
  };
  const files =
    message.attachments?.filter((file) => media.outputs.includes(file.id)) ??
    [];
  const elapsed = Math.max(
    0,
    Math.floor((now - (media.submittedAt ?? message.createdAt)) / 1000),
  );
  const cancellable = [
    "submitting",
    "queued",
    "in_progress",
    "paused",
    "query_expired",
  ].includes(media.status);
  return (
    <div className="space-y-2">
      {paused ? (
        <p className="text-note text-ink-muted">
          {t("assistant.video.pausedRegion", { apex: media.apex ?? apex })}
        </p>
      ) : pending ? (
        <p role="status" className="text-note tabular-nums text-ink-muted">
          {t(`assistant.video.status.${media.status}`)} ·{" "}
          {t("assistant.video.seconds", { count: elapsed })}
        </p>
      ) : (
        <p className="text-note text-ink-muted">
          {t(`assistant.video.status.${media.status}`)}
        </p>
      )}
      {media.status === "unknown" && (
        <p role="alert" className="text-note text-ink-muted">
          {t("assistant.errors.mediaOutcomeUnknown")} <ActivityLink />
        </p>
      )}
      {files.map((file) => (
        <div
          key={file.id}
          className="space-y-2 rounded-list border border-line p-2"
        >
          {file.status === "missing" ? (
            <p className="text-note text-ink-muted">
              {t(
                assistantErrorKey(
                  file.reason ?? "assistant_attachment_not_found",
                ),
              )}
            </p>
          ) : (
            <>
              <video
                aria-label={file.name}
                src={assistantMediaUrl(sessionId, file.id, owner, generation)}
                controls
                playsInline
                preload="metadata"
                className="max-h-96 w-full rounded-ctl"
              />
              <Button
                variant="ghost"
                disabled={busy || disabled}
                onClick={() => void perform(() => onSave(file))}
              >
                {t("assistant.media.save")}
              </Button>
            </>
          )}
        </div>
      ))}
      {media.cost != null && (
        <p className="text-note tabular-nums text-ink-muted">
          {t("assistant.media.cost", { cost: media.cost })}
        </p>
      )}
      {!paused && (
        <div className="flex flex-wrap gap-2">
          {cancellable && (
            <Button
              variant="ghost"
              disabled={busy || disabled}
              onClick={() => void perform(onCancel)}
            >
              {t("assistant.video.cancel")}
            </Button>
          )}
          {["download_failed", "query_expired", "paused"].includes(
            media.status,
          ) && (
            <Button
              variant="ghost"
              disabled={busy || disabled}
              onClick={() => void perform(onResume)}
            >
              {t(
                media.status === "download_failed"
                  ? "assistant.video.downloadAgain"
                  : "assistant.video.continueQuery",
              )}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
