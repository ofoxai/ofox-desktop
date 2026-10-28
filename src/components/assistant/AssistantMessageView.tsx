import { memo, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  ChevronDown,
  ExternalLink,
  Pencil,
  RotateCcw,
  RotateCw,
} from "lucide-react";
import type {
  AssistantAttachment,
  AssistantMessage,
  AssistantSource,
} from "@/lib/api/assistant";
import { assistantErrorKey } from "@/lib/api/assistant";
import { settingsApi } from "@/lib/api/settings";
import { splitUrlTail } from "@/lib/urlTail";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { isSendable } from "@/utils/assistantAttachments";
import { AttachmentList } from "./AssistantAttachments";
import type { AssistantActivity } from "./useAssistant";

const MessageMarkdown = memo(SessionMarkdown);

type ShownSource = AssistantSource & { site: string };

/**
 * A source as offered: its site (without `www.`) when it is an HTTPS page,
 * the only kind the backend keeps; anything else is never offered.
 */
function shownSource(source: AssistantSource): ShownSource[] {
  try {
    const url = new URL(source.url);
    return url.protocol === "https:"
      ? [{ ...source, site: url.hostname.replace(/^www\./, "") }]
      : [];
  } catch {
    return [];
  }
}

/**
 * A page URL as citations are compared: without its fragment, `utm_`
 * tracking or a trailing slash. Null for anything but an HTTPS URL.
 */
function citationKey(value: string) {
  try {
    const url = new URL(value);
    if (url.protocol !== "https:") return null;
    url.hash = "";
    for (const name of [...url.searchParams.keys()])
      if (name.startsWith("utm_")) url.searchParams.delete(name);
    return url.href.replace(/\/(?=$|\?)/, "");
  } catch {
    return null;
  }
}

/** The sources the answer links to, then the others, each in their order. */
function splitCited(sources: ShownSource[], content: string) {
  const linked = new Set(
    [...content.matchAll(/https:\/\/[^\s<>"'`()[\]]+/g)].flatMap(
      ([found]) => citationKey(splitUrlTail(found).url) ?? [],
    ),
  );
  const isCited = (source: ShownSource) =>
    content.includes(source.url) || linked.has(citationKey(source.url) ?? "");
  return {
    cited: sources.filter(isCited),
    rest: sources.filter((source) => !isCited(source)),
  };
}

/**
 * A reply's sources: those its answer cites, then the rest behind a
 * disclosure, numbered in that order.
 */
function SourceList({
  sources,
  content,
}: {
  sources: ShownSource[];
  content: string;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const label = useId();
  const toggle = useId();
  const restList = useId();
  const { cited, rest } = splitCited(sources, content);
  const item = (source: ShownSource, number: number) => (
    <li key={source.id}>
      <Button
        variant="link"
        title={source.url}
        className="h-auto max-w-full items-start justify-start whitespace-normal px-0 text-left"
        onClick={() =>
          void settingsApi
            .openExternal(source.url)
            .catch(() => toast.error(t("shell.openFailed")))
        }
      >
        {/* mt-0.5 centres the icon on the title's first line. */}
        <ExternalLink aria-hidden className="mt-0.5 h-3 w-3 shrink-0" />
        <span className="min-w-0 break-words">
          {number}. {source.title}
        </span>{" "}
        <span className="shrink-0 text-note text-ink-muted">{source.site}</span>
      </Button>
    </li>
  );
  return (
    <div className="mt-3 border-t border-line pt-2">
      <p id={label} className="mb-1 text-note text-ink-muted">
        {t("assistant.sources")}
      </p>
      {/* role="list": a list without bullets keeps its role in WebKit. */}
      {cited.length > 0 && (
        <ul role="list" className="space-y-1" aria-labelledby={label}>
          {cited.map((source, index) => item(source, index + 1))}
        </ul>
      )}
      {rest.length > 0 && (
        <>
          <Button
            id={toggle}
            variant="link"
            aria-expanded={expanded}
            aria-controls={restList}
            className="h-auto px-0 text-note text-ink-muted"
            onClick={() => setExpanded((open) => !open)}
          >
            <ChevronDown
              aria-hidden
              className={cn(
                "h-3 w-3 shrink-0 transition-transform motion-reduce:transition-none",
                expanded && "rotate-180",
              )}
            />
            {cited.length > 0
              ? t("assistant.sourcesMore", { count: rest.length })
              : t("assistant.sourcesShow", { count: rest.length })}
          </Button>
          <ul
            id={restList}
            role="list"
            hidden={!expanded}
            className="space-y-1"
            aria-labelledby={toggle}
          >
            {expanded &&
              rest.map((source, index) =>
                item(source, cited.length + index + 1),
              )}
          </ul>
        </>
      )}
    </div>
  );
}

/**
 * One user question or assistant reply. Only those two roles reach this view.
 * Its callbacks name the message they are about, so the page can hand every
 * message the same functions and a message that did not change is not drawn
 * again while another one streams.
 *
 * A question is a bubble on the right, with the files it was sent with under
 * it and that turn's outcome; an edit sends them again, leaving out those
 * that can't be read. A reply uses the full width. `onRetry` is given for a
 * question whose turn failed before the model kept it: it can only be sent
 * again, as a new turn. `activity` says what a streaming reply is doing while
 * one of its tools runs. `searchUnavailable` marks a reply whose run went
 * without the web search its conversation asked for. `latest` marks the
 * newest reply, whose actions stay in view; the others show theirs while the
 * pointer or keyboard focus is on the message.
 */
export const AssistantMessageView = memo(function AssistantMessageView({
  message,
  activity = null,
  searchUnavailable = false,
  latest = false,
  disabled,
  onEdit,
  onRegenerate,
  onRetry,
}: {
  message: AssistantMessage;
  activity?: AssistantActivity | null;
  searchUnavailable?: boolean;
  latest?: boolean;
  disabled: boolean;
  onEdit: (
    messageId: string,
    text: string,
    attachments: AssistantAttachment[],
  ) => Promise<boolean>;
  onRegenerate: (messageId: string) => void;
  onRetry?: (messageId: string) => void;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(message.content);
  const [editFiles, setEditFiles] = useState<AssistantAttachment[]>([]);
  const editor = useRef<HTMLTextAreaElement>(null);
  const isUser = message.role === "user";
  const streaming = message.status === "streaming";
  const files = message.attachments ?? [];
  const sources = (message.sources ?? []).flatMap(shownSource);
  const progress =
    streaming && activity ? t(`assistant.activity.${activity}`) : null;
  const statusLine = (text: string, className?: string) => (
    <p
      role="status"
      className={cn(
        "flex items-center gap-2 text-ctl text-ink-muted",
        className,
      )}
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
      {text}
    </p>
  );
  const resendable = files.filter(isSendable);
  // Edit and regenerate branch from the model's own history: a turn that
  // never reached it (it failed first) has no entry there to branch from.
  const branchable =
    !message.superseded &&
    Boolean(message.nativeEntryId) &&
    (isUser || Boolean(message.runId));
  const role = t(isUser ? "assistant.roles.user" : "assistant.roles.assistant");
  // A failed turn's question can only be sent again: that stays in view.
  const retryable = isUser && Boolean(onRetry) && !message.superseded;
  const actionsShown = latest || retryable;
  const hasActions = Boolean(message.content) || branchable || retryable;
  const body = (
    <article
      className={cn("group min-w-0 py-2", isUser && "flex flex-col items-end")}
      aria-label={role}
    >
      <h2 className="sr-only">{role}</h2>
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
        <div className="w-full space-y-2">
          <Textarea
            ref={editor}
            autoFocus
            aria-label={t("assistant.editMessage")}
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
          {files.length > 0 && (
            <>
              <AttachmentList
                files={editFiles}
                onRemove={(id) =>
                  setEditFiles((current) =>
                    current.filter((file) => file.id !== id),
                  )
                }
                onAllRemoved={() => editor.current?.focus()}
                removeDisabled={disabled}
              />
              <p className="text-note text-ink-muted">
                {[
                  editFiles.length > 0 && t("assistant.editKeepsFiles"),
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
              disabled={disabled || (!text.trim() && !editFiles.length)}
              onClick={() => {
                void onEdit(message.id, text, editFiles).then((sent) => {
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
        statusLine(progress || t("assistant.typing"))
      ) : (
        <>
          {/* A question sent with files alone has no words to show. */}
          {(message.content || !files.length) && (
            <div
              className={cn(
                "min-w-0 break-words text-ui leading-relaxed text-ink",
                isUser && "max-w-[80%] rounded-list bg-pill px-3.5 py-2",
              )}
            >
              <MessageMarkdown content={message.content} syntaxHighlight />
            </div>
          )}
          {files.length > 0 && (
            <AttachmentList
              files={files}
              className={cn(
                message.content && "mt-2",
                isUser && "max-w-[80%] justify-end",
              )}
            />
          )}
          {progress && statusLine(progress, "mt-2")}
        </>
      )}
      {/* Plain text: a live failure was already announced by the page's
          banner, and opening a conversation must not announce its old ones. */}
      {message.status === "error" && message.errorCode && (
        <p className="mt-2 text-ctl text-danger">
          {t(assistantErrorKey(message.errorCode))}
        </p>
      )}
      {searchUnavailable && !isUser && (
        <p className="mt-2 text-note text-ink-muted">
          {t("assistant.searchSkipped")}
        </p>
      )}
      {sources.length > 0 && (
        <SourceList sources={sources} content={message.content} />
      )}
      {!editing && !streaming && (
        <div
          className={cn(
            "mt-1 flex flex-wrap items-center gap-1.5",
            isUser && "justify-end",
          )}
        >
          {message.status === "stopped" && (
            <Badge
              variant="outline"
              className="border-line bg-pill px-1.5 py-0 text-note font-normal text-ink-muted"
            >
              {t("assistant.stopped")}
            </Badge>
          )}
          {message.status === "error" && (
            <Badge
              variant="outline"
              className="border-transparent bg-danger-soft px-1.5 py-0 text-note font-normal text-danger"
            >
              {t("assistant.failed")}
            </Badge>
          )}
          {/* Hidden by opacity only: the buttons stay in the tab order, and
              the row shows once one has focus. */}
          {hasActions && (
            <div
              className={cn(
                "flex items-center gap-1 transition-opacity motion-reduce:transition-none",
                !actionsShown &&
                  "opacity-0 focus-within:opacity-100 group-hover:opacity-100",
              )}
            >
              {message.content && <CopyButton text={message.content} />}
              {isUser && branchable && (
                <Button
                  variant="icon"
                  aria-label={t("assistant.editMessage")}
                  title={t("assistant.editMessage")}
                  disabled={disabled}
                  onClick={() => {
                    setText(message.content);
                    setEditFiles(resendable);
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
                  onClick={() => onRegenerate(message.id)}
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                </Button>
              )}
              {retryable && (
                <Button
                  variant="ghost"
                  disabled={disabled}
                  onClick={() => onRetry?.(message.id)}
                >
                  <RotateCw aria-hidden className="h-3.5 w-3.5" />
                  {t("assistant.retryMessage")}
                </Button>
              )}
            </div>
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
});
