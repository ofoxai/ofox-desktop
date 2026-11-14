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
import { useLoadingFox } from "@/components/ui/fox-loader";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  ArrowDown,
  ArrowUp,
  Download,
  Ellipsis,
  ListPlus,
  MessageSquare,
  PanelLeftClose,
  PanelLeftOpen,
  Pencil,
  Minimize2,
  Mic,
  Pin,
  PinOff,
  RefreshCw,
  Square,
  Trash2,
  RotateCcw,
  Upload,
  X,
} from "lucide-react";
import {
  assistantApi,
  assistantErrorKey,
  type AssistantMessage,
  type AssistantAttachment,
  type AssistantGenerationKind,
} from "@/lib/api/assistant";
import { isMac } from "@/lib/platform";
import { useAssistant } from "./useAssistant";
import {
  LocalApprovalCard,
  LocalChangesLine,
  PermissionControl,
  LocalTaskProgress,
} from "./AssistantLocalConfig";
import { MediaBlock } from "./MediaBlock";
import { VideoBlock } from "./VideoBlock";
import { SkillPicker } from "./SkillPicker";
import { ComposerPlus } from "./ComposerPlus";
import { generationModels } from "./MediaDefaultsFields";
import { GenerationCards } from "./GenerationCards";
import { useReadAloud } from "./useReadAloud";
import { AssistantMessageView } from "./AssistantMessageView";
import { AssistantContextUsage } from "./AssistantContextUsage";
import { AssistantAudioHistory } from "./AssistantAudioHistory";
import { AssistantAudioInput } from "./AssistantAudioInput";
import { AssistantQueue } from "./AssistantQueue";
import { MAX_MESSAGE_BYTES, MAX_QUEUED_MESSAGES } from "./useMessageQueue";
import { AttachmentList } from "./AssistantAttachments";
import { AssistantModelPicker } from "./AssistantModelPicker";
import {
  AssistantSidebar,
  type AssistantSidebarHandle,
} from "./AssistantSidebar";
import { InlineRename } from "./InlineRename";
import { ThinkingMenu } from "./ThinkingMenu";
import { WebSearchMenu, searchPriceNote } from "./WebSearchMenu";
import { useConversationActions } from "./useConversationActions";
import { useFileDrop } from "./useFileDrop";
import { usePendingFocus } from "./usePendingFocus";
import { insertAssistantQuote } from "@/utils/assistantQuote";
import { assistantMarkdown } from "@/utils/assistantMarkdown";
import { searchHighlight } from "@/utils/assistantSearch";
import { isSendable } from "@/utils/assistantAttachments";
import { MAX_PASTED_FILES, pastedFiles } from "@/utils/assistantPaste";
import { assistantRequests } from "@/utils/assistantRequests";
import { AssistantRequestDetails } from "./AssistantRequestDetails";
import { usageSummary } from "@/utils/assistantUsage";
import { turnVersions } from "@/utils/assistantVersions";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Switch } from "@/components/ui/switch";
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
import {
  useOptionalShell,
  useShellRefresh,
} from "@/components/shell/ShellContext";
import { cn } from "@/lib/utils";

interface Starter {
  id: string;
  /** The generation chip it turns on. */
  kind?: AssistantGenerationKind;
  /** Offered only where the runtime can make documents (a deck needs pptx). */
  requires?: "documents";
}

/**
 * What the assistant can do beyond chat, for a model that takes tools: each
 * shown only while the account has a model for it, then a summary.
 */
const SCENARIO_STARTERS: Starter[] = [
  { id: "image", kind: "image" },
  { id: "video", kind: "video" },
  { id: "speech", kind: "speech" },
  { id: "deck", kind: undefined, requires: "documents" },
  { id: "localCheck" },
];
/** For a model without tools, or an account with none of the above. */
const TEXT_STARTERS: Starter[] = [
  { id: "summary" },
  { id: "writing" },
  { id: "explain" },
];
/** A short scenario list is filled up from the text starters. */
const MIN_STARTERS = 3;

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

interface AssistantPageProps {
  enabled?: boolean;
  onLoginRequested?: () => void;
}

/** Signed out: one way in, and nothing loaded or registered. */
export function AssistantLoginRequired({
  onLoginRequested,
}: Pick<AssistantPageProps, "onLoginRequested">) {
  const { t } = useTranslation();
  // Beside the sidebar of a wide window the page has no frame (see AssistantPage).
  const wide = useOptionalShell()?.layout === "wide";
  return (
    <section
      className={cn(
        "grid h-full min-h-0 place-items-center bg-card px-6 py-10 text-center",
        !wide && "rounded-list border border-line",
      )}
    >
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
  const shell = useOptionalShell();
  // A wide window (see ShellLayout): no frame, roomier sizes, a wider
  // column, and the title only where the list does not show it.
  const wide = shell?.layout === "wide";
  const localEntryPending = useRef<number | null>(null);
  const entry = shell?.assistantLocation;
  useEffect(() => {
    if (
      !entry ||
      !assistant.bootstrap ||
      assistant.busy ||
      assistant.loading ||
      localEntryPending.current === entry.request
    )
      return;
    localEntryPending.current = entry.request;
    void assistant
      .startLocalConfig(t("assistant.local.toolPrompt", { tool: entry.tool }))
      .then((ok) => {
        if (ok) shell?.consumeAssistantLocation(entry.request);
      });
  }, [
    entry,
    assistant.bootstrap,
    assistant.busy,
    assistant.loading,
    assistant.startLocalConfig,
    shell,
    t,
  ]);
  const { detail, bootstrap, run, busy } = assistant;
  const [audioDialog, setAudioDialog] =
    useState<Awaited<ReturnType<typeof assistant.prepareAudio>>>(null);
  // A reply is read aloud in one click with the remembered voice.
  const readAloud = useReadAloud({
    bootstrap,
    prepare: assistant.prepareAudio,
  });
  const [renaming, setRenaming] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [backupDialog, setBackupDialog] = useState<{
    trigger: HTMLButtonElement | null;
    owner?: string | null;
    generation?: number;
  } | null>(null);
  const [excludeVideos, setExcludeVideos] = useState(false);
  const backupTrigger = useRef<HTMLButtonElement | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] =
    useState(readSidebarCollapsed);
  const sidebarId = useId();
  const searchNoteId = useId();
  const blockedHint = useId();
  const visionHint = useId();
  const lengthHintId = useId();
  const queueHintId = useId();
  const interactionActive = useInteractionActive();
  useEffect(() => {
    if (!interactionActive) setAudioDialog(null);
  }, [interactionActive]);
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
  const [requestScope, setRequestScope] = useState<string | null>(null);
  const [audioHistoryScope, setAudioHistoryScope] = useState<string | null>(
    null,
  );
  const requestScopeKey = `${bootstrap?.owner}:${bootstrap?.authorityGeneration}:${detail?.session.id}`;
  const audioHistoryKey = `${requestScopeKey}:${assistant.selectionKey}`;
  const requests =
    requestScope === requestScopeKey ? assistantRequests(detail) : [];
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
  // Ofox search runs only while the catalog prices it; Exa's is free, and
  // offered by a backend that reports the account's provider.
  const searchPrice = bootstrap?.search?.available
    ? bootstrap.search.unitPriceUsd
    : null;
  const exaOffered = bootstrap?.search?.provider !== undefined;
  const searchViaExa = exaOffered && assistant.searchProvider === "exa";
  // What the account's search costs, or why there is none.
  const searchNote = !bootstrap
    ? null
    : searchViaExa
      ? t("assistant.webSearchMenu.exaNote")
      : searchPrice !== null
        ? t("assistant.webSearchPrice", searchPriceNote(searchPrice))
        : exaOffered
          ? t("assistant.webSearchMenu.ofoxUnavailableNote")
          : t("assistant.webSearchUnavailable");
  const showSearchNote = available && searchNote !== null;
  // Under a reply whose free search was refused: the way to Ofox search,
  // while there is one.
  const { setSearchProvider } = assistant;
  const pickOfox = useCallback(
    () => void setSearchProvider("ofox"),
    [setSearchProvider],
  );
  const switchToOfoxSearch =
    searchPrice !== null && assistant.searchProvider !== "ofox"
      ? pickOfox
      : undefined;
  // A conversation still loading (or whose load failed) has no setting to
  // change yet; none is changed in the trash.
  const settingsLocked =
    !available ||
    trashed ||
    busy ||
    assistant.loading ||
    Boolean(assistant.selectedId && session?.id !== assistant.selectedId);
  // Tool, system and other internal records stay out of the transcript, and
  // so do the versions of a turn that are not the one shown: the switcher
  // under the shown one brings them back.
  const detailMessages = detail?.messages;
  const turns = useMemo(
    () =>
      (detailMessages ?? []).filter(
        (message) =>
          message.role === "user" ||
          message.role === "assistant" ||
          message.role === "media",
      ),
    [detailMessages],
  );
  const versions = useMemo(() => turnVersions(turns), [turns]);
  const messages = turns.filter((message) => !message.superseded);
  const emptyTranscript = !messages.length && !assistant.stream;
  // The newest reply keeps its actions in view; only a reply has them there.
  const newest =
    assistant.stream?.role === "assistant"
      ? assistant.stream
      : messages[messages.length - 1];
  const latestReplyId =
    newest && ["assistant", "media"].includes(newest.role)
      ? newest.id
      : undefined;
  // A failed or stopped reply can be asked for again when its question is
  // still there to send: with its words, or with files that can be read.
  const retryable = (message: (typeof messages)[number]) => {
    if (
      message.role !== "assistant" ||
      !message.runId ||
      (message.status !== "error" && message.status !== "stopped")
    )
      return false;
    const question = messages.find(
      (item) => item.role === "user" && item.runId === message.runId,
    );
    return Boolean(
      question &&
        (question.content.trim() || question.attachments?.some(isSendable)),
    );
  };
  const files = assistant.attachments;
  // Files whose last read failed outright are never sent: they must go first.
  const blockedFiles = files.some((file) => !isSendable(file));
  // A model the catalog says reads only text would refuse the turn (free of
  // charge); say so before it is sent.
  const composerModel = bootstrap?.models.find(
    (item) => item.id === (session?.modelId ?? assistant.newModelId),
  );
  // A conversation that generated directly with an image or video model
  // (before generation became the chat model's tools) opens on a chat model,
  // with that model as its generation default.
  const migrated = useRef<string | null>(null);
  useEffect(() => {
    const kind = composerModel?.kind;
    if (!session || !composerModel || (kind !== "image" && kind !== "video"))
      return;
    if (migrated.current === session.id || run) return;
    const chat = [bootstrap?.rememberedModels?.chat, bootstrap?.defaultModelId]
      .map((id) => bootstrap?.models.find((model) => model.id === id))
      .find((model) => model && (model.kind ?? "chat") === "chat");
    if (!chat) return;
    migrated.current = session.id;
    void assistant.update({
      modelId: chat.id,
      media: {
        ...session.media,
        [kind]: { ...session.media?.[kind], model: composerModel.id },
      },
    });
  }, [session, composerModel, run, bootstrap, assistant]);
  // Every model a generation tool can use: speech models come with the
  // audio tools.
  const generationCatalog = useMemo(
    () => [...(bootstrap?.models ?? []), ...(bootstrap?.audioModels ?? [])],
    [bootstrap?.models, bootstrap?.audioModels],
  );
  const localDefinition = Boolean(bootstrap?.localConfigDefinition);
  const deckAvailable = Boolean(
    bootstrap?.runtime.documents?.formats.includes("pptx"),
  );
  const starters = useMemo(() => {
    const offered = (starter: Starter) =>
      starter.kind
        ? generationModels(generationCatalog, starter.kind).length > 0
        : starter.requires === "documents"
          ? deckAvailable
          : localDefinition;
    const scenario =
      composerModel?.tools === false ? [] : SCENARIO_STARTERS.filter(offered);
    if (scenario.length === 0) return TEXT_STARTERS;
    const offeredStarters: Starter[] = [...scenario, { id: "summary" }];
    for (const starter of TEXT_STARTERS)
      if (
        offeredStarters.length < MIN_STARTERS &&
        !offeredStarters.some(({ id }) => id === starter.id)
      )
        offeredStarters.push(starter);
    return offeredStarters;
  }, [composerModel?.tools, generationCatalog, localDefinition, deckAvailable]);
  const queueCount = assistant.queue?.messages.length ?? 0;
  const queueMode = !session?.audioOnly && Boolean(run || queueCount);
  const queueFull = queueCount >= MAX_QUEUED_MESSAGES;
  const canQueue = assistant.queueAvailable && !files.length && !queueFull;
  const imagesNeedVision =
    composerModel?.supportsImages === false &&
    files.some((file) => file.mimeType.startsWith("image/"));
  // Generation needs a model that takes tools.
  const generationNeedsTools =
    (assistant.media.hints?.length ?? 0) > 0 && composerModel?.tools === false;
  // The API's reference limits, shown on the files before anything is sent.
  const referenceOverflow =
    assistant.media.hints?.includes("video") &&
    (files.filter((file) => file.mimeType.startsWith("image/")).length > 9 ||
      files.filter((file) => file.mimeType.startsWith("audio/")).length > 3 ||
      files.filter((file) => file.mimeType.startsWith("video/")).length > 1);
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
      queueMode && (queueFull || files.length > 0) && queueHintId,
    ]
      .filter(Boolean)
      .join(" ") || undefined;
  const focusComposer = useCallback(() => composer.current?.focus(), []);
  // A drop leaves focus where it was; only a focus the import's disabled
  // buttons dropped (on the page itself) goes to the message.
  const focusComposerIfLost = () => {
    const active = document.activeElement;
    if (!active || active === document.body || active === workspace.current)
      focusComposer();
  };
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

  // Reload the models, the list and the open conversation: the ⋯ menu's
  // 刷新, and the shell's refresh while this page is open.
  const reloadAll = async () => {
    await assistant.loadBootstrap(true);
    await assistant.refreshSessions();
    if (assistant.selectedId)
      await assistant.refreshDetail(assistant.selectedId);
  };
  useShellRefresh("assistant", reloadAll);

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
  }, [
    assistant.stream,
    messages.length,
    assistant.localApprovals.length,
    assistant.localPlan,
    assistant.localIntent,
  ]);
  useLayoutEffect(() => {
    const input = composer.current;
    if (!input) return;
    const [min, max] = wide ? [96, 200] : [76, 160];
    const resize = () => {
      // Clear the previous height so deleting text can shrink the field.
      input.style.height = "auto";
      input.style.height = `${Math.max(min, Math.min(max, input.scrollHeight))}px`;
    };
    resize();
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, [assistant.draft, assistant.selectedId, sidebarCollapsed, trashed, wide]);
  // Switching between the narrow and the wide page reflows the transcript
  // (other sizes, another width). One that followed its end stays there;
  // otherwise the message at its top stays where it was. Read while
  // rendering the switch, before the new sizes apply.
  const reflow = useRef({ wide, id: null as string | null, offset: 0 });
  if (reflow.current.wide !== wide) {
    const view = scroll.current;
    const top = view?.getBoundingClientRect().top ?? 0;
    const first = follow.current
      ? undefined
      : [
          ...(view?.querySelectorAll<HTMLElement>("[data-message-id]") ?? []),
        ].find((message) => message.getBoundingClientRect().bottom > top);
    reflow.current = {
      wide,
      id: first?.dataset.messageId ?? null,
      offset: first ? first.getBoundingClientRect().top - top : 0,
    };
  }
  const reflowed = useRef(wide);
  useLayoutEffect(() => {
    if (reflowed.current === wide) return;
    reflowed.current = wide;
    const view = scroll.current;
    if (!view) return;
    if (follow.current) {
      view.scrollTop = view.scrollHeight;
      return;
    }
    const { id, offset } = reflow.current;
    const message = id
      ? view.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(id)}"]`)
      : null;
    if (message)
      view.scrollTop +=
        message.getBoundingClientRect().top -
        view.getBoundingClientRect().top -
        offset;
  }, [wide]);
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
    if (queueMode) {
      if (assistant.enqueue()) focusComposer();
      return;
    }
    follow.current = true;
    void assistant.send().then((sent) => {
      if (sent) composer.current?.focus();
    });
  };
  const videoBlock = (message: AssistantMessage, submitting = false) =>
    session && bootstrap?.owner ? (
      <VideoBlock
        message={message}
        sessionId={session.id}
        owner={bootstrap.owner}
        generation={bootstrap.authorityGeneration}
        disabled={!available || assistant.loading || trashed}
        onSave={(file) =>
          assistantApi.saveMedia({
            sessionId: session.id,
            attachmentId: file.id,
            expectedOwner: bootstrap.owner!,
            expectedGeneration: bootstrap.authorityGeneration,
          })
        }
        onCancel={() =>
          submitting
            ? assistant.stop()
            : assistantApi.cancelVideo({
                sessionId: session.id,
                messageId: message.id,
                expectedOwner: bootstrap.owner!,
                expectedGeneration: bootstrap.authorityGeneration,
              })
        }
        onResume={() =>
          assistantApi.resumeVideo({
            sessionId: session.id,
            messageId: message.id,
            expectedOwner: bootstrap.owner!,
            expectedGeneration: bootstrap.authorityGeneration,
          })
        }
      />
    ) : undefined;
  const generationCards = (message: AssistantMessage, live = false) =>
    session && bootstrap?.owner && message.generations?.length ? (
      <GenerationCards
        message={message}
        sessionId={session.id}
        owner={bootstrap.owner}
        generation={bootstrap.authorityGeneration}
        models={generationCatalog}
        disabled={!available || assistant.loading || trashed}
        onSave={(file) =>
          assistantApi.saveMedia({
            sessionId: session.id,
            attachmentId: file.id,
            expectedOwner: bootstrap.owner!,
            expectedGeneration: bootstrap.authorityGeneration,
          })
        }
        onSaveAndOpen={(file) =>
          assistantApi.saveMedia({
            sessionId: session.id,
            attachmentId: file.id,
            expectedOwner: bootstrap.owner!,
            expectedGeneration: bootstrap.authorityGeneration,
            open: true,
          })
        }
        onCancelVideo={(item) =>
          item.status === "submitting"
            ? assistant.stop()
            : assistant.generationVideo("cancel", message, item)
        }
        onResumeVideo={(item) =>
          assistant.generationVideo("resume", message, item)
        }
        onRegenerate={
          live || !writable
            ? undefined
            : (item, input) => assistant.regenerateMedia(message, item, input)
        }
        sourceFile={(id) =>
          messages
            .flatMap((item) => item.attachments ?? [])
            .find((file) => file.id === id)
        }
      />
    ) : undefined;
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
  const actions = useRef({
    send: assistant.send,
    update: assistant.update,
    switchVersion: assistant.switchVersion,
    forkSession: assistant.forkSession,
    setDraft: assistant.setDraft,
    draft: assistant.draft,
    title: session?.title,
    messages,
    modelId: session?.modelId,
  });
  actions.current = {
    send: assistant.send,
    update: assistant.update,
    switchVersion: assistant.switchVersion,
    forkSession: assistant.forkSession,
    setDraft: assistant.setDraft,
    draft: assistant.draft,
    title: session?.title,
    messages,
    modelId: session?.modelId,
  };
  const editMessage = useCallback(
    (
      editMessageId: string,
      text: string,
      attachments: AssistantAttachment[],
      skill?: string,
    ) =>
      actions.current.send({
        text,
        attachments,
        editMessageId,
        skill,
        replaceSkill: true,
      }),
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
  // Asks again for a failed or stopped reply with its question's own words
  // and files; the backend decides whether that is an edit of the question
  // the model kept or a new turn. With `modelId` the conversation changes
  // model first (the model it asks is the conversation's). Settles once the
  // send is over: true when it went out.
  const retryReply = useCallback(async (replyId: string, modelId?: string) => {
    const { messages: shown, modelId: current } = actions.current;
    const reply = shown.find((item) => item.id === replyId);
    const question = shown.find(
      (item) => item.role === "user" && item.runId === reply?.runId,
    );
    if (!reply || !question) return false;
    if (
      modelId &&
      modelId !== current &&
      !(await actions.current.update({ modelId }))
    )
      return false;
    follow.current = true;
    // `send` is read only now. It takes the conversation's model from the
    // backend's record, so a render that has not happened yet changes nothing.
    return actions.current.send({
      text: question.content,
      attachments: question.attachments,
      retryMessageId: replyId,
    });
  }, []);
  const switchVersion = useCallback(
    (messageId: string) => void actions.current.switchVersion(messageId),
    [],
  );
  const forkMessage = useCallback(
    (messageId: string) => {
      const title = Array.from(
        actions.current.title || t("assistant.newConversation"),
      );
      const base =
        title.slice(0, 100).join("") + (title.length > 100 ? "…" : "");
      void actions.current.forkSession(
        messageId,
        t("assistant.forkTitle", { title: base }),
      );
    },
    [t],
  );
  const quoteText = useCallback((text: string) => {
    const input = composer.current;
    if (!input || input.disabled) return;
    const next = insertAssistantQuote(
      actions.current.draft,
      input.selectionStart,
      input.selectionEnd,
      text,
    );
    actions.current.setDraft(next.text);
    requestAnimationFrame(() => {
      input.focus();
      input.setSelectionRange(next.cursor, next.cursor);
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
  // Files dropped on the window join the draft only where Attach would work,
  // and only while this page is the one on screen.
  const { dragging } = useFileDrop({
    ready: interactionActive && writable,
    onDrop: (token) =>
      void assistant.dropAttachments(token).then(focusComposerIfLost),
    onError: assistant.showError,
  });
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

  const webSearchMenu = (
    <WebSearchMenu
      on={assistant.webSearch}
      provider={assistant.searchProvider}
      ofoxPrice={searchPrice}
      exaOffered={exaOffered}
      disabled={settingsLocked}
      describedBy={showSearchNote ? searchNoteId : undefined}
      onChange={(choice) => void assistant.chooseWebSearch(choice)}
    />
  );

  const column = wide
    ? "w-[clamp(840px,72%,1040px)] max-w-full"
    : "w-full max-w-[760px]";
  // A new conversation of a wide window starts with its composer in the
  // middle of the page, the starters under it.
  const centred = wide && emptyTranscript && !assistant.loading && !trashed;
  const drag = wide && isMac() ? { "data-tauri-drag-region": true } : {};
  const titleShown = !wide || sidebarCollapsed;
  const modelPicker = Boolean(session || bootstrap?.models.length) && (
    <AssistantModelPicker
      models={(bootstrap?.models ?? []).filter(
        (model) => (model.kind ?? "chat") === "chat",
      )}
      rememberedModels={bootstrap?.rememberedModels}
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
  );
  const starterButtons = writable && !assistant.draft.trim() && (
    <div
      className={cn(
        "flex flex-wrap justify-center gap-2",
        centred ? "mt-4" : "mt-5",
      )}
    >
      {starters.map(({ id, kind }) => (
        <Button
          key={id}
          variant="pill"
          className="h-8 px-3.5 text-ctl"
          onClick={() => {
            focusStarter.current = true;
            const hints = assistant.media.hints ?? [];
            if (kind && !hints.includes(kind))
              assistant.setMedia({
                ...assistant.media,
                hints: [...hints, kind],
              });
            assistant.setDraft(t(`assistant.starters.${id}.prompt`));
          }}
        >
          {t(`assistant.starters.${id}.label`)}
        </Button>
      ))}
    </div>
  );

  const opening = useLoadingFox(assistant.loading, {
    size: "block",
    label: t("assistant.loading"),
  });
  return (
    <div
      ref={workspace}
      tabIndex={-1}
      data-layout={wide ? "wide" : "narrow"}
      className={cn(
        "flex h-full min-h-0 overflow-hidden bg-card outline-none",
        wide
          ? "[--ctl-h:30px] [--fs-ctl:13.5px] [--fs-micro:11.5px] [--fs-nav:14px] [--fs-note:12px] [--fs-sub:12.5px] [--fs-title:17px] [--fs-ui:15px] [--icon-btn:30px]"
          : "rounded-list border border-line",
      )}
      onKeyDown={handleShortcuts}
    >
      <AssistantSidebar
        ref={sidebarHandle}
        id={sidebarId}
        wide={wide}
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
        onTransfer={(kind, trigger) => {
          if (kind === "backup") {
            setExcludeVideos(false);
            backupTrigger.current = trigger;
            setBackupDialog({
              trigger,
              owner: bootstrap?.owner,
              generation: bootstrap?.authorityGeneration,
            });
          } else {
            void assistant.transfer(kind).then(() => returnFocusTo(trigger));
          }
        }}
      />
      <section
        className="flex min-h-0 min-w-0 flex-1 flex-col"
        aria-label={t("assistant.chat")}
      >
        <header
          // The top of a wide window, the top bar gone, drags it.
          {...drag}
          className={cn(
            "flex shrink-0 items-center gap-3 px-4 py-2.5",
            !wide && "border-b border-line",
          )}
        >
          <div
            className={cn(
              "flex min-w-0 items-center gap-1",
              wide ? "shrink" : "flex-1",
            )}
          >
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
                // Kept for a screen reader where the list already shows it.
                className={cn(
                  "min-w-0 truncate text-ui font-semibold",
                  !titleShown && "sr-only",
                )}
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
          {modelPicker}
          {/* The empty rest of a wide top row: it drags the window, since
              Tauri drags only from the element pressed. */}
          {wide && (
            <div aria-hidden className="flex-1 self-stretch" {...drag} />
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
                  onSelect={() =>
                    void reloadAll().catch((e) =>
                      console.error("[Assistant] refresh failed", e),
                    )
                  }
                >
                  <RefreshCw aria-hidden className="h-3.5 w-3.5 shrink-0" />
                  {t("shell.refresh")}
                </DropdownMenuItem>
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
                <DropdownMenuItem
                  disabled={
                    busy ||
                    Boolean(run) ||
                    assistant.loading ||
                    sessionTrashed ||
                    !messages.length
                  }
                  onSelect={() => {
                    menuAction.current = () => {
                      void assistant.compact();
                    };
                  }}
                >
                  <Minimize2 aria-hidden className="h-3.5 w-3.5 shrink-0" />
                  <div className="min-w-0">
                    <span>{t("assistant.compact")}</span>
                    <span className="mt-0.5 block max-w-64 whitespace-normal text-note text-ink-muted">
                      {t("assistant.compactCost")}
                    </span>
                  </div>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={() => {
                    menuAction.current = () => setRequestScope(requestScopeKey);
                  }}
                >
                  {t("assistant.requestRecords")}
                </DropdownMenuItem>
                {(detail?.audioOperations?.length ?? 0) > 0 && (
                  <DropdownMenuItem
                    disabled={Boolean(
                      run ||
                        busy ||
                        assistant.loading ||
                        trashed ||
                        queueCount > 0 ||
                        detail?.session.id !== assistant.selectedId,
                    )}
                    onSelect={() => {
                      menuAction.current = () =>
                        setAudioHistoryScope(audioHistoryKey);
                    }}
                  >
                    {t("assistant.audioHistoryTitle")}
                  </DropdownMenuItem>
                )}
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
        {bootstrap &&
          (bootstrap.catalogRefreshing ||
            bootstrap.catalogRefreshFailed?.length ||
            bootstrap.modelUnavailableReason) && (
            <div
              role="status"
              className="flex items-center justify-between gap-3 border-b border-line px-4 py-2 text-note text-ink-muted"
            >
              {bootstrap.catalogRefreshing ? (
                <span>
                  {t(
                    bootstrap.models.length
                      ? "assistant.catalog.refreshing"
                      : "assistant.catalog.loading",
                  )}
                </span>
              ) : bootstrap.models.length > 0 ? (
                <span>{t("assistant.catalog.failed")}</span>
              ) : (
                <span />
              )}
              <Button
                variant="ghost"
                disabled={bootstrap.catalogRefreshing}
                onClick={() => void assistant.loadBootstrap(true)}
              >
                {t("assistant.catalog.retry")}
              </Button>
            </div>
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
              onClick={() => void assistant.loadBootstrap(true)}
            >
              {t("common.retry")}
            </Button>
          </div>
        )}
        {assistant.localModelNotice && (
          <p
            role="status"
            className="border-b border-line px-4 py-2 text-note text-ink-muted"
          >
            {t(`assistant.local.${assistant.localModelNotice}`)}
          </p>
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
              {assistant.errorDetail && (
                <span className="mt-1 block max-h-24 overflow-y-auto whitespace-pre-wrap text-note">
                  {assistant.errorDetail}
                </span>
              )}
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
        {/* The drop overlay covers what the files would be added to: the
            transcript and the composer. */}
        <div
          data-centred={centred ? "true" : "false"}
          className={cn(
            "relative flex min-h-0 flex-1 flex-col",
            centred && "justify-center pb-[12vh]",
          )}
        >
          <div
            className={cn(
              "relative flex min-h-0 flex-col",
              centred ? "flex-none" : "flex-1",
            )}
          >
            <div
              ref={scroll}
              onScroll={trackScroll}
              onWheel={endJump}
              onTouchMove={endJump}
              onPointerDown={endJump}
              onKeyDown={endJump}
              className={cn(
                "min-h-0 overflow-y-auto overscroll-contain px-4",
                centred ? "flex-none" : "flex-1",
              )}
              aria-busy={assistant.loading}
            >
              {opening.loading ? (
                opening.fox
              ) : (
                <>
                  {emptyTranscript && (
                    <div
                      className={cn(
                        "mx-auto flex max-w-lg flex-col items-center justify-center text-center",
                        centred ? "pb-2" : "min-h-full py-8",
                      )}
                    >
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
                      {!centred && starterButtons}
                      {!trashed && (
                        <p className="mt-4 text-balance text-note text-ink-muted">
                          {t(
                            assistant.localConfig
                              ? "assistant.local.boundary"
                              : "assistant.emptyBoundary",
                          )}
                        </p>
                      )}
                      {!trashed && keyMissing && (
                        <p className="mt-2 text-note text-ink-muted">
                          {t("assistant.keyMissingHint")}
                        </p>
                      )}
                    </div>
                  )}
                  <div
                    className={cn(
                      "mx-auto py-2",
                      wide ? column : "max-w-[760px]",
                    )}
                  >
                    {messages.map((message) => {
                      const turn = versions.get(message.id);
                      return (
                        <AssistantMessageView
                          onEnableLocal={
                            !assistant.localConfig &&
                            !settingsLocked &&
                            !assistant.run
                              ? () => void assistant.setLocalConfig(true)
                              : undefined
                          }
                          // The message that carries a switcher keeps its place
                          // when another version is shown, and the keyboard
                          // focus on the switcher with it.
                          key={turn ? `versions:${turn.group}` : message.id}
                          message={message}
                          immutable={Boolean(
                            message.runId &&
                              messages.some(
                                (item) =>
                                  item.runId === message.runId && item.media,
                              ),
                          )}
                          mediaBlock={
                            message.media?.kind === "video" ? (
                              videoBlock(message)
                            ) : message.media && session && bootstrap?.owner ? (
                              <MediaBlock
                                message={message}
                                sessionId={session.id}
                                owner={bootstrap.owner}
                                generation={bootstrap.authorityGeneration}
                                disabled={!writable}
                                onSave={(file) =>
                                  assistantApi.saveMedia({
                                    sessionId: session.id,
                                    attachmentId: file.id,
                                    expectedOwner: bootstrap.owner!,
                                    expectedGeneration:
                                      bootstrap.authorityGeneration,
                                  })
                                }
                              />
                            ) : undefined
                          }
                          generations={generationCards(message)}
                          models={bootstrap?.models}
                          currentModelId={session?.modelId}
                          versions={turn}
                          searchUnavailable={assistant.searchSkipped.has(
                            message.id,
                          )}
                          searchLimited={assistant.searchLimited.get(
                            message.id,
                          )}
                          onUseOfoxSearch={switchToOfoxSearch}
                          latest={message.id === latestReplyId}
                          highlight={transcriptHighlight}
                          disabled={!writable}
                          onEdit={editMessage}
                          onEditingChange={trackEdit}
                          onRegenerate={regenerateMessage}
                          onRetry={retryable(message) ? retryReply : undefined}
                          onSwitchVersion={switchVersion}
                          onFork={forkMessage}
                          onSpeak={
                            readAloud.available
                              ? (messageId, text) =>
                                  void readAloud.speak(messageId, text)
                              : undefined
                          }
                          onQuote={
                            !assistant.busy && !assistant.showTrash
                              ? quoteText
                              : undefined
                          }
                        />
                      );
                    })}
                    {assistant.stream &&
                      ["assistant", "media"].includes(
                        assistant.stream.role,
                      ) && (
                        <AssistantMessageView
                          onEnableLocal={
                            !assistant.localConfig &&
                            !settingsLocked &&
                            !assistant.run
                              ? () => void assistant.setLocalConfig(true)
                              : undefined
                          }
                          message={assistant.stream}
                          taskProgress={
                            <LocalTaskProgress
                              intent={assistant.localIntent}
                              warning={assistant.localWarning}
                              plan={assistant.localPlan}
                            />
                          }
                          mediaBlock={
                            assistant.stream.media?.kind === "video" ? (
                              videoBlock(assistant.stream, true)
                            ) : assistant.stream.media &&
                              session &&
                              bootstrap?.owner ? (
                              <MediaBlock
                                message={assistant.stream}
                                sessionId={session.id}
                                owner={bootstrap.owner}
                                generation={bootstrap.authorityGeneration}
                                disabled
                                onSave={async () => false}
                              />
                            ) : undefined
                          }
                          generations={generationCards(assistant.stream, true)}
                          retryAttempt={assistant.retryAttempt}
                          activity={assistant.activity}
                          thinking={assistant.thinking}
                          searchUnavailable={assistant.searchSkipped.has(
                            assistant.stream.id,
                          )}
                          searchLimited={assistant.searchLimited.get(
                            assistant.stream.id,
                          )}
                          onUseOfoxSearch={switchToOfoxSearch}
                          latest={assistant.stream.id === latestReplyId}
                          disabled
                          onEdit={noEdit}
                          onRegenerate={noRegenerate}
                        />
                      )}
                    {assistant.localApprovals.map((approval) => (
                      <LocalApprovalCard
                        key={approval.approvalId}
                        approval={approval}
                        onRespond={assistant.respondLocalApproval}
                      />
                    ))}
                    {assistant.localConfig &&
                      !assistant.stream &&
                      messages.length > 0 && (
                        <LocalChangesLine
                          key={session?.id ?? "new"}
                          changes={assistant.localChanges}
                          disabled={settingsLocked || !!assistant.run}
                          onRestore={assistant.restoreLocalChanges}
                        />
                      )}
                    {assistant.compacting && (
                      <p
                        role="status"
                        className="py-2 text-note text-ink-muted"
                      >
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
                className={cn(
                  "mx-auto flex flex-col items-center gap-3 rounded-list border border-line bg-card-2 px-4 py-4",
                  column,
                )}
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
              <div className={cn("mx-auto flex min-h-0 flex-col", column)}>
                <div className="flex min-h-0 flex-col rounded-list border border-line bg-card shadow-[0_1px_3px_rgba(0,0,0,.04)] transition-[border-color,box-shadow] focus-within:border-brand/60 focus-within:ring-2 focus-within:ring-brand/10">
                  {queueCount > 0 && assistant.queue && (
                    <AssistantQueue
                      key={assistant.selectedId}
                      queue={assistant.queue}
                      canResume={assistant.queueAvailable && !run}
                      onPause={assistant.pauseQueue}
                      onResume={() => {
                        // Move focus while the user is still here, never in a
                        // delayed completion that could follow navigation.
                        focusComposer();
                        void assistant.resumeQueue();
                      }}
                      onClear={assistant.clearQueue}
                      onEdit={assistant.editQueued}
                      onRemove={assistant.removeQueued}
                      focusComposer={focusComposer}
                    />
                  )}
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
                    {queueMode && (queueFull || files.length > 0) && (
                      <p
                        id={queueHintId}
                        className="mt-1.5 text-note text-warn"
                      >
                        {t(
                          queueFull
                            ? "assistant.queue.full"
                            : "assistant.queue.textOnly",
                          { max: MAX_QUEUED_MESSAGES },
                        )}
                      </p>
                    )}
                    {referenceOverflow && (
                      <p className="mt-1.5 text-note text-danger">
                        {t("assistant.generation.overLimit")}
                      </p>
                    )}
                    {generationNeedsTools && (
                      <p className="mt-1.5 text-note text-warn">
                        {t("assistant.generation.needsTools")}
                      </p>
                    )}
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
                  <SkillPicker
                    scope={`${bootstrap?.owner}:${bootstrap?.authorityGeneration}:${assistant.selectedId}`}
                    skill={assistant.skill}
                    onSkill={assistant.setSkill}
                    ref={composer}
                    rows={3}
                    className={cn(
                      "shrink-0 resize-none border-0 bg-transparent px-3 py-3 text-ui focus:ring-0",
                      wide
                        ? "max-h-[200px] min-h-[96px]"
                        : "max-h-40 min-h-[76px]",
                    )}
                    aria-label={t("assistant.message")}
                    placeholder={t("assistant.composerPlaceholder")}
                    value={assistant.draft}
                    disabled={!available}
                    readOnly={busy || trashed}
                    onTextChange={assistant.setDraft}
                    onPaste={(event) => {
                      const pasted = pastedFiles(event.clipboardData);
                      if (!pasted) return;
                      // Files and no words of their own: the field has
                      // nothing to paste.
                      event.preventDefault();
                      if (!writable || queueCount > 0) return;
                      // The skipped files are told once the import is under
                      // way, not for a paste that was refused.
                      void assistant.pasteAttachments(
                        pasted.files,
                        pasted.skipped > 0
                          ? () =>
                              toast.warning(
                                t("assistant.pasteSkipped", {
                                  count: pasted.skipped,
                                  max: MAX_PASTED_FILES,
                                }),
                              )
                          : undefined,
                      );
                    }}
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
                        if ((queueMode ? canQueue : writable) && !overLength)
                          send();
                      }
                    }}
                  />
                  <div className="flex shrink-0 items-center justify-between gap-2 pb-2 pl-2 pr-3">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <ComposerPlus
                        models={generationCatalog}
                        media={assistant.media}
                        onMedia={assistant.setMedia}
                        canAttach={writable && queueCount === 0}
                        onAttach={() =>
                          // The dialog leaves focus on a button disabled while
                          // it was open: the message takes it back.
                          void assistant.pickAttachments().then(focusComposer)
                        }
                        disabled={!available || trashed}
                      />
                      {bootstrap?.audioModels?.some(
                        (model) =>
                          model.kind === "transcription" && model.audio,
                      ) && (
                        <Button
                          variant="icon"
                          aria-label={t("assistant.audio.title")}
                          title={t("assistant.audio.title")}
                          disabled={!writable || queueCount > 0}
                          onClick={() => {
                            void assistant.prepareAudio().then((target) => {
                              if (target) setAudioDialog(target);
                            });
                          }}
                        >
                          <Mic aria-hidden className="h-3.5 w-3.5" />
                        </Button>
                      )}
                      {showSearchNote ? (
                        <Tooltip>
                          {/* The wrapper takes the hover while the menu is
                          locked. */}
                          <TooltipTrigger asChild>
                            <span className="inline-flex shrink-0">
                              {webSearchMenu}
                            </span>
                          </TooltipTrigger>
                          <TooltipContent side="top">
                            {searchNote}
                          </TooltipContent>
                        </Tooltip>
                      ) : (
                        webSearchMenu
                      )}
                      {showSearchNote && (
                        <span id={searchNoteId} className="sr-only">
                          {searchNote}
                        </span>
                      )}
                      {composerModel?.tools !== false && (
                        <PermissionControl
                          key={`${assistant.selectedId ?? "new"}:${bootstrap?.authorityGeneration ?? 0}`}
                          mode={assistant.localMode}
                          disabled={settingsLocked || !!assistant.run}
                          onMode={assistant.setLocalPermission}
                        />
                      )}
                      {/* A reply under way goes on as it began: the choice is
                      for the next message. */}
                      {
                        <ThinkingMenu
                          model={composerModel}
                          level={assistant.thinkingLevel}
                          disabled={settingsLocked}
                          onChange={assistant.setThinkingLevel}
                        />
                      }
                    </div>
                    <div className="flex shrink-0 items-center gap-2">
                      {showLength && (
                        <span className="text-note tabular-nums text-ink-muted">
                          {t("assistant.lengthHint", {
                            percent: lengthPercent,
                          })}
                        </span>
                      )}
                      {
                        <AssistantContextUsage
                          modelId={session?.modelId ?? ""}
                          usage={
                            assistant.compacting
                              ? undefined
                              : session?.contextUsage
                          }
                        />
                      }
                      {run && queueMode && (
                        <Button
                          variant="ghost"
                          className="h-7 px-2 text-note"
                          disabled={
                            assistant.compacting ||
                            !assistant.queueAvailable ||
                            !assistant.draft.trim() ||
                            files.length > 0 ||
                            Boolean(assistant.skill) ||
                            overLength
                          }
                          onClick={() =>
                            void assistant.steer().then((sent) => {
                              if (sent) focusComposer();
                            })
                          }
                        >
                          {t("assistant.queue.interrupt")}
                        </Button>
                      )}
                      {run && assistant.skill && (
                        <p className="max-w-48 text-note text-ink-muted">
                          {t("assistant.skills.queueOnly")}
                        </p>
                      )}
                      {run && (
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
                      )}
                      {(!run || queueMode) && (
                        <Tooltip>
                          {/* The wrapper takes the hover while Send is disabled:
                          a disabled button gets no pointer events. */}
                          <TooltipTrigger asChild>
                            <span className="inline-flex shrink-0">
                              <Button
                                size="icon"
                                className="h-7 w-7 rounded-full"
                                aria-label={t(
                                  queueMode
                                    ? "assistant.queue.add"
                                    : "assistant.send",
                                )}
                                aria-describedby={sendHints}
                                disabled={
                                  !(queueMode ? canQueue : writable) ||
                                  blockedFiles ||
                                  referenceOverflow ||
                                  overLength ||
                                  (!assistant.draft.trim() && !files.length)
                                }
                                onClick={send}
                              >
                                {queueMode ? (
                                  <ListPlus
                                    aria-hidden
                                    className="h-3.5 w-3.5"
                                  />
                                ) : (
                                  <ArrowUp
                                    aria-hidden
                                    className="h-3.5 w-3.5"
                                  />
                                )}
                              </Button>
                            </span>
                          </TooltipTrigger>
                          <TooltipContent side="top">
                            {t(
                              queueMode
                                ? "assistant.queue.addTooltip"
                                : "assistant.sendTooltip",
                            )}
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
                {centred && starterButtons}
              </div>
            )}
          </footer>
          {dragging && (
            <div
              aria-hidden
              data-testid="assistant-drop-overlay"
              className="pointer-events-none absolute inset-0 z-20 grid animate-fade place-items-center rounded-card border-2 border-dashed border-brand bg-card/85"
            >
              <p className="flex items-center gap-2 text-ui font-medium text-brand-ink">
                <Upload aria-hidden className="h-4 w-4" />
                {t("assistant.dropOverlay")}
              </p>
            </div>
          )}
        </div>
      </section>
      {interactionActive &&
        audioDialog &&
        audioDialog.expectedOwner === bootstrap?.owner &&
        audioDialog.expectedGeneration === bootstrap.authorityGeneration &&
        audioDialog.sessionId === assistant.selectedId &&
        audioDialog.selectionKey === assistant.selectionKey &&
        !trashed && (
          <AssistantAudioInput
            key={`${audioDialog.expectedOwner}:${audioDialog.expectedGeneration}:${audioDialog.sessionId}:${audioDialog.selectionKey}`}
            target={audioDialog}
            models={bootstrap.audioModels ?? []}
            rememberedModel={bootstrap.rememberedModels?.transcription}
            history={detail?.audioOperations?.filter(
              (item) => item.kind === "transcription",
            )}
            onInsert={(text) => assistant.insertAudioDraft(audioDialog, text)}
            onClose={() => {
              setAudioDialog(null);
              requestAnimationFrame(focusComposer);
            }}
          />
        )}
      <ConfirmDialog
        isOpen={
          backupDialog !== null &&
          backupDialog.owner === bootstrap?.owner &&
          backupDialog.generation === bootstrap?.authorityGeneration
        }
        title={t("assistant.backupTitle")}
        message={t("assistant.backupHint")}
        confirmText={t("assistant.createBackup")}
        variant="info"
        busy={assistant.transferring}
        details={
          <label className="flex items-center justify-between gap-3">
            <span>{t("assistant.excludeVideos")}</span>
            <Switch
              aria-label={t("assistant.excludeVideos")}
              checked={excludeVideos}
              onCheckedChange={setExcludeVideos}
              disabled={assistant.transferring}
            />
          </label>
        }
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          returnFocusTo(backupTrigger.current);
        }}
        onCancel={() => {
          const trigger = backupDialog?.trigger;
          setBackupDialog(null);
          returnFocusTo(trigger ?? null);
        }}
        onConfirm={() => {
          const trigger = backupDialog?.trigger;
          void assistant.transfer("backup", excludeVideos).then(() => {
            setBackupDialog(null);
            returnFocusTo(trigger ?? null);
          });
        }}
      />
      {interactionActive &&
        audioHistoryScope === audioHistoryKey &&
        bootstrap &&
        detail &&
        detail.session.id === assistant.selectedId &&
        !trashed &&
        !run &&
        !busy &&
        !assistant.loading &&
        queueCount === 0 && (
          <AssistantAudioHistory
            key={audioHistoryKey}
            target={{
              sessionId: detail.session.id,
              expectedOwner: bootstrap.owner,
              expectedGeneration: bootstrap.authorityGeneration,
            }}
            history={detail.audioOperations ?? []}
            attachments={detail.attachments}
            returnFocusRef={menuTrigger}
            onClose={() => setAudioHistoryScope(null)}
          />
        )}
      <AssistantRequestDetails
        requests={requests}
        returnFocusRef={menuTrigger}
        open={requestScope === requestScopeKey}
        onOpenChange={(open) => {
          if (!open) setRequestScope(null);
        }}
      />
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
