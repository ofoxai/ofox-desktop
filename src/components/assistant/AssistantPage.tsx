import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  AlertTriangle,
  ArrowUp,
  ArrowUpRight,
  Globe,
  Download,
  Upload,
  Loader2,
  MessageSquare,
  Paperclip,
  Pencil,
  Plus,
  Square,
  Trash2,
  RotateCcw,
  X,
} from "lucide-react";
import { assistantErrorKey, type AssistantUsage } from "@/lib/api/assistant";
import { fmtCost, fmtTokens } from "@/lib/format";
import { useAssistant } from "./useAssistant";
import { AssistantMessageView } from "./AssistantMessageView";
import { AttachmentList } from "./AssistantAttachments";
import { assistantMarkdown } from "@/utils/assistantMarkdown";
import { isSendable } from "@/utils/assistantAttachments";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { MenuSelect } from "@/components/ui/menu-select";
import { SearchInput } from "@/components/ui/search-input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useShellRefresh } from "@/components/shell/ShellContext";
import { cn } from "@/lib/utils";

const STARTERS = ["summary", "writing", "explain"] as const;

const usd = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  maximumSignificantDigits: 3,
});

/** The most web searches the runtime lets one reply make (its MAX_SEARCHES). */
const MAX_SEARCHES_PER_REPLY = 3;

/**
 * What one web search may cost: the gateway bills each search call, and one
 * search can make one or two (a search, then opening a page). A reply makes
 * at most MAX_SEARCHES_PER_REPLY searches.
 */
function searchPriceNote(unitPriceUsd: string) {
  const price = Number(unitPriceUsd);
  return {
    low: usd.format(price),
    high: usd.format(price * 2),
    max: MAX_SEARCHES_PER_REPLY,
  };
}

/**
 * A conversation's tokens and cost. The cost is shown only once its runs are
 * saved, and as unavailable whenever a request's cost is unknown: never as
 * $0 or as a partial sum. A cost that includes web searches says how many
 * search charges it holds (one search can bill two).
 */
function usageSummary(
  usage: AssistantUsage | undefined,
  settled: boolean,
  t: TFunction,
): string | null {
  if (!usage) return null;
  const input = usage.inputTokens ?? 0;
  const output = usage.outputTokens ?? 0;
  const counted = input > 0 || output > 0;
  const parts = counted
    ? [
        t("assistant.usageTokens", {
          input: fmtTokens(input),
          output: fmtTokens(output),
        }),
      ]
    : [];
  if (settled) {
    if (usage.usageCoverage === "incomplete")
      parts.push(t("assistant.costUnavailable"));
    else if (typeof usage.estimatedCostUsd === "string") {
      const cost = fmtCost(Number(usage.estimatedCostUsd));
      const searches = usage.searchCount ?? 0;
      parts.push(
        searches > 0
          ? t("assistant.usageCostWithSearches", { cost, count: searches })
          : t("assistant.usageCost", { cost }),
      );
    }
    // Requests were made but a model had no price.
    else if (counted || (usage.modelRequests ?? 0) > 0)
      parts.push(t("assistant.costUnavailable"));
  }
  return parts.length ? parts.join(" · ") : null;
}

interface AssistantPageProps {
  enabled?: boolean;
  onLoginRequested?: () => void;
}

/** Signed out: one way in, and nothing loaded or registered. */
export function AssistantLoginRequired({
  onLoginRequested,
}: Pick<AssistantPageProps, "onLoginRequested">) {
  const { t } = useTranslation();
  return (
    <section className="grid h-full min-h-0 place-items-center rounded-list border border-line bg-card px-6 py-10 text-center">
      <div className="flex max-w-sm flex-col items-center">
        <div className="mb-5 grid h-11 w-11 place-items-center rounded-card bg-brand-soft text-brand-ink">
          <MessageSquare aria-hidden className="h-5 w-5" />
        </div>
        <h1 className="text-title font-semibold text-ink">
          {t("assistant.title")}
        </h1>
        <p className="mb-6 mt-2 text-ui leading-relaxed text-ink-muted">
          {t("assistant.loginRequired")}
        </p>
        <Button onClick={onLoginRequested}>{t("toolCatalog.login")}</Button>
      </div>
    </section>
  );
}

export function AssistantPage({
  enabled = true,
  onLoginRequested,
}: AssistantPageProps) {
  return enabled ? (
    <AssistantWorkspace />
  ) : (
    <AssistantLoginRequired onLoginRequested={onLoginRequested} />
  );
}

function AssistantWorkspace() {
  const { t, i18n } = useTranslation();
  const assistant = useAssistant(true);
  const { detail, bootstrap, run, busy } = assistant;
  const [rename, setRename] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [exported, setExported] = useState(false);
  const blockedHint = useId();
  const visionHint = useId();
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const composer = useRef<HTMLTextAreaElement>(null);
  const conversationsButton = useRef<HTMLButtonElement>(null);
  const trashButton = useRef<HTMLButtonElement>(null);
  const deleteButton = useRef<HTMLButtonElement>(null);
  const renameButton = useRef<HTMLButtonElement>(null);
  const focusStarter = useRef(false);
  const session = detail?.session;
  const available = Boolean(
    bootstrap?.runtime.available &&
      bootstrap.models.length > 0 &&
      assistant.connected,
  );
  const trashed = assistant.showTrash || session?.deletedAt != null;
  const writable = available && !run && !busy && !assistant.loading && !trashed;
  // While a run goes on, or until its saved outcome is read back, the totals
  // are running ones and no cost is claimed.
  const usage = usageSummary(
    detail?.usage,
    !run && !assistant.stream && !session?.activeRun,
    t,
  );
  const keyMissing = bootstrap?.key.status === "missing";
  // Runs can search only while the catalog prices a search; a backend
  // without search says nothing about it.
  const searchPrice = bootstrap?.search?.available
    ? bootstrap.search.unitPriceUsd
    : null;
  const searchNote = !bootstrap
    ? null
    : searchPrice !== null
      ? t("assistant.webSearchPrice", searchPriceNote(searchPrice))
      : t("assistant.webSearchUnavailable");
  // What a search costs, or why there is none, whether search is on or not.
  const showSearchNote = available && searchNote !== null;
  // Search can be turned on only while it is offered, and always off, so a
  // conversation never keeps it on when it can't be seen to work. A
  // conversation still loading (or whose load failed) has no setting to
  // change yet.
  const searchLocked =
    !available ||
    trashed ||
    busy ||
    assistant.loading ||
    Boolean(assistant.selectedId && session?.id !== assistant.selectedId) ||
    (!assistant.webSearch && searchPrice === null);
  // The switch is lit only for a search that will happen: a conversation
  // saved with search on while it is unavailable still says "on" to
  // assistive tech (and can be turned off), but doesn't look on.
  const searchLit = assistant.webSearch && searchPrice !== null;
  // Tool, system and other internal records stay out of the transcript.
  const messages = (detail?.messages ?? []).filter(
    (message) => message.role === "user" || message.role === "assistant",
  );
  // Runs whose reply failed or stopped. Their question can be sent again
  // when the model never kept it (nothing to edit or regenerate from).
  const unfinishedRuns = new Set(
    messages
      .filter(
        (message) =>
          message.role === "assistant" &&
          (message.status === "error" || message.status === "stopped"),
      )
      .map((message) => message.runId),
  );
  const retryable = (message: (typeof messages)[number]) =>
    message.role === "user" &&
    !message.superseded &&
    !message.nativeEntryId &&
    Boolean(message.runId) &&
    unfinishedRuns.has(message.runId) &&
    // Sent again, it carries only the files that can still be read.
    Boolean(message.content.trim() || message.attachments?.some(isSendable));
  const files = assistant.attachments;
  // Files whose last read failed outright are never sent: they must go first.
  const blockedFiles = files.some((file) => !isSendable(file));
  // A model the catalog says reads only text would refuse the turn (free of
  // charge); say so before it is sent.
  const composerModel = bootstrap?.models.find(
    (item) => item.id === (session?.modelId ?? assistant.newModelId),
  );
  const imagesNeedVision =
    composerModel?.supportsImages === false &&
    files.some((file) => file.mimeType.startsWith("image/"));
  // What Send's state is about: the composer's warnings.
  const sendHints =
    [blockedFiles && blockedHint, imagesNeedVision && visionHint]
      .filter(Boolean)
      .join(" ") || undefined;
  const focusComposer = useCallback(() => composer.current?.focus(), []);

  useShellRefresh("assistant", async () => {
    await assistant.loadBootstrap();
    await assistant.refreshSessions();
    if (assistant.selectedId)
      await assistant.refreshDetail(assistant.selectedId);
  });

  useEffect(() => {
    setRename(null);
    setConfirmDelete(false);
    setExported(false);
    follow.current = true;
  }, [assistant.selectedId]);
  useEffect(() => {
    if (follow.current && scroll.current)
      scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [assistant.stream, messages.length]);
  useLayoutEffect(() => {
    if (!focusStarter.current) return;
    focusStarter.current = false;
    const input = composer.current;
    if (input && !input.disabled) {
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    }
  }, [assistant.draft]);

  const send = () => {
    follow.current = true;
    void assistant.send().then((sent) => {
      if (sent) composer.current?.focus();
    });
  };
  const stop = () =>
    void assistant.stop().then(() => composer.current?.focus());
  const saveTitle = () => {
    const title = rename?.trim();
    if (title)
      void assistant.update({ title }).then((saved) => {
        if (saved) setRename(null);
      });
  };

  return (
    <div className="flex h-full min-h-0 overflow-hidden rounded-list border border-line bg-card">
      <aside
        className="flex w-52 shrink-0 flex-col gap-2 border-r border-line bg-card-2 p-3"
        aria-label={t("assistant.conversations")}
      >
        <Button
          className="w-full"
          disabled={!available || busy}
          onClick={() => void assistant.create()}
        >
          <Plus aria-hidden className="h-3.5 w-3.5" />
          {t("assistant.newConversation")}
        </Button>
        <div className="grid shrink-0 grid-cols-2 gap-1">
          <Button
            variant="secondary"
            size="sm"
            disabled={busy || assistant.loading || !bootstrap?.owner}
            aria-label={t("assistant.backupTitle")}
            title={t("assistant.backupHint")}
            onClick={() => void assistant.transfer("backup")}
          >
            <Download aria-hidden className="h-3.5 w-3.5" />
            {t("assistant.backupShort")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={busy || assistant.loading || !bootstrap?.owner}
            aria-label={t("assistant.restoreBackupTitle")}
            title={t("assistant.backupHint")}
            onClick={() => void assistant.transfer("restore")}
          >
            <Upload aria-hidden className="h-3.5 w-3.5" />
            {t("assistant.restoreBackupShort")}
          </Button>
        </div>
        {assistant.transferResult && (
          <p
            role="status"
            className="px-1 text-note leading-relaxed text-ink-muted"
          >
            {assistant.transferResult.kind === "backup"
              ? t("assistant.backupSaved")
              : t("assistant.restoreSaved", {
                  imported: assistant.transferResult.imported,
                  count: assistant.transferResult.skipped,
                })}
          </p>
        )}
        <div
          className="grid grid-cols-2 gap-1"
          aria-label={t("assistant.historyView")}
        >
          <Button
            variant="ghost"
            ref={conversationsButton}
            disabled={assistant.transferring}
            className={cn(
              !assistant.showTrash && "bg-brand-soft text-brand-ink",
            )}
            aria-pressed={!assistant.showTrash}
            onClick={() => assistant.viewTrash(false)}
          >
            {t("assistant.conversations")}
          </Button>
          <Button
            variant="ghost"
            ref={trashButton}
            disabled={assistant.transferring}
            className={cn(
              assistant.showTrash && "bg-brand-soft text-brand-ink",
            )}
            aria-pressed={assistant.showTrash}
            onClick={() => assistant.viewTrash(true)}
          >
            {t("assistant.trash")}
          </Button>
        </div>
        <SearchInput
          className="w-full"
          aria-label={t("assistant.search")}
          placeholder={t("assistant.search")}
          value={assistant.search}
          onChange={assistant.setSearch}
        />
        <div className="min-h-0 flex-1 overflow-y-auto">
          <ul className="space-y-1" aria-label={t("assistant.conversations")}>
            {assistant.sessions.map((item) => (
              <li key={item.id}>
                <button
                  type="button"
                  disabled={assistant.transferring}
                  aria-current={
                    assistant.selectedId === item.id ? "page" : undefined
                  }
                  onClick={() => assistant.select(item.id)}
                  className={cn(
                    "w-full rounded-ctl px-2 py-2 text-left text-ctl transition-colors hover:bg-pill focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand disabled:cursor-wait disabled:opacity-50",
                    assistant.selectedId === item.id &&
                      "bg-brand-soft text-brand-ink",
                  )}
                >
                  <span className="block truncate">
                    {item.title || t("assistant.untitled")}
                  </span>
                  <span className="mt-0.5 block truncate text-note text-ink-muted">
                    {item.activeRun
                      ? t("assistant.running")
                      : new Date(item.updatedAt).toLocaleDateString(
                          i18n.language,
                        )}
                  </span>
                </button>
              </li>
            ))}
            {assistant.unreadable.map((item) => (
              <li
                key={item.id}
                className="rounded-ctl px-2 py-2 text-ctl text-ink-muted"
              >
                <span className="flex items-center gap-1.5 font-medium text-danger">
                  <AlertTriangle aria-hidden className="h-3.5 w-3.5" />
                  {t("assistant.unreadable")}
                </span>
                <span className="mt-0.5 block text-note">
                  {t(assistantErrorKey(item.error))}
                </span>
              </li>
            ))}
          </ul>
          {!assistant.sessions.length && !assistant.unreadable.length && (
            <p className="px-2 py-6 text-center text-note text-ink-muted">
              {t(
                assistant.search
                  ? "assistant.noMatches"
                  : assistant.showTrash
                    ? "assistant.trashEmpty"
                    : "assistant.noConversations",
              )}
            </p>
          )}
        </div>
        <section
          aria-label={t("assistant.allUsage")}
          className="shrink-0 border-t border-line px-1 pt-2 text-note text-ink-muted"
        >
          <h2 className="font-medium text-ink">{t("assistant.allUsage")}</h2>
          <p className="mt-1 leading-relaxed">
            {assistant.usageUnavailable
              ? t("assistant.usageUnavailable")
              : assistant.totalUsage
                ? usageSummary(assistant.totalUsage, true, t)
                : t("common.loading")}
          </p>
          <p className="mt-1 leading-relaxed">
            {t("assistant.usageIncludesTrash")}
          </p>
        </section>
        <p className="border-t border-line px-1 pt-2 text-note leading-relaxed text-ink-muted">
          {t("assistant.boundary")}
        </p>
      </aside>
      <section
        className="flex min-h-0 min-w-0 flex-1 flex-col"
        aria-label={t("assistant.chat")}
      >
        <header className="flex shrink-0 items-center justify-between gap-3 border-b border-line px-4 py-2.5">
          <div className="flex min-w-0 items-center gap-1">
            <h1 className="min-w-0 truncate text-ui font-semibold">
              {session?.title || t("assistant.title")}
            </h1>
            {session && (
              <Button
                variant="icon"
                ref={renameButton}
                title={t("assistant.rename")}
                aria-label={t("assistant.rename")}
                onClick={() => setRename(session.title)}
              >
                <Pencil aria-hidden className="h-3.5 w-3.5" />
              </Button>
            )}
          </div>
          {session && (
            <div className="flex shrink-0 items-center gap-1">
              <Button
                variant="icon"
                aria-label={t("assistant.exportMarkdown")}
                title={t("assistant.exportMarkdown")}
                disabled={busy || Boolean(run) || assistant.loading}
                onClick={() => {
                  setExported(false);
                  void assistant
                    .exportMarkdown((snapshot) =>
                      assistantMarkdown(snapshot, t, i18n.language),
                    )
                    .then(setExported);
                }}
              >
                <Download aria-hidden className="h-3.5 w-3.5" />
              </Button>
              {session.deletedAt != null ? (
                <>
                  <Button
                    variant="icon"
                    aria-label={t("assistant.restore")}
                    title={t("assistant.restore")}
                    disabled={busy || Boolean(run)}
                    onClick={() =>
                      void assistant.setDeleted(false).then((saved) => {
                        if (saved) conversationsButton.current?.focus();
                      })
                    }
                  >
                    <RotateCcw aria-hidden className="h-3.5 w-3.5" />
                  </Button>
                  <Button
                    variant="icon"
                    ref={deleteButton}
                    aria-label={t("assistant.deletePermanently")}
                    title={t("assistant.deletePermanently")}
                    disabled={busy || Boolean(run)}
                    onClick={() => setConfirmDelete(true)}
                  >
                    <Trash2 aria-hidden className="h-3.5 w-3.5 text-danger" />
                  </Button>
                </>
              ) : (
                <Button
                  variant="icon"
                  aria-label={t("assistant.moveToTrash")}
                  title={t("assistant.moveToTrash")}
                  disabled={busy}
                  onClick={() =>
                    void assistant.setDeleted(true).then((saved) => {
                      if (saved) conversationsButton.current?.focus();
                    })
                  }
                >
                  <Trash2 aria-hidden className="h-3.5 w-3.5" />
                </Button>
              )}
            </div>
          )}
          {Boolean(session || bootstrap?.models.length) && (
            <MenuSelect
              aria-label={t("assistant.model")}
              value={session?.modelId ?? assistant.newModelId}
              onChange={(modelId) =>
                session
                  ? void assistant.update({ modelId })
                  : assistant.chooseNewModel(modelId)
              }
              disabled={!writable}
              searchable
              searchPlaceholder={t("assistant.searchModels")}
              placeholder={session?.modelId || t("assistant.model")}
              width={310}
              wrapNotes
              align="end"
              triggerClassName="!max-w-[min(250px,100%)]"
              options={(bootstrap?.models ?? []).map((model) => ({
                value: model.id,
                label: model.name,
                note: [
                  model.contextWindow
                    ? t("assistant.contextWindow", {
                        tokens: fmtTokens(model.contextWindow),
                      })
                    : "",
                  model.inputPricePerMillion
                    ? t("assistant.modelPrice", {
                        input: model.inputPricePerMillion,
                        output: model.outputPricePerMillion ?? "—",
                      })
                    : "",
                ]
                  .filter(Boolean)
                  .join(" · "),
              }))}
            />
          )}
        </header>
        {exported && (
          <p
            role="status"
            className="shrink-0 px-4 py-2 text-note text-ink-muted"
          >
            {t("assistant.markdownSaved")}
          </p>
        )}
        {bootstrap?.modelUnavailableReason && (
          <p
            role="status"
            className="border-b border-line px-4 py-2 text-note text-ink-muted"
          >
            {t("assistant.modelUnavailable")}
            <span className="block">
              {t(assistantErrorKey(bootstrap.modelUnavailableReason))}
            </span>
          </p>
        )}
        {bootstrap && !bootstrap.runtime.available && (
          <div
            role="alert"
            className="border-b border-line bg-danger-soft px-4 py-3 text-ctl text-danger"
          >
            <p className="font-medium">{t("assistant.runtimeUnavailable")}</p>
            <p className="mt-1">{t("assistant.runtimeUnavailableHint")}</p>
            <Button
              variant="ghost"
              className="mt-1"
              onClick={() => void assistant.loadBootstrap()}
            >
              {t("common.retry")}
            </Button>
          </div>
        )}
        {assistant.error !== null && (
          <div
            role="alert"
            className="flex items-start justify-between gap-3 border-b border-line bg-danger-soft px-4 py-2 text-ctl text-danger"
          >
            <p className="break-words">
              {t(assistantErrorKey(assistant.error))}
            </p>
            <Button
              variant="icon"
              aria-label={t("common.close")}
              onClick={() => assistant.setError(null)}
            >
              <X aria-hidden className="h-3.5 w-3.5" />
            </Button>
          </div>
        )}
        <div
          ref={scroll}
          onScroll={() => {
            if (scroll.current)
              follow.current =
                scroll.current.scrollHeight -
                  scroll.current.scrollTop -
                  scroll.current.clientHeight <
                100;
          }}
          className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4"
          aria-busy={assistant.loading}
        >
          {assistant.loading ? (
            <div
              role="status"
              className="flex justify-center gap-2 py-12 text-ctl text-ink-muted"
            >
              <Loader2 aria-hidden className="h-4 w-4 animate-spin" />
              {t("assistant.loading")}
            </div>
          ) : (
            <>
              {!messages.length && !assistant.stream && (
                <div className="mx-auto flex min-h-full max-w-md flex-col items-center justify-center py-8 text-center">
                  <div className="mb-4 grid h-10 w-10 place-items-center rounded-card bg-brand-soft text-brand-ink">
                    <MessageSquare aria-hidden className="h-5 w-5" />
                  </div>
                  <h2 className="text-title font-semibold">
                    {t(trashed ? "assistant.trash" : "assistant.welcome")}
                  </h2>
                  <p className="mt-2 max-w-sm text-ctl leading-relaxed text-ink-muted">
                    {t(
                      trashed
                        ? session
                          ? "assistant.trashHint"
                          : "assistant.trashSelect"
                        : "assistant.description",
                    )}
                  </p>
                  {writable && !assistant.draft.trim() && (
                    <div className="mt-5 grid w-full max-w-[300px] gap-2">
                      {STARTERS.map((id) => (
                        <Button
                          key={id}
                          variant="secondary"
                          className="h-auto min-h-9 justify-between whitespace-normal px-3 py-2 text-left font-normal"
                          onClick={() => {
                            focusStarter.current = true;
                            assistant.setDraft(
                              t(`assistant.starters.${id}.prompt`),
                            );
                          }}
                        >
                          <span>{t(`assistant.starters.${id}.label`)}</span>
                          <ArrowUpRight
                            aria-hidden
                            className="h-3.5 w-3.5 shrink-0 text-ink-faint"
                          />
                        </Button>
                      ))}
                    </div>
                  )}
                  {available && !trashed && (
                    <p className="mt-4 text-note text-ink-muted">
                      {t("assistant.startHint")}
                    </p>
                  )}
                </div>
              )}
              <div className="mx-auto max-w-[760px]">
                {messages.map((message) => (
                  <AssistantMessageView
                    key={message.id}
                    message={message}
                    searchUnavailable={assistant.searchSkipped.has(message.id)}
                    disabled={!writable}
                    onEdit={(text, attachments) =>
                      assistant.send({
                        text,
                        attachments,
                        editMessageId: message.id,
                      })
                    }
                    onRegenerate={() =>
                      void assistant.send({
                        text: "",
                        regenerateMessageId: message.id,
                      })
                    }
                    onRetry={
                      retryable(message)
                        ? () => {
                            follow.current = true;
                            void assistant.send({
                              text: message.content,
                              attachments: message.attachments,
                            });
                          }
                        : undefined
                    }
                  />
                ))}
                {assistant.stream?.role === "assistant" && (
                  <AssistantMessageView
                    message={assistant.stream}
                    activity={assistant.activity}
                    searchUnavailable={assistant.searchSkipped.has(
                      assistant.stream.id,
                    )}
                    disabled
                    onEdit={async () => false}
                    onRegenerate={() => undefined}
                  />
                )}
                {assistant.compacting && (
                  <p role="status" className="py-2 text-note text-ink-muted">
                    {t("assistant.compacting")}
                  </p>
                )}
              </div>
            </>
          )}
        </div>
        {session?.deletedAt != null && (
          <p
            role="status"
            className="border-t border-line px-4 py-2 text-note text-ink-muted"
          >
            {t("assistant.trashHint")}
          </p>
        )}
        <footer className="flex max-h-[50%] min-h-0 shrink-0 flex-col bg-card px-4 pb-3 pt-2">
          <div className="mx-auto flex min-h-0 w-full max-w-[760px] flex-col">
            <div className="flex min-h-0 flex-col rounded-list border border-line bg-card shadow-[0_1px_3px_rgba(0,0,0,.04)] transition-[border-color,box-shadow] focus-within:border-brand/60 focus-within:ring-2 focus-within:ring-brand/10">
              {/* Kept mounted: its live region announces the warnings, and
                  the list hands focus back once its last file is removed.
                  Only the list scrolls: the warnings that explain why Send
                  is disabled stay in view below it. */}
              <div
                className={cn(
                  "min-h-0",
                  files.length > 0 && "max-h-36 overflow-y-auto px-3 pt-3",
                )}
              >
                <AttachmentList
                  files={files}
                  onRemove={assistant.removeAttachment}
                  onAllRemoved={focusComposer}
                  removeDisabled={busy || Boolean(run) || trashed}
                />
              </div>
              <div aria-live="polite" className="shrink-0 px-3">
                {blockedFiles && (
                  <p id={blockedHint} className="mt-1.5 text-note text-danger">
                    {t("assistant.attachmentsBlocked")}
                  </p>
                )}
                {imagesNeedVision && (
                  <p id={visionHint} className="mt-1.5 text-note text-warn">
                    {t("assistant.imagesNeedVision")}
                  </p>
                )}
              </div>
              <Textarea
                ref={composer}
                rows={3}
                className="max-h-40 min-h-[76px] shrink-0 resize-none border-0 bg-transparent px-3 py-3 text-ui focus:ring-0"
                aria-label={t("assistant.message")}
                placeholder={t("assistant.composerPlaceholder")}
                value={assistant.draft}
                disabled={!available}
                readOnly={busy || trashed}
                onChange={(event) => assistant.setDraft(event.target.value)}
                onKeyDown={(event) => {
                  // WKWebView reports the Enter that confirms an IME candidate
                  // with isComposing false but keyCode 229.
                  if (
                    event.key === "Enter" &&
                    !event.shiftKey &&
                    !event.nativeEvent.isComposing &&
                    event.keyCode !== 229
                  ) {
                    event.preventDefault();
                    if (writable) send();
                  }
                }}
              />
              <div className="flex shrink-0 items-center justify-between gap-2 pb-2 pl-2 pr-3">
                <div className="flex min-w-0 items-center gap-1.5">
                  <Button
                    variant="icon"
                    aria-label={t("assistant.attach")}
                    title={t("assistant.attach")}
                    disabled={!writable}
                    onClick={() =>
                      // The dialog leaves focus on a button disabled while
                      // it was open: the message takes it back.
                      void assistant.pickAttachments().then(focusComposer)
                    }
                  >
                    <Paperclip aria-hidden className="h-3.5 w-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    aria-pressed={assistant.webSearch}
                    title={searchNote ?? undefined}
                    disabled={searchLocked}
                    onClick={() => assistant.setWebSearch(!assistant.webSearch)}
                    className={cn(
                      "shrink-0 px-2",
                      searchLit &&
                        "bg-brand-soft text-brand-ink hover:bg-brand-soft hover:text-brand-ink",
                    )}
                  >
                    <Globe aria-hidden className="h-3.5 w-3.5" />
                    {t("assistant.webSearch")}
                  </Button>
                  <span className="min-w-0 truncate text-note text-ink-muted">
                    {run ? t("assistant.running") : t("assistant.sendHint")}
                  </span>
                </div>
                {run ? (
                  <Button variant="secondary" onClick={stop}>
                    <Square aria-hidden className="h-3.5 w-3.5" />
                    {t("assistant.stop")}
                  </Button>
                ) : (
                  <Button
                    aria-label={t("assistant.send")}
                    aria-describedby={sendHints}
                    disabled={
                      !writable ||
                      blockedFiles ||
                      (!assistant.draft.trim() && !files.length)
                    }
                    onClick={send}
                  >
                    <ArrowUp aria-hidden className="h-3.5 w-3.5" />
                    {t("assistant.send")}
                  </Button>
                )}
              </div>
            </div>
            {(keyMissing ||
              usage ||
              showSearchNote ||
              assistant.webSearchWaits) && (
              <div className="mt-2 flex shrink-0 flex-wrap items-start justify-between gap-x-3 gap-y-1 text-note text-ink-muted">
                {keyMissing && <p>{t("assistant.keyMissingHint")}</p>}
                {showSearchNote && <p>{searchNote}</p>}
                {assistant.webSearchWaits && (
                  <p role="status">{t("assistant.webSearchNextMessage")}</p>
                )}
                {usage && (
                  <p
                    aria-label={t("assistant.conversationUsage")}
                    className="ml-auto text-right"
                  >
                    {usage}
                  </p>
                )}
              </div>
            )}
          </div>
        </footer>
      </section>
      <Dialog
        open={confirmDelete}
        onOpenChange={(open) => {
          if (!busy) setConfirmDelete(open);
        }}
      >
        <DialogContent
          className="max-w-sm"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            (deleteButton.current ?? trashButton.current)?.focus();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("assistant.deleteTitle")}</DialogTitle>
            <DialogDescription>
              {t("assistant.deleteDescription")}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              autoFocus
              variant="ghost"
              disabled={busy}
              onClick={() => setConfirmDelete(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              disabled={busy}
              onClick={() =>
                void assistant.deleteSession().then((deleted) => {
                  if (deleted) setConfirmDelete(false);
                })
              }
            >
              {t("assistant.deletePermanently")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog
        open={rename !== null}
        onOpenChange={(open) => {
          if (!open) setRename(null);
        }}
      >
        <DialogContent
          className="max-w-sm"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            (renameButton.current ?? conversationsButton.current)?.focus();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("assistant.rename")}</DialogTitle>
            <DialogDescription>
              {t("assistant.renameDescription")}
            </DialogDescription>
          </DialogHeader>
          <div className="px-[18px] py-3">
            <Input
              autoFocus
              aria-label={t("assistant.conversationTitle")}
              value={rename ?? ""}
              maxLength={120}
              onChange={(event) => setRename(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") saveTitle();
              }}
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRename(null)}>
              {t("common.cancel")}
            </Button>
            <Button disabled={!rename?.trim()} onClick={saveTitle}>
              {t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
