import { useCallback, useEffect, useRef, useState } from "react";
import {
  assistantApi,
  assistantErrorCode,
  type AssistantAttachment,
  type AssistantBootstrap,
  type AssistantEvent,
  type AssistantMessage,
  type AssistantRunRef,
  type AssistantSession,
  type AssistantSessionDetail,
  type AssistantUnreadableSession,
  type AssistantUpdateRequest,
} from "@/lib/api/assistant";
import { isSendable } from "./AssistantAttachments";

const freshId = () => crypto.randomUUID();
const IDENTITY_CHANGED = "assistant_identity_changed";

/** A backend or renderer error code; the page turns it into a localized message. */
export type AssistantError = string;

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

/** All asynchronous replies are fenced by the selected session and component lifetime. */
export function useAssistant(enabled: boolean) {
  const [bootstrap, setBootstrap] = useState<AssistantBootstrap | null>(null);
  const [chosenModel, setChosenModel] = useState<string | null>(null);
  const newModelId = chosenModel ?? bootstrap?.defaultModelId ?? "";
  // Web search for the conversation the first message creates. A new
  // conversation starts with the last choice made on the switch for this
  // account in this window (off until one is made), like the model picker.
  const [newWebSearch, setNewWebSearchState] = useState(false);
  const newWebSearchRef = useRef(newWebSearch);
  const lastWebSearch = useRef(false);
  const [search, setSearch] = useState("");
  const [sessions, setSessions] = useState<AssistantSession[]>([]);
  const [unreadable, setUnreadable] = useState<AssistantUnreadableSession[]>(
    [],
  );
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<AssistantSessionDetail | null>(null);
  const [draft, setDraftState] = useState("");
  const [attachments, setAttachmentsState] = useState<AssistantAttachment[]>(
    [],
  );
  const [stream, setStream] = useState<AssistantMessage | null>(null);
  const [run, setRunState] = useState<AssistantRunRef | null>(null);
  const [tools, setTools] = useState<RunningTool[]>([]);
  const [runSearch, setRunSearchState] = useState<RunSearch | null>(null);
  const runSearchRef = useRef(runSearch);
  // Replies whose run went without the web search their conversation asked
  // for (it was unavailable when they were sent), by message id.
  const [searchSkipped, setSearchSkipped] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [compacting, setCompacting] = useState(false);
  const [error, setError] = useState<AssistantError | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [connected, setConnected] = useState(false);
  const mounted = useRef(false);
  const epoch = useRef(0);
  const listSequence = useRef(0);
  const detailSequence = useRef(0);
  const selectionSequence = useRef(0);
  const updateSequence = useRef(0);
  const draftSequence = useRef(0);
  const createdSession = useRef<string | null>(null);
  const selectedRef = useRef(selectedId);
  const runRef = useRef(run);
  const draftRef = useRef(draft);
  const drafts = useRef(new Map<string, string>());
  // The files of each conversation's draft, kept like its text.
  const attachmentsRef = useRef(attachments);
  const attachmentDrafts = useRef(new Map<string, AssistantAttachment[]>());
  const draftWrites = useRef(new Map<string, Promise<void>>());
  const draftAuthority = useRef(0);
  const savedDrafts = useRef(new Map<string, SavedDraft>());
  const bootstrapRef = useRef<AssistantBootstrap | null>(null);
  const pendingSends = useRef(new Map<string, { cancelled: boolean }>());
  const refreshSessionsRef = useRef<(generation?: number) => Promise<void>>(
    async () => undefined,
  );
  /** Sign-in expired: nothing of the account is listed until it is restored. */
  const signedOut = useRef(false);
  const reloadAuthority = useRef<() => void>(() => undefined);
  selectedRef.current = selectedId;
  draftRef.current = draft;

  const setNewWebSearch = useCallback((value: boolean) => {
    newWebSearchRef.current = value;
    setNewWebSearchState(value);
  }, []);

  const setRunSearch = useCallback((value: RunSearch | null) => {
    runSearchRef.current = value;
    setRunSearchState(value);
  }, []);

  const setRun = useCallback((value: AssistantRunRef | null) => {
    // Another run, or none: the tools the last one started are not followed.
    if (value?.runId !== runRef.current?.runId) setTools([]);
    runRef.current = value;
    setRunState(value);
  }, []);

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

  const showError = useCallback((value: unknown) => {
    if (!mounted.current) return;
    const code = assistantErrorCode(value);
    setError(code);
    // A sign-in, sign-out or region change moved the backend's authority on:
    // the owner and generation every write carries must be read again.
    if (code === IDENTITY_CHANGED) reloadAuthority.current();
  }, []);

  /** The backend's authority now, or null when it could not be read. */
  const loadBootstrap = useCallback(async () => {
    const generation = epoch.current;
    // A sign-in that completes while this read runs moves the authority on
    // once more: read it again instead of leaving the page disabled.
    for (let attempt = 0; ; attempt += 1) {
      try {
        const value = await assistantApi.bootstrap();
        if (!mounted.current || generation !== epoch.current) return null;
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
  }, []);
  reloadAuthority.current = () => void loadBootstrap();

  /**
   * Save a conversation's draft text and, when given and changed, its files.
   * Writes to one conversation run in order.
   */
  const persistDraft = useCallback(
    (sessionId: string, text: string, files?: string[]) => {
      const generation = draftAuthority.current;
      const task = (draftWrites.current.get(sessionId) ?? Promise.resolve())
        .catch(() => undefined)
        .then(async () => {
          if (generation !== draftAuthority.current) return;
          const saved = savedDrafts.current.get(sessionId);
          const newFiles =
            files && !(saved?.files && sameIds(saved.files, files))
              ? files
              : undefined;
          if (saved?.text === text && !newFiles) return;
          const write = () =>
            assistantApi.updateSession({
              sessionId,
              draft: text,
              ...(newFiles ? { draftAttachmentIds: newFiles } : {}),
              ...expectedIdentity(bootstrapRef.current),
            });
          try {
            await write();
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
            await write();
          }
          if (generation === draftAuthority.current)
            savedDrafts.current.set(sessionId, {
              text,
              files: newFiles ?? saved?.files,
            });
        });
      draftWrites.current.set(sessionId, task);
      return task;
    },
    [loadBootstrap],
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
        const list = await assistantApi.listSessions(false, search);
        if (
          !mounted.current ||
          generation !== epoch.current ||
          sequence !== listSequence.current
        )
          return;
        setSessions(list.sessions);
        setUnreadable(list.unreadable);
      } catch (reason) {
        if (sequence === listSequence.current && generation === epoch.current)
          showError(reason);
      }
    },
    [search, showError],
  );
  refreshSessionsRef.current = refreshSessions;

  const refreshDetail = useCallback(
    async (sessionId: string) => {
      const sequence = ++detailSequence.current;
      const generation = epoch.current;
      try {
        const value = await assistantApi.getSession(sessionId);
        if (
          !mounted.current ||
          generation !== epoch.current ||
          selectedRef.current !== sessionId ||
          sequence !== detailSequence.current
        )
          return false;
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
                message.role === "assistant" && message.status === "streaming",
            )
          : undefined;
        savedDrafts.current.set(sessionId, {
          text: value.session.draft,
          files: value.session.draftAttachmentIds,
        });
        const alreadyFollowing = Boolean(runRef.current);
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
        if (!preparing) setRun(value.session.activeRun);
        if (!drafts.current.has(sessionId)) setDraft(value.session.draft);
        if (!attachmentDrafts.current.has(sessionId))
          setAttachments(
            value.session.draftAttachmentIds.flatMap(
              (id) => value.attachments.find((file) => file.id === id) ?? [],
            ),
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
      attachmentDrafts.current.clear();
      savedDrafts.current.clear();
      setSelectedId(null);
      setDetail(null);
      setSessions([]);
      setUnreadable([]);
      setStream(null);
      setCompacting(false);
      setBusy(false);
      setLoading(false);
      setRun(null);
      setDraft("");
      setAttachments([]);
      setBootstrap(null);
      bootstrapRef.current = null;
      setChosenModel(null);
      lastWebSearch.current = false;
      setNewWebSearch(false);
      setRunSearch(null);
      setSearchSkipped(new Set());
      setError(null);
    };
    if (!enabled) {
      reset();
      return;
    }
    void loadBootstrap();
    void assistantApi
      .onAuthChange((state) => {
        reset();
        signedOut.current = state === "expired";
        if (state === "restored") {
          void loadBootstrap();
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
    enabled,
    loadBootstrap,
    setAttachments,
    setDraft,
    setNewWebSearch,
    setRun,
    setRunSearch,
    showError,
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
        .then(() => {
          if (
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

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let off: (() => void) | undefined;
    const blank = (runId: string): AssistantMessage => ({
      id: `stream-${runId}`,
      role: "assistant",
      content: "",
      createdAt: Date.now(),
      status: "streaming",
    });
    void assistantApi
      .subscribe((event: AssistantEvent) => {
        if (disposed) return;
        // The identity generation this event arrived under: a list read it
        // asks for is fenced on it.
        const generation = epoch.current;
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
          return;
        }
        // session and event types this build does not know change nothing
        // on screen: the transcript and session are read again at run_end.
        switch (event.type) {
          case "run_start": {
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
              ...(previous ?? blank(event.runId)),
              id: messageId,
            }));
            return;
          }
          case "text_delta":
          case "thinking_delta": {
            const { delta } = event.data;
            if (typeof delta !== "string") return;
            const thinking = event.type === "thinking_delta";
            setStream((previous) => {
              const value = previous ?? blank(event.runId);
              return thinking
                ? { ...value, reasoning: (value.reasoning ?? "") + delta }
                : { ...value, content: value.content + delta };
            });
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
              ...(previous ?? blank(event.runId)),
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
              message?.role === "assistant" &&
              typeof message.id === "string" &&
              typeof message.content === "string"
            )
              setStream(message);
            return;
          }
          case "error":
            // A stop sends no error; a failure is shown by its code alone.
            showError(event.data);
            return;
          case "run_end":
            setRun(null);
            setCompacting(false);
            // Keep the streamed reply until the canonical saved transcript is available.
            void refreshDetail(event.sessionId).then((loaded) => {
              if (loaded && selectedRef.current === event.sessionId)
                setStream(null);
            });
            void refreshSessionsRef.current(generation);
            return;
        }
      })
      .then((unsubscribe) => {
        if (disposed) unsubscribe();
        else {
          off = unsubscribe;
          setConnected(true);
        }
      })
      .catch(showError);
    return () => {
      disposed = true;
      off?.();
      setConnected(false);
    };
  }, [enabled, refreshDetail, setRun, setRunSearch, showError]);

  const select = useCallback(
    (id: string | null) => {
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
      if (previous && drafts.current.has(previous))
        void persistDraft(
          previous,
          draftRef.current,
          attachmentDrafts.current.get(previous)?.map((file) => file.id),
        ).catch(showError);
      selectedRef.current = id;
      selectionSequence.current += 1;
      updateSequence.current += 1;
      createdSession.current = null;
      setNewWebSearch(lastWebSearch.current);
      detailSequence.current += 1;
      setBusy(false);
      setSelectedId(id);
      setLoading(Boolean(id));
      setDetail(null);
      setStream(null);
      setCompacting(false);
      setRun(null);
      draftRef.current = id ? (drafts.current.get(id) ?? "") : "";
      setDraftState(draftRef.current);
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
      setRun,
      setNewWebSearch,
      persistDraft,
      showError,
    ],
  );

  const ensureSession = useCallback(async () => {
    if (selectedRef.current) {
      if (detail?.session.id === selectedRef.current) return detail.session;
      throw new Error("assistant_session_loading");
    }
    const generation = epoch.current;
    const selection = selectionSequence.current;
    const value = await assistantApi.createSession({
      modelId: newModelId || undefined,
      webSearch: newWebSearchRef.current,
    });
    if (
      !mounted.current ||
      generation !== epoch.current ||
      selection !== selectionSequence.current
    )
      throw new Error("assistant_request_expired");
    selectedRef.current = value.id;
    createdSession.current = value.id;
    setNewWebSearch(lastWebSearch.current);
    drafts.current.set(value.id, draftRef.current);
    attachmentDrafts.current.set(value.id, attachmentsRef.current);
    setSelectedId(value.id);
    setDetail({ session: value, messages: [], attachments: [], usage: {} });
    void refreshSessions();
    return value;
  }, [detail, newModelId, refreshSessions, setNewWebSearch]);

  const create = useCallback(async () => {
    if (busy) return;
    // From "no conversation" this keeps the unsent draft; ensureSession carries it over.
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

  const update = useCallback(
    async (patch: Omit<AssistantUpdateRequest, "sessionId">) => {
      const sessionId = selectedRef.current;
      if (!sessionId) return;
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
        if (patch.modelId) setChosenModel(value.modelId);
        void refreshSessions();
        return value;
      } catch (reason) {
        if (generation === epoch.current && sequence === updateSequence.current)
          showError(reason);
      }
    },
    [refreshSessions, showError],
  );

  /**
   * Web search for the open conversation, or for the one the first message
   * creates. A running reply goes on as it started; the next send uses it.
   */
  const setWebSearch = useCallback(
    (on: boolean) => {
      lastWebSearch.current = on;
      if (selectedRef.current) void update({ webSearch: on });
      else setNewWebSearch(on);
    },
    [update, setNewWebSearch],
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
      text?: string;
      attachments?: AssistantAttachment[];
    }) => {
      const { editMessageId, regenerateMessageId } = options ?? {};
      const text = options?.text ?? draftRef.current;
      const fromComposer = options?.text === undefined;
      const revision = Boolean(editMessageId || regenerateMessageId);
      // A regenerate reads its turn's own files again. An edit or a question
      // sent again carries its files, leaving out those that can't be read;
      // the composer's files are sent only when all of them can be.
      const chosen = regenerateMessageId
        ? []
        : editMessageId
          ? (detail?.messages.find((message) => message.id === editMessageId)
              ?.attachments ?? [])
          : fromComposer
            ? attachmentsRef.current
            : (options?.attachments ?? []);
      const files = fromComposer ? chosen : chosen.filter(isSendable);
      if (
        busy ||
        loading ||
        runRef.current ||
        files.some((file) => !isSendable(file)) ||
        // An edit sent without files keeps all of its original's.
        (editMessageId && chosen.length > 0 && !files.length) ||
        (!text.trim() && !files.length && !regenerateMessageId)
      )
        return false;
      setBusy(true);
      setError(null);
      const originalDraft = draftRef.current;
      const originalFiles = attachmentsRef.current;
      let sessionId: string | undefined;
      let preparingRunId: string | undefined;
      let preparation: { cancelled: boolean } | undefined;
      const generation = epoch.current;
      const selection = selectionSequence.current;
      try {
        const session = await ensureSession();
        sessionId = session.id;
        if (selectedRef.current !== sessionId) return false;
        const reference = { runId: freshId(), operationId: freshId() };
        preparingRunId = reference.runId;
        preparation = { cancelled: false };
        pendingSends.current.set(reference.runId, preparation);
        detailSequence.current += 1;
        setRun(reference);
        setRunSearch({
          runId: reference.runId,
          requested: session.webSearch,
          got: null,
        });
        setStream({
          id: `stream-${reference.runId}`,
          role: "assistant",
          content: "",
          createdAt: Date.now(),
          status: "streaming",
        });
        if (fromComposer) {
          setDraft("");
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
          );
        if (generation !== epoch.current)
          throw new Error("assistant_request_expired");
        if (preparation.cancelled) throw new Error("assistant_cancelled");
        await assistantApi.send({
          sessionId,
          ...reference,
          text,
          attachmentIds: files.map((file) => file.id),
          ...(editMessageId ? { editMessageId } : {}),
          ...(regenerateMessageId ? { regenerateMessageId } : {}),
        });
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
        return true;
      } catch (reason) {
        if (
          mounted.current &&
          generation === epoch.current &&
          selection === selectionSequence.current &&
          (!sessionId || selectedRef.current === sessionId)
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
          if (fromComposer && !attachmentsRef.current.length)
            setAttachments(originalFiles);
          if (!preparation?.cancelled) showError(reason);
        }
        return false;
      } finally {
        if (preparingRunId) pendingSends.current.delete(preparingRunId);
        if (
          mounted.current &&
          generation === epoch.current &&
          selection === selectionSequence.current
        )
          setBusy(false);
      }
    },
    [
      detail,
      busy,
      loading,
      ensureSession,
      setAttachments,
      setDraft,
      setRun,
      setRunSearch,
      showError,
      persistDraft,
    ],
  );

  /**
   * Open the file dialog and add the files it imports to the draft, creating
   * the conversation first when there is none. Cancelling changes nothing.
   */
  const pickAttachments = useCallback(async () => {
    if (busy || loading || runRef.current) return;
    setBusy(true);
    setError(null);
    const generation = epoch.current;
    const selection = selectionSequence.current;
    try {
      const session = await ensureSession();
      const added = await assistantApi.pickAttachments(session.id);
      if (
        !added.length ||
        !mounted.current ||
        generation !== epoch.current ||
        selectedRef.current !== session.id
      )
        return;
      const fresh = (list: AssistantAttachment[]) =>
        added.filter((file) => !list.some((item) => item.id === file.id));
      setDetail((old) =>
        old?.session.id === session.id
          ? {
              ...old,
              attachments: [...old.attachments, ...fresh(old.attachments)],
            }
          : old,
      );
      setAttachments([
        ...attachmentsRef.current,
        ...fresh(attachmentsRef.current),
      ]);
    } catch (reason) {
      if (
        generation === epoch.current &&
        selection === selectionSequence.current
      )
        showError(reason);
    } finally {
      if (
        mounted.current &&
        generation === epoch.current &&
        selection === selectionSequence.current
      )
        setBusy(false);
    }
  }, [busy, loading, ensureSession, setAttachments, showError]);

  /** Take a file out of the draft; it stays in the conversation's files. */
  const removeAttachment = useCallback(
    (id: string) =>
      setAttachments(attachmentsRef.current.filter((file) => file.id !== id)),
    [setAttachments],
  );

  const stop = useCallback(async () => {
    const active = runRef.current;
    const sessionId = selectedRef.current;
    if (!active || !sessionId) return;
    const generation = epoch.current;
    const preparing = pendingSends.current.get(active.runId);
    if (preparing) preparing.cancelled = true;
    try {
      await assistantApi.stop(sessionId, active.runId, active.operationId);
    } catch (reason) {
      if (generation === epoch.current && selectedRef.current === sessionId)
        showError(reason);
    }
  }, [showError]);

  // The latest tool still running says what the reply is doing.
  const activity = tools[tools.length - 1]?.activity ?? null;
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

  return {
    bootstrap,
    newModelId,
    chooseNewModel: setChosenModel,
    webSearch,
    setWebSearch,
    webSearchWaits,
    searchSkipped,
    search,
    setSearch,
    sessions,
    unreadable,
    selectedId,
    detail,
    draft,
    setDraft,
    attachments,
    pickAttachments,
    removeAttachment,
    stream,
    activity,
    run,
    compacting,
    error,
    setError,
    loading,
    busy,
    connected,
    select,
    create,
    update,
    send,
    stop,
    refreshSessions,
    refreshDetail,
    loadBootstrap,
    showError,
  };
}
