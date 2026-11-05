import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  assistantApi,
  assistantErrorCode,
  type AssistantAttachment,
  type AssistantPermissionMode,
  type AssistantApprovalChoice,
  type AssistantLocalChange,
  type AssistantPasteTarget,
  type AssistantImageRequest,
  type AssistantVideoRequest,
  type AssistantBackupReport,
  type AssistantBootstrap,
  type AssistantCatalogSnapshot,
  type AssistantEvent,
  type AssistantMessage,
  type AssistantPreferences,
  type AssistantRunRef,
  type AssistantRestoreReport,
  type AssistantSession,
  type AssistantSessionDetail,
  type AssistantThinkingLevel,
  type AssistantUnreadableSession,
  type AssistantUpdateRequest,
  type AssistantUsage,
  type AssistantGeneration,
  type AssistantMediaDefaults,
  type AssistantRegenerateResult,
} from "@/lib/api/assistant";
import { asDraftFile, isSendable } from "@/utils/assistantAttachments";
import { currentThinking, type ThinkingChoice } from "@/utils/assistantModels";
import { useFinishNotice } from "./useFinishNotice";
import { useInteractionActive } from "@/components/ui/interaction-scope";
import {
  useMessageQueue,
  MAX_MESSAGE_BYTES,
  type QueueTicket,
} from "./useMessageQueue";

function subscriptionReadiness() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
function catalogPatch(catalog: AssistantCatalogSnapshot) {
  return {
    models: catalog.models,
    audioModels: catalog.audioModels,
    defaultModelId: catalog.defaultModelId,
    modelUnavailableReason: catalog.modelUnavailableReason,
    partial: catalog.partial,
    search: catalog.search,
    catalogRevision: catalog.catalogRevision,
    catalogRefreshing: catalog.catalogRefreshing,
    catalogRefreshFailed: catalog.catalogRefreshFailed,
  };
}
type ApprovalEvent = Extract<AssistantEvent, { type: "approval" }>;
type PlanStep = {
  step: string;
  status: "pending" | "in_progress" | "completed";
};
const freshId = () => crypto.randomUUID();
const IDENTITY_CHANGED = "assistant_identity_changed";
/** A backup or restore refused because a reply or a file import is going on. */
const TRANSFER_BUSY = "assistant_transfer_busy";

/** A backend or renderer error code; the page turns it into a localized message. */
export type AssistantError = string;

/** What the last backup or restore did, for the page to tell. */
export type AssistantTransferResult =
  | ({ kind: "backup" } & AssistantBackupReport)
  | ({ kind: "restore" } & AssistantRestoreReport);

/** What a running reply is doing, as the user is told; never a tool's name. */
export type AssistantActivity = "searching" | "readingPage" | "readingFile";

const TOOL_ACTIVITY = new Map<string, AssistantActivity>([
  ["web_search", "searching"],
  ["web_fetch", "readingPage"],
  ["read_attachment", "readingFile"],
]);

/** A tool call of the followed run that has not ended, by its call id. */
interface RunningTool {
  id: string;
  activity: AssistantActivity;
}

/**
 * Web search for a run this window sent: whether its conversation asked for
 * it, and whether the run got it (from its `run_start`; null until then).
 */
interface RunSearch {
  runId: string;
  requested: boolean;
  got: boolean | null;
}

/**
 * How long the followed run's reply has thought: the spans that are over, and
 * the one under way. A span runs from the host's `thinking_start` to its
 * `thinking_end` (or the reply's next text, tool call or end).
 */
export interface AssistantThinking {
  runId: string;
  /** Milliseconds of the spans that are over. */
  doneMs: number;
  /** When the span under way began (epoch ms); null between spans. */
  since: number | null;
}

/** A reply with nothing in it yet, for the run that is writing it. */
const blankReply = (runId: string): AssistantMessage => ({
  id: `stream-${runId}`,
  role: "assistant",
  content: "",
  createdAt: Date.now(),
  status: "streaming",
});

/** The words of the followed run that arrived since the screen last changed. */
interface PendingDeltas {
  runId: string;
  text: string;
  thinking: string;
}

interface DraftWriteScope {
  held: boolean;
}

interface PendingSend extends AssistantRunRef {
  compactOnly?: boolean;
  sessionId: string;
  cancelled: boolean;
}

/** A conversation's draft as last saved; `files` unknown until read or written. */
interface SavedDraft {
  text: string;
  files?: string[];
}

const sameIds = (a: string[], b: string[]) =>
  a.length === b.length && a.every((id, index) => id === b[index]);

/** The owner check the backend applies to writes that outlive a sign-in. */
function expectedIdentity(identity: AssistantBootstrap | null) {
  return identity?.owner && identity.authorityGeneration !== undefined
    ? {
        expectedOwner: identity.owner,
        expectedGeneration: identity.authorityGeneration,
      }
    : {};
}

/** The owner check for a write that must name the account it was made for. */
function requireIdentity(identity: AssistantBootstrap | null) {
  if (!identity?.owner || identity.authorityGeneration === undefined)
    throw new Error(IDENTITY_CHANGED);
  return {
    expectedOwner: identity.owner,
    expectedGeneration: identity.authorityGeneration,
  };
}

/**
 * What a file import brought into a conversation, and the failure that
 * stopped it early (files imported before it are kept).
 */
interface ImportedFiles {
  added: AssistantAttachment[];
  failure?: unknown;
}

type ImportFiles = (sessionId: string) => Promise<ImportedFiles>;

/** All asynchronous replies are fenced by the selected session and component lifetime. */
export function useAssistant(enabled: boolean) {
  const interactionActive = useInteractionActive();
  const activeRef = useRef(interactionActive);
  activeRef.current = interactionActive;
  const { queues, controls: messageQueue } = useMessageQueue();
  useLayoutEffect(() => {
    if (!interactionActive) messageQueue.pauseAll();
  }, [interactionActive, messageQueue]);
  const [bootstrap, setBootstrap] = useState<AssistantBootstrap | null>(null);
  const [chosenModel, setChosenModel] = useState<string | null>(null);
  const newModelId = chosenModel ?? bootstrap?.defaultModelId ?? "";
  // The user picked the model rather than taking the default shown.
  const newModelPicked = chosenModel !== null;
  // Web search for the conversation the first message creates. A new
  // conversation starts with the last choice made on the switch for this
  // account in this window (off until one is made), like the model picker.
  const [newWebSearch, setNewWebSearchState] = useState(false);
  const newWebSearchRef = useRef(newWebSearch);
  const lastWebSearch = useRef(false);
  const [newLocalConfig, setNewLocalConfig] = useState(false);
  const newLocalConfigRef = useRef(false);
  // The generation defaults and chips of the conversation the first message
  // creates; saved on it once it exists.
  const [newMedia, setNewMedia] = useState<AssistantMediaDefaults>({});
  const newMediaRef = useRef(newMedia);
  newMediaRef.current = newMedia;
  const [localModes, setLocalModes] = useState<
    Record<string, AssistantPermissionMode>
  >({});
  const [localApprovals, setLocalApprovals] = useState<ApprovalEvent[]>([]);
  const [localChanges, setLocalChanges] = useState<AssistantLocalChange[]>([]);
  const [localPlan, setLocalPlan] = useState<PlanStep[]>([]);
  const [localIntent, setLocalIntent] = useState<string | null>(null);
  const [localWarning, setLocalWarning] = useState<string | null>(null);
  const [localModelNotice, setLocalModelNotice] = useState<string | null>(null);
  // Thinking for the conversation the first message creates: it belongs to
  // the model chosen for it, so choosing another model clears it.
  const [newThinkingLevel, setNewThinkingState] =
    useState<AssistantThinkingLevel | null>(null);
  const newThinkingRef = useRef(newThinkingLevel);
  const [search, setSearch] = useState("");
  const [showTrash, setShowTrash] = useState(false);
  const [sessions, setSessions] = useState<AssistantSession[]>([]);
  const [unreadable, setUnreadable] = useState<AssistantUnreadableSession[]>(
    [],
  );
  const [trashCount, setTrashCount] = useState(0);
  // For a search: the first message each listed conversation matches.
  const [matches, setMatches] = useState<Record<string, string>>({});
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<AssistantSessionDetail | null>(null);
  const detailRef = useRef(detail);
  detailRef.current = detail;
  const [draft, setDraftState] = useState("");
  const [skill, setSkillState] = useState<string | undefined>();
  const skillRef = useRef<string | undefined>();
  const skillDrafts = useRef(new Map<string, string | undefined>());
  const skillVersion = useRef(0);
  const skillVersions = useRef(new Map<string, number>());
  const setSkill = useCallback((value: string | undefined) => {
    skillVersion.current += 1;
    skillRef.current = value;
    skillDrafts.current.set(selectedRef.current ?? "", value);
    skillVersions.current.set(selectedRef.current ?? "", skillVersion.current);
    setSkillState(value);
  }, []);
  const [attachments, setAttachmentsState] = useState<AssistantAttachment[]>(
    [],
  );
  const [stream, setStream] = useState<AssistantMessage | null>(null);
  const [thinking, setThinking] = useState<AssistantThinking | null>(null);
  const thinkingRef = useRef<AssistantThinking | null>(null);
  const [run, setRunState] = useState<AssistantRunRef | null>(null);
  const [tools, setTools] = useState<RunningTool[]>([]);
  // The files a run this window sent reads before its model request: those
  // whose outcome has not come in yet.
  const [reading, setReading] = useState<{
    runId: string;
    ids: string[];
  } | null>(null);
  const [runSearch, setRunSearchState] = useState<RunSearch | null>(null);
  const runSearchRef = useRef(runSearch);
  // Replies whose run went without the web search their conversation asked
  // for (it was unavailable when they were sent), by message id.
  const [searchSkipped, setSearchSkipped] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [compacting, setCompacting] = useState(false);
  const [retryAttempt, setRetryAttempt] = useState<1 | 2 | null>(null);
  const listenerReady = useRef<ReturnType<typeof subscriptionReadiness> | null>(
    null,
  );
  if (!listenerReady.current) listenerReady.current = subscriptionReadiness();
  const bootstrapSequence = useRef(0);
  const catalogEvents = useRef(new Map<string, AssistantCatalogSnapshot>());
  const audioFinished = useRef(new Set<string>());
  const audioPreferenceTime = useRef<{
    identity: string;
    revision: number;
    times: Partial<Record<"speech" | "transcription", number>>;
    models: Partial<
      Record<"speech" | "transcription", { revision: number; model: string }>
    >;
    choices: Record<
      string,
      { revision: number; voice: string; format: string }
    >;
  }>({ identity: "", revision: 0, times: {}, models: {}, choices: {} });
  const [error, setErrorCode] = useState<AssistantError | null>(null);
  const [errorDetail, setErrorDetail] = useState<string | null>(null);
  const setError = useCallback((value: AssistantError | null) => {
    setErrorCode(value);
    setErrorDetail(null);
  }, []);
  const [loading, setLoading] = useState(false);
  const [operationBusy, setBusyState] = useState(false);
  const operationBusyRef = useRef(false);
  const setBusy = useCallback((value: boolean) => {
    operationBusyRef.current = value;
    setBusyState(value);
  }, []);
  const [transferring, setTransferring] = useState(false);
  const transferInFlight = useRef(false);
  const [transferResult, setTransferResult] =
    useState<AssistantTransferResult | null>(null);
  const busy = operationBusy || transferring;
  const [connected, setConnected] = useState(false);
  // The account's assistant settings; null until read, and the defaults
  // apply meanwhile.
  const [preferences, setPreferences] = useState<AssistantPreferences | null>(
    null,
  );
  const mounted = useRef(false);
  const epoch = useRef(0);
  const listSequence = useRef(0);
  const detailSequence = useRef(0);
  const selectionSequence = useRef(0);
  const updateSequence = useRef(0);
  const draftSequence = useRef(0);
  const createdSession = useRef<string | null>(null);
  const selectedRef = useRef(selectedId);
  const showTrashRef = useRef(showTrash);
  const runRef = useRef(run);
  // A regenerate under way, by its run: the question of the version it
  // would replace, and whether the user called it off (declined what it
  // asked to generate, or stopped it). Called off, that version is shown
  // again once the run ends; the new one stays among the versions.
  const regenerating = useRef<{
    runId: string;
    sessionId: string;
    questionId: string;
    cancelled: boolean;
  } | null>(null);
  const [restoreVersion, setRestoreVersion] = useState<{
    sessionId: string;
    questionId: string;
  } | null>(null);
  const stopRef = useRef<() => Promise<void>>(async () => undefined);
  const draftRef = useRef(draft);
  const drafts = useRef(new Map<string, string>());
  // The files of each conversation's draft, kept like its text.
  const attachmentsRef = useRef(attachments);
  const attachmentDrafts = useRef(new Map<string, AssistantAttachment[]>());
  const draftWrites = useRef(new Map<string, Promise<boolean>>());
  // Import and send admission own the draft until their files are durable.
  // Replacing the scope also invalidates saves queued before that operation.
  const draftScopes = useRef(new Map<string, DraftWriteScope>());
  const draftAuthority = useRef(0);
  // A file import is under way (set at once; `busy` follows on the next render).
  const importing = useRef(false);
  const savedDrafts = useRef(new Map<string, SavedDraft>());
  const bootstrapRef = useRef<AssistantBootstrap | null>(null);
  const modelRef = useRef("");
  modelRef.current = detail?.session.modelId ?? newModelId;
  const videoNotices = useRef(new Set<string>());
  const videoProgressSequence = useRef(0);
  const videoProgress = useRef(
    new Map<
      string,
      { sequence: number; event: AssistantEvent & { type: "media_progress" } }
    >(),
  );
  const pendingSends = useRef(new Map<string, PendingSend>());
  const refreshSessionsRef = useRef<(generation?: number) => Promise<void>>(
    async () => undefined,
  );
  /** Sign-in expired: nothing of the account is listed until it is restored. */
  const signedOut = useRef(false);
  const reloadAuthority = useRef<() => void>(() => undefined);
  selectedRef.current = selectedId;
  showTrashRef.current = showTrash;
  draftRef.current = draft;

  const setNewWebSearch = useCallback((value: boolean) => {
    newWebSearchRef.current = value;
    setNewWebSearchState(value);
  }, []);

  const setNewThinking = useCallback((value: AssistantThinkingLevel | null) => {
    newThinkingRef.current = value;
    setNewThinkingState(value);
  }, []);

  const writeThinking = useCallback((value: AssistantThinking | null) => {
    thinkingRef.current = value;
    setThinking(value);
  }, []);

  /** The followed run's reply starts a span of thinking (or goes on with one). */
  const startThinking = useCallback(
    (runId: string) => {
      const old = thinkingRef.current;
      const base =
        old?.runId === runId ? old : { runId, doneMs: 0, since: null };
      if (base.since === null) writeThinking({ ...base, since: Date.now() });
    },
    [writeThinking],
  );

  /**
   * Its reply stopped thinking, spoke, called a tool or ended: the span under
   * way is over.
   */
  const endThinking = useCallback(
    (runId: string) => {
      const old = thinkingRef.current;
      if (old?.runId !== runId || old.since === null) return;
      writeThinking({
        ...old,
        doneMs: old.doneMs + Math.max(0, Date.now() - old.since),
        since: null,
      });
    },
    [writeThinking],
  );

  const setRunSearch = useCallback((value: RunSearch | null) => {
    runSearchRef.current = value;
    setRunSearchState(value);
  }, []);

  const setRun = useCallback((value: AssistantRunRef | null) => {
    // Another run, or none: the tools the last one started are not followed.
    if (value?.runId !== runRef.current?.runId) {
      setRetryAttempt(null);
      setTools([]);
      setLocalApprovals([]);
      setLocalIntent(null);
      setLocalPlan([]);
      setLocalWarning(null);
    }
    runRef.current = value;
    setRunState(value);
  }, []);

  // Words stream in many times a second: they are gathered here and put on
  // screen once per animation frame, or at once when the run says anything
  // else (its last words must never wait for a frame that may not come).
  const pendingDeltas = useRef<PendingDeltas | null>(null);
  const deltaFrame = useRef<number | null>(null);

  /** Forgets the words not yet shown: their reply is no longer on screen. */
  const dropDeltas = useCallback(() => {
    if (
      deltaFrame.current !== null &&
      typeof cancelAnimationFrame === "function"
    )
      cancelAnimationFrame(deltaFrame.current);
    deltaFrame.current = null;
    pendingDeltas.current = null;
  }, []);

  const flushDeltas = useCallback(() => {
    const pending = pendingDeltas.current;
    dropDeltas();
    // Only the run still followed has a reply on screen to add to.
    if (!pending || runRef.current?.runId !== pending.runId) return;
    setStream((previous) => {
      const value = previous ?? blankReply(pending.runId);
      return {
        ...value,
        content: value.content + pending.text,
        ...(pending.thinking
          ? { reasoning: (value.reasoning ?? "") + pending.thinking }
          : {}),
      };
    });
  }, [dropDeltas]);

  const queueDelta = useCallback(
    (runId: string, field: "text" | "thinking", delta: string) => {
      let pending = pendingDeltas.current;
      if (pending?.runId !== runId) {
        flushDeltas();
        pending = { runId, text: "", thinking: "" };
        pendingDeltas.current = pending;
      }
      pending[field] += delta;
      if (deltaFrame.current !== null) return;
      if (typeof requestAnimationFrame !== "function") flushDeltas();
      else
        deltaFrame.current = requestAnimationFrame(() => {
          deltaFrame.current = null;
          flushDeltas();
        });
    },
    [flushDeltas],
  );

  const setDraft = useCallback((value: string) => {
    draftRef.current = value;
    if (selectedRef.current) drafts.current.set(selectedRef.current, value);
    setDraftState(value);
  }, []);

  const setAttachments = useCallback((value: AssistantAttachment[]) => {
    attachmentsRef.current = value;
    if (selectedRef.current)
      attachmentDrafts.current.set(selectedRef.current, value);
    setAttachmentsState(value);
  }, []);

  const showError = useCallback(
    (value: unknown) => {
      if (!mounted.current) return;
      const code = assistantErrorCode(value);
      setError(code);
      // Only media diagnostics supplied by the host have this explicit contract:
      // Rust scrubs credentials and caps their length before emitting them.
      if (
        code.startsWith("assistant_media_") &&
        value &&
        typeof value === "object"
      ) {
        const detail = (value as { detail?: unknown }).detail;
        if (typeof detail === "string") setErrorDetail(detail.slice(0, 2000));
      }
      // A sign-in, sign-out or region change moved the backend's authority on:
      // the owner and generation every write carries must be read again.
      if (code === IDENTITY_CHANGED) {
        messageQueue.clear();
        reloadAuthority.current();
      }
    },
    [messageQueue],
  );

  /** The backend's authority now, or null when it could not be read. */
  const loadBootstrap = useCallback(
    async (refreshCatalog = false) => {
      const generation = epoch.current;
      const sequence = ++bootstrapSequence.current;
      const audioRevision = audioPreferenceTime.current.revision;
      await listenerReady.current!.promise;
      if (
        !mounted.current ||
        generation !== epoch.current ||
        sequence !== bootstrapSequence.current
      )
        return null;
      // A sign-in that completes while this read runs moves the authority on
      // once more: read it again instead of leaving the page disabled.
      for (let attempt = 0; ; attempt += 1) {
        try {
          const value = { ...(await assistantApi.bootstrap(refreshCatalog)) };
          if (
            !mounted.current ||
            generation !== epoch.current ||
            sequence !== bootstrapSequence.current
          )
            return null;
          const previous = bootstrapRef.current;
          const pending = catalogEvents.current.get(
            `${value.owner}:${value.authorityGeneration}`,
          );
          if (
            pending &&
            pending.catalogRevision >= (value.catalogRevision ?? 0)
          )
            Object.assign(value, catalogPatch(pending));
          if (
            previous?.owner === value.owner &&
            previous.authorityGeneration === value.authorityGeneration &&
            (previous.catalogRevision ?? 0) > (value.catalogRevision ?? 0)
          )
            Object.assign(
              value,
              catalogPatch(
                previous as AssistantBootstrap & AssistantCatalogSnapshot,
              ),
            );
          if (
            previous &&
            (previous.owner !== value.owner ||
              previous.authorityGeneration !== value.authorityGeneration)
          ) {
            messageQueue.clear();
            skillDrafts.current.clear();
            skillVersions.current.clear();
            setSkill(undefined);
          }
          if (
            previous?.owner === value.owner &&
            previous.authorityGeneration === value.authorityGeneration &&
            audioPreferenceTime.current.revision !== audioRevision
          ) {
            const cache = audioPreferenceTime.current;
            for (const kind of ["speech", "transcription"] as const) {
              const update = cache.models[kind];
              if (update && update.revision > audioRevision)
                value.rememberedModels = {
                  ...value.rememberedModels,
                  [kind]: update.model,
                };
            }
            for (const [model, choice] of Object.entries(cache.choices)) {
              if (choice.revision > audioRevision)
                value.speechPreferences = {
                  ...value.speechPreferences,
                  [model]: { voice: choice.voice, format: choice.format },
                };
            }
          }
          bootstrapRef.current = value;
          setBootstrap(value);
          return value;
        } catch (reason) {
          const code = assistantErrorCode(reason);
          if (attempt === 0 && code === IDENTITY_CHANGED) continue;
          // Not through showError: a failed read must not ask for itself again.
          if (mounted.current && generation === epoch.current) setError(code);
          return null;
        }
      }
    },
    [messageQueue],
  );
  reloadAuthority.current = () => void loadBootstrap();

  /**
   * Read the account's settings again. Rejects when they cannot be read;
   * the page keeps what it had.
   */
  const reloadPreferences = useCallback(async () => {
    const generation = epoch.current;
    const value = await assistantApi.getPreferences();
    if (mounted.current && generation === epoch.current) setPreferences(value);
    return value;
  }, []);

  /** Without the settings the page goes on with the defaults. */
  const loadPreferences = useCallback(
    () =>
      void reloadPreferences().catch((reason) =>
        console.warn("[Assistant] could not read the settings", reason),
      ),
    [reloadPreferences],
  );

  /**
   * Save every setting for the account and authority the page shows;
   * resolves with them as saved. The instructions apply from the next
   * message on.
   */
  const savePreferences = useCallback(async (next: AssistantPreferences) => {
    const generation = epoch.current;
    try {
      const saved = await assistantApi.setPreferences({
        ...next,
        ...expectedIdentity(bootstrapRef.current),
      });
      if (mounted.current && generation === epoch.current)
        setPreferences(saved);
      return saved;
    } catch (reason) {
      if (assistantErrorCode(reason) === IDENTITY_CHANGED)
        reloadAuthority.current();
      throw reason;
    }
  }, []);

  /**
   * Save a conversation's draft text and, when given and changed, its files.
   * Writes to one conversation run in order.
   */
  const persistDraft = useCallback(
    (
      sessionId: string,
      text: string,
      files?: string[],
      held?: DraftWriteScope,
    ) => {
      const generation = draftAuthority.current;
      const scope = draftScopes.current.get(sessionId);
      if (scope?.held && scope !== held) return Promise.resolve(false);
      const task = (draftWrites.current.get(sessionId) ?? Promise.resolve())
        .catch(() => undefined)
        .then(async () => {
          if (
            generation !== draftAuthority.current ||
            draftScopes.current.get(sessionId) !== scope
          )
            return false;
          const saved = savedDrafts.current.get(sessionId);
          const newFiles =
            files && !(saved?.files && sameIds(saved.files, files))
              ? files
              : undefined;
          if (saved?.text === text && !newFiles) return true;
          const write = () =>
            assistantApi.updateSession({
              sessionId,
              draft: text,
              ...(newFiles ? { draftAttachmentIds: newFiles } : {}),
              ...expectedIdentity(bootstrapRef.current),
            });
          let result: AssistantSession;
          try {
            result = await write();
          } catch (reason) {
            if (assistantErrorCode(reason) !== IDENTITY_CHANGED) throw reason;
            // The authority moved on (a sign-in or region change completed).
            // Saving a draft is safe to repeat once, for the same account.
            const owner = bootstrapRef.current?.owner;
            const current = await loadBootstrap();
            if (
              !current ||
              current.owner !== owner ||
              generation !== draftAuthority.current
            )
              throw reason;
            result = await write();
          }
          if (generation !== draftAuthority.current) return false;
          savedDrafts.current.set(sessionId, {
            text,
            files: newFiles ?? saved?.files,
          });
          if (result.cleanupWarning) showError(result.cleanupWarning);
          return true;
        });
      draftWrites.current.set(sessionId, task);
      return task;
    },
    [loadBootstrap, showError],
  );

  const releaseDraft = useCallback(
    async (sessionId: string, scope: DraftWriteScope, generation: number) => {
      if (draftScopes.current.get(sessionId) !== scope) return;
      draftScopes.current.set(sessionId, { held: false });
      if (generation !== epoch.current || !drafts.current.has(sessionId))
        return;
      // Autosaves blocked by the operation may have contained newer text.
      // Flush the current cache even when this conversation is now hidden.
      await persistDraft(
        sessionId,
        drafts.current.get(sessionId) ?? "",
        attachmentDrafts.current.get(sessionId)?.map((file) => file.id),
      ).catch(showError);
    },
    [persistDraft, showError],
  );

  useEffect(
    () => () => {
      draftAuthority.current += 1;
    },
    [],
  );

  /** `generation`: the identity generation the caller acts for. */
  const refreshSessions = useCallback(
    async (generation = epoch.current) => {
      if (signedOut.current || generation !== epoch.current) return;
      const sequence = ++listSequence.current;
      try {
        const listed = await assistantApi.listSessions(showTrash, search);
        if (
          !mounted.current ||
          generation !== epoch.current ||
          sequence !== listSequence.current
        )
          return;
        setSessions(listed.sessions);
        setUnreadable(listed.unreadable);
        setTrashCount(listed.trashCount);
        setMatches(listed.matches);
      } catch (reason) {
        if (sequence === listSequence.current && generation === epoch.current)
          showError(reason);
      }
    },
    [search, showTrash, showError],
  );
  refreshSessionsRef.current = refreshSessions;

  const refreshDetail = useCallback(
    async (sessionId: string) => {
      const sequence = ++detailSequence.current;
      const generation = epoch.current;
      const mediaSequence = videoProgressSequence.current;
      try {
        let value = await assistantApi.getSession(sessionId);
        if (
          !mounted.current ||
          generation !== epoch.current ||
          selectedRef.current !== sessionId ||
          sequence !== detailSequence.current
        )
          return false;
        // A read begun before a progress event must not roll its saved video
        // back. Prefer a genuinely newer session snapshot over the event.
        for (const progress of videoProgress.current.values()) {
          const { event } = progress;
          if (
            progress.sequence <= mediaSequence ||
            event.sessionId !== sessionId ||
            event.data.updatedAt < value.session.updatedAt ||
            event.data.owner !== bootstrapRef.current?.owner ||
            event.data.generation !== bootstrapRef.current?.authorityGeneration
          )
            continue;
          const { message, usage, updatedAt } = event.data;
          value = {
            ...value,
            session: { ...value.session, updatedAt },
            usage,
            messages: value.messages.some((item) => item.id === message.id)
              ? value.messages.map((item) =>
                  item.id === message.id ? message : item,
                )
              : [...value.messages, message],
            attachments: [
              ...new Map(
                [...value.attachments, ...(message.attachments ?? [])].map(
                  (file) => [file.id, file],
                ),
              ).values(),
            ],
          };
        }
        // While this window is still preparing a send (the first send may be
        // minting the key) the backend has no run yet: keep the local run and
        // the optimistic question, or later events would be dropped.
        const local = runRef.current;
        const preparing = Boolean(
          local &&
            pendingSends.current.has(local.runId) &&
            !value.session.activeRun,
        );
        // A reply still running (also after a restart) keeps streaming into
        // the same bubble instead of showing a saved copy next to it.
        const resumed = value.session.activeRun
          ? value.messages.find(
              (message) =>
                ["assistant", "media"].includes(message.role) &&
                message.status === "streaming",
            )
          : undefined;
        savedDrafts.current.set(sessionId, {
          text: value.session.draft,
          files: value.session.draftAttachmentIds,
        });
        const alreadyFollowing = Boolean(runRef.current);
        setLocalModes((old) => ({
          ...old,
          [sessionId]: value.permissionMode ?? "default",
        }));
        setDetail((old) => {
          if (preparing)
            return {
              ...value,
              messages: [
                ...value.messages,
                ...(old?.messages ?? []).filter((item) =>
                  item.id.startsWith("pending-"),
                ),
              ],
            };
          return resumed
            ? {
                ...value,
                messages: value.messages.filter(
                  (message) => message.id !== resumed.id,
                ),
              }
            : value;
        });
        if (resumed && !alreadyFollowing) setStream(resumed);
        if (!preparing) {
          setRun(value.session.activeRun);
          setLocalApprovals(value.pendingApprovals ?? []);
        }
        setCompacting(
          preparing
            ? pendingSends.current.get(local!.runId)?.compactOnly === true
            : value.session.compacting === true,
        );
        if (!drafts.current.has(sessionId)) setDraft(value.session.draft);
        // A saved draft's files, as the next message would send them.
        if (!attachmentDrafts.current.has(sessionId))
          setAttachments(
            value.session.draftAttachmentIds.flatMap((id) => {
              const file = value.attachments.find((item) => item.id === id);
              return file ? asDraftFile(file) : [];
            }),
          );
        return true;
      } catch (reason) {
        if (
          generation === epoch.current &&
          selectedRef.current === sessionId &&
          sequence === detailSequence.current
        )
          showError(reason);
        return false;
      }
    },
    [setAttachments, setDraft, setRun, showError],
  );

  const savedDraft = detail?.session.draft ?? "";
  const savedFileKey = (detail?.session.draftAttachmentIds ?? []).join("\n");
  const fileKey = attachments.map((file) => file.id).join("\n");
  const detailSessionId = detail?.session.id;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      epoch.current += 1;
    };
  }, []);

  useEffect(() => {
    const cleanup: (() => void)[] = [];
    let disposed = false;
    const reset = () => {
      messageQueue.clear();
      signedOut.current = false;
      epoch.current += 1;
      draftAuthority.current += 1;
      for (const pending of pendingSends.current.values())
        pending.cancelled = true;
      pendingSends.current.clear();
      listSequence.current += 1;
      detailSequence.current += 1;
      selectedRef.current = null;
      drafts.current.clear();
      skillDrafts.current.clear();
      skillVersions.current.clear();
      setSkill(undefined);
      attachmentDrafts.current.clear();
      savedDrafts.current.clear();
      draftScopes.current.clear();
      setSelectedId(null);
      setDetail(null);
      setSessions([]);
      setShowTrash(false);
      setTrashCount(0);
      transferInFlight.current = false;
      setTransferring(false);
      setTransferResult(null);
      setUnreadable([]);
      dropDeltas();
      setStream(null);
      writeThinking(null);
      setCompacting(false);
      setBusy(false);
      setLoading(false);
      setRun(null);
      setDraft("");
      setAttachments([]);
      setBootstrap(null);
      bootstrapRef.current = null;
      catalogEvents.current.clear();
      videoProgress.current.clear();
      videoNotices.current.clear();
      setChosenModel(null);
      lastWebSearch.current = false;
      newLocalConfigRef.current = false;
      setNewLocalConfig(false);
      setLocalModes({});
      setLocalChanges([]);
      setLocalPlan([]);
      setLocalWarning(null);
      setLocalModelNotice(null);
      setNewWebSearch(false);
      setNewThinking(null);
      setRunSearch(null);
      setSearchSkipped(new Set());
      setPreferences(null);
      setError(null);
    };
    if (!enabled) {
      reset();
      return;
    }
    void loadBootstrap();
    loadPreferences();
    void assistantApi
      .onAuthChange((state) => {
        reset();
        signedOut.current = state === "expired";
        if (state === "restored") {
          void loadBootstrap();
          loadPreferences();
          void refreshSessionsRef.current();
        }
      })
      .then((off) => (disposed ? off() : cleanup.push(off)))
      .catch(showError);
    return () => {
      disposed = true;
      epoch.current += 1;
      cleanup.forEach((off) => off());
    };
  }, [
    dropDeltas,
    enabled,
    loadBootstrap,
    loadPreferences,
    messageQueue,
    setAttachments,
    setDraft,
    setNewThinking,
    setNewWebSearch,
    setRun,
    setRunSearch,
    showError,
    writeThinking,
  ]); // List refresh uses its current request fencing separately.

  useEffect(() => {
    if (!enabled) return;
    const timer = setTimeout(() => void refreshSessions(), 180);
    return () => clearTimeout(timer);
  }, [enabled, refreshSessions]);

  const loadSelected = useCallback(
    (sessionId: string) => {
      setLoading(true);
      void refreshDetail(sessionId).finally(() => {
        if (mounted.current && selectedRef.current === sessionId)
          setLoading(false);
      });
    },
    [refreshDetail],
  );

  useEffect(() => {
    if (!enabled || !selectedId) return;
    if (createdSession.current === selectedId) {
      createdSession.current = null;
      setLoading(false);
      return;
    }
    loadSelected(selectedId);
  }, [enabled, selectedId, loadSelected]);

  // Persist only a draft which still belongs to the displayed session. The backend also checks owner.
  useEffect(() => {
    if (
      !selectedId ||
      detailSessionId !== selectedId ||
      (draft === savedDraft && fileKey === savedFileKey)
    )
      return;
    const generation = epoch.current;
    const sequence = ++draftSequence.current;
    const files = fileKey ? fileKey.split("\n") : [];
    const timer = setTimeout(() => {
      void persistDraft(selectedId, draft, files)
        .then((saved) => {
          if (
            saved &&
            generation === epoch.current &&
            sequence === draftSequence.current &&
            selectedRef.current === selectedId
          )
            setDetail((old) =>
              old
                ? {
                    ...old,
                    session: {
                      ...old.session,
                      draft,
                      draftAttachmentIds: files,
                    },
                  }
                : old,
            );
        })
        .catch((reason) => {
          if (generation === epoch.current) showError(reason);
        });
    }, 600);
    return () => {
      clearTimeout(timer);
      draftSequence.current += 1;
    };
  }, [
    selectedId,
    draft,
    fileKey,
    detailSessionId,
    savedDraft,
    savedFileKey,
    persistDraft,
    showError,
  ]);

  // A reply that finished out of the user's sight is told (see
  // useFinishNotice); `select` is only called once the page is rendered.
  const notifyFinish = useFinishNotice({
    enabled:
      enabled && !signedOut.current && preferences?.notifyOnFinish === true,
    authority: () => epoch.current,
    openSessionId: selectedId,
    titleOf: (id) =>
      sessions.find((item) => item.id === id)?.title ??
      (detail?.session.id === id ? detail.session.title : ""),
    open: (id) => select(id),
  });

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let off: (() => void) | undefined;
    const readiness = listenerReady.current!;
    // The model is at work (it writes, thinks or calls a tool): the run read
    // its files first, even if an outcome never came in.
    const doneReading = (runId: string) =>
      setReading((old) => (old?.runId === runId ? null : old));
    // Only the name, and the cost when the event has it: whatever else the
    // session holds is read with the list and the transcript as usual.
    const showTitle = (
      { id, title, titleSource }: AssistantSession,
      usage?: AssistantUsage,
    ) => {
      const named = <T extends AssistantSession>(item: T): T =>
        item.id === id ? { ...item, title, titleSource } : item;
      setSessions((old) => old.map(named));
      setDetail((old) =>
        old?.session.id === id
          ? { ...old, session: named(old.session), usage: usage ?? old.usage }
          : old,
      );
    };
    void assistantApi
      .subscribe((event: AssistantEvent) => {
        if (disposed) return;
        // The identity generation this event arrived under: a list read it
        // asks for is fenced on it.
        const generation = epoch.current;
        if (event.type === "catalog_state") {
          const { owner, generation: authority, catalog } = event.data;
          const current = bootstrapRef.current;
          if (
            signedOut.current ||
            !Number.isSafeInteger(catalog?.catalogRevision) ||
            catalog.catalogRevision < 0 ||
            !Array.isArray(catalog.models) ||
            typeof catalog.catalogRefreshing !== "boolean" ||
            (current &&
              (owner !== current.owner ||
                authority !== current.authorityGeneration))
          )
            return;
          const key = `${owner}:${authority}`;
          const saved = catalogEvents.current.get(key);
          if (saved && saved.catalogRevision > catalog.catalogRevision) return;
          catalogEvents.current.set(key, catalog);
          if (catalogEvents.current.size > 8)
            catalogEvents.current.delete(
              catalogEvents.current.keys().next().value!,
            );
          if (
            current &&
            catalog.catalogRevision >= (current.catalogRevision ?? 0)
          ) {
            const next = { ...current, ...catalogPatch(catalog) };
            bootstrapRef.current = next;
            setBootstrap(next);
          }
          return;
        }
        if (event.type === "audio_state") {
          const {
            owner,
            generation: authority,
            running,
            result,
            usage,
          } = event.data;
          const identity = bootstrapRef.current;
          if (
            signedOut.current ||
            !identity ||
            owner !== identity.owner ||
            authority !== identity.authorityGeneration ||
            result?.operationId !== event.operationId ||
            result.runId !== event.runId ||
            !["transcription", "speech"].includes(result.kind) ||
            !result.audio
          )
            return;
          const key = `${owner}:${authority}:${event.sessionId}:${event.operationId}`;
          if (audioFinished.current.has(key)) return;
          // Native admission saved these choices before this submitted event.
          // Completion order is not preference order across conversations.
          const preferenceIdentity = `${owner}:${authority}`;
          if (audioPreferenceTime.current.identity !== preferenceIdentity)
            audioPreferenceTime.current = {
              identity: preferenceIdentity,
              revision: 0,
              times: {},
              models: {},
              choices: {},
            };
          const preferenceKind = result.kind;
          if (
            running &&
            result.status === "submitting" &&
            Number.isFinite(event.data.updatedAt) &&
            event.data.updatedAt >=
              (audioPreferenceTime.current.times[preferenceKind] ?? -Infinity)
          ) {
            audioPreferenceTime.current.revision++;
            audioPreferenceTime.current.times[preferenceKind] =
              event.data.updatedAt;
            const params = result.params ?? {};
            const choice =
              preferenceKind === "speech" &&
              typeof params.voice === "string" &&
              /^[A-Za-z0-9]{1,64}$/.test(params.voice) &&
              typeof params.responseFormat === "string" &&
              ["wav", "mp3", "opus"].includes(params.responseFormat)
                ? { voice: params.voice, format: params.responseFormat }
                : null;
            audioPreferenceTime.current.models[preferenceKind] = {
              revision: audioPreferenceTime.current.revision,
              model: result.model,
            };
            if (choice)
              audioPreferenceTime.current.choices[result.model] = {
                revision: audioPreferenceTime.current.revision,
                ...choice,
              };
            const next = {
              ...identity,
              rememberedModels: {
                ...identity.rememberedModels,
                [preferenceKind]: result.model,
              },
              ...(choice
                ? {
                    speechPreferences: {
                      ...identity.speechPreferences,
                      [result.model]: choice,
                    },
                  }
                : {}),
            };
            bootstrapRef.current = next;
            setBootstrap(next);
          }
          if (!running) {
            audioFinished.current.add(key);
            if (audioFinished.current.size > 512)
              audioFinished.current.delete(
                audioFinished.current.values().next().value!,
              );
            notifyFinish({
              sessionId: event.sessionId,
              status: result.status === "completed" ? "completed" : "error",
            });
          }
          void refreshSessionsRef.current(generation);
          if (selectedRef.current !== event.sessionId) return;
          detailSequence.current++;
          const active = { runId: event.runId, operationId: event.operationId };
          if (running) {
            if (
              (!runRef.current || runRef.current.runId === event.runId) &&
              (!detailRef.current ||
                detailRef.current.session.updatedAt <= event.data.updatedAt)
            ) {
              setRun(active);
              setStream(null);
            }
          } else if (runRef.current?.runId === event.runId) setRun(null);
          setDetail((old) => {
            if (old?.session.id !== event.sessionId) return old;
            const newer =
              old.session.updatedAt > event.data.updatedAt ||
              (old.session.activeRun &&
                old.session.activeRun.runId !== event.runId);
            const activeRun = running
              ? newer
                ? old.session.activeRun
                : active
              : newer
                ? old.session.activeRun
                : null;
            return {
              ...old,
              usage: newer ? old.usage : usage,
              session: {
                ...old.session,
                activeRun,
                updatedAt: Math.max(
                  old.session.updatedAt,
                  event.data.updatedAt,
                ),
                audioOnly: newer
                  ? old.session.audioOnly
                  : activeRun
                    ? true
                    : undefined,
              },
              audioOperations: [
                ...(old.audioOperations ?? []).filter(
                  (item) => item.operationId !== event.operationId,
                ),
                result,
              ],
            };
          });
          return;
        }
        if (event.type === "media_progress") {
          const {
            owner,
            generation: authority,
            apex,
            messageId,
            message,
            status,
            usage,
          } = event.data;
          const identity = bootstrapRef.current;
          const toolCallId = event.data.toolCallId ?? null;
          const video = toolCallId
            ? message?.generations?.find(
                (item) => item.toolCallId === toolCallId,
              )
            : message?.media;
          if (
            signedOut.current ||
            !identity ||
            owner !== identity.owner ||
            authority !== identity.authorityGeneration ||
            message?.id !== messageId ||
            video?.kind !== "video" ||
            video.apex !== apex ||
            (!toolCallId && message.runId !== event.runId) ||
            video.status !== status
          )
            return;
          const progressKey = `${event.sessionId}:${messageId}`;
          videoProgress.current.delete(progressKey);
          videoProgress.current.set(progressKey, {
            sequence: ++videoProgressSequence.current,
            event,
          });
          if (videoProgress.current.size > 512)
            videoProgress.current.delete(
              videoProgress.current.keys().next().value!,
            );
          setDetail((old) => {
            if (
              old?.session.id !== event.sessionId ||
              old.session.updatedAt > event.data.updatedAt
            )
              return old;
            const previous = old.messages.find(
              (item) =>
                item.id === messageId &&
                (toolCallId !== null || item.runId === event.runId),
            );
            const before = toolCallId
              ? previous?.generations?.find(
                  (item) => item.toolCallId === toolCallId,
                )
              : previous?.media;
            if (
              !previous ||
              before?.kind !== "video" ||
              (before.status === "completed" && status !== "completed")
            )
              return old;
            return {
              ...old,
              session: { ...old.session, updatedAt: event.data.updatedAt },
              usage,
              messages: old.messages.map((item) =>
                item.id === messageId ? message : item,
              ),
              attachments: [
                ...new Map(
                  [...old.attachments, ...(message.attachments ?? [])].map(
                    (file) => [file.id, file],
                  ),
                ).values(),
              ],
            };
          });
          if (["completed", "failed", "error", "expired"].includes(status)) {
            const key = `${owner}:${authority}:${event.sessionId}:${messageId}:${toolCallId ?? ""}`;
            if (!videoNotices.current.has(key)) {
              videoNotices.current.add(key);
              notifyFinish({
                sessionId: event.sessionId,
                status: status === "completed" ? "completed" : "error",
              });
              void refreshSessionsRef.current(generation);
            }
          }
          return;
        }
        // A generation shows as soon as it changes, from a reply being
        // written or from a regeneration started on its card.
        if (event.type === "generation") {
          if (event.sessionId !== selectedRef.current) return;
          const { messageId, generation: made, attachments } = event.data;
          const apply = (message: AssistantMessage): AssistantMessage => {
            const others = (message.generations ?? []).filter(
              (item) => item.toolCallId !== made.toolCallId,
            );
            const kept = message.generations?.findIndex(
              (item) => item.toolCallId === made.toolCallId,
            );
            const generations =
              kept !== undefined && kept >= 0
                ? message.generations!.map((item, index) =>
                    index === kept ? made : item,
                  )
                : [...others, made];
            return {
              ...message,
              generations,
              attachments: [
                ...new Map(
                  [...(message.attachments ?? []), ...attachments].map(
                    (file) => [file.id, file],
                  ),
                ).values(),
              ],
            };
          };
          setStream((previous) =>
            previous?.id === messageId ? apply(previous) : previous,
          );
          setDetail((old) =>
            old?.session.id === event.sessionId
              ? {
                  ...old,
                  messages: old.messages.map((item) =>
                    item.id === messageId ? apply(item) : item,
                  ),
                  attachments: [
                    ...new Map(
                      [...old.attachments, ...attachments].map((file) => [
                        file.id,
                        file,
                      ]),
                    ).values(),
                  ],
                }
              : old,
          );
          return;
        }
        if (event.type === "run_start")
          void refreshSessionsRef.current(generation);
        // Any conversation's reply, followed or not.
        if (event.type === "run_end")
          notifyFinish({
            sessionId: event.sessionId,
            status: event.data.status,
          });
        const current = runRef.current;
        if (
          !current ||
          event.sessionId !== selectedRef.current ||
          event.runId !== current.runId ||
          event.operationId !== current.operationId
        ) {
          // A run this view no longer follows (another conversation, or one
          // left behind) still changes the history's "Replying…" marks.
          // `run_end` is always its last event, also after an `error`.
          if (event.type === "run_end")
            void refreshSessionsRef.current(generation);
          // A saved session no followed run published, such as a title
          // generated after a reply: the list and the header show its name,
          // and the open conversation the cost the title added.
          else if (event.type === "session")
            showTitle(event.data.session, event.data.usage);
          return;
        }
        // Anything but more words shows the words that came before it first,
        // so a reply that ends, fails or is replaced never loses its last ones.
        if (event.type !== "text_delta" && event.type !== "thinking_delta")
          flushDeltas();
        // session and event types this build does not know change nothing
        // on screen: the transcript and session are read again at run_end.
        // A retry notice belongs only to the followed live request. Model or
        // tool progress, a replacement turn, failure and completion clear it.
        if (
          [
            "thinking_start",
            "text_delta",
            "thinking_delta",
            "tool_start",
            "message",
            "error",
            "run_end",
            "steer_applied",
          ].includes(event.type)
        )
          setRetryAttempt(null);
        switch (event.type) {
          case "retrying":
            if (event.data.attempt === 1 || event.data.attempt === 2)
              setRetryAttempt(event.data.attempt);
            return;
          case "skill_results": {
            const { messageId, group } = event.data;
            if (
              !Array.isArray(group?.results) ||
              group.results.length > 8 ||
              typeof group.toolCallId !== "string"
            )
              return;
            const apply = (message: AssistantMessage): AssistantMessage => ({
              ...message,
              skillResults: [
                ...(message.skillResults ?? []).filter(
                  (g) => g.toolCallId !== group.toolCallId,
                ),
                group,
              ],
            });
            setStream((previous) =>
              apply(
                previous?.id === messageId
                  ? previous
                  : { ...blankReply(event.runId), id: messageId },
              ),
            );
            setDetail((previous) =>
              previous
                ? {
                    ...previous,
                    messages: previous.messages.map((m) =>
                      m.id === messageId ? apply(m) : m,
                    ),
                  }
                : previous,
            );
            return;
          }
          case "approval":
            setLocalApprovals((old) => [
              ...old.filter((a) => a.data.approvalId !== event.data.approvalId),
              event,
            ]);
            return;
          case "approval_end":
            setLocalApprovals((old) =>
              old.filter((a) => a.data.approvalId !== event.data.approvalId),
            );
            return;
          case "task_plan":
            setLocalPlan(event.data.steps);
            return;
          // The run's cost is shown with its reply, once saved.
          case "task_status":
            if (event.data.warning) setLocalWarning(event.data.warning);
            return;
          case "run_start": {
            if (event.data.compactOnly) {
              setCompacting(true);
              return;
            }
            // The reply this run writes, and whether it got web search.
            const { messageId, webSearch } = event.data;
            if (typeof messageId !== "string" || typeof webSearch !== "boolean")
              return;
            const sent = runSearchRef.current;
            const requested =
              sent?.runId === event.runId ? sent.requested : webSearch;
            setRunSearch({ runId: event.runId, requested, got: webSearch });
            if (requested && !webSearch)
              setSearchSkipped((old) => new Set(old).add(messageId));
            setStream((previous) => ({
              ...(previous ?? blankReply(event.runId)),
              id: messageId,
            }));
            return;
          }
          case "steer_applied": {
            endThinking(event.runId);
            writeThinking(null);
            const { messages, messageId } = event.data;
            setDetail((old) =>
              old && old.session.id === event.sessionId
                ? {
                    ...old,
                    messages: [
                      ...old.messages.filter(
                        (m) => !messages.some((next) => next.id === m.id),
                      ),
                      ...messages.filter((message) => message.id !== messageId),
                    ].sort((a, b) => a.createdAt - b.createdAt),
                  }
                : old,
            );
            setStream({ ...blankReply(event.runId), id: messageId });
            return;
          }
          case "thinking_start":
            doneReading(event.runId);
            startThinking(event.runId);
            return;
          case "thinking_end":
            endThinking(event.runId);
            return;
          case "text_delta":
          case "thinking_delta": {
            const { delta } = event.data;
            if (typeof delta !== "string") return;
            doneReading(event.runId);
            if (event.type === "text_delta") endThinking(event.runId);
            queueDelta(
              event.runId,
              event.type === "thinking_delta" ? "thinking" : "text",
              delta,
            );
            return;
          }
          case "usage": {
            // The conversation's running totals; its cost is read once the
            // run is saved.
            const { usage } = event.data;
            setDetail((old) =>
              old?.session.id === event.sessionId ? { ...old, usage } : old,
            );
            return;
          }
          case "attachment": {
            // How a file of this turn was read: kept on the file and on the
            // question it was sent with until the saved transcript is read.
            const { id, status, code, details } = event.data;
            if (typeof id !== "string") return;
            setReading((old) =>
              old?.runId === event.runId
                ? { ...old, ids: old.ids.filter((item) => item !== id) }
                : old,
            );
            const read = (file: AssistantAttachment) =>
              file.id === id
                ? { ...file, status, reason: code, details }
                : file;
            const pending = `pending-${event.operationId}`;
            setDetail((old) =>
              old?.session.id === event.sessionId
                ? {
                    ...old,
                    attachments: old.attachments.map(read),
                    messages: old.messages.map((message) =>
                      message.role === "user" &&
                      message.attachments &&
                      (message.id === pending || message.runId === event.runId)
                        ? {
                            ...message,
                            attachments: message.attachments.map(read),
                          }
                        : message,
                    ),
                  }
                : old,
            );
            return;
          }
          case "tool_start": {
            const { name, toolCallId } = event.data;
            setLocalIntent(event.data.intent ?? null);
            doneReading(event.runId);
            endThinking(event.runId);
            const activity =
              typeof name === "string" ? TOOL_ACTIVITY.get(name) : undefined;
            if (!activity || typeof toolCallId !== "string") return;
            setTools((running) => [
              ...running.filter((tool) => tool.id !== toolCallId),
              { id: toolCallId, activity },
            ]);
            return;
          }
          case "tool_end": {
            const { toolCallId } = event.data;
            setTools((running) =>
              running.filter((tool) => tool.id !== toolCallId),
            );
            return;
          }
          case "sources": {
            // All of the reply's sources so far, already saved.
            const { sources } = event.data;
            if (!Array.isArray(sources)) return;
            setStream((previous) => ({
              ...(previous ?? blankReply(event.runId)),
              sources,
            }));
            return;
          }
          case "compaction_start":
            setCompacting(true);
            return;
          case "compaction_end":
            setCompacting(false);
            return;
          case "message": {
            // The saved reply; anything but an assistant message is internal.
            const { message } = event.data;
            if (
              ["assistant", "media"].includes(message?.role) &&
              typeof message.id === "string" &&
              typeof message.content === "string"
            ) {
              // The saved reply carries how long it thought.
              writeThinking(null);
              setStream(message);
            }
            return;
          }
          case "error":
            // A stop sends no error; a failure is shown by its code alone.
            messageQueue.pause(event.sessionId);
            endThinking(event.runId);
            showError(event.data);
            return;
          case "run_end": {
            if (event.data.status !== "completed" || event.data.stopped)
              messageQueue.pause(event.sessionId);
            writeThinking(null);
            setRun(null);
            setCompacting(false);
            const regenerate =
              regenerating.current?.runId === event.runId
                ? regenerating.current
                : null;
            if (regenerate) regenerating.current = null;
            // A successful event alone is not enough to dispatch another run:
            // its saved history must be visible first. A failed read pauses.
            void refreshDetail(event.sessionId).then((loaded) => {
              if (generation !== epoch.current || !mounted.current) return;
              if (loaded && selectedRef.current === event.sessionId) {
                if (regenerate?.cancelled)
                  setRestoreVersion({
                    sessionId: regenerate.sessionId,
                    questionId: regenerate.questionId,
                  });
                setStream(null);
                if (!runRef.current) messageQueue.completed(event.sessionId);
              } else messageQueue.pause(event.sessionId);
            });
            void refreshSessionsRef.current(generation);
            return;
          }
        }
      })
      .then((unsubscribe) => {
        readiness.resolve();
        if (disposed) unsubscribe();
        else {
          off = unsubscribe;
          setConnected(true);
        }
      })
      .catch((reason) => {
        readiness.resolve();
        if (!disposed) showError(reason);
      });
    return () => {
      disposed = true;
      listenerReady.current = subscriptionReadiness();
      // Words received before a new subscription still belong on screen.
      flushDeltas();
      off?.();
      setConnected(false);
    };
  }, [
    enabled,
    endThinking,
    flushDeltas,
    messageQueue,
    notifyFinish,
    queueDelta,
    refreshDetail,
    setRun,
    setRunSearch,
    showError,
    startThinking,
    writeThinking,
  ]);

  const select = useCallback(
    (id: string | null) => {
      // Creating a record for a new backup draft must finish before another
      // selection can replace the composer that ensureSession carries over.
      if (transferInFlight.current) return;
      const previous = selectedRef.current;
      // Already showing or loading it: reloading here would leave the pane on
      // "Loading…", and leaving "no conversation" for itself would drop the
      // unsent draft. Only a conversation whose load failed is read again.
      if (id === previous) {
        if (id && !loading && detail?.session.id !== id) {
          setError(null);
          loadSelected(id);
        }
        return;
      }
      if (previous) messageQueue.pause(previous);
      if (previous && drafts.current.has(previous))
        void persistDraft(
          previous,
          draftRef.current,
          attachmentDrafts.current.get(previous)?.map((file) => file.id),
        ).catch(showError);
      selectedRef.current = id;
      newLocalConfigRef.current = false;
      setNewLocalConfig(false);
      setLocalModelNotice(null);
      setLocalChanges([]);
      setLocalPlan([]);
      setLocalWarning(null);
      selectionSequence.current += 1;
      updateSequence.current += 1;
      createdSession.current = null;
      setNewWebSearch(lastWebSearch.current);
      detailSequence.current += 1;
      setBusy(Boolean(id && draftScopes.current.get(id)?.held));
      setSelectedId(id);
      setLoading(Boolean(id));
      setDetail(null);
      dropDeltas();
      setStream(null);
      writeThinking(null);
      setNewThinking(null);
      setCompacting(false);
      const pending = [...pendingSends.current.values()].find(
        (item) => item.sessionId === id,
      );
      setCompacting(pending?.compactOnly === true);
      setRun(
        pending
          ? { runId: pending.runId, operationId: pending.operationId }
          : null,
      );
      draftRef.current = id ? (drafts.current.get(id) ?? "") : "";
      setDraftState(draftRef.current);
      skillRef.current = skillDrafts.current.get(id ?? "");
      setSkillState(skillRef.current);
      skillVersion.current += 1;
      attachmentsRef.current = id
        ? (attachmentDrafts.current.get(id) ?? [])
        : [];
      setAttachmentsState(attachmentsRef.current);
      setError(null);
    },
    [
      loading,
      detail,
      loadSelected,
      messageQueue,
      dropDeltas,
      setRun,
      setNewThinking,
      setNewWebSearch,
      persistDraft,
      showError,
      writeThinking,
    ],
  );

  /**
   * Show the trash or the conversations: none of the other list stays on
   * screen, and no read of it still on its way is applied, while the list of
   * this one is read (the view changing reads it).
   */
  const switchList = useCallback((deleted: boolean) => {
    listSequence.current += 1;
    setSessions([]);
    setUnreadable([]);
    setMatches({});
    showTrashRef.current = deleted;
    setShowTrash(deleted);
  }, []);

  const viewTrash = useCallback(
    (deleted: boolean) => {
      if (transferInFlight.current || deleted === showTrash) return;
      select(null);
      switchList(deleted);
    },
    [select, showTrash, switchList],
  );

  const ensureSession = useCallback(async () => {
    if (selectedRef.current) {
      if (detail?.session.id === selectedRef.current) return detail.session;
      throw new Error("assistant_session_loading");
    }
    const generation = epoch.current;
    const selection = selectionSequence.current;
    let value = await assistantApi.createSession({
      modelId: newModelId || undefined,
      // Remembered even while some models are missing; a default is not.
      ...(newModelPicked ? { modelPicked: true } : {}),
      webSearch: newWebSearchRef.current,
      ...(newLocalConfigRef.current
        ? { capabilities: [{ id: "local_config", version: 1 }] }
        : {}),
    });
    // A new conversation takes no thinking level when it is created: the one
    // chosen for it is saved right after, if its model still offers it.
    const level = newThinkingRef.current;
    const createdModel = bootstrapRef.current?.models.find(
      (item) => item.id === value.modelId,
    );
    let thinkingFailure: unknown;
    if (level && currentThinking(createdModel, level) === level) {
      try {
        value = await assistantApi.updateSession({
          sessionId: value.id,
          thinkingLevel: level,
          ...expectedIdentity(bootstrapRef.current),
        });
      } catch (reason) {
        thinkingFailure = reason;
      }
    }
    if (
      !mounted.current ||
      generation !== epoch.current ||
      selection !== selectionSequence.current
    )
      throw new Error("assistant_request_expired");
    if (thinkingFailure !== undefined) showError(thinkingFailure);
    const chosenMedia = newMediaRef.current;
    if (Object.keys(chosenMedia).length > 0) {
      try {
        value = await assistantApi.updateSession({
          sessionId: value.id,
          media: { ...value.media, ...chosenMedia },
          ...expectedIdentity(bootstrapRef.current),
        });
        setNewMedia({});
      } catch (reason) {
        showError(reason);
      }
    }
    setNewThinking(null);
    selectedRef.current = value.id;
    createdSession.current = value.id;
    setNewWebSearch(lastWebSearch.current);
    drafts.current.set(value.id, draftRef.current);
    skillDrafts.current.set(value.id, skillRef.current);
    skillVersions.current.set(value.id, skillVersion.current);
    skillDrafts.current.delete("");
    attachmentDrafts.current.set(value.id, attachmentsRef.current);
    setSelectedId(value.id);
    setDetail({ session: value, messages: [], attachments: [], usage: {} });
    void refreshSessionsRef.current();
    return value;
  }, [
    detail,
    newModelId,
    newModelPicked,
    setNewThinking,
    setNewWebSearch,
    showError,
  ]);

  const prepareAudio = useCallback(async () => {
    if (
      operationBusyRef.current ||
      transferInFlight.current ||
      runRef.current ||
      loading ||
      showTrashRef.current
    )
      return null;
    const generation = epoch.current;
    setBusy(true);
    try {
      const session = await ensureSession();
      if (
        !mounted.current ||
        generation !== epoch.current ||
        selectedRef.current !== session.id
      )
        return null;
      return {
        sessionId: session.id,
        ...requireIdentity(bootstrapRef.current),
        selectionKey: selectionSequence.current,
      };
    } catch (reason) {
      if (generation === epoch.current) showError(reason);
      return null;
    } finally {
      if (mounted.current && generation === epoch.current) setBusy(false);
    }
  }, [ensureSession, loading, setBusy, showError]);

  const insertAudioDraft = useCallback(
    (target: AssistantPasteTarget & { selectionKey: number }, text: string) => {
      const identity = bootstrapRef.current;
      if (
        signedOut.current ||
        selectedRef.current !== target.sessionId ||
        selectionSequence.current !== target.selectionKey ||
        identity?.owner !== target.expectedOwner ||
        identity.authorityGeneration !== target.expectedGeneration ||
        detail?.session.deletedAt != null
      )
        return false;
      const draft = draftRef.current;
      const next = `${draft}${draft && !draft.endsWith("\n") ? "\n" : ""}${text}`;
      if (new TextEncoder().encode(next).length > MAX_MESSAGE_BYTES) {
        showError("assistant_message_too_large");
        return false;
      }
      setDraft(next);
      return true;
    },
    [detail?.session.deletedAt, setDraft, showError],
  );

  const create = useCallback(async () => {
    if (busy) return;
    // From "no conversation" this keeps the unsent draft; ensureSession carries it over.
    setShowTrash(false);
    select(null);
    setError(null);
    if (!newModelId) return;
    setBusy(true);
    const selection = selectionSequence.current;
    const generation = epoch.current;
    try {
      await ensureSession();
    } catch (reason) {
      if (
        selection === selectionSequence.current &&
        generation === epoch.current
      )
        showError(reason);
    } finally {
      if (
        mounted.current &&
        selection === selectionSequence.current &&
        generation === epoch.current
      )
        setBusy(false);
    }
  }, [busy, newModelId, ensureSession, select, showError]);

  const rememberModel = useCallback((modelId: string) => {
    setBootstrap((old) => {
      if (!old) return old;
      const model = old.models.find((item) => item.id === modelId);
      if (!model) return old;
      const kind = model.kind ?? "chat";
      if (!["chat", "image", "video"].includes(kind)) return old;
      return {
        ...old,
        rememberedModels: { ...old.rememberedModels, [kind]: modelId },
      };
    });
  }, []);

  const update = useCallback(
    async (patch: Omit<AssistantUpdateRequest, "sessionId">) => {
      const sessionId = selectedRef.current;
      if (!sessionId) return;
      if (patch.modelId) messageQueue.pause(sessionId);
      const generation = epoch.current;
      const sequence = ++updateSequence.current;
      try {
        const value = await assistantApi.updateSession({
          sessionId,
          ...patch,
          ...expectedIdentity(bootstrapRef.current),
        });
        if (
          !mounted.current ||
          generation !== epoch.current ||
          selectedRef.current !== sessionId ||
          sequence !== updateSequence.current
        )
          return;
        setDetail((old) => (old ? { ...old, session: value } : old));
        if (patch.modelId) {
          setChosenModel(value.modelId);
          rememberModel(value.modelId);
        }
        if (value.cleanupWarning) showError(value.cleanupWarning);
        void refreshSessionsRef.current();
        return value;
      } catch (reason) {
        if (generation === epoch.current && sequence === updateSequence.current)
          showError(reason);
      }
    },
    [showError, rememberModel, messageQueue],
  );

  /**
   * Move a conversation to the trash or back. `sid` names it: the open one by
   * default, or another one (an undo made after the selection moved on). The
   * other one is not the open one's business: the busy state, the composer
   * draft, the selection and the list being shown are left as they are.
   */
  const setDeleted = useCallback(
    async (deleted: boolean, sid: string | null = selectedRef.current) => {
      if (!sid) return false;
      const open = sid === selectedRef.current;
      if (open ? busy || loading : transferInFlight.current) return false;
      const generation = epoch.current;
      messageQueue.pause(sid);
      if (open) {
        setBusy(true);
        setError(null);
      }
      try {
        if (open)
          await persistDraft(
            sid,
            draftRef.current,
            attachmentsRef.current.map((file) => file.id),
          );
        if (
          generation !== epoch.current ||
          (open && selectedRef.current !== sid)
        )
          return false;
        const saved = await assistantApi.updateSession({
          sessionId: sid,
          deleted,
          ...expectedIdentity(bootstrapRef.current),
        });
        if (generation !== epoch.current) return false;
        if (selectedRef.current === sid) select(null);
        // Restored, the conversation goes back to the conversations view, whose
        // list is read once the view has changed: reading the trash now would
        // flash the restored conversation among the trashed ones.
        if (open && !deleted && showTrashRef.current) switchList(false);
        else void refreshSessionsRef.current();
        if (saved.cleanupWarning) showError(saved.cleanupWarning);
        return true;
      } catch (reason) {
        if (generation === epoch.current) showError(reason);
        return false;
      } finally {
        if (
          open &&
          generation === epoch.current &&
          (!selectedRef.current || selectedRef.current === sid)
        )
          setBusy(false);
      }
    },
    [busy, loading, persistDraft, select, switchList, showError, messageQueue],
  );

  /**
   * Pin a conversation to the top of the list, or unpin it: the open one or
   * any other. Resolves true once it is saved.
   */
  const setPinned = useCallback(
    async (sessionId: string, pinned: boolean) => {
      const generation = epoch.current;
      try {
        const value = await assistantApi.updateSession({
          sessionId,
          pinned,
          ...expectedIdentity(bootstrapRef.current),
        });
        if (!mounted.current || generation !== epoch.current) return false;
        setDetail((old) =>
          old?.session.id === sessionId ? { ...old, session: value } : old,
        );
        void refreshSessionsRef.current();
        return true;
      } catch (reason) {
        if (generation === epoch.current) showError(reason);
        return false;
      }
    },
    [showError],
  );

  const deleteSession = useCallback(async () => {
    const sid = selectedRef.current;
    const identity = bootstrapRef.current;
    if (
      !sid ||
      busy ||
      loading ||
      !identity?.owner ||
      identity.authorityGeneration === undefined
    )
      return false;
    const generation = epoch.current;
    setBusy(true);
    setError(null);
    // Invalidate queued saves, then wait for a write already in progress.
    draftScopes.current.set(sid, { held: true });
    const forget = () => {
      drafts.current.delete(sid);
      attachmentDrafts.current.delete(sid);
      savedDrafts.current.delete(sid);
      messageQueue.clear(sid);
      if (selectedRef.current === sid) select(null);
    };
    try {
      await draftWrites.current.get(sid)?.catch(() => undefined);
      if (generation !== epoch.current) return false;
      await assistantApi.deleteSession({
        sessionId: sid,
        expectedOwner: identity.owner,
        expectedGeneration: identity.authorityGeneration,
      });
      if (generation !== epoch.current) return false;
      forget();
      void refreshSessionsRef.current();
      return true;
    } catch (reason) {
      if (generation === epoch.current) {
        // The namespace move succeeded; only disk removal needs a retry.
        if (assistantErrorCode(reason) === "assistant_delete_pending") forget();
        void refreshSessionsRef.current();
        showError(reason);
      }
      return false;
    } finally {
      if (generation === epoch.current) {
        draftScopes.current.set(sid, { held: false });
        if (!selectedRef.current || selectedRef.current === sid) setBusy(false);
      }
    }
  }, [busy, loading, select, showError, messageQueue]);

  /**
   * Delete every conversation in the trash for good. Resolves with what was
   * done, or null when nothing was attempted or the request failed (the
   * failure is shown). Some may stay: see `AssistantEmptyTrashReport`.
   */
  const emptyTrash = useCallback(async () => {
    const identity = bootstrapRef.current;
    if (
      busy ||
      loading ||
      !identity?.owner ||
      identity.authorityGeneration === undefined
    )
      return null;
    const generation = epoch.current;
    setBusy(true);
    setError(null);
    try {
      const report = await assistantApi.emptyTrash({
        expectedOwner: identity.owner,
        expectedGeneration: identity.authorityGeneration,
      });
      if (generation !== epoch.current) return null;
      // A trashed conversation that is open may be gone now. Its composer is
      // read-only, so there is no draft to keep, and saving one for a
      // conversation that no longer exists would fail.
      const open = selectedRef.current;
      if (open && showTrashRef.current) {
        drafts.current.delete(open);
        attachmentDrafts.current.delete(open);
        savedDrafts.current.delete(open);
        select(null);
      }
      void refreshSessionsRef.current();
      return report;
    } catch (reason) {
      if (generation === epoch.current) {
        void refreshSessionsRef.current();
        showError(reason);
      }
      return null;
    } finally {
      if (generation === epoch.current) setBusy(false);
    }
  }, [busy, loading, select, showError]);

  const transfer = useCallback(
    async (kind: "backup" | "restore", excludeVideos = false) => {
      const identity = bootstrapRef.current;
      if (
        busy ||
        transferInFlight.current ||
        loading ||
        !identity?.owner ||
        identity.authorityGeneration === undefined
      )
        return;
      const generation = epoch.current;
      const current = () =>
        mounted.current &&
        generation === epoch.current &&
        bootstrapRef.current?.owner === identity.owner;
      transferInFlight.current = true;
      setTransferring(true);
      setTransferResult(null);
      setError(null);
      try {
        // Another conversation is sending, or importing files: a backup or
        // restore now would meet it half done.
        if ([...draftScopes.current.values()].some((scope) => scope.held))
          throw new Error(TRANSFER_BUSY);
        // A new unsent composer has no store record yet. Preserve it too, even
        // when the user subsequently cancels the file dialog.
        if (
          !selectedRef.current &&
          (draftRef.current.trim() || attachmentsRef.current.length)
        ) {
          // Its record is an active conversation, not one in the trash the
          // page may be showing: show the conversations.
          if (showTrashRef.current) switchList(false);
          await ensureSession();
        }
        for (const [sid, text] of drafts.current) {
          if (!current()) return;
          if (
            !(await persistDraft(
              sid,
              text,
              attachmentDrafts.current.get(sid)?.map((file) => file.id),
            ))
          )
            throw new Error(TRANSFER_BUSY);
        }
        if (!current()) return;
        const request = {
          expectedOwner: identity.owner,
          expectedGeneration: identity.authorityGeneration,
        };
        if (kind === "backup") {
          const report = await assistantApi.backup(
            excludeVideos ? { ...request, excludeVideos: true } : request,
          );
          if (current() && report) setTransferResult({ kind, ...report });
        } else {
          const report = await assistantApi.restore(request);
          if (!current() || !report) return;
          setTransferResult({ kind, ...report });
          // The archive may have filled in settings this profile never saved.
          loadPreferences();
          // An empty profile may have acquired its model preference from the
          // archive. Read it before the next create can overwrite it; a
          // restore that imported nothing leaves the user's own choice alone.
          if (report.imported > 0) {
            const restored = await loadBootstrap();
            if (!current()) return;
            setChosenModel(null);
            if (!restored) {
              // Do not let an old default overwrite the restored preference
              // while its refresh is unavailable. A refresh can retry the read.
              bootstrapRef.current = null;
              setBootstrap(null);
              return;
            }
          }
          await refreshSessionsRef.current();
          if (!current()) return;
          if (selectedRef.current) await refreshDetail(selectedRef.current);
          if (current() && report.warning) showError(report.warning);
        }
      } catch (reason) {
        if (current()) showError(reason);
      } finally {
        if (generation === epoch.current && mounted.current) {
          transferInFlight.current = false;
          setTransferring(false);
        }
      }
    },
    [
      busy,
      loading,
      ensureSession,
      persistDraft,
      loadBootstrap,
      loadPreferences,
      refreshDetail,
      switchList,
      showError,
    ],
  );

  /** Save the draft before reading the export snapshot; native cancellation is quiet. */
  const exportMarkdown = useCallback(
    async (format: (snapshot: AssistantSessionDetail) => string) => {
      const sid = selectedRef.current;
      const identity = bootstrapRef.current;
      if (
        !sid ||
        busy ||
        loading ||
        runRef.current ||
        !identity?.owner ||
        identity.authorityGeneration === undefined
      )
        return false;
      const generation = epoch.current;
      const selection = selectionSequence.current;
      const current = () =>
        mounted.current &&
        generation === epoch.current &&
        selection === selectionSequence.current &&
        selectedRef.current === sid;
      setBusy(true);
      setError(null);
      try {
        if (
          !(await persistDraft(
            sid,
            draftRef.current,
            attachmentsRef.current.map((file) => file.id),
          ))
        )
          return false;
        if (!current()) return false;
        const snapshot = await assistantApi.getSession(sid);
        if (!current()) return false;
        const saved = await assistantApi.exportMarkdown({
          sessionId: sid,
          expectedOwner: identity.owner,
          expectedGeneration: identity.authorityGeneration,
          markdown: format(snapshot),
        });
        return current() && saved;
      } catch (reason) {
        if (current()) showError(reason);
        return false;
      } finally {
        if (current()) setBusy(false);
      }
    },
    [busy, loading, persistDraft, showError],
  );

  /**
   * The generation defaults and chips of the open conversation, or of the
   * one the first message creates. Shown at once; saved for the account's
   * next conversation too.
   */
  const setMedia = useCallback(
    (media: AssistantMediaDefaults) => {
      if (!selectedRef.current) {
        setNewMedia(media);
        return;
      }
      setDetail((old) =>
        old ? { ...old, session: { ...old.session, media } } : old,
      );
      void update({ media });
    },
    [update],
  );

  /** Make a result card's generation again; null when it was refused. */
  const regenerateMedia = useCallback(
    async (
      message: AssistantMessage,
      item: AssistantGeneration,
      input: {
        prompt: string;
        model: string;
        params: Record<string, unknown>;
        confirmedCostUsd?: string | null;
        confirmUnknownCost?: boolean;
      },
    ): Promise<AssistantRegenerateResult | null> => {
      const sessionId = selectedRef.current;
      if (!sessionId) return null;
      try {
        return await assistantApi.regenerateMedia({
          sessionId,
          operationId: freshId(),
          messageId: message.id,
          toolCallId: item.toolCallId,
          ...input,
          ...requireIdentity(bootstrapRef.current),
        });
      } catch (reason) {
        showError(reason);
        return null;
      }
    },
    [showError],
  );

  /** Cancel or resume a generation tool's video. */
  const generationVideo = useCallback(
    (
      action: "cancel" | "resume",
      message: AssistantMessage,
      item: AssistantGeneration,
    ) => {
      const sessionId = selectedRef.current;
      if (!sessionId) return Promise.resolve();
      const request = {
        sessionId,
        messageId: message.id,
        toolCallId: item.toolCallId,
        ...requireIdentity(bootstrapRef.current),
      };
      return action === "cancel"
        ? assistantApi.cancelVideo(request)
        : assistantApi.resumeVideo(request);
    },
    [],
  );

  /**
   * Web search for the open conversation, or for the one the first message
   * creates. A running reply goes on as it started; the next send uses it.
   * A change the open conversation refused is not the last choice a new
   * conversation starts from.
   */
  const setWebSearch = useCallback(
    (on: boolean) => {
      if (!selectedRef.current) {
        lastWebSearch.current = on;
        setNewWebSearch(on);
        return;
      }
      void update({ webSearch: on }).then((saved) => {
        if (saved) lastWebSearch.current = saved.webSearch;
      });
    },
    [update, setNewWebSearch],
  );

  const setLocalConfig = useCallback(
    async (on: boolean) => {
      if (busy || loading || runRef.current) return false;
      const definition = bootstrapRef.current?.localConfigDefinition;
      const currentModel = detailRef.current?.session.modelId ?? newModelId;
      const recommended =
        on && !definition?.suitableModels.includes(currentModel)
          ? bootstrapRef.current?.models.find(
              (m) =>
                m.id === definition?.recommendedModel &&
                (m.kind ?? "chat") === "chat" &&
                m.tools !== false,
            )
          : undefined;
      if (!selectedRef.current) {
        newLocalConfigRef.current = on;
        setNewLocalConfig(on);
        if (recommended) {
          setChosenModel(recommended.id);
          setNewThinking(null);
        }
      } else {
        const saved = await update({
          capabilities: on ? [{ id: "local_config", version: 1 }] : [],
          ...(recommended ? { modelId: recommended.id } : {}),
        });
        if (!saved) return false;
        setLocalModes((old) => ({ ...old, [saved.id]: "default" }));
      }
      setLocalModelNotice(
        on
          ? recommended
            ? "modelSwitched"
            : !definition?.suitableModels.includes(currentModel)
              ? "modelUnconfigured"
              : null
          : null,
      );
      return true;
    },
    [busy, loading, newModelId, update, setNewThinking],
  );

  const localConfig =
    detail?.session.capabilities.some((c) => c.id === "local_config") ??
    newLocalConfig;
  useEffect(() => {
    setLocalChanges([]);
    if (!localConfig || !selectedId || run) return;
    const authority = bootstrapRef.current;
    if (!authority?.owner) return;
    let active = true;
    void assistantApi
      .localChanges({ sessionId: selectedId, ...requireIdentity(authority) })
      .then((value) => {
        if (active) setLocalChanges(value);
      })
      .catch((reason) => {
        if (active) showError(reason);
      });
    return () => {
      active = false;
    };
  }, [localConfig, selectedId, run, showError]);

  const setLocalPermission = useCallback(
    async (mode: AssistantPermissionMode, confirmedFullAccess = false) => {
      if (busy || loading || runRef.current) return false;
      const generation = epoch.current;
      try {
        const session = await ensureSession();
        const saved = await assistantApi.setPermission({
          sessionId: session.id,
          mode,
          confirmedFullAccess,
          ...requireIdentity(bootstrapRef.current),
        });
        if (generation !== epoch.current || selectedRef.current !== session.id)
          return false;
        setLocalModes((old) => ({ ...old, [session.id]: saved }));
        return true;
      } catch (reason) {
        if (generation === epoch.current) showError(reason);
        return false;
      }
    },
    [busy, loading, ensureSession, showError],
  );

  const respondLocalApproval = useCallback(
    async (id: string, choice: AssistantApprovalChoice) => {
      const event = localApprovals.find((a) => a.data.approvalId === id);
      if (!event) return false;
      try {
        await assistantApi.respondApproval({
          sessionId: event.sessionId,
          runId: event.runId,
          operationId: event.operationId,
          approvalId: id,
          choice,
          expectedOwner: event.data.expectedOwner,
          expectedGeneration: event.data.expectedGeneration,
        });
        setLocalApprovals((old) => old.filter((a) => a !== event));
        // A regenerate whose generation is declined is called off: nothing
        // more is asked of the model, and the version it replaced comes back.
        const regenerate = regenerating.current;
        if (choice === "deny" && regenerate?.runId === event.runId) {
          regenerate.cancelled = true;
          void stopRef.current();
        }
        return true;
      } catch (reason) {
        if (selectedRef.current === event.sessionId) showError(reason);
        return false;
      }
    },
    [localApprovals, showError],
  );

  const restoreLocalChanges = useCallback(async () => {
    const sid = selectedRef.current;
    const generation = epoch.current;
    if (!sid || busy || loading || runRef.current) return false;
    setBusy(true);
    try {
      const value = await assistantApi.restoreLocalChanges({
        sessionId: sid,
        ...requireIdentity(bootstrapRef.current),
      });
      if (generation !== epoch.current || selectedRef.current !== sid)
        return false;
      setLocalChanges(value);
      return true;
    } catch (reason) {
      if (generation === epoch.current && selectedRef.current === sid)
        showError(reason);
      return false;
    } finally {
      if (generation === epoch.current) setBusy(false);
    }
  }, [busy, loading, showError]);

  const startLocalConfig = useCallback(
    async (prompt: string) => {
      if (busy || loading || transferInFlight.current) return false;
      const generation = epoch.current;
      const identity = requireIdentity(bootstrapRef.current);
      const definition = bootstrapRef.current?.localConfigDefinition;
      const recommended = bootstrapRef.current?.models.find(
        (m) =>
          m.id === definition?.recommendedModel &&
          (m.kind ?? "chat") === "chat" &&
          m.tools !== false,
      );
      try {
        const session = await assistantApi.createSession({
          modelId: recommended?.id ?? newModelId,
          webSearch: false,
          capabilities: [{ id: "local_config", version: 1 }],
        });
        await assistantApi.updateSession({
          sessionId: session.id,
          draft: prompt,
          ...identity,
        });
        if (generation !== epoch.current) return false;
        drafts.current.set(session.id, prompt);
        select(session.id);
        return true;
      } catch (reason) {
        if (generation === epoch.current) showError(reason);
        return false;
      }
    },
    [busy, loading, newModelId, select, showError],
  );

  /**
   * Thinking for the open conversation, or for the one the first message
   * creates. It never stops a running reply: the next send uses it.
   */
  const setThinkingLevel = useCallback(
    (choice: ThinkingChoice) => {
      if (!selectedRef.current) {
        setNewThinking(choice === "default" ? null : choice);
        return;
      }
      void update({ thinkingLevel: choice });
    },
    [update, setNewThinking],
  );

  /** A model chosen for the next new conversation; its thinking choice starts over. */
  const chooseNewModel = useCallback(
    (modelId: string) => {
      setChosenModel(modelId);
      rememberModel(modelId);
      setNewThinking(null);
    },
    [setNewThinking, rememberModel],
  );

  /**
   * Send the composer's draft and files, edit or regenerate an earlier turn,
   * or (`text`, with the question's `attachments`) send a stored question
   * again as a new turn, leaving the draft.
   */

  const send = useCallback(
    async (options?: {
      editMessageId?: string;
      regenerateMessageId?: string;
      /** A failed or stopped reply to ask again for, with `text` and `attachments`. */
      retryMessageId?: string;
      text?: string;
      skill?: string;
      replaceSkill?: boolean;
      attachments?: AssistantAttachment[];
      media?: AssistantImageRequest;
      video?: AssistantVideoRequest;
      quoteId?: string;
      /** Internal ordinary-queue admission; never forwarded to the runtime. */
      queueTicket?: QueueTicket;
    }) => {
      const { editMessageId, regenerateMessageId, retryMessageId } =
        options ?? {};
      const text = options?.text ?? draftRef.current;
      const fromComposer = options?.text === undefined;
      const chosenSkill = fromComposer ? skillRef.current : options?.skill;
      const originalSkill = skillRef.current;
      let clearedSkillVersion: number | undefined;
      const revision = Boolean(editMessageId || regenerateMessageId);
      // A regenerate reads its turn's own files again. An edit or a question
      // sent again carries the files its caller chose, leaving out those that
      // can't be read; the composer's files are sent only when all of them can
      // be.
      const chosen = regenerateMessageId
        ? []
        : options?.video
          ? (options.attachments ?? [])
          : fromComposer
            ? attachmentsRef.current
            : (options?.attachments ?? []);
      const files = fromComposer ? chosen : chosen.filter(isSendable);
      // What the run reads first: a regenerate reads its question's files.
      const regenerated = regenerateMessageId
        ? detail?.messages.find((message) => message.id === regenerateMessageId)
        : undefined;
      const question = regenerated
        ? detail?.messages.find(
            (message) =>
              message.role === "user" &&
              Boolean(message.runId) &&
              message.runId === regenerated.runId,
          )
        : undefined;
      const read = regenerated ? (question?.attachments ?? []) : files;
      if (
        busy ||
        operationBusyRef.current ||
        transferInFlight.current ||
        loading ||
        showTrash ||
        detail?.session.deletedAt != null ||
        runRef.current ||
        files.some((file) => !isSendable(file)) ||
        (!text.trim() && !files.length && !regenerateMessageId)
      )
        return false;
      setBusy(true);
      setError(null);
      const originalDraft = draftRef.current;
      const originalFiles = attachmentsRef.current;
      let sessionId: string | undefined;
      let preparingRunId: string | undefined;
      let draftScope: DraftWriteScope | undefined;
      let preparation: PendingSend | undefined;
      const generation = epoch.current;
      const selection = selectionSequence.current;
      const ticket = options?.queueTicket;
      const queueCurrent = () =>
        !ticket ||
        (!ticket.cancelled &&
          mounted.current &&
          activeRef.current &&
          generation === epoch.current &&
          selection === selectionSequence.current &&
          selectedRef.current === ticket.sessionId &&
          messageQueue.get(ticket.sessionId)?.sending === ticket &&
          bootstrapRef.current?.models.some(
            (model) =>
              model.id === modelRef.current &&
              (model.kind ?? "chat") === "chat",
          ));
      try {
        const authority = requireIdentity(bootstrapRef.current);
        const session = await ensureSession();
        sessionId = session.id;
        if (
          !mounted.current ||
          generation !== epoch.current ||
          selectedRef.current !== sessionId ||
          !queueCurrent()
        )
          return false;
        if (!ticket) messageQueue.manualRun(sessionId);
        if (fromComposer) {
          if (draftScopes.current.get(sessionId)?.held)
            throw new Error("assistant_session_busy");
          draftScope = { held: true };
          draftScopes.current.set(sessionId, draftScope);
        }
        const reference = { runId: freshId(), operationId: freshId() };
        preparingRunId = reference.runId;
        // Only a version the model kept can be shown again.
        regenerating.current = question?.nativeEntryId
          ? {
              runId: reference.runId,
              sessionId,
              questionId: question.id,
              cancelled: false,
            }
          : null;
        preparation = { ...reference, sessionId, cancelled: false };
        pendingSends.current.set(reference.runId, preparation);
        detailSequence.current += 1;
        setRun(reference);
        setRunSearch({
          runId: reference.runId,
          requested: session.webSearch,
          got: null,
        });
        setReading({
          runId: reference.runId,
          ids: read.map((file) => file.id),
        });
        setStream({
          id: `stream-${reference.runId}`,
          role: "assistant",
          content: "",
          createdAt: Date.now(),
          status: "streaming",
          ...(options?.video
            ? {
                media: {
                  kind: "video" as const,
                  model: session.modelId,
                  params: options.video,
                  inputIds: files.map((file) => file.id),
                  status: "submitting" as const,
                  outputs: [],
                  cost: null,
                },
              }
            : options?.media
              ? {
                  media: {
                    kind: "image" as const,
                    model: session.modelId,
                    params: options.media,
                    inputIds: files.map((file) => file.id),
                    status: "submitting" as const,
                    outputs: [],
                    cost: null,
                  },
                }
              : {}),
        });
        if (fromComposer) {
          setDraft("");
          setSkill(undefined);
          clearedSkillVersion = skillVersion.current;
          setAttachments([]);
        }
        if (!regenerateMessageId)
          setDetail((old) =>
            old
              ? {
                  ...old,
                  messages: [
                    ...old.messages,
                    {
                      id: `pending-${reference.operationId}`,
                      role: "user",
                      content: text,
                      skill: chosenSkill,
                      createdAt: Date.now(),
                      revisionOf: editMessageId,
                      attachments: files,
                    },
                  ],
                }
              : old,
          );
        if (fromComposer)
          await persistDraft(
            sessionId,
            originalDraft,
            originalFiles.map((file) => file.id),
            draftScope,
          );
        if (generation !== epoch.current)
          throw new Error("assistant_request_expired");
        if (!queueCurrent()) preparation.cancelled = true;
        if (preparation.cancelled) throw new Error("assistant_cancelled");
        await assistantApi.send({
          sessionId,
          ...reference,
          ...authority,
          text,
          ...(chosenSkill ? { skill: chosenSkill } : {}),
          ...(options?.replaceSkill ? { replaceSkill: true } : {}),
          attachmentIds: files.map((file) => file.id),
          ...(options?.media ? { media: options.media } : {}),
          ...(options?.video
            ? { video: options.video, quoteId: options.quoteId }
            : {}),
          ...(editMessageId ? { editMessageId, replaceAttachments: true } : {}),
          ...(regenerateMessageId ? { regenerateMessageId } : {}),
          ...(retryMessageId ? { retryMessageId } : {}),
        });
        // Admission is over even if saving the next draft still takes time.
        // A return to this conversation must trust the backend's run state.
        pendingSends.current.delete(reference.runId);
        if (draftScope && draftScopes.current.get(sessionId) === draftScope) {
          draftScopes.current.set(sessionId, { held: false });
          draftScope = undefined;
        }
        // The backend has registered the run: a conversation read that began
        // before then may still answer without it, and must not end it here.
        // (Unless its run_end came first and already cleared it.)
        const following = () => runRef.current?.runId === reference.runId;
        if (following()) detailSequence.current += 1;
        // The send obtained the assistant's key, minting it the first time.
        const identity = bootstrapRef.current;
        if (
          generation === epoch.current &&
          identity?.key.status === "missing"
        ) {
          bootstrapRef.current = { ...identity, key: { status: "ready" } };
          setBootstrap(bootstrapRef.current);
        }
        if (!revision && generation === epoch.current) {
          savedDrafts.current.delete(sessionId);
          await persistDraft(
            sessionId,
            drafts.current.get(sessionId) ?? "",
            (attachmentDrafts.current.get(sessionId) ?? []).map(
              (file) => file.id,
            ),
          ).catch(showError);
        }
        if (
          generation === epoch.current &&
          selectedRef.current === sessionId &&
          selection !== selectionSequence.current
        )
          await refreshDetail(sessionId);
        return true;
      } catch (reason) {
        if (regenerating.current?.runId === preparingRunId)
          regenerating.current = null;
        if (fromComposer && sessionId && generation === epoch.current) {
          if (!drafts.current.get(sessionId))
            drafts.current.set(sessionId, originalDraft);
          if (
            clearedSkillVersion !== undefined &&
            skillVersions.current.get(sessionId) === clearedSkillVersion
          )
            skillDrafts.current.set(sessionId, originalSkill);
          if (!attachmentDrafts.current.get(sessionId)?.length)
            attachmentDrafts.current.set(sessionId, originalFiles);
        }
        if (
          mounted.current &&
          generation === epoch.current &&
          (sessionId
            ? selectedRef.current === sessionId
            : selection === selectionSequence.current)
        ) {
          setRun(null);
          setStream(null);
          setDetail((old) =>
            old
              ? {
                  ...old,
                  messages: old.messages.filter(
                    (item) => !item.id.startsWith("pending-"),
                  ),
                }
              : old,
          );
          if (fromComposer && !draftRef.current) setDraft(originalDraft);
          if (
            fromComposer &&
            sessionId &&
            clearedSkillVersion !== undefined &&
            skillVersions.current.get(sessionId) === clearedSkillVersion
          )
            setSkill(originalSkill);
          if (fromComposer && !attachmentsRef.current.length)
            setAttachments(originalFiles);
          if (!preparation?.cancelled) showError(reason);
        }
        return false;
      } finally {
        if (preparingRunId) pendingSends.current.delete(preparingRunId);
        if (sessionId && draftScope)
          await releaseDraft(sessionId, draftScope, generation);
        if (
          mounted.current &&
          generation === epoch.current &&
          (sessionId
            ? selectedRef.current === sessionId
            : selection === selectionSequence.current)
        )
          setBusy(false);
      }
    },
    [
      detail,
      busy,
      loading,
      showTrash,
      ensureSession,
      setAttachments,
      setDraft,
      setRun,
      setRunSearch,
      showError,
      persistDraft,
      releaseDraft,
      refreshDetail,
      messageQueue,
    ],
  );

  const queue = selectedId ? queues.get(selectedId) : undefined;
  const chatModel = bootstrap?.models.some(
    (model) =>
      model.id === (detail?.session.modelId ?? newModelId) &&
      (model.kind ?? "chat") === "chat",
  );
  const queueAvailable = Boolean(
    enabled &&
      interactionActive &&
      connected &&
      bootstrap?.runtime.available &&
      bootstrap.owner &&
      bootstrap.authorityGeneration !== undefined &&
      chatModel &&
      !busy &&
      !loading &&
      !showTrash &&
      detail?.session.id === selectedId &&
      detail?.session.deletedAt == null,
  );

  /** Only text is queued; attachment imports remain disabled during a reply. */
  const enqueue = useCallback(() => {
    const sid = selectedRef.current;
    if (
      !queueAvailable ||
      operationBusyRef.current ||
      !sid ||
      attachmentsRef.current.length ||
      (!runRef.current && !messageQueue.get(sid)?.messages.length)
    )
      return false;
    if (!messageQueue.enqueue(sid, draftRef.current, skillRef.current))
      return false;
    setDraft("");
    setSkill(undefined);
    return true;
  }, [queueAvailable, messageQueue, setDraft]);

  const steerSending = useRef(false);
  const steer = useCallback(async () => {
    const active = runRef.current;
    const sid = selectedRef.current;
    const text = draftRef.current;
    if (
      !active ||
      !sid ||
      compacting ||
      !queueAvailable ||
      steerSending.current ||
      attachmentsRef.current.length ||
      skillRef.current ||
      !text.trim() ||
      new TextEncoder().encode(text).length > MAX_MESSAGE_BYTES
    )
      return false;
    const generation = epoch.current;
    const selection = selectionSequence.current;
    const draftVersion = draftSequence.current;
    steerSending.current = true;
    try {
      const authority = requireIdentity(bootstrapRef.current);
      await assistantApi.steer({
        sessionId: sid,
        ...active,
        ...authority,
        id: crypto.randomUUID(),
        text,
      });
      if (
        mounted.current &&
        generation === epoch.current &&
        selection === selectionSequence.current &&
        sid === selectedRef.current &&
        draftRef.current === text &&
        draftVersion === draftSequence.current
      )
        setDraft("");
      return true;
    } catch (reason) {
      if (generation === epoch.current && sid === selectedRef.current)
        showError(reason);
      return false;
    } finally {
      steerSending.current = false;
    }
  }, [queueAvailable, compacting, setDraft, showError]);

  /** An explicit resume rechecks the saved conversation before any send. */
  const resumeQueue = useCallback(async () => {
    const sid = selectedRef.current;
    if (!sid || !queueAvailable || operationBusyRef.current || runRef.current)
      return;
    const snapshot = messageQueue.get(sid);
    if (!snapshot?.messages.length || snapshot.sending) return;
    const generation = epoch.current;
    const selection = selectionSequence.current;
    const current = () =>
      mounted.current &&
      activeRef.current &&
      generation === epoch.current &&
      selection === selectionSequence.current &&
      selectedRef.current === sid &&
      messageQueue.get(sid) === snapshot;
    setBusy(true);
    try {
      if ((await refreshDetail(sid)) && current() && !runRef.current) {
        setStream(null);
        messageQueue.resume(sid);
      }
    } finally {
      if (
        mounted.current &&
        generation === epoch.current &&
        selection === selectionSequence.current
      )
        setBusy(false);
    }
  }, [queueAvailable, messageQueue, refreshDetail, setBusy]);

  useEffect(() => {
    if (
      !queueAvailable ||
      !selectedId ||
      operationBusyRef.current ||
      runRef.current ||
      stream ||
      !queue?.ready ||
      queue.paused ||
      queue.sending
    )
      return;
    const ticket = messageQueue.claim(selectedId);
    if (!ticket) return;
    void send({
      text: ticket.message.text,
      skill: ticket.message.skill,
      attachments: [],
      queueTicket: ticket,
    }).then((sent) => messageQueue.settle(ticket, sent));
  }, [queueAvailable, selectedId, queue, stream, run, send, messageQueue]);

  /**
   * Leave a conversation this window just created for a pick that imported
   * nothing: back to no conversation, with the unsent words, and the empty
   * conversation goes to the trash.
   */
  const discardCreated = useCallback(
    async (sessionId: string) => {
      const text = draftRef.current;
      // Nothing of it is saved on the way out: it is about to be trashed.
      drafts.current.delete(sessionId);
      attachmentDrafts.current.delete(sessionId);
      select(null);
      setDraft(text);
      await assistantApi.updateSession({
        sessionId,
        deleted: true,
        ...expectedIdentity(bootstrapRef.current),
      });
      void refreshSessionsRef.current();
    },
    [select, setDraft],
  );

  /**
   * Import files into the open conversation and add them to the draft, saved
   * at once. With no conversation open one is created first, and trashed
   * again when nothing is imported. `importFiles` does the import; one that
   * stops early with a failure keeps the files it had imported, and the
   * failure is told. `onStart` is called once the import is under way, never
   * for one that is refused because another is.
   */
  const importAttachments = useCallback(
    async (importFiles: ImportFiles, onStart?: () => void) => {
      if (
        importing.current ||
        busy ||
        loading ||
        runRef.current ||
        showTrash ||
        detail?.session.deletedAt != null
      )
        return;
      importing.current = true;
      setBusy(true);
      setError(null);
      onStart?.();
      const generation = epoch.current;
      const selection = selectionSequence.current;
      const creates = !selectedRef.current;
      let sessionId: string | undefined;
      let scope: DraftWriteScope | undefined;
      // Files are in the conversation: it is no longer empty.
      let imported = false;
      const current = () =>
        mounted.current &&
        generation === epoch.current &&
        selectedRef.current === sessionId;
      try {
        const id = (await ensureSession()).id;
        sessionId = id;
        if (!current()) return;
        if (draftScopes.current.get(id)?.held)
          throw new Error("assistant_session_busy");
        scope = { held: true };
        draftScopes.current.set(id, scope);
        // Drain earlier writes and commit removals before the native import can
        // add its own saved references. Autosave/leave writes cannot pass it.
        await persistDraft(
          id,
          draftRef.current,
          attachmentsRef.current.map((file) => file.id),
          scope,
        );
        if (!current()) return;
        const { added, failure } = await importFiles(id);
        if (generation !== epoch.current) return;
        if (!added.length) {
          // Nothing came in: the failure is the whole outcome.
          if (failure !== undefined) throw failure;
          if (creates && current()) await discardCreated(id).catch(showError);
          return;
        }
        imported = true;
        const existing = attachmentDrafts.current.get(id) ?? [];
        const files = [
          ...existing,
          ...added.filter(
            (file) => !existing.some((item) => item.id === file.id),
          ),
        ];
        attachmentDrafts.current.set(id, files);
        if (current()) {
          setAttachments(files);
          setDetail((old) =>
            old?.session.id === id
              ? {
                  ...old,
                  attachments: [
                    ...old.attachments,
                    ...added.filter(
                      (file) =>
                        !old.attachments.some((item) => item.id === file.id),
                    ),
                  ],
                }
              : old,
          );
        }
        const text = drafts.current.get(id) ?? "";
        const ids = files.map((file) => file.id);
        // Import already committed these files. Forget the pre-import snapshot
        // so a local removal or an unchanged empty text cannot skip this write.
        savedDrafts.current.delete(id);
        await persistDraft(id, text, ids, scope);
        if (current())
          setDetail((old) =>
            old?.session.id === id
              ? {
                  ...old,
                  session: {
                    ...old.session,
                    draft: text,
                    draftAttachmentIds: ids,
                  },
                }
              : old,
          );
        if (failure !== undefined && current()) showError(failure);
      } catch (reason) {
        if (
          generation !== epoch.current ||
          (sessionId
            ? selectedRef.current !== sessionId
            : selection !== selectionSequence.current)
        )
          return;
        // A failure after the import (saving the draft) keeps the conversation
        // that holds the files; only one that is still empty goes to the trash.
        const created =
          creates && !imported && sessionId && current() ? sessionId : null;
        const discarding = created ? discardCreated(created) : null;
        showError(reason);
        await discarding?.catch(showError);
      } finally {
        if (sessionId && scope)
          await releaseDraft(sessionId, scope, generation);
        if (
          mounted.current &&
          generation === epoch.current &&
          (sessionId
            ? selectedRef.current === sessionId
            : selection === selectionSequence.current)
        )
          setBusy(false);
        importing.current = false;
      }
    },
    [
      busy,
      loading,
      showTrash,
      detail?.session.deletedAt,
      ensureSession,
      discardCreated,
      persistDraft,
      releaseDraft,
      setAttachments,
      showError,
    ],
  );

  /** Open the file dialog and add the files it imports to the draft. */
  const pickAttachments = useCallback(
    () =>
      importAttachments(async (sessionId) => ({
        added: await assistantApi.pickAttachments(sessionId),
      })),
    [importAttachments],
  );

  /**
   * Add the files dropped on the window to the draft. `token` is the one the
   * `assistant:drop` event carried: the backend holds the paths.
   */
  const dropAttachments = useCallback(
    (token: string) =>
      importAttachments(async (sessionId) => ({
        added: await assistantApi.importDrop({
          sessionId,
          token,
          ...requireIdentity(bootstrapRef.current),
        }),
      })),
    [importAttachments],
  );

  /**
   * Add pasted files to the draft, one import each, in order. The first one
   * that fails ends it: the files before it stay, the rest are not tried.
   * `onStart` is called once the import is under way.
   */
  const pasteAttachments = useCallback(
    (files: File[], onStart?: () => void) =>
      importAttachments(async (sessionId) => {
        const added: AssistantAttachment[] = [];
        try {
          const target = {
            sessionId,
            ...requireIdentity(bootstrapRef.current),
          };
          for (const file of files)
            added.push(...(await assistantApi.importPasted(file, target)));
        } catch (failure) {
          return { added, failure };
        }
        return { added };
      }, onStart),
    [importAttachments],
  );

  /** Saving the new draft removes unused copies; files in message history stay. */
  const removeAttachment = useCallback(
    (id: string) =>
      setAttachments(attachmentsRef.current.filter((file) => file.id !== id)),
    [setAttachments],
  );

  const stop = useCallback(async () => {
    const active = runRef.current;
    const sessionId = selectedRef.current;
    if (!active || !sessionId) return;
    messageQueue.pause(sessionId);
    const generation = epoch.current;
    const preparing = pendingSends.current.get(active.runId);
    if (preparing) preparing.cancelled = true;
    if (regenerating.current?.runId === active.runId)
      regenerating.current.cancelled = true;
    try {
      await assistantApi.stop(sessionId, active.runId, active.operationId);
    } catch (reason) {
      if (generation === epoch.current && selectedRef.current === sessionId)
        showError(reason);
    }
  }, [showError, messageQueue]);
  useEffect(() => {
    stopRef.current = stop;
  }, [stop]);

  // Set at once: a second press that comes before the render that disables
  // the switcher must not start a second switch.
  const switchInFlight = useRef(false);
  /**
   * Show another version of a turn: the one whose question is `messageId`.
   * The conversation is busy meanwhile (nothing else is sent, and the
   * backend refuses a switch while a reply runs). Resolves true once the
   * conversation shows it.
   */
  const switchVersion = useCallback(
    async (messageId: string) => {
      const sid = selectedRef.current;
      const identity = bootstrapRef.current;
      if (
        !sid ||
        busy ||
        loading ||
        showTrash ||
        detail?.session.deletedAt != null ||
        runRef.current ||
        switchInFlight.current ||
        !identity?.owner ||
        identity.authorityGeneration === undefined
      )
        return false;
      const generation = epoch.current;
      const selection = selectionSequence.current;
      const current = () =>
        mounted.current &&
        generation === epoch.current &&
        selection === selectionSequence.current &&
        selectedRef.current === sid;
      messageQueue.pause(sid);
      switchInFlight.current = true;
      setBusy(true);
      setError(null);
      try {
        const value = await assistantApi.switchVersion({
          sessionId: sid,
          messageId,
          expectedOwner: identity.owner,
          expectedGeneration: identity.authorityGeneration,
        });
        if (!current()) return false;
        // A read that began before the switch would bring the old version back.
        detailSequence.current += 1;
        setDetail(value);
        void refreshSessionsRef.current();
        return true;
      } catch (reason) {
        if (current()) showError(reason);
        return false;
      } finally {
        switchInFlight.current = false;
        if (current()) setBusy(false);
      }
    },
    [
      busy,
      loading,
      showTrash,
      detail?.session.deletedAt,
      showError,
      messageQueue,
    ],
  );

  // A regenerate called off: once nothing runs, the version it would have
  // replaced is shown again.
  useEffect(() => {
    if (!restoreVersion || busy || loading || run) return;
    setRestoreVersion(null);
    if (selectedRef.current === restoreVersion.sessionId)
      void switchVersion(restoreVersion.questionId);
  }, [restoreVersion, busy, loading, run, switchVersion]);

  const forkSession = useCallback(
    async (messageId: string, title: string) => {
      const sid = selectedRef.current,
        identity = bootstrapRef.current;
      if (
        !sid ||
        busy ||
        loading ||
        showTrash ||
        detail?.session.deletedAt != null ||
        runRef.current ||
        switchInFlight.current ||
        !identity?.owner ||
        identity.authorityGeneration === undefined
      )
        return false;
      const generation = epoch.current,
        selection = selectionSequence.current;
      const current = () =>
        mounted.current &&
        generation === epoch.current &&
        selection === selectionSequence.current &&
        selectedRef.current === sid;
      messageQueue.pause(sid);
      switchInFlight.current = true;
      setBusy(true);
      setError(null);
      try {
        const value = await assistantApi.forkSession({
          sessionId: sid,
          messageId,
          title,
          expectedOwner: identity.owner,
          expectedGeneration: identity.authorityGeneration,
        });
        if (!current()) return false;
        select(value.session.id);
        void refreshSessionsRef.current();
        return true;
      } catch (reason) {
        if (current()) showError(reason);
        return false;
      } finally {
        switchInFlight.current = false;
        if (current()) setBusy(false);
      }
    },
    [
      busy,
      loading,
      showTrash,
      detail?.session.deletedAt,
      messageQueue,
      select,
      showError,
    ],
  );

  const compact = useCallback(async () => {
    const sid = selectedRef.current;
    const identity = bootstrapRef.current;
    if (
      !sid ||
      !identity?.owner ||
      identity.authorityGeneration === undefined ||
      busy ||
      loading ||
      showTrash ||
      runRef.current ||
      switchInFlight.current ||
      detail?.session.deletedAt != null ||
      !detail?.messages.length ||
      !chatModel
    )
      return false;
    const generation = epoch.current,
      selection = selectionSequence.current;
    const current = () =>
      mounted.current &&
      generation === epoch.current &&
      selection === selectionSequence.current &&
      selectedRef.current === sid;
    const active = { runId: freshId(), operationId: freshId() };
    const preparing: PendingSend = {
      ...active,
      sessionId: sid,
      cancelled: false,
      compactOnly: true,
    };
    pendingSends.current.set(active.runId, preparing);
    messageQueue.pause(sid);
    switchInFlight.current = true;
    setBusy(true);
    setError(null);
    setRun(active);
    setCompacting(true);
    try {
      await assistantApi.compact({
        sessionId: sid,
        ...active,
        expectedOwner: identity.owner,
        expectedGeneration: identity.authorityGeneration,
      });
      if (preparing.cancelled) {
        if (current())
          await assistantApi.stop(sid, active.runId, active.operationId);
        return false;
      }
      return current();
    } catch (reason) {
      if (current()) {
        setRun(null);
        setCompacting(false);
        if (!preparing.cancelled) showError(reason);
      }
      return false;
    } finally {
      if (pendingSends.current.get(active.runId) === preparing)
        pendingSends.current.delete(active.runId);
      switchInFlight.current = false;
      if (current()) setBusy(false);
    }
  }, [
    busy,
    loading,
    showTrash,
    detail,
    chatModel,
    messageQueue,
    setRun,
    showError,
  ]);

  // The latest tool still running says what the reply is doing; before its
  // model request, a run reads the files it was sent with.
  const activity =
    tools[tools.length - 1]?.activity ??
    (reading?.runId === run?.runId && reading?.ids.length
      ? "readingFile"
      : null);
  // The open conversation's setting; off while it loads.
  const webSearch = selectedId
    ? detail?.session.id === selectedId && detail.session.webSearch
    : newWebSearch;
  // The switch now differs from what the followed run asked for and from
  // what it got: the change waits for the next message.
  const followedSearch =
    runSearch && runSearch.runId === run?.runId ? runSearch : null;
  const webSearchWaits = Boolean(
    followedSearch &&
      webSearch !== followedSearch.requested &&
      webSearch !== (followedSearch.got ?? followedSearch.requested),
  );

  // The followed run's reply, while it goes on.
  const liveThinking = run && thinking?.runId === run.runId ? thinking : null;
  // The open conversation's saved level; off while it loads.
  const thinkingLevel = selectedId
    ? detail?.session.id === selectedId
      ? (detail.session.thinkingLevel ?? null)
      : null
    : newThinkingLevel;

  return {
    bootstrap,
    media: detail?.session.media ?? newMedia,
    setMedia,
    regenerateMedia,
    generationVideo,
    prepareAudio,
    insertAudioDraft,
    newModelId,
    chooseNewModel,
    thinkingLevel,
    setThinkingLevel,
    thinking: liveThinking,
    webSearch,
    setWebSearch,
    localConfig,
    setLocalConfig,
    startLocalConfig,
    localModelNotice,
    localMode: selectedId ? (localModes[selectedId] ?? "default") : "default",
    localApprovals: localApprovals.map((e) => e.data),
    localChanges,
    localPlan,
    localIntent,
    localWarning,
    setLocalPermission,
    respondLocalApproval,
    restoreLocalChanges,
    webSearchWaits,
    searchSkipped,
    search,
    setSearch,
    showTrash,
    viewTrash,
    setDeleted,
    setPinned,
    deleteSession,
    emptyTrash,
    exportMarkdown,
    transfer,
    transferring,
    transferResult,
    sessions,
    unreadable,
    trashCount,
    matches,
    selectedId,
    selectionKey: selectionSequence.current,
    detail,
    draft,
    setDraft,
    skill,
    setSkill,
    queue,
    queueAvailable,
    enqueue,
    steer,
    resumeQueue,
    pauseQueue: () => {
      if (selectedRef.current) messageQueue.pause(selectedRef.current);
    },
    clearQueue: () => {
      if (selectedRef.current) messageQueue.clear(selectedRef.current);
    },
    editQueued: (id: string, text: string) =>
      selectedRef.current
        ? messageQueue.edit(selectedRef.current, id, text)
        : false,
    removeQueued: (id: string) => {
      if (selectedRef.current) messageQueue.remove(selectedRef.current, id);
    },
    attachments,
    pickAttachments,
    dropAttachments,
    pasteAttachments,
    removeAttachment,
    stream,
    activity,
    run,
    compacting,
    retryAttempt,
    error,
    errorDetail,
    setError,
    loading,
    busy,
    connected,
    select,
    create,
    update,
    send,
    stop,
    switchVersion,
    forkSession,
    compact,
    refreshSessions,
    refreshDetail,
    loadBootstrap,
    showError,
    preferences,
    reloadPreferences,
    savePreferences,
  };
}
