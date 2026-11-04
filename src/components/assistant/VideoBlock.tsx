import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Download, Loader2 } from "lucide-react";
import type {
  AssistantMessage,
  AssistantAttachment,
  AssistantMediaStatus,
} from "@/lib/api/assistant";
import { assistantErrorKey, assistantMediaUrl } from "@/lib/api/assistant";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { Button } from "@/components/ui/button";
import { ActivityLink } from "./MediaBlock";
import { MEDIA_PLACEHOLDER, MediaToolButton, MediaTools } from "./MediaTools";

export function VideoBlock({
  message,
  media: shown,
  sessionId,
  owner,
  generation,
  disabled,
  onSave,
  onCancel,
  onResume,
  tools,
}: {
  message: AssistantMessage;
  /** A generation of the reply; its own media when left out. */
  media?: {
    kind: string;
    status: AssistantMediaStatus;
    outputs: string[];
    cost: string | null;
    apex?: string | null;
    submittedAt?: number | null;
  };
  sessionId: string;
  owner: string;
  generation: number;
  disabled?: boolean;
  onSave: (file: AssistantAttachment) => Promise<unknown>;
  onCancel: () => Promise<unknown>;
  onResume: () => Promise<unknown>;
  /** More of the result's actions; the result's details are among them. */
  tools?: ReactNode;
}) {
  const { t } = useTranslation();
  const { apex, isLoading } = useOfoxApex();
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(Date.now);
  const media = shown ?? message.media;
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
  const actions = !paused &&
    (cancellable ||
      ["download_failed", "query_expired", "paused"].includes(
        media.status,
      )) && (
      <div className="flex flex-wrap justify-center gap-2">
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
    );
  return (
    <div className="w-full max-w-[420px] space-y-2">
      {files.length > 0 ? (
        files.map((file) =>
          file.status === "missing" ? (
            <p
              key={file.id}
              className={`${MEDIA_PLACEHOLDER} aspect-video w-full`}
            >
              {t(
                assistantErrorKey(
                  file.reason ?? "assistant_attachment_not_found",
                ),
              )}
            </p>
          ) : (
            <div key={file.id} className="group/media relative">
              <video
                aria-label={file.name}
                src={assistantMediaUrl(sessionId, file.id, owner, generation)}
                controls
                playsInline
                preload="metadata"
                className="block aspect-video w-full rounded-list bg-black object-contain"
              />
              <MediaTools>
                <MediaToolButton
                  label={t("assistant.media.save")}
                  disabled={busy || disabled}
                  onClick={() => void perform(() => onSave(file))}
                >
                  <Download aria-hidden />
                </MediaToolButton>
                {tools}
              </MediaTools>
            </div>
          ),
        )
      ) : (
        // Where the video will be: its state, and what can be done about it.
        <div
          className={`group/media relative ${MEDIA_PLACEHOLDER} aspect-video w-full`}
        >
          {paused ? (
            <p>
              {t("assistant.video.pausedRegion", { apex: media.apex ?? apex })}
            </p>
          ) : pending ? (
            <p role="status" className="flex items-center gap-2 tabular-nums">
              <Loader2 aria-hidden className="h-4 w-4 animate-spin" />
              {t(`assistant.video.status.${media.status}`)} ·{" "}
              {t("assistant.video.seconds", { count: elapsed })}
            </p>
          ) : (
            <p>{t(`assistant.video.status.${media.status}`)}</p>
          )}
          {media.status === "unknown" && (
            <p role="alert">
              {t("assistant.errors.mediaOutcomeUnknown")} <ActivityLink />
            </p>
          )}
          {actions}
          {tools && <MediaTools>{tools}</MediaTools>}
        </div>
      )}
      {files.length > 0 && actions}
      {/* A generation's cost is in its details; an older result shows it. */}
      {!tools && media.cost != null && (
        <p className="text-note tabular-nums text-ink-muted">
          {t("assistant.media.cost", { cost: media.cost })}
        </p>
      )}
    </div>
  );
}
