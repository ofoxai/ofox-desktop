import {
  useCallback,
  useDeferredValue,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { toast } from "sonner";
import {
  ArrowDown,
  ArrowUp,
  Download,
  Ellipsis,
  Globe,
  Loader2,
  MessageSquare,
  PanelLeftClose,
  PanelLeftOpen,
  Paperclip,
  Pencil,
  Pin,
  PinOff,
  Square,
  Trash2,
  RotateCcw,
  X,
} from "lucide-react";
import {
  assistantErrorKey,
  type AssistantAttachment,
  type AssistantUsage,
} from "@/lib/api/assistant";
import { fmtCost, fmtTokens } from "@/lib/format";
import { isMac } from "@/lib/platform";
import { useAssistant } from "./useAssistant";
import { AssistantMessageView } from "./AssistantMessageView";
import { AttachmentList } from "./AssistantAttachments";
import { AssistantModelPicker } from "./AssistantModelPicker";
import {
  AssistantSidebar,
  type AssistantSidebarHandle,
} from "./AssistantSidebar";
import { InlineRename } from "./InlineRename";
import { ThinkingMenu } from "./ThinkingMenu";
import { useConversationActions } from "./useConversationActions";
import { usePendingFocus } from "./usePendingFocus";
import { assistantMarkdown } from "@/utils/assistantMarkdown";
import { searchHighlight } from "@/utils/assistantSearch";
import { isSendable } from "@/utils/assistantAttachments";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useInteractionActive } from "@/components/ui/interaction-scope";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useShellRefresh } from "@/components/shell/ShellContext";
import { cn } from "@/lib/utils";

const STARTERS = ["summary", "writing", "explain"] as const;

/** The most bytes of text one message may carry (the backend's limit). */
const MAX_MESSAGE_BYTES = 128_000;
/** From this share of the limit the composer says how full the message is. */
const LENGTH_HINT_FROM = 0.8;

const noEdit = async () => false;
const noRegenerate = () => undefined;

const SIDEBAR_COLLAPSED_KEY = "ofox.assistant.sidebarCollapsed";

// Storage can be unavailable (a private window, blocked site data): the
// sidebar then opens expanded and the choice lasts for this window only.
function readSidebarCollapsed() {
  try {
    return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}
function saveSidebarCollapsed(collapsed: boolean) {
  try {
    localStorage.setItem(SIDEBAR_COLLAPSED_KEY, collapsed ? "1" : "0");
  } catch {
    // See readSidebarCollapsed.
  }
}

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
  // Tooltips need a provider, and the root mounts none.
  return enabled ? (
    <TooltipProvider delayDuration={300}>
      <AssistantWorkspace />
    </TooltipProvider>
  ) : (
    <AssistantLoginRequired onLoginRequested={onLoginRequested} />
  );
}

function AssistantWorkspace() {
  const { t, i18n } = useTranslation();
  const assistant = useAssistant(true);
  const { detail, bootstrap, run, busy } = assistant;
  const [renaming, setRenaming] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] =
    useState(readSidebarCollapsed);
  const sidebarId = useId();
  const blockedHint = useId();
  const visionHint = useId();
  const searchNoteId = useId();
  const lengthHintId = useId();
  const interactionActive = useInteractionActive();
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  // The view is more than a screen above the end of the conversation, and
  // whether something new has come in below since it left it.
  const [awayFromEnd, setAwayFromEnd] = useState(false);
  const [unseenBelow, setUnseenBelow] = useState(false);
  // The jump to the end is under way: its first scroll events are still far
  // from the end and must not end the following.
  const jumping = useRef(false);
  const composer = useRef<HTMLTextAreaElement>(null);
  // The workspace itself can hold focus (tabIndex -1), so a click on text
  // that has none leaves it inside instead of on <body>.
  const workspace = useRef<HTMLDivElement>(null);
  // How many messages are being edited: a new conversation would drop them.
  const openEdits = useRef(0);
  const usageLabelId = useId();
  const newConversationButton = useRef<HTMLButtonElement>(null);
  const trashEntry = useRef<HTMLButtonElement>(null);
  const trashBack = useRef<HTMLButtonElement>(null);
  const emptyTrashButton = useRef<HTMLButtonElement>(null);
  const sidebarHandle = useRef<AssistantSidebarHandle>(null);
  const sidebarToggle = useRef<HTMLButtonElement>(null);
  const menuTrigger = useRef<HTMLButtonElement>(null);
  // What the header menu's chosen item does, once the menu has closed.
  const menuAction = useRef<(() => void) | null>(null);
  // A button that is disabled while its action runs loses focus; once the
  // action is over (done, cancelled or failed) it takes focus back.
  const refocus = useRef<HTMLButtonElement | null>(null);
  const busyNow = useRef(busy);
  busyNow.current = busy;
  const focusStarter = useRef(false);
  const focusSoon = usePendingFocus();
  const session = detail?.session;
  const available = Boolean(
    bootstrap?.runtime.available &&
      bootstrap.models.length > 0 &&
      assistant.connected,
  );
  const sessionTrashed = session?.deletedAt != null;
  const trashed = assistant.showTrash || sessionTrashed;
  // Closes with the conversation it asks about, e.g. once only its files are
  // left to delete.
  const deleteDialogOpen = confirmDelete && session !== undefined;
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
  // A conversation still loading (or whose load failed) has no setting to
  // change yet; none is changed in the trash.
  const settingsLocked =
    !available ||
    trashed ||
    busy ||
    assistant.loading ||
    Boolean(assistant.selectedId && session?.id !== assistant.selectedId);
  // Search can be turned on only while it is offered, and always off, so a
  // conversation never keeps it on when it can't be seen to work.
  const searchLocked =
    settingsLocked || (!assistant.webSearch && searchPrice === null);
  // The switch is lit only for a search that will happen: a conversation
  // saved with search on while it is unavailable still says "on" to
  // assistive tech (and can be turned off), but doesn't look on.
  const searchLit = assistant.webSearch && searchPrice !== null;
  // Tool, system and other internal records stay out of the transcript.
  const messages = (detail?.messages ?? []).filter(
    (message) => message.role === "user" || message.role === "assistant",
  );
  const emptyTranscript = !messages.length && !assistant.stream;
  // The newest reply keeps its actions in view; only a reply has them there.
  const newest =
    assistant.stream?.role === "assistant"
      ? assistant.stream
      : messages[messages.length - 1];
  const latestReplyId = newest?.role === "assistant" ? newest.id : undefined;
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
  // The backend refuses a message of more than MAX_MESSAGE_BYTES of text.
  const draftBytes = useMemo(
    () => new TextEncoder().encode(assistant.draft).length,
    [assistant.draft],
  );
  const overLength = draftBytes > MAX_MESSAGE_BYTES;
  const lengthPercent = Math.floor((draftBytes / MAX_MESSAGE_BYTES) * 100);
  const showLength =
    !overLength && draftBytes >= MAX_MESSAGE_BYTES * LENGTH_HINT_FROM;
  // What Send's state is about: the composer's warnings.
  const sendHints =
    [
      blockedFiles && blockedHint,
      imagesNeedVision && visionHint,
      overLength && lengthHintId,
    ]
      .filter(Boolean)
      .join(" ") || undefined;
  const focusComposer = useCallback(() => composer.current?.focus(), []);
  const returnFocusTo = (button: HTMLButtonElement | null) => {
    refocus.current = button;
    settleFocus();
  };
  function settleFocus() {
    const button = refocus.current;
    if (!button) return;
    if (button.disabled) {
      // Still disabled while the action runs: try again after the next render.
      if (!busyNow.current) refocus.current = null;
      return;
    }
    refocus.current = null;
    // Not when the user has already moved on to something else (the
    // workspace itself, which a click on plain text focuses, is nothing).
    const active = document.activeElement;
    if (
      !active ||
      active === document.body ||
      active === workspace.current ||
      active === button
    )
      button.focus();
  }
  useEffect(settleFocus);

  useShellRefresh("assistant", async () => {
    await assistant.loadBootstrap();
    await assistant.refreshSessions();
    if (assistant.selectedId)
      await assistant.refreshDetail(assistant.selectedId);
  });

  useEffect(() => {
    setRenaming(false);
    setConfirmDelete(false);
    follow.current = true;
    jumping.current = false;
    setAwayFromEnd(false);
    setUnseenBelow(false);
  }, [assistant.selectedId]);
  useEffect(() => {
    const view = scroll.current;
    if (!view) return;
    if (follow.current) view.scrollTop = view.scrollHeight;
    else setUnseenBelow(true);
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
  const trackScroll = () => {
    const view = scroll.current;
    if (!view) return;
    const atEnd = view.scrollHeight - view.scrollTop - view.clientHeight < 100;
    if (jumping.current) {
      if (!atEnd) return;
      jumping.current = false;
    }
    follow.current = atEnd;
    setAwayFromEnd(!atEnd);
    if (atEnd) setUnseenBelow(false);
  };
  // The reader taking the scroll over ends a jump that is still under way.
  const endJump = () => {
    jumping.current = false;
  };
  const jumpToEnd = (hadFocus: boolean) => {
    const view = scroll.current;
    if (!view) return;
    // The button goes away with the jump: the focus it had goes to where the
    // reader writes, instead of falling to the page.
    if (hadFocus) {
      const input = composer.current;
      (input && !input.disabled ? input : workspace.current)?.focus();
    }
    follow.current = true;
    setAwayFromEnd(false);
    setUnseenBelow(false);
    if (typeof view.scrollTo !== "function") {
      view.scrollTop = view.scrollHeight;
      return;
    }
    jumping.current = true;
    view.scrollTo({
      top: view.scrollHeight,
      behavior: window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
        ? "auto"
        : "smooth",
    });
  };
  const newConversation = () =>
    void assistant.create().then(() => focusSoon(composer));
  // Shortcuts of the page: only while it is the one on screen, and never for
  // a key that confirms an input method candidate. An Escape that something
  // inside (a menu, the title field) already used is not ours either.
  const handleShortcuts = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!interactionActive) return;
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    // ⌘N on macOS, where Ctrl+N is the text field's own "next line"; Ctrl+N
    // elsewhere.
    const modifier = isMac()
      ? event.metaKey && !event.ctrlKey
      : event.ctrlKey && !event.metaKey;
    if (
      modifier &&
      !event.shiftKey &&
      !event.altKey &&
      event.key.toLowerCase() === "n"
    ) {
      // Not a key held down, nor one pressed in a menu, popover or dialog
      // (drawn outside the workspace, but their events reach it through
      // React), nor while an edit is open that the new conversation would
      // silently drop.
      if (event.repeat || !event.currentTarget.contains(event.target as Node))
        return;
      if (!available || busy || renaming || openEdits.current > 0) return;
      event.preventDefault();
      newConversation();
    } else if (event.key === "Escape" && run && !event.defaultPrevented) {
      // Left alone when no reply is running: the app's own Escape goes on.
      event.preventDefault();
      stop();
    }
  };
  // The Stop button's tooltip takes the first Escape for itself (and then
  // keeps it from the workspace): it stops the reply too, once.
  const stopOnTooltipEscape = (event: globalThis.KeyboardEvent) => {
    if (!interactionActive) return;
    if (event.isComposing || event.keyCode === 229) return;
    stop();
  };

  // Everything a message needs to act, reached through one ref so that the
  // functions below never change and a message that did not change is not
  // drawn again while another one streams.
  const actions = useRef({ send: assistant.send, messages });
  actions.current = { send: assistant.send, messages };
  const editMessage = useCallback(
    (editMessageId: string, text: string, attachments: AssistantAttachment[]) =>
      actions.current.send({ text, attachments, editMessageId }),
    [],
  );
  const regenerateMessage = useCallback(
    (regenerateMessageId: string) =>
      void actions.current.send({ text: "", regenerateMessageId }),
    [],
  );
  const trackEdit = useCallback((open: boolean) => {
    openEdits.current += open ? 1 : -1;
  }, []);
  const retryMessage = useCallback((messageId: string) => {
    const message = actions.current.messages.find(
      (item) => item.id === messageId,
    );
    if (!message) return;
    follow.current = true;
    void actions.current.send({
      text: message.content,
      attachments: message.attachments,
    });
  }, []);
  const toggleSidebar = () => {
    const collapsed = !sidebarCollapsed;
    setSidebarCollapsed(collapsed);
    saveSidebarCollapsed(collapsed);
  };
  // With the list collapsed its entries are hidden: focus goes to the toggle
  // that opens it, the nearest control still on screen.
  const landOnSidebar = (
    entry: RefObject<HTMLButtonElement>,
  ): RefObject<HTMLButtonElement> => (sidebarCollapsed ? sidebarToggle : entry);
  const saveTitle = async (title: string) =>
    Boolean(await assistant.update({ title }));
  const exportConversation = () =>
    void assistant
      .exportMarkdown((snapshot) =>
        assistantMarkdown(snapshot, t, i18n.language),
      )
      .then((saved) => {
        if (saved) toast.success(t("assistant.markdownSaved"));
        returnFocusTo(menuTrigger.current);
      });
  // `trigger` is the Restore button that was pressed: it takes focus back
  // if the restore failed.
  const restore = (trigger: HTMLButtonElement) =>
    void assistant.setDeleted(false).then((saved) => {
      // Back in the conversations, whose footer has the trash entry again.
      if (saved) focusSoon(landOnSidebar(trashEntry));
      else returnFocusTo(trigger);
    });
  const askToDelete = () => {
    // A failure from before is not this dialog's to show.
    assistant.setError(null);
    setConfirmDelete(true);
  };
  // Placed after the effects above that scroll the transcript.
  const conversations = useConversationActions({
    assistant,
    sidebarCollapsed,
    sidebar: sidebarHandle,
    scroll,
    follow,
    focusSoon,
    landOnSidebar,
    composer,
    sidebarToggle,
    trashEntry,
    trashBack,
    emptyTrashButton,
  });
  const emptyTrash = conversations.emptyTrash;
  const moveToTrash = () => {
    if (!session) return;
    conversations.moveToTrash(session.id, session.title, () =>
      returnFocusTo(menuTrigger.current),
    );
  };
  // What the sidebar's search marks in the transcript: it may trail the box
  // while a long conversation is drawn again for each letter typed.
  const transcriptHighlight = useDeferredValue(
    searchHighlight(assistant.search),
  );

  const webSearchToggle = (
    <Button
      variant="ghost"
      aria-pressed={assistant.webSearch}
      aria-describedby={showSearchNote ? searchNoteId : undefined}
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
  );

  return (
    <div
      ref={workspace}
      tabIndex={-1}
      className="flex h-full min-h-0 overflow-hidden rounded-list border border-line bg-card outline-none"
      onKeyDown={handleShortcuts}
    >
      <AssistantSidebar
        ref={sidebarHandle}
        id={sidebarId}
        collapsed={sidebarCollapsed}
        assistant={assistant}
        canCreate={available && !busy}
        onCreate={newConversation}
        onOpen={conversations.openFromList}
        onMoveToTrash={(item) => conversations.moveToTrash(item.id, item.title)}
        onEmptyTrash={emptyTrash.ask}
        newConversationRef={newConversationButton}
        trashEntryRef={trashEntry}
        trashBackRef={trashBack}
        emptyTrashRef={emptyTrashButton}
        onTransfer={(kind, trigger) =>
          void assistant.transfer(kind).then(() => returnFocusTo(trigger))
        }
      />
      <section
        className="flex min-h-0 min-w-0 flex-1 flex-col"
        aria-label={t("assistant.chat")}
      >
        <header className="flex shrink-0 items-center gap-3 border-b border-line px-4 py-2.5">
          <div className="flex min-w-0 flex-1 items-center gap-1">
            <Button
              ref={sidebarToggle}
              variant="icon"
              aria-label={t("assistant.sidebar")}
              title={t(
                sidebarCollapsed
                  ? "assistant.showSidebar"
                  : "assistant.hideSidebar",
              )}
              aria-expanded={!sidebarCollapsed}
              aria-controls={sidebarId}
              onClick={toggleSidebar}
            >
              {sidebarCollapsed ? (
                <PanelLeftOpen aria-hidden className="h-3.5 w-3.5" />
              ) : (
                <PanelLeftClose aria-hidden className="h-3.5 w-3.5" />
              )}
            </Button>
            {renaming && session ? (
              <InlineRename
                title={session.title}
                onSave={saveTitle}
                onClose={(byKeyboard) => {
                  setRenaming(false);
                  if (byKeyboard) focusSoon(menuTrigger);
                }}
              />
            ) : (
              <h1
                className="min-w-0 truncate text-ui font-semibold"
                // A conversation in the trash is read-only until restored.
                onDoubleClick={
                  session && !sessionTrashed
                    ? () => setRenaming(true)
                    : undefined
                }
              >
                {session?.title || t("assistant.title")}
              </h1>
            )}
          </div>
          {Boolean(session || bootstrap?.models.length) && (
            <AssistantModelPicker
              models={bootstrap?.models ?? []}
              value={session?.modelId ?? assistant.newModelId}
              onChange={(modelId) =>
                session
                  ? void assistant.update({ modelId })
                  : assistant.chooseNewModel(modelId)
              }
              partial={Boolean(bootstrap?.partial?.length)}
              disabled={!writable}
              placeholder={session?.modelId || t("assistant.model")}
            />
          )}
          {session && (
            <DropdownMenu modal={false}>
              <DropdownMenuTrigger asChild>
                <Button
                  ref={menuTrigger}
                  variant="icon"
                  aria-label={t("assistant.conversationActions")}
                  title={t("assistant.conversationActions")}
                  className="shrink-0 data-[state=open]:bg-pill data-[state=open]:text-ink"
                >
                  <Ellipsis aria-hidden className="h-4 w-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent
                align="end"
                className="min-w-[160px] max-w-[280px]"
                // The label below is skipped when arrowing through the items.
                aria-describedby={usage ? usageLabelId : undefined}
                // An item acts once the menu is closed, so the dialog or the
                // field it opens is not the focus the menu gives back.
                onCloseAutoFocus={(event) => {
                  const action = menuAction.current;
                  if (!action) return;
                  menuAction.current = null;
                  event.preventDefault();
                  menuTrigger.current?.focus();
                  action();
                }}
              >
                <DropdownMenuItem
                  disabled={sessionTrashed}
                  onSelect={() => {
                    menuAction.current = () => setRenaming(true);
                  }}
                >
                  <Pencil aria-hidden className="h-3.5 w-3.5 shrink-0" />
                  {t("assistant.rename")}
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={sessionTrashed}
                  onSelect={() =>
                    void assistant.setPinned(
                      session.id,
                      session.pinnedAt === null,
                    )
                  }
                >
                  {session.pinnedAt === null ? (
                    <Pin aria-hidden className="h-3.5 w-3.5 shrink-0" />
                  ) : (
                    <PinOff aria-hidden className="h-3.5 w-3.5 shrink-0" />
                  )}
                  {t(
                    session.pinnedAt === null
                      ? "assistant.pin"
                      : "assistant.unpin",
                  )}
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={busy || Boolean(run) || assistant.loading}
                  onSelect={() => {
                    menuAction.current = exportConversation;
                  }}
                >
                  <Download aria-hidden className="h-3.5 w-3.5 shrink-0" />
                  {t("assistant.exportMarkdown")}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                {usage && (
                  <>
                    <DropdownMenuLabel
                      id={usageLabelId}
                      className="whitespace-normal px-2 py-1 text-note font-normal normal-case tracking-normal text-ink-muted"
                    >
                      {t("assistant.conversationUsageLabel", { usage })}
                    </DropdownMenuLabel>
                    <DropdownMenuSeparator />
                  </>
                )}
                {sessionTrashed ? (
                  <DropdownMenuItem
                    disabled={busy || Boolean(run)}
                    className="text-danger"
                    onSelect={() => {
                      menuAction.current = askToDelete;
                    }}
                  >
                    <Trash2 aria-hidden className="h-3.5 w-3.5 shrink-0" />
                    {t("assistant.deletePermanently")}
                  </DropdownMenuItem>
                ) : (
                  <DropdownMenuItem
                    disabled={busy}
                    className="text-danger"
                    onSelect={() => {
                      menuAction.current = moveToTrash;
                    }}
                  >
                    <Trash2 aria-hidden className="h-3.5 w-3.5 shrink-0" />
                    {t("assistant.moveToTrash")}
                  </DropdownMenuItem>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </header>
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
        {/* While a delete dialog is open it shows the failure itself: a
            banner behind it would be announced a second time. */}
        {assistant.error !== null && !deleteDialogOpen && !emptyTrash.open && (
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
        <div className="relative flex min-h-0 flex-1 flex-col">
          <div
            ref={scroll}
            onScroll={trackScroll}
            onWheel={endJump}
            onTouchMove={endJump}
            onPointerDown={endJump}
            onKeyDown={endJump}
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
                {emptyTranscript && (
                  <div className="mx-auto flex min-h-full max-w-lg flex-col items-center justify-center py-8 text-center">
                    <h2 className="text-title font-semibold">
                      {t(trashed ? "assistant.trash" : "assistant.welcome")}
                    </h2>
                    {/* A trashed conversation says so below, where its composer
                        would be. */}
                    {trashed && !session && (
                      <p className="mt-2 max-w-sm text-ctl leading-relaxed text-ink-muted">
                        {t("assistant.trashSelect")}
                      </p>
                    )}
                    {writable && !assistant.draft.trim() && (
                      <div className="mt-5 flex flex-wrap justify-center gap-2">
                        {STARTERS.map((id) => (
                          <Button
                            key={id}
                            variant="pill"
                            className="h-8 px-3.5 text-ctl"
                            onClick={() => {
                              focusStarter.current = true;
                              assistant.setDraft(
                                t(`assistant.starters.${id}.prompt`),
                              );
                            }}
                          >
                            {t(`assistant.starters.${id}.label`)}
                          </Button>
                        ))}
                      </div>
                    )}
                    {!trashed && (
                      <p className="mt-4 text-balance text-note text-ink-muted">
                        {t("assistant.emptyBoundary")}
                      </p>
                    )}
                    {!trashed && keyMissing && (
                      <p className="mt-2 text-note text-ink-muted">
                        {t("assistant.keyMissingHint")}
                      </p>
                    )}
                  </div>
                )}
                <div className="mx-auto max-w-[760px] py-2">
                  {messages.map((message) => (
                    <AssistantMessageView
                      key={message.id}
                      message={message}
                      searchUnavailable={assistant.searchSkipped.has(
                        message.id,
                      )}
                      latest={message.id === latestReplyId}
                      highlight={transcriptHighlight}
                      disabled={!writable}
                      onEdit={editMessage}
                      onEditingChange={trackEdit}
                      onRegenerate={regenerateMessage}
                      onRetry={retryable(message) ? retryMessage : undefined}
                    />
                  ))}
                  {assistant.stream?.role === "assistant" && (
                    <AssistantMessageView
                      message={assistant.stream}
                      activity={assistant.activity}
                      thinking={assistant.thinking}
                      searchUnavailable={assistant.searchSkipped.has(
                        assistant.stream.id,
                      )}
                      latest={assistant.stream.id === latestReplyId}
                      disabled
                      onEdit={noEdit}
                      onRegenerate={noRegenerate}
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
          {awayFromEnd && !assistant.loading && (
            <Button
              variant="secondary"
              size="icon"
              // A mid-grey ring: the card's own border is too faint to see
              // against the transcript in the dark theme.
              className="absolute bottom-3 left-1/2 h-8 w-8 -translate-x-1/2 rounded-full border-ink-muted text-ink shadow-md"
              aria-label={t(
                unseenBelow
                  ? "assistant.scrollToEndNew"
                  : "assistant.scrollToEnd",
              )}
              title={t("assistant.scrollToEnd")}
              onClick={(event) =>
                jumpToEnd(document.activeElement === event.currentTarget)
              }
            >
              <ArrowDown aria-hidden className="h-4 w-4" />
              {unseenBelow && (
                <span
                  aria-hidden
                  data-testid="scroll-to-end-dot"
                  className="absolute right-0 top-0 h-2.5 w-2.5 rounded-full border-2 border-card bg-brand"
                />
              )}
            </Button>
          )}
        </div>
        <footer className="flex max-h-[50%] min-h-0 shrink-0 flex-col bg-card px-4 pb-3 pt-2">
          {session && sessionTrashed ? (
            // Read-only until restored: no composer, and the way back.
            <div
              role="status"
              className="mx-auto flex w-full max-w-[760px] flex-col items-center gap-3 rounded-list border border-line bg-card-2 px-4 py-4"
            >
              <p className="text-center text-ctl text-ink-2">
                {t("assistant.trashHint")}
              </p>
              <Button
                variant="secondary"
                disabled={busy || Boolean(run)}
                onClick={(event) => restore(event.currentTarget)}
              >
                <RotateCcw aria-hidden className="h-3.5 w-3.5" />
                {t("assistant.restore")}
              </Button>
            </div>
          ) : (
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
                    <p
                      id={blockedHint}
                      className="mt-1.5 text-note text-danger"
                    >
                      {t("assistant.attachmentsBlocked")}
                    </p>
                  )}
                  {imagesNeedVision && (
                    <p id={visionHint} className="mt-1.5 text-note text-warn">
                      {t("assistant.imagesNeedVision")}
                    </p>
                  )}
                  {overLength && (
                    <p
                      id={lengthHintId}
                      className="mt-1.5 text-note text-danger"
                    >
                      {t("assistant.lengthExceeded")}
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
                      if (writable && !overLength) send();
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
                    {showSearchNote ? (
                      <Tooltip>
                        {/* The wrapper takes the hover while the switch is off. */}
                        <TooltipTrigger asChild>
                          <span className="inline-flex shrink-0">
                            {webSearchToggle}
                          </span>
                        </TooltipTrigger>
                        <TooltipContent side="top">{searchNote}</TooltipContent>
                      </Tooltip>
                    ) : (
                      webSearchToggle
                    )}
                    {showSearchNote && (
                      <span id={searchNoteId} className="sr-only">
                        {searchNote}
                      </span>
                    )}
                    {/* A reply under way goes on as it began: the choice is
                      for the next message. */}
                    <ThinkingMenu
                      model={composerModel}
                      level={assistant.thinkingLevel}
                      disabled={settingsLocked}
                      onChange={assistant.setThinkingLevel}
                    />
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {showLength && (
                      <span className="text-note tabular-nums text-ink-muted">
                        {t("assistant.lengthHint", { percent: lengthPercent })}
                      </span>
                    )}
                    {run ? (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="secondary"
                            size="icon"
                            className="h-7 w-7 rounded-full"
                            aria-label={t("assistant.stop")}
                            aria-keyshortcuts="Escape"
                            onClick={stop}
                          >
                            <Square
                              aria-hidden
                              className="h-3 w-3 fill-current"
                            />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent
                          side="top"
                          onEscapeKeyDown={stopOnTooltipEscape}
                        >
                          {t("assistant.stopTooltip")}
                        </TooltipContent>
                      </Tooltip>
                    ) : (
                      <Tooltip>
                        {/* The wrapper takes the hover while Send is disabled:
                          a disabled button gets no pointer events. */}
                        <TooltipTrigger asChild>
                          <span className="inline-flex shrink-0">
                            <Button
                              size="icon"
                              className="h-7 w-7 rounded-full"
                              aria-label={t("assistant.send")}
                              aria-describedby={sendHints}
                              disabled={
                                !writable ||
                                blockedFiles ||
                                overLength ||
                                (!assistant.draft.trim() && !files.length)
                              }
                              onClick={send}
                            >
                              <ArrowUp aria-hidden className="h-3.5 w-3.5" />
                            </Button>
                          </span>
                        </TooltipTrigger>
                        <TooltipContent side="top">
                          {t("assistant.sendTooltip")}
                        </TooltipContent>
                      </Tooltip>
                    )}
                  </div>
                </div>
              </div>
              {assistant.webSearchWaits && (
                <p
                  role="status"
                  className="mt-2 shrink-0 text-note text-ink-muted"
                >
                  {t("assistant.webSearchNextMessage")}
                </p>
              )}
            </div>
          )}
        </footer>
      </section>
      <ConfirmDialog
        isOpen={deleteDialogOpen}
        title={t("assistant.deleteTitle")}
        message={t("assistant.deleteDescription")}
        confirmText={t("assistant.deletePermanently")}
        busy={busy}
        // The page's banner is behind the dialog: a failure shows here.
        error={
          assistant.error !== null
            ? t(assistantErrorKey(assistant.error))
            : undefined
        }
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          // Cancelled, the conversation is still open; deleted, it is gone.
          (menuTrigger.current ?? landOnSidebar(trashBack).current)?.focus();
        }}
        onConfirm={() =>
          void assistant.deleteSession().then((deleted) => {
            if (deleted) setConfirmDelete(false);
          })
        }
        onCancel={() => {
          setConfirmDelete(false);
          // The failure was told in the dialog: it does not wait in the banner.
          assistant.setError(null);
        }}
      />
      <ConfirmDialog
        isOpen={emptyTrash.open}
        title={t("assistant.emptyTrashTitle")}
        message={t("assistant.emptyTrashDescription", {
          count: emptyTrash.count,
        })}
        confirmText={t("assistant.emptyTrashAction")}
        busy={busy}
        error={
          assistant.error !== null
            ? t(assistantErrorKey(assistant.error))
            : undefined
        }
        onCloseAutoFocus={emptyTrash.onCloseAutoFocus}
        onConfirm={emptyTrash.confirm}
        onCancel={emptyTrash.cancel}
      />
    </div>
  );
}
