import { SkillChip, SkillPicker } from "./SkillPicker";
import { memo, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  ChevronDown,
  ExternalLink,
  MoreHorizontal,
  GitFork,
  Pencil,
  Volume2,
  RotateCcw,
} from "lucide-react";
import type {
  AssistantAttachment,
  AssistantMessage,
  AssistantModel,
  AssistantSource,
} from "@/lib/api/assistant";
import { assistantErrorKey } from "@/lib/api/assistant";
import { settingsApi } from "@/lib/api/settings";
import { splitUrlTail } from "@/lib/urlTail";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { cn } from "@/lib/utils";
import { isSendable } from "@/utils/assistantAttachments";
import type { TurnVersions } from "@/utils/assistantVersions";
import { AttachmentList } from "./AssistantAttachments";
import { ReasoningBlock } from "./ReasoningBlock";
import { ReplyMeta } from "./ReplyMeta";
import { RetryCard } from "./RetryCard";
import { VersionSwitcher } from "./VersionSwitcher";
import type { AssistantActivity, AssistantThinking } from "./useAssistant";

import { AssistantQuote } from "./AssistantQuote";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

const MessageMarkdown = memo(SessionMarkdown);

/** No catalog yet: one array, so a message that did not change is not drawn again. */
const NO_MODELS: ReadonlyArray<AssistantModel> = [];

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
 * One user question or assistant reply. Only those two roles reach this view,
 * and only the versions of a turn that are on screen.
 * Its callbacks name the message they are about, so the page can hand every
 * message the same functions and a message that did not change is not drawn
 * again while another one streams.
 *
 * A question is a bubble on the right, with the files it was sent with under
 * it and that turn's outcome; an edit sends them again, leaving out those
 * that can't be read. A reply uses the full width, with a line under it: its
 * actions, its model and time, and a popover with what it used. A failed or
 * stopped reply, when `onRetry` is given, carries a card to ask again (with
 * the same model or another of `models`; `currentModelId` is the
 * conversation's). `versions` is given to the message that carries a turn's
 * `‹2/3›` switcher: it asks `onSwitchVersion` for the question of the version
 * to show, and is off while `disabled`. `activity` says what a streaming
 * reply is doing while one of its tools runs; `thinking` times the reasoning
 * it is streaming; `taskProgress`, what its local configuration run is at,
 * shows under it while it streams. `searchUnavailable` marks a reply whose
 * run went without the web search its conversation asked for. `highlight`
 * marks what a conversation search looks for in the text. `latest` marks the
 * newest reply, whose actions stay in view; the others show theirs while the
 * pointer or keyboard focus is on the message.
 */
export const AssistantMessageView = memo(function AssistantMessageView({
  message,
  activity = null,
  retryAttempt = null,
  thinking = null,
  searchUnavailable = false,
  latest = false,
  highlight = "",
  models = NO_MODELS,
  currentModelId = "",
  versions,
  disabled,
  onEdit,
  onEditingChange,
  onRegenerate,
  onRetry,
  onSwitchVersion,
  onFork,
  onQuote,
  onSpeak,
  onEnableLocal,
  immutable = false,
  mediaBlock,
  generations,
  taskProgress,
}: {
  message: AssistantMessage;
  immutable?: boolean;
  mediaBlock?: React.ReactNode;
  /** The cards of what the reply's generation tools made. */
  generations?: React.ReactNode;
  /** A local configuration run's step, warning and plan, while it streams. */
  taskProgress?: React.ReactNode;
  activity?: AssistantActivity | null;
  retryAttempt?: 1 | 2 | null;
  /** A streaming reply's thinking so far, for its live timer. */
  thinking?: AssistantThinking | null;
  searchUnavailable?: boolean;
  latest?: boolean;
  /** What the sidebar's search looks for: marked wherever the text has it. */
  highlight?: string;
  disabled: boolean;
  onEdit: (
    messageId: string,
    text: string,
    attachments: AssistantAttachment[],
    skill?: string,
  ) => Promise<boolean>;
  /** Tells the page whether this message is being edited. */
  onEditingChange?: (editing: boolean) => void;
  /** The catalog's models: for the names under replies and the retry menu. */
  models?: ReadonlyArray<AssistantModel>;
  currentModelId?: string;
  versions?: TurnVersions;
  onRegenerate: (messageId: string) => void;
  /**
   * Asks again for a failed or stopped reply, with `modelId` when another
   * model was picked. Settles when the send is over (false: it did not go
   * out); the card waits for that.
   */
  onRetry?: (messageId: string, modelId?: string) => Promise<boolean> | void;
  onSwitchVersion?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
  onQuote?: (text: string) => void;
  onSpeak?: (messageId: string, text: string) => void;
  onEnableLocal?: () => void;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(message.content);
  const [editSkill, setEditSkill] = useState(message.skill);
  const [editFiles, setEditFiles] = useState<AssistantAttachment[]>([]);
  // The reply's usage popover is open: its row stays in view meanwhile.
  const [detailsOpen, setDetailsOpen] = useState(false);
  const editor = useRef<HTMLTextAreaElement>(null);
  // A message that carries a turn's switcher is kept when another version is
  // shown (so the switcher keeps its focus): what is open on the old one is
  // not carried over.
  useEffect(() => {
    setEditing(false);
    setDetailsOpen(false);
  }, [message.id]);
  // Also told when the message goes away with its edit still open.
  useEffect(() => {
    if (!editing) return;
    onEditingChange?.(true);
    return () => onEditingChange?.(false);
  }, [editing, onEditingChange]);
  const isUser = message.role === "user";
  const streaming = message.status === "streaming";
  // A generation's files show on its card, not in the reply's file list.
  const generated = new Set(
    (message.generations ?? []).flatMap((item) => item.outputs),
  );
  const files = (message.attachments ?? []).filter(
    (file) => !generated.has(file.id),
  );
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
    !immutable &&
    !message.media &&
    !message.superseded &&
    Boolean(message.nativeEntryId) &&
    (isUser || Boolean(message.runId));
  const forkable = Boolean(
    onFork && !immutable && !message.superseded && message.nativeEntryId,
  );
  const role = t(isUser ? "assistant.roles.user" : "assistant.roles.assistant");
  // A failed or stopped reply carries the card that asks again; that does
  // what Regenerate would, so only the card is offered.
  const retryCard =
    !message.media &&
    !isUser &&
    Boolean(onRetry) &&
    !message.superseded &&
    (message.status === "error" || message.status === "stopped");
  const actionsShown = latest || detailsOpen;
  const hasActions =
    Boolean(message.content) ||
    branchable ||
    forkable ||
    Boolean(versions) ||
    // Every settled reply says its model and time.
    !isUser;
  return (
    // The room between two turns is the actions row of the one above (always
    // kept, so that showing it moves nothing): no more is added to it.
    <article
      className={cn("group min-w-0 pt-1", isUser && "flex flex-col items-end")}
      aria-label={role}
      // What a search result scrolls to.
      data-message-id={message.id}
    >
      <h2 className="sr-only">{role}</h2>
      <ReasoningBlock
        reasoning={message.reasoning}
        reasoningMs={message.reasoningMs}
        thinking={thinking}
      />
      {message.media && mediaBlock ? (
        mediaBlock
      ) : editing ? (
        <div className="w-full space-y-2">
          <SkillPicker
            scope={message.id}
            skill={editSkill}
            onSkill={setEditSkill}
            ref={editor}
            autoFocus
            aria-label={t("assistant.editMessage")}
            value={text}
            onTextChange={setText}
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
                void (
                  editSkill
                    ? onEdit(message.id, text, editFiles, editSkill)
                    : onEdit(message.id, text, editFiles)
                ).then((sent) => {
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
        // The thinking caption above already says so, with its timer.
        retryAttempt || (thinking?.since != null && !progress) ? null : (
          statusLine(progress || t("assistant.typing"))
        )
      ) : (
        <>
          {isUser && message.skill && (
            <div className="mb-1 max-w-full">
              <SkillChip name={message.skill} />
            </div>
          )}
          {/* A question sent with files alone has no words to show. */}
          {(message.content || !files.length) && (
            <div
              className={cn(
                // The markdown's first and last block keep no outer margin, or
                // it would add to the room between turns (and fatten a bubble).
                "min-w-0 break-words text-ui leading-relaxed text-ink [&>div>:first-child]:mt-0 [&>div>:last-child]:mb-0",
                isUser && "max-w-[80%] rounded-list bg-pill px-3.5 py-2",
              )}
            >
              <AssistantQuote
                key={message.id}
                onQuote={!isUser && !immutable ? onQuote : undefined}
              >
                <MessageMarkdown
                  content={
                    !isUser
                      ? message.content.replaceAll(
                          "[[enable_local_config]]",
                          "",
                        )
                      : message.content
                  }
                  highlight={highlight}
                  syntaxHighlight
                  // Only replies draw math; a question shows as it was typed
                  // (`find . \( -name a \)` is a command, not a formula).
                  math={!isUser}
                  previews={!isUser}
                />
              </AssistantQuote>
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
          {/* The reply only offers it; turning the capability on stays the user's click. */}
          {!isUser &&
            message.content.includes("[[enable_local_config]]") &&
            onEnableLocal && (
              <Button
                variant="outline"
                className="mt-2"
                disabled={disabled || streaming || immutable}
                onClick={onEnableLocal}
              >
                {t("assistant.local.enable")}
              </Button>
            )}
          {progress && statusLine(progress, "mt-2")}
        </>
      )}
      {generations}
      {streaming && taskProgress}
      {streaming && retryAttempt && (
        <p role="status" className="mt-2 text-ctl text-ink-muted">
          {t("assistant.retrying", { attempt: retryAttempt })}
        </p>
      )}
      {retryCard ? (
        <RetryCard
          key={message.id}
          errorCode={message.status === "error" ? message.errorCode : null}
          models={models}
          currentModelId={currentModelId}
          disabled={disabled}
          onRetry={(modelId) => onRetry?.(message.id, modelId)}
        />
      ) : (
        // Plain text: a live failure was already announced by the page's
        // banner, and opening a conversation must not announce its old ones.
        message.status === "error" &&
        message.errorCode && (
          <p className="mt-2 text-ctl text-danger">
            {t(assistantErrorKey(message.errorCode))}
          </p>
        )
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
            "mt-0.5 flex min-w-0 items-center gap-1.5",
            isUser && "justify-end",
          )}
        >
          {message.status === "stopped" && (
            <Badge
              variant="outline"
              className="border-line bg-pill px-1.5 py-0 text-note font-normal text-ink-ctl"
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
                "flex min-w-0 items-center gap-1 transition-opacity motion-reduce:transition-none",
                !actionsShown &&
                  "opacity-0 focus-within:opacity-100 group-hover:opacity-100",
              )}
            >
              {message.content && <CopyButton text={message.content} />}
              {isUser && branchable && (
                <Button
                  variant="icon"
                  className="h-6 w-6"
                  aria-label={t("assistant.editMessage")}
                  title={t("assistant.editMessage")}
                  disabled={disabled}
                  onClick={() => {
                    setText(message.content);
                    setEditSkill(message.skill);
                    setEditFiles(resendable);
                    setEditing(true);
                  }}
                >
                  <Pencil className="h-3.5 w-3.5" />
                </Button>
              )}
              {onSpeak &&
                !immutable &&
                message.role === "assistant" &&
                message.status === "completed" &&
                !message.superseded &&
                !message.media &&
                message.content.trim() && (
                  <Button
                    variant="icon"
                    className="h-6 w-6"
                    aria-label={t("assistant.speech.read")}
                    title={t("assistant.speech.read")}
                    disabled={disabled}
                    onClick={() => onSpeak(message.id, message.content)}
                  >
                    <Volume2 aria-hidden className="h-3.5 w-3.5" />
                  </Button>
                )}
              {!isUser && branchable && !retryCard && (
                <Button
                  variant="icon"
                  className="h-6 w-6"
                  aria-label={t("assistant.regenerate")}
                  title={t("assistant.regenerate")}
                  disabled={disabled}
                  onClick={() => onRegenerate(message.id)}
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                </Button>
              )}
              {forkable && (
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      variant="icon"
                      className="h-6 w-6"
                      aria-label={t("assistant.messageActions")}
                      disabled={disabled}
                    >
                      <MoreHorizontal className="h-3.5 w-3.5" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem onSelect={() => onFork?.(message.id)}>
                      <GitFork className="mr-2 h-3.5 w-3.5" />
                      {t("assistant.fork")}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              )}
              {versions && (
                <VersionSwitcher
                  versions={versions}
                  disabled={disabled}
                  onSwitch={(messageId) => onSwitchVersion?.(messageId)}
                />
              )}
              {!isUser && (
                <ReplyMeta
                  key={message.id}
                  message={message}
                  models={models}
                  onOpenChange={setDetailsOpen}
                />
              )}
            </div>
          )}
        </div>
      )}
    </article>
  );
});
