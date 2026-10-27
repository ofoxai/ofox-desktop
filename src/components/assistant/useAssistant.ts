import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type SetStateAction,
} from "react";
import { listen } from "@tauri-apps/api/event";
import {
  assistantApi,
  type AssistantApproval,
  type AssistantApprovalDecision,
  type AssistantAttachment,
  type AssistantBootstrap,
  type AssistantEvent,
  type AssistantKind,
  type AssistantMemory,
  type AssistantMessage,
  type AssistantOperation,
  type AssistantRunRef,
  type AssistantSession,
  type AssistantSessionDetail,
  type AssistantUpdateRequest,
} from "@/lib/api/assistant";

const freshId = () => crypto.randomUUID();

/** All asynchronous replies are fenced by the selected session and component lifetime. */
export function useAssistant(enabled: boolean) {
  const [bootstrap, setBootstrap] = useState<AssistantBootstrap | null>(null);
  const [kind, setKind] = useState<AssistantKind>("general");
  const [newModels, setNewModels] = useState<
    Partial<Record<AssistantKind, string>>
  >({});
  const newModelId =
    newModels[kind] ??
    bootstrap?.defaultModelIds?.[kind] ??
    bootstrap?.defaultModelId ??
    "";
  const [deleted, setDeleted] = useState(false);
  const [search, setSearch] = useState("");
  const [sessions, setSessions] = useState<AssistantSession[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<AssistantSessionDetail | null>(null);
  const [draft, setDraftState] = useState("");
  const [attachments, setAttachmentsState] = useState<AssistantAttachment[]>(
    [],
  );
  const [stream, setStream] = useState<AssistantMessage | null>(null);
  const [run, setRunState] = useState<AssistantRunRef | null>(null);
  const [operations, setOperations] = useState<AssistantOperation[]>([]);
  const [approval, setApproval] = useState<AssistantApproval | null>(null);
  const [compacting, setCompacting] = useState(false);
  const [error, setError] = useState<string | null>(null);
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
  const kindRef = useRef(kind);
  const drafts = useRef(new Map<string, string>());
  const attachmentDrafts = useRef(new Map<string, AssistantAttachment[]>());
  const attachmentsRef = useRef<AssistantAttachment[]>([]);
  const draftWrites = useRef(new Map<string, Promise<void>>());
  const draftAuthority = useRef(0);
  const savedDrafts = useRef(new Map<string, string>());
  const readOnlyDrafts = useRef(new Set<string>());
  const bootstrapRef = useRef<AssistantBootstrap | null>(null);
  const pendingSends = useRef(new Map<string, { cancelled: boolean }>());
  const refreshSessionsRef = useRef<() => Promise<void>>(async () => undefined);
  selectedRef.current = selectedId;
  kindRef.current = kind;
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
    if (mounted.current)
      setError(value instanceof Error ? value.message : String(value));
  }, []);

  const setAttachments = useCallback(
    (value: SetStateAction<AssistantAttachment[]>) => {
      const next =
        typeof value === "function" ? value(attachmentsRef.current) : value;
      attachmentsRef.current = next;
      if (selectedRef.current)
        attachmentDrafts.current.set(selectedRef.current, next);
      setAttachmentsState(next);
    },
    [],
  );

  const persistDraft = useCallback(
    (sessionId: string, text: string, files: AssistantAttachment[]) => {
      const generation = draftAuthority.current;
      const identity = bootstrapRef.current;
      const task = (draftWrites.current.get(sessionId) ?? Promise.resolve())
        .catch(() => undefined)
        .then(async () => {
          if (generation !== draftAuthority.current) return;
          const snapshot = JSON.stringify([text, files.map((file) => file.id)]);
          if (
            readOnlyDrafts.current.has(sessionId) ||
            savedDrafts.current.get(sessionId) === snapshot
          )
            return;
          await assistantApi.updateSession({
            sessionId,
            draft: text,
            draftAttachmentIds: files.map((file) => file.id),
            ...(identity?.owner && identity.authorityGeneration !== undefined
              ? {
                  expectedOwner: identity.owner,
                  expectedGeneration: identity.authorityGeneration,
                }
              : {}),
          });
          if (generation === draftAuthority.current)
            savedDrafts.current.set(sessionId, snapshot);
        });
      draftWrites.current.set(sessionId, task);
      return task;
    },
    [],
  );

  const flushDrafts = useCallback(async () => {
    await Promise.all(
      [...drafts.current].map(([sessionId, text]) =>
        persistDraft(
          sessionId,
          text,
          attachmentDrafts.current.get(sessionId) ?? [],
        ),
      ),
    );
  }, [persistDraft]);

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
      if (generation === epoch.current) showError(reason);
    }
  }, [showError]);

  const refreshSessions = useCallback(async () => {
    const sequence = ++listSequence.current;
    const generation = epoch.current;
    try {
      const rows = await assistantApi.listSessions(kind, deleted, search);
      if (
        !mounted.current ||
        generation !== epoch.current ||
        sequence !== listSequence.current
      )
        return;
      setSessions(rows);
    } catch (reason) {
      if (sequence === listSequence.current && generation === epoch.current)
        showError(reason);
    }
  }, [kind, deleted, search, showError]);
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
        const resumed = value.session.activeRun
          ? value.messages.find(
              (message) =>
                message.role === "assistant" && message.status === "streaming",
            )
          : undefined;
        savedDrafts.current.set(
          sessionId,
          JSON.stringify([
            value.session.draft ?? "",
            value.session.draftAttachmentIds ?? [],
          ]),
        );
        if (value.session.deletedAt) readOnlyDrafts.current.add(sessionId);
        else readOnlyDrafts.current.delete(sessionId);
        const alreadyFollowing = Boolean(runRef.current);
        setDetail(
          resumed
            ? {
                ...value,
                messages: value.messages.filter(
                  (message) => message.id !== resumed.id,
                ),
              }
            : value,
        );
        if (resumed && !alreadyFollowing) setStream(resumed);
        setRun(value.session.activeRun ?? null);
        setApproval(value.pendingApproval ?? null);
        setOperations(value.operations ?? []);
        if (!drafts.current.has(sessionId)) setDraft(value.session.draft ?? "");
        if (!attachmentDrafts.current.has(sessionId)) {
          const ids = value.session.draftAttachmentIds ?? [];
          setAttachments(
            ids.flatMap(
              (id) => value.attachments?.find((file) => file.id === id) ?? [],
            ),
          );
        }
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
    [setDraft, setAttachments, setRun, showError],
  );

  const savedDraft = detail?.session.draft ?? "";
  const savedDraftAttachments = JSON.stringify(
    detail?.session.draftAttachmentIds ?? [],
  );
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
      readOnlyDrafts.current.clear();
      setSelectedId(null);
      setDetail(null);
      setSessions([]);
      setAttachments([]);
      setStream(null);
      setApproval(null);
      setOperations([]);
      setCompacting(false);
      setBusy(false);
      setLoading(false);
      setRun(null);
      setDraft("");
      setBootstrap(null);
      bootstrapRef.current = null;
      setNewModels({});
      setError(null);
    };
    if (!enabled) {
      reset();
      return;
    }
    void loadBootstrap();
    for (const name of ["ofox-auth-expired", "ofox-auth-restored"]) {
      void listen(name, () => {
        reset();
        if (name === "ofox-auth-restored") {
          void loadBootstrap();
          void refreshSessionsRef.current();
        }
      })
        .then((off) => (disposed ? off() : cleanup.push(off)))
        .catch(showError);
    }
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
    const ids = attachments.map((file) => file.id);
    if (
      !selectedId ||
      detailSessionId !== selectedId ||
      (draft === savedDraft && JSON.stringify(ids) === savedDraftAttachments)
    )
      return;
    const generation = epoch.current;
    const sequence = ++draftSequence.current;
    const timer = setTimeout(() => {
      void persistDraft(selectedId, draft, attachments)
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
                    session: { ...old.session, draft, draftAttachmentIds: ids },
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
    attachments,
    detailSessionId,
    savedDraft,
    savedDraftAttachments,
    persistDraft,
    showError,
  ]);

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let off: (() => void) | undefined;
    void assistantApi
      .subscribe((event: AssistantEvent) => {
        const current = runRef.current;
        if (
          !current ||
          event.sessionId !== selectedRef.current ||
          event.runId !== current.runId ||
          event.operationId !== current.operationId
        )
          return;
        const data = event.data ?? {};
        if (event.type === "text_delta" || event.type === "thinking_delta") {
          if (typeof data.delta !== "string") return;
          const delta = data.delta;
          setStream((previous) => {
            const value = previous ?? {
              id: `stream-${event.runId}`,
              role: "assistant" as const,
              content: "",
              createdAt: Date.now(),
              status: "streaming" as const,
            };
            return event.type === "text_delta"
              ? { ...value, content: value.content + delta }
              : { ...value, reasoning: (value.reasoning ?? "") + delta };
          });
        } else if (event.type === "sources" && Array.isArray(data.sources)) {
          setStream((previous) => ({
            ...(previous ?? {
              id: `stream-${event.runId}`,
              role: "assistant",
              content: "",
              createdAt: Date.now(),
            }),
            sources: data.sources,
          }));
        } else if (
          event.type === "approval" ||
          event.type === "approval_required"
        ) {
          if (data.approval) setApproval(data.approval);
        } else if (
          event.type === "tool_start" ||
          event.type === "tool_end" ||
          event.type === "operation"
        ) {
          const operation = data.operation;
          if (operation)
            setOperations((old) => [
              ...old.filter((item) => item.id !== operation.id),
              operation,
            ]);
        } else if (event.type === "compaction_start") {
          setCompacting(true);
        } else if (event.type === "compaction_end") {
          setCompacting(false);
        } else if (event.type === "usage" && data.usage) {
          setDetail((old) => (old ? { ...old, usage: data.usage } : old));
        } else if (
          event.type === "message_end" &&
          typeof data.message === "object" &&
          data.message &&
          typeof data.message.id === "string" &&
          typeof data.message.content === "string"
        ) {
          setStream(data.message);
        } else if (event.type === "error") {
          setError(
            typeof data.message === "string"
              ? data.message
              : (data.code ?? "assistant.error"),
          );
        } else if (event.type === "run_end" || event.type === "done") {
          setRun(null);
          setApproval(null);
          setCompacting(false);
          // Keep the streamed reply until the canonical saved transcript is available.
          void refreshDetail(event.sessionId).then((loaded) => {
            if (loaded && selectedRef.current === event.sessionId)
              setStream(null);
          });
          void refreshSessionsRef.current();
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
      if (previous && drafts.current.has(previous))
        void persistDraft(
          previous,
          draftRef.current,
          attachmentsRef.current,
        ).catch(showError);
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
      attachmentsRef.current = id
        ? (attachmentDrafts.current.get(id) ?? [])
        : [];
      setAttachmentsState(attachmentsRef.current);
      setApproval(null);
      setOperations([]);
      setCompacting(false);
      setRun(null);
      draftRef.current = id ? (drafts.current.get(id) ?? "") : "";
      setDraftState(draftRef.current);
      setError(null);
    },
    [setDraft, setRun, persistDraft, showError],
  );

  const switchKind = useCallback(
    (next: AssistantKind) => {
      if (next === kindRef.current) return;
      kindRef.current = next;
      listSequence.current += 1;
      select(null);
      setKind(next);
      setDeleted(false);
      setSearch("");
      setSessions([]);
    },
    [select],
  );

  const ensureSession = useCallback(async () => {
    if (selectedRef.current) {
      if (detail?.session.id === selectedRef.current) return detail.session;
      throw new Error("assistant.sessionLoading");
    }
    const generation = epoch.current;
    const selection = selectionSequence.current;
    const value = await assistantApi.createSession({
      kind,
      modelId: newModelId || undefined,
      permissionMode: "ask",
      webSearch: false,
    });
    if (
      !mounted.current ||
      generation !== epoch.current ||
      selection !== selectionSequence.current ||
      kindRef.current !== kind
    )
      throw new Error("assistant.requestExpired");
    selectedRef.current = value.id;
    createdSession.current = value.id;
    drafts.current.set(value.id, draftRef.current);
    setSelectedId(value.id);
    setDetail({ session: value, messages: [], memories: [] });
    void refreshSessions();
    return value;
  }, [detail, kind, newModelId, refreshSessions]);

  const create = useCallback(async () => {
    if (busy) return;
    select(null);
    setDeleted(false);
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
        const identity = bootstrapRef.current;
        const value = await assistantApi.updateSession({
          sessionId,
          ...patch,
          ...(identity?.owner && identity.authorityGeneration !== undefined
            ? {
                expectedOwner: identity.owner,
                expectedGeneration: identity.authorityGeneration,
              }
            : {}),
        });
        if (
          !mounted.current ||
          generation !== epoch.current ||
          selectedRef.current !== sessionId ||
          sequence !== updateSequence.current
        )
          return;
        setDetail((old) => (old ? { ...old, session: value } : old));
        if (value.deletedAt) readOnlyDrafts.current.add(sessionId);
        else readOnlyDrafts.current.delete(sessionId);
        if (patch.modelId)
          setNewModels((old) => ({ ...old, [value.kind]: value.modelId }));
        if (patch.deleted) select(null);
        void refreshSessions();
        return value;
      } catch (reason) {
        if (generation === epoch.current && sequence === updateSequence.current)
          showError(reason);
      }
    },
    [refreshSessions, select, showError],
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
      const sentAttachments = options?.editMessageId
        ? (detail?.messages.find(
            (message) => message.id === options.editMessageId,
          )?.attachments ?? [])
        : options?.regenerateMessageId
          ? []
          : attachments;
      if (
        busy ||
        loading ||
        runRef.current ||
        sentAttachments.some((item) => item.status !== "ready") ||
        (!text.trim() &&
          !sentAttachments.length &&
          !options?.regenerateMessageId)
      )
        return false;
      setBusy(true);
      setError(null);
      const originalDraft = draftRef.current;
      const originalAttachments = attachments;
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
        setOperations([]);
        setStream({
          id: `stream-${reference.runId}`,
          role: "assistant",
          content: "",
          createdAt: Date.now(),
          status: "streaming",
        });
        if (!revision) {
          setDraft("");
          setAttachments([]);
        }
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
                      attachments: sentAttachments,
                      createdAt: Date.now(),
                      revisionOf: options?.editMessageId,
                    },
                  ],
                }
              : old,
          );
        if (!revision)
          await persistDraft(sessionId, originalDraft, originalAttachments);
        if (generation !== epoch.current)
          throw new Error("assistant.requestExpired");
        if (preparation.cancelled) throw new Error("assistant.stopped");
        await assistantApi.send({
          sessionId,
          ...reference,
          text,
          attachmentIds: sentAttachments.map((item) => item.id),
          ...options,
        });
        if (!revision && generation === epoch.current) {
          savedDrafts.current.delete(sessionId);
          await persistDraft(
            sessionId,
            drafts.current.get(sessionId) ?? "",
            attachmentDrafts.current.get(sessionId) ?? [],
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
          if (!revision) {
            if (!draftRef.current) setDraft(originalDraft);
            setAttachments((current) =>
              current.length ? current : originalAttachments,
            );
          }
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
      attachments,
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

  const pickAttachments = useCallback(async () => {
    if (busy || runRef.current) return;
    setBusy(true);
    const generation = epoch.current;
    const selection = selectionSequence.current;
    try {
      const session = await ensureSession();
      const added = await assistantApi.pickAttachments(session.id);
      if (
        mounted.current &&
        generation === epoch.current &&
        selectedRef.current === session.id
      )
        setAttachments((current) => [
          ...current,
          ...added.filter(
            (item) => !current.some((existing) => existing.id === item.id),
          ),
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
  }, [busy, ensureSession, showError]);

  const respondApproval = useCallback(
    async (decision: AssistantApprovalDecision) => {
      const current = runRef.current;
      const sessionId = selectedRef.current;
      if (!approval || !current || !sessionId) return;
      const generation = epoch.current;
      try {
        await assistantApi.respondApproval({
          sessionId,
          ...current,
          approvalId: approval.id,
          decision,
        });
        if (
          generation === epoch.current &&
          selectedRef.current === sessionId &&
          runRef.current?.operationId === current.operationId
        )
          setApproval(null);
      } catch (reason) {
        if (generation === epoch.current) showError(reason);
      }
    },
    [approval, showError],
  );

  const saveMemory = useCallback(
    async (content: string, id?: string) => {
      const sessionId = selectedRef.current;
      if (!sessionId) return;
      const generation = epoch.current;
      try {
        const memory = await assistantApi.saveMemory({
          sessionId,
          id,
          content,
        });
        if (generation === epoch.current && selectedRef.current === sessionId)
          setDetail((old) =>
            old
              ? {
                  ...old,
                  memories: [
                    ...old.memories.filter(
                      (item: AssistantMemory) => item.id !== memory.id,
                    ),
                    memory,
                  ],
                }
              : old,
          );
        if (generation === epoch.current && selectedRef.current === sessionId)
          return memory;
      } catch (reason) {
        if (generation === epoch.current) showError(reason);
      }
    },
    [showError],
  );

  const deleteMemory = useCallback(
    async (memoryId: string) => {
      const sessionId = selectedRef.current;
      if (!sessionId) return;
      const generation = epoch.current;
      try {
        await assistantApi.deleteMemory(sessionId, memoryId);
        if (generation === epoch.current && selectedRef.current === sessionId)
          setDetail((old) =>
            old
              ? {
                  ...old,
                  memories: old.memories.filter((item) => item.id !== memoryId),
                }
              : old,
          );
      } catch (reason) {
        if (generation === epoch.current) showError(reason);
      }
    },
    [showError],
  );

  return {
    bootstrap,
    newModelId,
    chooseNewModel: (modelId: string) =>
      setNewModels((old) => ({ ...old, [kind]: modelId })),
    kind,
    switchKind,
    deleted,
    setDeleted,
    search,
    setSearch,
    sessions,
    selectedId,
    detail,
    draft,
    setDraft,
    attachments,
    setAttachments,
    stream,
    run,
    operations,
    approval,
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
    pickAttachments,
    respondApproval,
    saveMemory,
    deleteMemory,
    refreshSessions,
    refreshDetail,
    loadBootstrap,
    flushDrafts,
    forgetSession: (sessionId: string) => {
      drafts.current.delete(sessionId);
      attachmentDrafts.current.delete(sessionId);
      savedDrafts.current.delete(sessionId);
      draftWrites.current.delete(sessionId);
      readOnlyDrafts.current.add(sessionId);
    },
    showError,
  };
}
