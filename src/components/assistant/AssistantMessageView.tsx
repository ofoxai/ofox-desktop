import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  ExternalLink,
  MessageSquare,
  Pencil,
  RotateCcw,
  RotateCw,
  UserRound,
} from "lucide-react";
import type { AssistantMessage, AssistantSource } from "@/lib/api/assistant";
import { settingsApi } from "@/lib/api/settings";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { AttachmentList, isSendable } from "./AssistantAttachments";

const MessageMarkdown = memo(SessionMarkdown);

/** The backend keeps only HTTPS sources; anything else is never offered. */
function isHttps(source: AssistantSource) {
  try {
    return new URL(source.url).protocol === "https:";
  } catch {
    return false;
  }
}

/**
 * One user question or assistant reply. Only those two roles reach this view.
 * `onRetry` is given for a question whose turn failed before the model kept
 * it: it can only be sent again, as a new turn. A question shows the files it
 * was sent with and that turn's outcome; an edit sends them again, leaving
 * out those that can't be read.
 */
export function AssistantMessageView({
  message,
  disabled,
  onEdit,
  onRegenerate,
  onRetry,
}: {
  message: AssistantMessage;
  disabled: boolean;
  onEdit: (text: string) => Promise<boolean>;
  onRegenerate: () => void;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(message.content);
  const isUser = message.role === "user";
  const streaming = message.status === "streaming";
  const files = message.attachments ?? [];
  const sources = (message.sources ?? []).filter(isHttps);
  const resendable = files.filter(isSendable);
  // An edit with no files of its own keeps all of the original's, so one
  // whose files all fail to read could only fail the same way again.
  const noReadableFiles = files.length > 0 && resendable.length === 0;
  // Edit and regenerate branch from the model's own history: a turn that
  // never reached it (it failed first) has no entry there to branch from.
  const branchable =
    !message.superseded &&
    Boolean(message.nativeEntryId) &&
    (isUser || Boolean(message.runId));
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
          {files.length > 0 && (
            <>
              <AttachmentList files={files} />
              <p
                className={cn(
                  "text-note",
                  noReadableFiles ? "text-danger" : "text-ink-muted",
                )}
              >
                {noReadableFiles
                  ? t("assistant.editNoReadableFiles")
                  : [
                      t("assistant.editKeepsFiles"),
                      resendable.length < files.length &&
                        t("assistant.editDropsFiles"),
                    ]
                      .filter(Boolean)
                      .join(" ")}
              </p>
            </>
          )}
          <p className="text-note text-ink-muted">
            {t("assistant.editPreservesOriginal")}
          </p>
          <div className="flex gap-2">
            <Button
              disabled={
                disabled ||
                noReadableFiles ||
                (!text.trim() && !resendable.length)
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
        <>
          {/* A question sent with files alone has no words to show. */}
          {(message.content || !files.length) && (
            <div
              className={cn(
                "min-w-0 break-words text-ui leading-relaxed text-ink",
                isUser && "rounded-list bg-card-2 px-3 py-2.5",
              )}
            >
              <MessageMarkdown content={message.content} syntaxHighlight />
            </div>
          )}
          {files.length > 0 && (
            <AttachmentList
              files={files}
              className={message.content ? "mt-2" : undefined}
            />
          )}
        </>
      )}
      {sources.length > 0 && (
        <div className="mt-3 border-t border-line pt-2">
          <p className="mb-1 text-note text-ink-muted">
            {t("assistant.sources")}
          </p>
          <ul className="space-y-1" aria-label={t("assistant.sources")}>
            {sources.map((source, index) => (
              <li key={source.id}>
                <Button
                  variant="link"
                  title={source.url}
                  className="h-auto max-w-full justify-start whitespace-normal px-0 text-left"
                  onClick={() =>
                    void settingsApi
                      .openExternal(source.url)
                      .catch(() => toast.error(t("shell.openFailed")))
                  }
                >
                  <ExternalLink aria-hidden className="h-3 w-3 shrink-0" />
                  {index + 1}. {source.title}
                </Button>
              </li>
            ))}
          </ul>
        </div>
      )}
      {!editing && !streaming && (
        <div className="mt-2 flex items-center gap-1">
          {message.content && <CopyButton text={message.content} />}
          {isUser && branchable && (
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
          {!isUser && branchable && (
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
          {isUser && onRetry && !message.superseded && (
            <Button variant="ghost" disabled={disabled} onClick={onRetry}>
              <RotateCw aria-hidden className="h-3.5 w-3.5" />
              {t("assistant.retryMessage")}
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
