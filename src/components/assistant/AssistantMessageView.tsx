import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ExternalLink,
  Pencil,
  RotateCcw,
  FileText,
  MessageSquare,
  UserRound,
} from "lucide-react";
import type { AssistantMessage } from "@/lib/api/assistant";
import { settingsApi } from "@/lib/api/settings";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

const MessageMarkdown = memo(SessionMarkdown);

export function AssistantMessageView({
  message,
  disabled,
  onEdit,
  onRegenerate,
  onError,
}: {
  message: AssistantMessage;
  disabled: boolean;
  onEdit: (text: string) => Promise<boolean>;
  onRegenerate: () => void;
  onError: (error: unknown) => void;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(message.content);
  const body = (
    <article
      className="min-w-0 py-4"
      aria-label={t(`assistant.roles.${message.role}`)}
    >
      <div className="mb-2 flex items-center gap-2 text-ctl font-semibold text-ink-2">
        <span
          aria-hidden
          className={cn(
            "grid h-6 w-6 shrink-0 place-items-center rounded-ctl",
            message.role === "assistant"
              ? "bg-brand-soft text-brand-ink"
              : "bg-pill text-ink-muted",
          )}
        >
          {message.role === "user" ? (
            <UserRound className="h-3.5 w-3.5" />
          ) : (
            <MessageSquare className="h-3.5 w-3.5" />
          )}
        </span>
        {t(`assistant.roles.${message.role}`)}
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
      ) : (
        <div
          className={cn(
            "min-w-0 break-words text-ui leading-relaxed text-ink",
            message.role === "user" && "rounded-list bg-card-2 px-3 py-2.5",
          )}
        >
          <MessageMarkdown content={message.content} syntaxHighlight />
        </div>
      )}
      {!!message.attachments?.length && (
        <ul
          className="mt-3 flex flex-wrap gap-2"
          aria-label={t("assistant.attachments")}
        >
          {message.attachments.map((attachment) => (
            <li
              key={attachment.id}
              className="flex min-w-0 max-w-full items-center gap-2 rounded-ctl border border-line bg-card-2 px-2 py-1 text-ctl"
            >
              <FileText className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{attachment.name}</span>
            </li>
          ))}
        </ul>
      )}
      {!!message.sources?.length && (
        <div className="mt-3 border-t border-line pt-2">
          <p className="mb-1 text-note text-ink-muted">
            {t("assistant.sources")}
          </p>
          <ul className="space-y-1">
            {message.sources
              .filter((source) => /^https?:\/\//i.test(source.url))
              .map((source, index) => (
                <li key={source.id}>
                  <Button
                    variant="link"
                    className="h-auto max-w-full justify-start whitespace-normal px-0 text-left"
                    onClick={() =>
                      void settingsApi.openExternal(source.url).catch(onError)
                    }
                  >
                    <ExternalLink className="h-3 w-3 shrink-0" />
                    {index + 1}. {source.title}
                  </Button>
                </li>
              ))}
          </ul>
        </div>
      )}
      {!editing && (
        <div className="mt-2 flex items-center gap-1">
          {message.content && <CopyButton text={message.content} />}
          {message.role === "user" && !message.superseded && (
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
          {message.role === "assistant" && !message.superseded && (
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
