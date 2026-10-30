import { useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import type {
  AssistantAttachment,
  AssistantAudioResult,
  AssistantPasteTarget,
} from "@/lib/api/assistant";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { AssistantAudioPlayer } from "./AssistantAudioPlayer";

/** Local saved files and text only; no catalog, recorder or generation API. */
export function AssistantAudioHistory({
  target,
  history,
  attachments,
  returnFocusRef,
  onClose,
}: {
  target: AssistantPasteTarget;
  history: readonly AssistantAudioResult[];
  attachments: readonly AssistantAttachment[];
  returnFocusRef: RefObject<HTMLElement>;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [selectedId, setSelectedId] = useState(
    () => history.at(-1)?.operationId,
  );
  const selected =
    history.find((item) => item.operationId === selectedId) ?? history.at(-1);
  const id =
    selected?.kind === "speech"
      ? selected.outputs?.[0]
      : selected?.inputIds?.[0];
  const info =
    selected?.kind === "speech" ? selected.audio.output : selected?.audio.input;
  const file = id ? attachments.find((file) => file.id === id) : undefined;
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        className="flex max-h-[85dvh] max-w-xl flex-col overflow-hidden"
        onCloseAutoFocus={(event) => {
          if (returnFocusRef.current?.isConnected) {
            event.preventDefault();
            returnFocusRef.current.focus();
          }
        }}
      >
        <DialogHeader>
          <DialogTitle>{t("assistant.audioHistoryTitle")}</DialogTitle>
          <DialogDescription>
            {t("assistant.audioHistoryHint")}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 space-y-3 overflow-y-auto px-4 py-3">
          {history.length > 1 && (
            <div className="max-h-28 space-y-1 overflow-y-auto">
              {[...history].reverse().map((item, index) => (
                <Button
                  key={item.operationId}
                  variant={
                    selected?.operationId === item.operationId
                      ? "secondary"
                      : "ghost"
                  }
                  aria-pressed={selected?.operationId === item.operationId}
                  aria-label={`${t("assistant.audioHistoryItem", { number: history.length - index })}: ${t(`assistant.requestKind.${item.kind}`)} ${item.model}`}
                  title={item.model}
                  className="h-auto w-full justify-start py-1.5 text-left text-note"
                  onClick={() => setSelectedId(item.operationId)}
                >
                  <span className="min-w-0 truncate">
                    {history.length - index}.{" "}
                    {t(`assistant.requestKind.${item.kind}`)} · {item.model}
                  </span>
                </Button>
              ))}
            </div>
          )}
          {selected ? (
            <section className="min-w-0 space-y-2 rounded-lg border border-line p-3">
              <h3 className="break-words text-note font-medium text-ink">
                {selected.model}
              </h3>
              <p className="text-note text-ink-muted">
                {t(`assistant.requestKind.${selected.kind}`)} ·{" "}
                {t(`assistant.requestStatus.${selected.status}`)}
              </p>
              {selected.audio.text && (
                <p className="max-h-40 overflow-y-auto whitespace-pre-wrap break-words text-note text-ink-2 select-text">
                  {selected.audio.text}
                </p>
              )}
              {id ? (
                <div className="min-w-0 space-y-1">
                  <p className="text-note text-ink-muted">
                    {t(
                      selected.kind === "speech"
                        ? "assistant.audioHistoryOutput"
                        : "assistant.audioHistoryOriginal",
                    )}
                  </p>
                  <AssistantAudioPlayer
                    key={`${selected.operationId}:${id}`}
                    target={target}
                    attachmentId={id}
                    mimeType={info?.mimeType ?? file?.mimeType ?? ""}
                    durationMs={info?.durationMs}
                  />
                </div>
              ) : (
                <p className="text-note text-ink-muted">
                  {t("assistant.audioHistoryNoFile")}
                </p>
              )}
            </section>
          ) : (
            <p className="text-note text-ink-muted">
              {t("assistant.audioHistoryEmpty")}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="secondary" onClick={onClose}>
            {t("common.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
