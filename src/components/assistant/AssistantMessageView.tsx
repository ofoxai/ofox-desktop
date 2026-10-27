import { memo, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  ChevronDown,
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
      ([url]) => citationKey(url.replace(/[.,;:!?]+$/, "")) ?? [],
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
        className="h-auto max-w-full justify-start whitespace-normal px-0 text-left"
        onClick={() =>
          void settingsApi
            .openExternal(source.url)
            .catch(() => toast.error(t("shell.openFailed")))
        }
      >
        <ExternalLink aria-hidden className="h-3 w-3 shrink-0" />
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
 * `onRetry` is given for a question whose turn failed before the model kept
 * it: it can only be sent again, as a new turn. A question shows the files it
 * was sent with and that turn's outcome; an edit sends them again, leaving
 * out those that can't be read. `activity` says what a streaming reply is
 * doing while one of its tools runs. `searchUnavailable` marks a reply whose
 * run went without the web search its conversation asked for.
 */
export function AssistantMessageView({
  message,
  activity = null,
  searchUnavailable = false,
  disabled,
  onEdit,
  onRegenerate,
  onRetry,
}: {
  message: AssistantMessage;
  activity?: AssistantActivity | null;
  searchUnavailable?: boolean;
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
        statusLine(progress || t("assistant.typing"))
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
          {progress && statusLine(progress, "mt-2")}
        </>
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
