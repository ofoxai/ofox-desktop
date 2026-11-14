import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAssistant } from "@/components/assistant/useAssistant";
import { useMessageQueue } from "@/components/assistant/useMessageQueue";
import type {
  AssistantBootstrap,
  AssistantAttachment,
  AssistantEvent,
  AssistantRunRef,
  AssistantSession,
  AssistantSessionDetail,
} from "@/lib/api/assistant";

const mocks = vi.hoisted(() => ({
  bootstrap: vi.fn(),
  getPreferences: vi.fn(),
  listSessions: vi.fn(),
  getSession: vi.fn(),
  updateSession: vi.fn(),
  localChanges: vi.fn(),
  send: vi.fn(),
  stop: vi.fn(),
  subscribe: vi.fn(),
  onAuthChange: vi.fn(),
  finishNotice: vi.fn(),
  listener: undefined as ((event: AssistantEvent) => void) | undefined,
  auth: undefined as ((state: "expired" | "restored") => void) | undefined,
}));
vi.mock("@/lib/api/assistant", async (original) => ({
  ...(await original<typeof import("@/lib/api/assistant")>()),
  assistantApi: mocks,
}));
vi.mock("@/components/assistant/useFinishNotice", () => ({
  useFinishNotice: () => mocks.finishNotice,
}));

const identity = { owner: "a".repeat(64), authorityGeneration: 1 };
const bootstrap = {
  ...identity,
  runtime: { available: true, reason: null, version: "test" },
  models: [{ id: "chat", name: "Chat", kind: "chat" }],
  defaultModelId: "chat",
  modelUnavailableReason: null,
  key: { status: "ready" },
  search: { available: true },
} as AssistantBootstrap;
const session: AssistantSession = {
  id: "progress-session",
  title: "Progress",
  titleSource: "user",
  assistantId: "ofox",
  capabilities: [],
  modelId: "chat",
  webSearch: false,
  draft: "",
  draftAttachmentIds: [],
  createdAt: 1,
  updatedAt: 1,
  deletedAt: null,
  pinnedAt: null,
  activeRun: null,
};
const active = { runId: "run-a", operationId: "op-a" };
let current: AssistantSessionDetail;
let now: number;
function referenceFile() {
  current.attachments = [
    {
      id: "deck",
      name: "Deck.pptx",
      mimeType:
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
      sizeBytes: 100,
      status: "ready",
      reason: null,
      details: null,
      origin: "generated",
    },
  ];
}

beforeEach(() => {
  vi.clearAllMocks();
  now = 10_000;
  vi.spyOn(Date, "now").mockImplementation(() => now);
  current = {
    session: { ...session },
    messages: [],
    attachments: [],
    usage: {},
  };
  mocks.bootstrap.mockResolvedValue(bootstrap);
  mocks.getPreferences.mockResolvedValue({ notifyOnFinish: false });
  mocks.listSessions.mockImplementation(async () => ({
    sessions: [current.session],
    unreadable: [],
    trashCount: 0,
    matches: {},
  }));
  mocks.getSession.mockImplementation(async () => current);
  mocks.updateSession.mockImplementation(async (request) => {
    current = { ...current, session: { ...current.session, ...request } };
    return current.session;
  });
  mocks.localChanges.mockResolvedValue([]);
  mocks.send.mockResolvedValue(active);
  mocks.stop.mockResolvedValue(undefined);
  mocks.subscribe.mockImplementation(async (listener) => {
    mocks.listener = listener;
    return () => undefined;
  });
  mocks.onAuthChange.mockImplementation(async (auth) => {
    mocks.auth = auth;
    return () => undefined;
  });
});

describe("presentation choice data flow", () => {
  it.each([
    [
      "PPTX",
      "application/vnd.openxmlformats-officedocument.presentationml.presentation",
      6,
    ],
    ["HTML", "text/html", null],
  ] as const)(
    "quotes a completed generated %s from its saved canonical attachment",
    async (_kind, mimeType, page) => {
      const file: AssistantAttachment = {
        id: "generated-document",
        name: `Generated.${page === null ? "html" : "pptx"}`,
        mimeType,
        sizeBytes: 128_272,
        status: "ready",
        reason: null,
        details: null,
        origin: "generated",
        sourceMessageId: "finished-reply",
      };
      current.attachments = [file];
      current.messages = [
        {
          id: "finished-reply",
          role: "assistant",
          content: "",
          createdAt: 2,
          status: "error",
          errorCode: "assistant_cost_unavailable",
          attachments: [file],
          generations: [
            {
              toolCallId: "document-call",
              kind: "document",
              model: "",
              params: { format: page === null ? "html" : "pptx" },
              inputIds: [],
              status: "completed",
              outputs: [file.id],
              cost: "0",
            },
          ],
        },
      ];
      const hook = await open(null);
      act(() => hook.result.current.setDraft("Please revise this document"));
      emit(
        "tool_start",
        {
          messageId: "other-reply",
          toolCallId: "other-call",
          name: "create_document",
          args: {},
        },
        { runId: "other-run", operationId: "other-operation" },
        "other-session",
      );
      const reference = {
        attachmentId: file.id,
        version: "b".repeat(64),
        page,
      };
      act(() =>
        expect(hook.result.current.addFileReference(reference)).toBe(true),
      );
      expect(hook.result.current.fileReferences).toEqual([reference]);
      expect(hook.result.current.draft).toBe("Please revise this document");
      expect(mocks.send).not.toHaveBeenCalled();
    },
  );

  it.each([
    [
      "PPTX",
      "application/vnd.openxmlformats-officedocument.presentationml.presentation",
      6,
    ],
    ["HTML", "text/html", null],
  ] as const)(
    "quotes a generated %s received in a saved reply snapshot before history refresh",
    async (_kind, mimeType, page) => {
      const hook = await open();
      const file: AssistantAttachment = {
        id: "live-document",
        name: `Live.${page === null ? "html" : "pptx"}`,
        mimeType,
        sizeBytes: 128_272,
        status: "ready",
        reason: null,
        details: null,
        origin: "generated",
        sourceMessageId: "reply",
      };
      emit("message", {
        message: {
          id: "reply",
          role: "assistant",
          content: "Your document is ready.",
          createdAt: 2,
          status: "completed",
          attachments: [file],
          generations: [
            {
              toolCallId: "document-call",
              kind: "document",
              model: "",
              params: { format: page === null ? "html" : "pptx" },
              inputIds: [],
              status: "completed",
              outputs: [file.id],
              cost: "0",
            },
          ],
        },
      });
      expect(hook.result.current.detail?.attachments).toEqual([]);
      expect(hook.result.current.detail?.messages).toEqual([]);
      expect(hook.result.current.stream?.attachments).toEqual([file]);
      const reference = {
        attachmentId: file.id,
        version: "b".repeat(64),
        page,
      };
      const target = {
        sessionId: session.id,
        expectedOwner: identity.owner,
        expectedGeneration: identity.authorityGeneration,
        selectionKey: hook.result.current.selectionKey,
      };
      act(() => {
        expect(
          hook.result.current.addFileReference(reference, {
            ...target,
            expectedOwner: "c".repeat(64),
          }),
        ).toBe(false);
        expect(
          hook.result.current.addFileReference(reference, {
            ...target,
            expectedGeneration: 2,
          }),
        ).toBe(false);
        expect(
          hook.result.current.addFileReference(
            {
              ...reference,
              attachmentId: "unrelated",
            },
            target,
          ),
        ).toBe(false);
        expect(
          hook.result.current.addFileReference(
            {
              ...reference,
              page: page === null ? 1 : null,
            },
            target,
          ),
        ).toBe(false);
      });
      act(() =>
        expect(hook.result.current.addFileReference(reference, target)).toBe(
          true,
        ),
      );
      expect(hook.result.current.fileReferences).toEqual([reference]);
      expect(mocks.send).not.toHaveBeenCalled();
      const oldAdd = hook.result.current.addFileReference;
      act(() => hook.result.current.select(null));
      act(() => expect(oldAdd(reference, target)).toBe(false));
      expect(hook.result.current.fileReferences).toEqual([]);
    },
  );

  it("does not let an older ready reply override the canonical missing file", async () => {
    referenceFile();
    const file = current.attachments[0];
    current.attachments = [{ ...file, status: "missing" }];
    const hook = await open();
    emit("message", {
      message: {
        id: "reply",
        role: "assistant",
        content: "Your document is ready.",
        createdAt: 2,
        attachments: [file],
      },
    });
    act(() =>
      expect(
        hook.result.current.addFileReference({
          attachmentId: file.id,
          version: "b".repeat(64),
          page: 1,
        }),
      ).toBe(false),
    );
    expect(hook.result.current.fileReferences).toEqual([]);
  });

  it("accepts an explicit count-only image grant only with an active visual mode and a positive image limit", async () => {
    const hook = await open(null);
    act(() =>
      hook.result.current.setPresentationVisualPolicy({
        mode: "auto",
        maxImages: 2,
        maxCostUsd: "count-only",
      }),
    );
    expect(hook.result.current.presentationVisualPolicy.maxCostUsd).toBe(
      "count-only",
    );
    act(() =>
      hook.result.current.setPresentationVisualPolicy({
        mode: "off",
        maxImages: 2,
        maxCostUsd: "count-only",
      }),
    );
    expect(hook.result.current.presentationVisualPolicy.mode).toBe("auto");
    act(() =>
      hook.result.current.setPresentationVisualPolicy({
        mode: "auto",
        maxImages: 0,
        maxCostUsd: "count-only",
      }),
    );
    expect(hook.result.current.presentationVisualPolicy.maxImages).toBe(2);
    act(() => hook.result.current.setDraft("Use count-only visuals"));
    await act(async () => {
      await hook.result.current.send();
    });
    expect(mocks.send.mock.calls[0][0].presentationVisualPolicy).toEqual({
      mode: "auto",
      maxImages: 2,
      maxCostUsd: "count-only",
    });
    expect(hook.result.current.presentationVisualPolicy).toEqual({
      mode: "off",
      maxImages: 0,
      maxCostUsd: "0",
    });
  });
  it("freezes a structured current-page reference without sending when the user only quotes it", async () => {
    current.attachments = [
      {
        id: "deck",
        name: "Deck.pptx",
        mimeType:
          "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        sizeBytes: 100,
        status: "ready",
        reason: null,
        details: null,
        origin: "generated",
      },
    ];
    const hook = await open(null);
    const reference = {
      attachmentId: "deck",
      version: "b".repeat(64),
      page: 2,
    };
    act(() => {
      hook.result.current.addFileReference(reference);
      hook.result.current.setDraft("Please improve this page");
    });
    expect(mocks.send).not.toHaveBeenCalled();
    reference.page = 5;
    await act(async () => {
      await hook.result.current.send();
    });
    expect(mocks.send.mock.calls[0][0].fileReferences).toEqual([
      { attachmentId: "deck", version: "b".repeat(64), page: 2 },
    ]);
  });
  it("clears references on navigation and rejects a late quote from an old selection or account", async () => {
    referenceFile();
    const hook = await open(null);
    const reference = {
      attachmentId: "deck",
      version: "b".repeat(64),
      page: 2,
    };
    const oldAdd = hook.result.current.addFileReference;
    const target = {
      sessionId: session.id,
      expectedOwner: identity.owner,
      expectedGeneration: identity.authorityGeneration,
      selectionKey: hook.result.current.selectionKey,
    };
    act(() => expect(oldAdd(reference, target)).toBe(true));
    act(() => hook.result.current.select(null));
    expect(hook.result.current.fileReferences).toEqual([]);
    act(() => expect(oldAdd(reference, target)).toBe(false));
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    act(() =>
      expect(hook.result.current.addFileReference(reference, target)).toBe(
        false,
      ),
    );
    act(() =>
      expect(hook.result.current.addFileReference(reference)).toBe(true),
    );
    act(() => mocks.auth?.("expired"));
    expect(hook.result.current.fileReferences).toEqual([]);
  });
  it("deduplicates page references, caps them at 20, and replaces an older version of the same file", async () => {
    referenceFile();
    const hook = await open(null);
    act(() => {
      for (let page = 1; page <= 20; page += 1)
        expect(
          hook.result.current.addFileReference({
            attachmentId: "deck",
            version: "b".repeat(64),
            page,
          }),
        ).toBe(true);
      expect(
        hook.result.current.addFileReference({
          attachmentId: "deck",
          version: "b".repeat(64),
          page: 20,
        }),
      ).toBe(true);
      expect(
        hook.result.current.addFileReference({
          attachmentId: "deck",
          version: "b".repeat(64),
          page: 21,
        }),
      ).toBe(false);
    });
    expect(hook.result.current.fileReferences).toHaveLength(20);
    act(() =>
      expect(
        hook.result.current.addFileReference({
          attachmentId: "deck",
          version: "c".repeat(64),
          page: 1,
        }),
      ).toBe(true),
    );
    expect(hook.result.current.fileReferences).toEqual([
      { attachmentId: "deck", version: "c".repeat(64), page: 1 },
    ]);
    act(() => {
      expect(
        hook.result.current.addFileReference({
          attachmentId: "unrelated",
          version: "c".repeat(64),
          page: 1,
        }),
      ).toBe(false);
      expect(
        hook.result.current.addFileReference({
          attachmentId: "deck",
          version: "c".repeat(64),
          page: 0,
        }),
      ).toBe(false);
    });
  });

  it("dispatches a frozen queued reference instead of a newer composer page", async () => {
    referenceFile();
    const hook = await open();
    act(() => {
      expect(
        hook.result.current.addFileReference({
          attachmentId: "deck",
          version: "b".repeat(64),
          page: 2,
        }),
      ).toBe(true);
      hook.result.current.setDraft("Improve queued page");
    });
    act(() => expect(hook.result.current.enqueue()).toBe(true));
    expect(hook.result.current.fileReferences).toEqual([]);
    act(() =>
      hook.result.current.addFileReference({
        attachmentId: "deck",
        version: "b".repeat(64),
        page: 7,
      }),
    );
    current = {
      ...current,
      session: { ...current.session, activeRun: null },
      messages: [],
    };
    emit("run_end", { status: "completed", stopped: false });
    await waitFor(() => expect(mocks.send).toHaveBeenCalled());
    expect(mocks.send.mock.calls[0][0].fileReferences).toEqual([
      { attachmentId: "deck", version: "b".repeat(64), page: 2 },
    ]);
    expect(hook.result.current.fileReferences[0].page).toBe(7);
  });

  it("restores a failed draft's references without overwriting a newer quote", async () => {
    referenceFile();
    const hook = await open(null);
    act(() => {
      hook.result.current.addFileReference({
        attachmentId: "deck",
        version: "b".repeat(64),
        page: 2,
      });
      hook.result.current.setDraft("Improve this");
    });
    mocks.send.mockRejectedValueOnce(
      new Error("assistant_storage_unavailable"),
    );
    await act(async () => {
      await hook.result.current.send();
    });
    expect(hook.result.current.fileReferences[0].page).toBe(2);
    let fail!: (reason: Error) => void;
    mocks.send.mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          fail = reject;
        }),
    );
    let sending!: Promise<boolean>;
    act(() => {
      sending = hook.result.current.send();
    });
    await waitFor(() => expect(mocks.send).toHaveBeenCalledTimes(2));
    act(() =>
      hook.result.current.addFileReference({
        attachmentId: "deck",
        version: "b".repeat(64),
        page: 7,
      }),
    );
    await act(async () => {
      fail(new Error("assistant_storage_unavailable"));
      await sending;
    });
    expect(hook.result.current.fileReferences.map((item) => item.page)).toEqual(
      [7],
    );
  });

  it.each(["edit", "retry", "regenerate"])(
    "inherits the original user's page for %s instead of the current draft",
    async (kind) => {
      referenceFile();
      const original = {
        attachmentId: "deck",
        version: "b".repeat(64),
        page: 3,
      };
      current.messages = [
        {
          id: "question",
          role: "user",
          runId: "old-run",
          content: "Original",
          createdAt: 1,
          fileReferences: [original],
        },
        {
          id: "reply",
          role: "assistant",
          runId: "old-run",
          content: "Result",
          createdAt: 2,
          status: "stopped",
        },
      ];
      const hook = await open(null);
      act(() => hook.result.current.addFileReference({ ...original, page: 7 }));
      await act(async () => {
        await hook.result.current.send({
          ...(kind === "regenerate" ? {} : { text: "Revised" }),
          ...(kind === "edit"
            ? { editMessageId: "question" }
            : kind === "retry"
              ? { retryMessageId: "reply" }
              : { regenerateMessageId: "reply" }),
        });
      });
      expect(mocks.send.mock.calls[0][0].fileReferences).toEqual([original]);
      expect(hook.result.current.fileReferences[0].page).toBe(7);
    },
  );
  it("freezes the sent visual budget and clears all draft authorization on sign out", async () => {
    const hook = await open(null);
    const policy = { mode: "auto" as const, maxImages: 1, maxCostUsd: "0.25" };
    act(() => {
      hook.result.current.setPresentationVisualPolicy(policy);
      hook.result.current.setDraft("Make an academic PPT");
    });
    await act(async () => {
      await hook.result.current.send();
    });
    expect(mocks.send.mock.calls[0][0].presentationVisualPolicy).toEqual(
      policy,
    );
    expect(hook.result.current.presentationVisualPolicy).toEqual({
      mode: "off",
      maxImages: 0,
      maxCostUsd: "0",
    });
    act(() =>
      hook.result.current.setPresentationVisualPolicy({
        mode: "illustration",
        maxImages: 4,
        maxCostUsd: "2",
      }),
    );
    expect(mocks.send.mock.calls[0][0].presentationVisualPolicy.maxImages).toBe(
      1,
    );
    act(() => mocks.auth?.("expired"));
    expect(hook.result.current.presentationVisualPolicy).toEqual({
      mode: "off",
      maxImages: 0,
      maxCostUsd: "0",
    });
  });

  it("keeps draft choices per conversation and clears them on authority change", async () => {
    const hook = await open(null);
    act(() => hook.result.current.setPresentationStyleId("cartesian"));
    expect(hook.result.current.presentationStyleId).toBe("cartesian");
    expect(mocks.send).not.toHaveBeenCalled();
    act(() => hook.result.current.select(null));
    expect(hook.result.current.presentationStyleId).toBeUndefined();
    act(() => hook.result.current.setPresentationStyleId("auto"));
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    expect(hook.result.current.presentationStyleId).toBe("cartesian");
    act(() => mocks.auth?.("expired"));
    expect(hook.result.current.presentationStyleId).toBeUndefined();
  });

  it("freezes the sent selection before admission and preserves a newer draft choice", async () => {
    const hook = await open(null);
    let finish!: () => void;
    mocks.send.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    act(() => {
      hook.result.current.setPresentationStyleId("cartesian");
      hook.result.current.setDraft("Make slides");
    });
    let sending!: Promise<boolean>;
    act(() => {
      sending = hook.result.current.send();
    });
    await waitFor(() => expect(mocks.send).toHaveBeenCalled());
    expect(mocks.send.mock.calls[0][0].presentationStyleId).toBe("cartesian");
    expect(hook.result.current.presentationStyleId).toBeUndefined();
    act(() => hook.result.current.setPresentationStyleId("product-launch"));
    await act(async () => {
      finish();
      await sending;
    });
    expect(hook.result.current.presentationStyleId).toBe("product-launch");
    expect(
      hook.result.current.detail?.messages.find((m) => m.role === "user")
        ?.presentationStyleId,
    ).toBe("cartesian");
  });

  it("freezes explicit auto as well as a manual style in queued messages", () => {
    const hook = renderHook(() => useMessageQueue());
    act(() => {
      hook.result.current.controls.enqueue(
        "session",
        "First",
        undefined,
        "cartesian",
      );
      hook.result.current.controls.enqueue(
        "session",
        "Next",
        undefined,
        "auto",
      );
    });
    expect(
      hook.result.current.queues
        .get("session")
        ?.messages.map((m) => m.presentationStyleId),
    ).toEqual(["cartesian", "auto"]);
    act(() =>
      hook.result.current.controls.edit(
        "session",
        hook.result.current.queues.get("session")!.messages[0].id,
        "Edited text",
      ),
    );
    expect(
      hook.result.current.queues.get("session")?.messages[0]
        .presentationStyleId,
    ).toBe("cartesian");
  });

  it("dispatches the queued choice rather than a newer composer choice", async () => {
    const hook = await open();
    act(() => {
      hook.result.current.setPresentationStyleId("cartesian");
      hook.result.current.setDraft("Queued deck");
    });
    expect(hook.result.current.queueAvailable).toBe(true);
    act(() => expect(hook.result.current.enqueue()).toBe(true));
    act(() => hook.result.current.setPresentationStyleId("product-launch"));
    current = {
      ...current,
      session: { ...current.session, activeRun: null },
      messages: [],
    };
    emit("run_end", { status: "completed", stopped: false });
    await waitFor(() => expect(mocks.send).toHaveBeenCalled());
    expect(mocks.send.mock.calls[0][0].presentationStyleId).toBe("cartesian");
    expect(hook.result.current.presentationStyleId).toBe("product-launch");
  });

  it("consumes the queued image budget once and restores failed admission only before newer edits", async () => {
    const queued = await open();
    const policy = { mode: "auto" as const, maxImages: 2, maxCostUsd: "0.4" };
    act(() => {
      queued.result.current.setPresentationVisualPolicy(policy);
      queued.result.current.setDraft("Queued visuals");
    });
    act(() => expect(queued.result.current.enqueue()).toBe(true));
    expect(queued.result.current.presentationVisualPolicy).toEqual({
      mode: "off",
      maxImages: 0,
      maxCostUsd: "0",
    });
    expect(
      queued.result.current.queue?.messages[0].presentationVisualPolicy,
    ).toEqual(policy);
    queued.unmount();
    const hook = await open(null);
    act(() => {
      hook.result.current.setPresentationVisualPolicy(policy);
      hook.result.current.setDraft("First");
    });
    mocks.send.mockRejectedValueOnce(
      new Error("assistant_storage_unavailable"),
    );
    await act(async () => {
      await hook.result.current.send();
    });
    expect(hook.result.current.presentationVisualPolicy).toEqual(policy);
    let reject!: (reason: Error) => void;
    mocks.send.mockImplementationOnce(
      () =>
        new Promise((_resolve, fail) => {
          reject = fail;
        }),
    );
    let sending!: Promise<boolean>;
    act(() => {
      sending = hook.result.current.send();
    });
    await waitFor(() => expect(mocks.send).toHaveBeenCalledTimes(2));
    act(() =>
      hook.result.current.setPresentationVisualPolicy({
        mode: "illustration",
        maxImages: 1,
        maxCostUsd: "0.1",
      }),
    );
    await act(async () => {
      reject(new Error("assistant_storage_unavailable"));
      await sending;
    });
    expect(hook.result.current.presentationVisualPolicy).toEqual({
      mode: "illustration",
      maxImages: 1,
      maxCostUsd: "0.1",
    });
  });

  it("restores a failed send's choice without overwriting a newer choice", async () => {
    const hook = await open(null);
    act(() => {
      hook.result.current.setPresentationStyleId("cartesian");
      hook.result.current.setDraft("Slides");
    });
    mocks.send.mockRejectedValueOnce(
      new Error("assistant_storage_unavailable"),
    );
    await act(async () => {
      await hook.result.current.send();
    });
    expect(hook.result.current.presentationStyleId).toBe("cartesian");
  });
});
afterEach(() => vi.restoreAllMocks());

async function open(run: AssistantRunRef | null = active) {
  current.session.activeRun = run;
  if (run)
    current.messages = [
      {
        id: "reply",
        role: "assistant",
        content: "",
        createdAt: 9_000,
        status: "streaming",
        runId: run.runId,
      },
    ];
  const hook = renderHook(({ enabled }) => useAssistant(enabled), {
    initialProps: { enabled: true },
  });
  await waitFor(() => expect(hook.result.current.bootstrap).not.toBeNull());
  act(() => hook.result.current.select(session.id));
  await waitFor(() =>
    expect(hook.result.current.detail?.session.id).toBe(session.id),
  );
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  return hook;
}
function emit(
  type: AssistantEvent["type"],
  data: unknown,
  reference = active,
  sessionId = session.id,
) {
  act(() =>
    mocks.listener?.({ ...reference, sessionId, type, data } as AssistantEvent),
  );
}
function start(id: string, name = "read", intent = "Read source") {
  emit("tool_start", {
    messageId: "reply",
    toolCallId: id,
    name,
    args: {},
    intent,
  });
}
function end(id: string, success = true) {
  emit("tool_end", {
    messageId: "reply",
    toolCallId: id,
    name: "read",
    success,
    code: success ? null : "assistant_tool_failed",
  });
}

describe("observed assistant progress", () => {
  it("shows argument preparation without inventing execution or clearing another running call", async () => {
    const hook = await open();
    start("failed", "create_document");
    end("failed", false);
    const previous = hook.result.current.runProgress?.activities;
    const prepare = () =>
      emit("tool_preparing" as AssistantEvent["type"], {
        messageId: "reply",
        name: "create_document",
        toolCallId: "next-document",
      });
    now += 100;
    prepare();
    expect(hook.result.current.activity).toBe("preparingToolRequest");
    expect(hook.result.current.localIntent).toBeNull();
    expect(hook.result.current.runProgress?.activities).toBe(previous);
    expect(hook.result.current.runProgress?.lastEventAt).toBe(now);
    now += 100;
    emit("text_delta", { messageId: "reply", delta: "Still composing" });
    expect(hook.result.current.activity).toBe("preparingToolRequest");
    start("independent", "read");
    expect(hook.result.current.activity).toBe("readingFile");
    end("independent");
    expect(hook.result.current.activity).toBe("preparingToolRequest");
    start("next-document", "create_document");
    expect(hook.result.current.activity).toBe("makingDocument");
    prepare();
    end("next-document", false);
    prepare();
    expect(hook.result.current.activity).toBeNull();
    expect(hook.result.current.runProgress?.activities.at(-1)).toEqual({
      id: "next-document",
      activity: "makingDocument",
      status: "failed",
    });
  });

  it("rejects unsafe, stale and terminal preparation and clears it on stop and scope changes", async () => {
    const hook = await open();
    const prepare = (
      name = "create_document",
      toolCallId = "document",
      reference = active,
      sid = session.id,
    ) =>
      emit(
        "tool_preparing" as AssistantEvent["type"],
        { name, toolCallId, messageId: "reply", args: { source: "private" } },
        reference,
        sid,
      );
    prepare("unknown-tool");
    prepare("create_document", "bad\ncall");
    prepare("create_document", "old", { ...active, operationId: "old" });
    prepare("create_document", "other", active, "other-session");
    expect(hook.result.current.activity).toBeNull();
    prepare();
    expect(hook.result.current.activity).toBe("preparingToolRequest");
    await act(async () => hook.result.current.stop());
    expect(hook.result.current.activity).toBeNull();
    prepare();
    expect(hook.result.current.activity).toBeNull();
    act(() => hook.result.current.select(null));
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    prepare();
    expect(hook.result.current.activity).toBeNull();
    const next = { runId: "next-run", operationId: "next-operation" };
    current.session.activeRun = next;
    await act(async () => {
      await hook.result.current.refreshDetail(session.id);
    });
    prepare("create_document", "next", next);
    expect(hook.result.current.activity).toBe("preparingToolRequest");
    emit("error", { code: "assistant_runtime_failed" }, next);
    prepare("create_document", "next", next);
    expect(hook.result.current.activity).toBeNull();
    act(() => mocks.auth?.("expired"));
    prepare();
    expect(hook.result.current.activity).toBeNull();
  });

  it("labels freezing a presentation plan separately from querying styles", async () => {
    const hook = await open();
    start("freeze", "freeze_presentation_plan");
    expect(hook.result.current.activity).toBe("confirmingPlan");
    expect(hook.result.current.runProgress?.activities).toEqual([
      { id: "freeze", activity: "confirmingPlan", status: "running" },
    ]);
  });

  it("clears matching non-executed requests and does not revive them after retry or completion", async () => {
    const hook = await open();
    const prepare = (toolCallId: string, active?: boolean) =>
      emit("tool_preparing", {
        name: "create_document",
        toolCallId,
        messageId: "reply",
        ...(active === undefined ? {} : { active }),
      });
    prepare("first");
    prepare("second");
    prepare("first", false);
    expect(hook.result.current.activity).toBe("preparingToolRequest");
    prepare("second", false);
    prepare("second");
    expect(hook.result.current.activity).toBeNull();
    expect(hook.result.current.runProgress?.activities).toEqual([]);
    prepare("retry-old");
    emit("retrying", { attempt: 1 });
    prepare("retry-old");
    expect(hook.result.current.activity).toBeNull();
    prepare("retry-new");
    expect(hook.result.current.activity).toBe("preparingToolRequest");
    current.session.activeRun = null;
    emit("run_end", { status: "completed", stopped: false });
    prepare("late");
    expect(hook.result.current.activity).toBeNull();
    expect(hook.result.current.runProgress).toBeNull();
  });

  it("starts before run_start while send admission is pending", async () => {
    const hook = await open(null);
    let finish!: () => void;
    mocks.send.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    act(() => hook.result.current.setDraft("Make a presentation"));
    let sending!: Promise<boolean>;
    act(() => {
      sending = hook.result.current.send();
    });
    await waitFor(() => expect(mocks.send).toHaveBeenCalled());
    const request = mocks.send.mock.calls[0][0];
    expect(hook.result.current.runProgress).toEqual({
      runId: request.runId,
      startedAt: now,
      lastEventAt: now,
      activities: [],
    });
    now += 500;
    emit("run_start", { messageId: "reply", webSearch: false }, request);
    expect(hook.result.current.runProgress?.startedAt).toBe(10_000);
    expect(hook.result.current.runProgress?.lastEventAt).toBe(now);
    await act(async () => {
      finish();
      await sending;
    });
  });

  it("keeps semantic progress times separate from token streaming and stale events", async () => {
    const hook = await open();
    expect(hook.result.current.runProgress?.startedAt).toBe(9_000);
    now += 100;
    emit("thinking_start", { messageId: "reply" });
    const semantic = hook.result.current.runProgress;
    expect(semantic?.lastEventAt).toBe(now);
    now += 100;
    emit("thinking_delta", { messageId: "reply", delta: "Reasoning" });
    emit("text_delta", { messageId: "reply", delta: "Text" });
    emit("task_plan", { steps: [] }, { ...active, operationId: "stale" });
    emit("task_plan", { steps: [] }, active, "another-session");
    expect(hook.result.current.runProgress).toBe(semantic);
    emit("task_plan", { steps: [{ step: "Read", status: "in_progress" }] });
    expect(hook.result.current.runProgress?.lastEventAt).toBe(now);
  });

  it("records actual bounded call results and clears only the matching tool intent", async () => {
    const hook = await open();
    start("first", "read", "First");
    start("second", "write", "Second");
    end("first");
    expect(hook.result.current.localIntent).toBe("Second");
    expect(hook.result.current.activity).toBe("editingFile");
    end("second", false);
    expect(hook.result.current.localIntent).toBeNull();
    expect(hook.result.current.activity).toBeNull();
    expect(hook.result.current.runProgress?.activities).toEqual([
      { id: "first", activity: "readingFile", status: "completed" },
      { id: "second", activity: "editingFile", status: "failed" },
    ]);
    end("unmatched");
    for (let index = 0; index < 5; index++) {
      start(`extra-${index}`);
      end(`extra-${index}`);
    }
    expect(
      hook.result.current.runProgress?.activities.map((item) => item.id),
    ).toEqual(["extra-1", "extra-2", "extra-3", "extra-4"]);
  });

  it.each([
    ["stat", "readingFile"],
    ["ls", "readingFile"],
    ["find", "readingFile"],
    ["grep", "readingFile"],
    ["edit", "editingFile"],
    ["bash", "runningCommand"],
    ["powershell", "runningCommand"],
    ["tool_status", "checkingResult"],
    ["diagnose_in_folder", "checkingResult"],
    ["verify_in_folder", "checkingResult"],
  ])(
    "maps %s to a safe category without exposing arguments",
    async (name, category) => {
      const hook = await open();
      start("call", name, "Private raw intent");
      expect(hook.result.current.activity).toBe(category);
      expect(hook.result.current.runProgress?.activities).toEqual([
        { id: "call", activity: category, status: "running" },
      ]);
    },
  );

  it("clears progress on completion, selection and authority changes", async () => {
    const hook = await open();
    start("call");
    current = {
      ...current,
      session: { ...current.session, activeRun: null },
      messages: [],
    };
    emit("run_end", { status: "completed", stopped: false });
    expect(hook.result.current.runProgress).toBeNull();
    await act(async () => {
      await Promise.resolve();
    });
    current.session.activeRun = { runId: "run-b", operationId: "op-b" };
    await act(async () => {
      await hook.result.current.refreshDetail(session.id);
    });
    expect(hook.result.current.runProgress?.runId).toBe("run-b");
    expect(hook.result.current.runProgress?.activities).toEqual([]);
    act(() => hook.result.current.select(null));
    expect(hook.result.current.runProgress).toBeNull();
    act(() => hook.result.current.select(session.id));
    await waitFor(() =>
      expect(hook.result.current.runProgress?.runId).toBe("run-b"),
    );
    act(() => mocks.auth?.("expired"));
    expect(hook.result.current.runProgress).toBeNull();
    start("stale");
    expect(hook.result.current.runProgress).toBeNull();
    hook.rerender({ enabled: false });
    expect(hook.result.current.runProgress).toBeNull();
  });
});

describe("explicit presentation text cost threshold snapshots", () => {
  it.each(["edit", "retry", "regenerate"])(
    "leaves %s threshold inheritance to Host without borrowing the later draft",
    async (kind) => {
      const original = {
        mode: "off" as const,
        maxImages: 0,
        maxCostUsd: "0",
        textMaxCostUsd: "0.10",
      };
      current.messages = [
        {
          id: "question",
          role: "user",
          runId: "old-run",
          content: "Original",
          createdAt: 1,
          presentationVisualPolicy: original,
        },
        {
          id: "reply",
          role: "assistant",
          runId: "old-run",
          content: "Result",
          createdAt: 2,
          status: "stopped",
        },
      ];
      const hook = await open(null);
      act(() =>
        hook.result.current.setPresentationVisualPolicy({
          ...original,
          textMaxCostUsd: "0.9",
        }),
      );
      await act(async () => {
        await hook.result.current.send({
          ...(kind === "regenerate" ? {} : { text: "Revised" }),
          ...(kind === "edit"
            ? { editMessageId: "question" }
            : kind === "retry"
              ? { retryMessageId: "reply" }
              : { regenerateMessageId: "reply" }),
        });
      });
      expect(
        mocks.send.mock.calls[0][0].presentationVisualPolicy,
      ).toBeUndefined();
      expect(
        mocks.send.mock.calls[0][0].replacePresentationVisualPolicy,
      ).not.toBe(true);
      expect(hook.result.current.presentationVisualPolicy.textMaxCostUsd).toBe(
        "0.9",
      );
    },
  );
  it.each(["0", "0.009", "1.000001", "2", "NaN", "1e-1", "0.1234567"])(
    "does not commit an invalid text threshold %s to the send draft",
    async (amount) => {
      const hook = await open(null);
      const original = hook.result.current.presentationVisualPolicy;
      const invalid = { ...original, textMaxCostUsd: amount };
      act(() => hook.result.current.setPresentationVisualPolicy(invalid));
      expect(hook.result.current.presentationVisualPolicy).toEqual(original);
      expect(mocks.send).not.toHaveBeenCalled();
    },
  );

  it("freezes the explicit threshold once and preserves a newer draft edit during admission", async () => {
    const hook = await open(null);
    let finish!: () => void;
    mocks.send.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    const policy = {
      mode: "off" as const,
      maxImages: 0,
      maxCostUsd: "0",
      textMaxCostUsd: "0.10",
    };
    act(() => {
      hook.result.current.setPresentationVisualPolicy(policy);
      hook.result.current.setDraft("Make slides");
    });
    let sending!: Promise<boolean>;
    act(() => {
      sending = hook.result.current.send();
    });
    await waitFor(() => expect(mocks.send).toHaveBeenCalledTimes(1));
    expect(
      mocks.send.mock.calls[0][0].presentationVisualPolicy.textMaxCostUsd,
    ).toBe("0.10");
    expect(
      hook.result.current.presentationVisualPolicy.textMaxCostUsd,
    ).toBeUndefined();
    policy.textMaxCostUsd = "0.9";
    act(() =>
      hook.result.current.setPresentationVisualPolicy({
        ...policy,
        textMaxCostUsd: "0.20",
      }),
    );
    await act(async () => {
      finish();
      await sending;
    });
    expect(
      mocks.send.mock.calls[0][0].presentationVisualPolicy.textMaxCostUsd,
    ).toBe("0.10");
    expect(hook.result.current.presentationVisualPolicy.textMaxCostUsd).toBe(
      "0.20",
    );
    act(() => mocks.auth?.("expired"));
    expect(
      hook.result.current.presentationVisualPolicy.textMaxCostUsd,
    ).toBeUndefined();
  });

  it("keeps queued and claimed text thresholds independent from later edits", () => {
    const hook = renderHook(() => useMessageQueue());
    const policy = {
      mode: "off" as const,
      maxImages: 0,
      maxCostUsd: "0",
      textMaxCostUsd: "0.10",
    };
    act(() =>
      hook.result.current.controls.enqueue(
        "session",
        "Slides",
        undefined,
        "auto",
        policy,
      ),
    );
    policy.textMaxCostUsd = "0.9";
    const queued = hook.result.current.controls.get("session")!.messages[0];
    expect(queued.presentationVisualPolicy?.textMaxCostUsd).toBe("0.10");
    let ticket!: ReturnType<typeof hook.result.current.controls.claim>;
    act(() => {
      hook.result.current.controls.resume("session");
      ticket = hook.result.current.controls.claim("session");
    });
    queued.presentationVisualPolicy!.textMaxCostUsd = "0.8";
    expect(ticket?.message.presentationVisualPolicy?.textMaxCostUsd).toBe(
      "0.10",
    );
  });
});
