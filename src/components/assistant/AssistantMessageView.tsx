import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { MessageSquare, Pencil, RotateCcw, UserRound } from "lucide-react";
import type { AssistantMessage } from "@/lib/api/assistant";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

const MessageMarkdown = memo(SessionMarkdown);

/** One user question or assistant reply. Only those two roles reach this view. */
export function AssistantMessageView({
  message,
  disabled,
  onEdit,
  onRegenerate,
}: {
  message: AssistantMessage;
  disabled: boolean;
  onEdit: (text: string) => Promise<boolean>;
  onRegenerate: () => void;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(message.content);
  const isUser = message.role === "user";
  const streaming = message.status === "streaming";
  const role = t(isUser ? "assistant.roles.user" : "assistant.roles.assistant");
  const body = (
    <article className="min-w-0 py-4" aria-label={role}>
      <div className="mb-2 flex items-center gap-2 text-ctl font-semibold text-ink-2">
        <span
          aria-hidden
          className={cn(
            "grid h-6 w-6 shrink-0 place-items-center rounded-ctl",
            isUser ? "bg-pill text-ink-muted" : "bg-brand-soft text-brand-ink",
          )}
        >
          {isUser ? (
            <UserRound className="h-3.5 w-3.5" />
          ) : (
            <MessageSquare className="h-3.5 w-3.5" />
          )}
        </span>
        {role}
        {message.status === "stopped" && (
          <span className="font-normal text-ink-muted">
            {t("assistant.stopped")}
          </span>
        )}
        {message.status === "error" && (
          <span className="font-normal text-danger">
            {t("assistant.failed")}
          </span>
        )}
      </div>
      {message.reasoning && (
        <details className="mb-3 rounded-ctl bg-card-2 px-3 py-2 text-ctl text-ink-muted">
          <summary className="cursor-pointer">
            {t("assistant.reasoning")}
          </summary>
          <div className="mt-2 whitespace-pre-wrap break-words">
            {message.reasoning}
          </div>
        </details>
      )}
      {editing ? (
        <div className="space-y-2">
          <Textarea
            autoFocus
            aria-label={t("assistant.editMessage")}
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
          <p className="text-note text-ink-muted">
            {t("assistant.editPreservesOriginal")}
          </p>
          <div className="flex gap-2">
            <Button
              disabled={
                disabled || (!text.trim() && !message.attachments?.length)
              }
              onClick={() => {
                void onEdit(text).then((sent) => {
                  if (sent) setEditing(false);
                });
              }}
            >
              {t("assistant.resend")}
            </Button>
            <Button variant="ghost" onClick={() => setEditing(false)}>
              {t("common.cancel")}
            </Button>
          </div>
        </div>
      ) : streaming && !message.content ? (
        <p
          role="status"
          className="flex items-center gap-2 text-ctl text-ink-muted"
        >
          <span aria-hidden className="flex gap-1">
            {[0, 1, 2].map((dot) => (
              <span
                key={dot}
                className="h-1.5 w-1.5 animate-pulse rounded-full bg-ink-faint motion-reduce:animate-none"
                style={{ animationDelay: `${dot * 200}ms` }}
              />
            ))}
          </span>
          {t("assistant.typing")}
        </p>
      ) : (
        <div
          className={cn(
            "min-w-0 break-words text-ui leading-relaxed text-ink",
            isUser && "rounded-list bg-card-2 px-3 py-2.5",
          )}
        >
          <MessageMarkdown content={message.content} syntaxHighlight />
        </div>
      )}
      {!editing && !streaming && (
        <div className="mt-2 flex items-center gap-1">
          {message.content && <CopyButton text={message.content} />}
          {isUser && !message.superseded && (
            <Button
              variant="icon"
              aria-label={t("assistant.editMessage")}
              title={t("assistant.editMessage")}
              disabled={disabled}
              onClick={() => {
                setText(message.content);
                setEditing(true);
              }}
            >
              <Pencil className="h-3.5 w-3.5" />
            </Button>
          )}
          {!isUser && !message.superseded && (
            <Button
              variant="icon"
              aria-label={t("assistant.regenerate")}
              title={t("assistant.regenerate")}
              disabled={disabled}
              onClick={onRegenerate}
            >
              <RotateCcw className="h-3.5 w-3.5" />
            </Button>
          )}
        </div>
      )}
    </article>
  );
  return message.superseded ? (
    <details className="rounded-ctl border border-line px-3 text-ink-muted">
      <summary className="cursor-pointer py-2 text-ctl">
        {t("assistant.previousVersion")}
      </summary>
      {body}
    </details>
  ) : (
    body
  );
}
