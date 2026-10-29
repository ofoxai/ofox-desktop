import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Pencil, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { type MessageQueue, validQueuedText } from "./useMessageQueue";

interface Props {
  queue: MessageQueue;
  canResume: boolean;
  onPause: () => void;
  onResume: () => void;
  onClear: () => void;
  onEdit: (id: string, text: string) => boolean;
  onRemove: (id: string) => void;
  focusComposer: () => void;
}

/** Unsent text stays out of the transcript and never replaces its draft. */
export function AssistantQueue({
  queue,
  canResume,
  onPause,
  onResume,
  onClear,
  onEdit,
  onRemove,
  focusComposer,
}: Props) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<{ id: string; text: string } | null>(
    null,
  );
  const closeEdit = () => {
    setEditing(null);
    focusComposer();
  };
  const save = () => {
    if (editing && onEdit(editing.id, editing.text)) closeEdit();
  };
  return (
    <section
      aria-label={t("assistant.queue.title")}
      className="flex max-h-40 min-h-0 flex-col border-b border-line px-3 py-2"
    >
      <div className="flex shrink-0 items-center gap-2 text-note">
        <p aria-live="polite" className="min-w-0 flex-1 text-ink-muted">
          {t("assistant.queue.count", { count: queue.messages.length })}
          {queue.paused && (
            <span className="ml-2 text-warn">
              {t("assistant.queue.paused")}
            </span>
          )}
        </p>
        <Button
          variant="ghost"
          className="h-6 px-1.5 text-note"
          disabled={queue.paused && (!canResume || Boolean(editing))}
          onClick={queue.paused ? onResume : onPause}
        >
          {t(queue.paused ? "assistant.queue.resume" : "assistant.queue.pause")}
        </Button>
        <Button
          variant="ghost"
          className="h-6 px-1.5 text-note"
          onClick={() => {
            onClear();
            closeEdit();
          }}
        >
          {t("assistant.queue.clear")}
        </Button>
      </div>
      <ol className="min-h-0 space-y-1 overflow-y-auto overscroll-contain">
        {queue.messages.map((message, index) => {
          const sending = queue.sending?.message.id === message.id;
          return (
            <li key={message.id} className="rounded-ctl bg-card-2 px-2 py-1">
              {editing?.id === message.id ? (
                <div className="space-y-1">
                  <Textarea
                    autoFocus
                    rows={2}
                    className="max-h-24 min-h-16 resize-none text-ctl"
                    aria-label={t("assistant.queue.editMessage")}
                    value={editing.text}
                    onChange={(event) =>
                      setEditing({ ...editing, text: event.target.value })
                    }
                    onKeyDown={(event) => {
                      if (
                        event.nativeEvent.isComposing ||
                        event.keyCode === 229
                      )
                        return;
                      if (event.key === "Escape") {
                        event.preventDefault();
                        closeEdit();
                      } else if (event.key === "Enter" && !event.shiftKey) {
                        event.preventDefault();
                        save();
                      }
                    }}
                  />
                  <div className="flex justify-end gap-1">
                    <Button variant="ghost" onClick={closeEdit}>
                      {t("common.cancel")}
                    </Button>
                    <Button
                      disabled={!validQueuedText(editing.text)}
                      onClick={save}
                    >
                      {t("common.save")}
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="flex min-w-0 items-center gap-2">
                  <span
                    aria-hidden
                    className="text-note tabular-nums text-ink-faint"
                  >
                    {index + 1}
                  </span>
                  <p
                    className="min-w-0 flex-1 truncate text-ctl"
                    title={message.text}
                  >
                    {message.text}
                  </p>
                  {sending && (
                    <Loader2
                      aria-label={t("assistant.queue.sending")}
                      className="h-3.5 w-3.5 shrink-0 animate-spin text-ink-muted"
                    />
                  )}
                  <Button
                    variant="icon"
                    className="h-6 w-6 shrink-0"
                    aria-label={t("assistant.queue.edit", {
                      position: index + 1,
                    })}
                    disabled={sending}
                    onClick={() => {
                      onPause();
                      setEditing({ id: message.id, text: message.text });
                    }}
                  >
                    <Pencil aria-hidden className="h-3 w-3" />
                  </Button>
                  <Button
                    variant="icon"
                    className="h-6 w-6 shrink-0"
                    aria-label={t("assistant.queue.remove", {
                      position: index + 1,
                    })}
                    disabled={sending}
                    onClick={() => {
                      onRemove(message.id);
                      focusComposer();
                    }}
                  >
                    <X aria-hidden className="h-3 w-3" />
                  </Button>
                </div>
              )}
            </li>
          );
        })}
      </ol>
      <p className="mt-1 shrink-0 text-note text-ink-faint">
        {t("assistant.queue.hint")}
      </p>
    </section>
  );
}
