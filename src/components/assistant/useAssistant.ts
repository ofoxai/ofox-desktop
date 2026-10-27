import { useCallback, useEffect, useRef, useState } from "react";
import {
  assistantApi,
  assistantErrorCode,
  type AssistantBootstrap,
  type AssistantEvent,
  type AssistantMessage,
  type AssistantRunRef,
  type AssistantSession,
  type AssistantSessionDetail,
  type AssistantUnreadableSession,
  type AssistantUpdateRequest,
} from "@/lib/api/assistant";

const freshId = () => crypto.randomUUID();
const IDENTITY_CHANGED = "assistant_identity_changed";

/** A backend or renderer error code; the page turns it into a localized message. */
export type AssistantError = string;

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
  const [search, setSearch] = useState("");
  const [sessions, setSessions] = useState<AssistantSession[]>([]);
  const [unreadable, setUnreadable] = useState<AssistantUnreadableSession[]>(
    [],
  );
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<AssistantSessionDetail | null>(null);
  const [draft, setDraftState] = useState("");
  const [stream, setStream] = useState<AssistantMessage | null>(null);
  const [run, setRunState] = useState<AssistantRunRef | null>(null);
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
  const draftWrites = useRef(new Map<string, Promise<void>>());
  const draftAuthority = useRef(0);
  const savedDrafts = useRef(new Map<string, string>());
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

  const setRun = useCallback((value: AssistantRunRef | null) => {
    runRef.current = value;
    setRunState(value);
  }, []);

  const setDraft = useCallback((value: string) => {
    draftRef.current = value;
    if (selectedRef.current) drafts.current.set(selectedRef.current, value);
    setDraftState(value);
  }, []);

  const showError = useCallback((value: unknown) => {
    if (!mounted.current) return;
    const code = assistantErrorCode(value);
    setError(code);
    // A sign-in, sign-out or region change moved the backend's authority on:
    // the owner and generation every write carries must be read again.
    if (code === IDENTITY_CHANGED) reloadAuthority.current();
  }, []);

  const persistDraft = useCallback((sessionId: string, text: string) => {
    const generation = draftAuthority.current;
    const task = (draftWrites.current.get(sessionId) ?? Promise.resolve())
      .catch(() => undefined)
      .then(async () => {
        if (generation !== draftAuthority.current) return;
        if (savedDrafts.current.get(sessionId) === text) return;
        await assistantApi.updateSession({
          sessionId,
          draft: text,
          ...expectedIdentity(bootstrapRef.current),
        });
        if (generation === draftAuthority.current)
          savedDrafts.current.set(sessionId, text);
      });
    draftWrites.current.set(sessionId, task);
    return task;
  }, []);

  useEffect(
    () => () => {
      draftAuthority.current += 1;
    },
    [],
  );

  const loadBootstrap = useCallback(async () => {
    const generation = epoch.current;
    try {
      const value = await assistantApi.bootstrap();
      if (mounted.current && generation === epoch.current) {
        bootstrapRef.current = value;
        setBootstrap(value);
      }
    } catch (reason) {
      // Not through showError: a failed read must not ask for itself again.
      if (mounted.current && generation === epoch.current)
        setError(assistantErrorCode(reason));
    }
  }, []);
  reloadAuthority.current = () => void loadBootstrap();

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
        savedDrafts.current.set(sessionId, value.session.draft);
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
    [setDraft, setRun, showError],
  );

  const savedDraft = detail?.session.draft ?? "";
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
      setBootstrap(null);
      bootstrapRef.current = null;
      setChosenModel(null);
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
  }, [enabled, loadBootstrap, setDraft, setRun, showError]); // List refresh uses its current request fencing separately.

  useEffect(() => {
    if (!enabled) return;
    const timer = setTimeout(() => void refreshSessions(), 180);
    return () => clearTimeout(timer);
  }, [enabled, refreshSessions]);

  useEffect(() => {
    if (!enabled || !selectedId) return;
    if (createdSession.current === selectedId) {
      createdSession.current = null;
      setLoading(false);
      return;
    }
    setLoading(true);
    void refreshDetail(selectedId).finally(() => {
      if (mounted.current && selectedRef.current === selectedId)
        setLoading(false);
    });
  }, [enabled, selectedId, refreshDetail]);

  // Persist only a draft which still belongs to the displayed session. The backend also checks owner.
  useEffect(() => {
    if (!selectedId || detailSessionId !== selectedId || draft === savedDraft)
      return;
    const generation = epoch.current;
    const sequence = ++draftSequence.current;
    const timer = setTimeout(() => {
      void persistDraft(selectedId, draft)
        .then(() => {
          if (
            generation === epoch.current &&
            sequence === draftSequence.current &&
            selectedRef.current === selectedId
          )
            setDetail((old) =>
              old ? { ...old, session: { ...old.session, draft } } : old,
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
  }, [selectedId, draft, detailSessionId, savedDraft, persistDraft, showError]);

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
        // run_start, session and event types this build does not know change
        // nothing on screen: the transcript and session are read again at
        // run_end.
        switch (event.type) {
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
  }, [enabled, refreshDetail, setRun, showError]);

  const select = useCallback(
    (id: string | null) => {
      const previous = selectedRef.current;
      // Already showing it: reloading here would leave the pane on "Loading…",
      // and leaving "no conversation" for itself would drop the unsent draft.
      if (id === previous) return;
      if (previous && drafts.current.has(previous))
        void persistDraft(previous, draftRef.current).catch(showError);
      selectedRef.current = id;
      selectionSequence.current += 1;
      updateSequence.current += 1;
      createdSession.current = null;
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
      setError(null);
    },
    [setRun, persistDraft, showError],
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
      webSearch: false,
    });
    if (
      !mounted.current ||
      generation !== epoch.current ||
      selection !== selectionSequence.current
    )
      throw new Error("assistant_request_expired");
    selectedRef.current = value.id;
    createdSession.current = value.id;
    drafts.current.set(value.id, draftRef.current);
    setSelectedId(value.id);
    setDetail({ session: value, messages: [], attachments: [], usage: {} });
    void refreshSessions();
    return value;
  }, [detail, newModelId, refreshSessions]);

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

  const send = useCallback(
    async (options?: {
      editMessageId?: string;
      regenerateMessageId?: string;
      text?: string;
    }) => {
      const text = options?.text ?? draftRef.current;
      const revision = Boolean(
        options?.editMessageId || options?.regenerateMessageId,
      );
      // Attachments arrive in a later stage; an edit keeps whatever its original carried.
      const attachmentIds = options?.editMessageId
        ? (detail?.messages
            .find((message) => message.id === options.editMessageId)
            ?.attachments?.map((item) => item.id) ?? [])
        : [];
      if (
        busy ||
        loading ||
        runRef.current ||
        (!text.trim() && !attachmentIds.length && !options?.regenerateMessageId)
      )
        return false;
      setBusy(true);
      setError(null);
      const originalDraft = draftRef.current;
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
        setStream({
          id: `stream-${reference.runId}`,
          role: "assistant",
          content: "",
          createdAt: Date.now(),
          status: "streaming",
        });
        if (!revision) setDraft("");
        if (!options?.regenerateMessageId)
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
                      revisionOf: options?.editMessageId,
                    },
                  ],
                }
              : old,
          );
        if (!revision) await persistDraft(sessionId, originalDraft);
        if (generation !== epoch.current)
          throw new Error("assistant_request_expired");
        if (preparation.cancelled) throw new Error("assistant_cancelled");
        await assistantApi.send({
          sessionId,
          ...reference,
          text,
          attachmentIds,
          ...options,
        });
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
          if (!revision && !draftRef.current) setDraft(originalDraft);
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
      setDraft,
      setRun,
      showError,
      persistDraft,
    ],
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

  return {
    bootstrap,
    newModelId,
    chooseNewModel: setChosenModel,
    search,
    setSearch,
    sessions,
    unreadable,
    selectedId,
    detail,
    draft,
    setDraft,
    stream,
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
