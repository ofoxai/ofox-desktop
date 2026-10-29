import { beforeEach, describe, expect, it, vi } from "vitest";
import { listen } from "@tauri-apps/api/event";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zh from "@/i18n/locales/zh.json";
import {
  ASSISTANT_ERROR_MESSAGES,
  assistantApi,
  assistantErrorKey,
} from "@/lib/api/assistant";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const events = vi.hoisted(() => ({
  handlers: new Map<string, (event?: { payload: unknown }) => void>(),
  off: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (name: string, handler: (event?: { payload: unknown }) => void) => {
      events.handlers.set(name, handler);
      return events.off;
    },
  ),
}));

const locales = { en, ja, zh } as Record<
  string,
  { assistant: { errors: Record<string, string> } }
>;

beforeEach(() => {
  invoke.mockReset();
});

/** jsdom's File has no `arrayBuffer()`, which every supported WebView has. */
function fileOf(bytes: Uint8Array, name: string, type = ""): File {
  const file = new File([bytes.slice().buffer as ArrayBuffer], name, { type });
  Object.defineProperty(file, "arrayBuffer", {
    value: async () => bytes.buffer.slice(0),
  });
  return file;
}

describe("assistant commands", () => {
  it("lists one filter of sessions with the damaged ones as the backend names them", async () => {
    invoke.mockResolvedValue({
      sessions: [{ id: "a", title: "Saved" }],
      unreadable: [{ id: "b", error: "assistant_invalid_session" }],
    });
    const list = await assistantApi.listSessions(false, "notes");
    expect(invoke).toHaveBeenCalledWith("assistant_list_sessions", {
      deleted: false,
      search: "notes",
    });
    expect(list).toEqual({
      sessions: [{ id: "a", title: "Saved" }],
      unreadable: [{ id: "b", error: "assistant_invalid_session" }],
    });
  });

  it("passes on the first matching message of each searched conversation", async () => {
    invoke.mockResolvedValue({
      sessions: [{ id: "a", title: "Trip" }],
      unreadable: [],
      trashCount: 0,
      matches: { a: "m1" },
    });
    const list = await assistantApi.listSessions(false, "kyoto");
    expect(list.matches).toEqual({ a: "m1" });
  });

  it("pins with an update that carries only the pin", async () => {
    invoke.mockResolvedValue({ id: "a", pinnedAt: 5 });
    await assistantApi.updateSession({ sessionId: "a", pinned: true });
    expect(invoke).toHaveBeenCalledWith("assistant_update_session", {
      request: { sessionId: "a", pinned: true },
    });
  });

  it("empties the trash with the confirmed authority and returns the counts", async () => {
    invoke.mockResolvedValue({ deleted: 3, failed: 1 });
    const report = await assistantApi.emptyTrash({
      expectedOwner: "owner",
      expectedGeneration: 7,
    });
    expect(invoke).toHaveBeenCalledWith("assistant_empty_trash", {
      request: { expectedOwner: "owner", expectedGeneration: 7 },
    });
    expect(report).toEqual({ deleted: 3, failed: 1 });
  });

  it("switches versions with the confirmed authority and returns the conversation", async () => {
    const detail = {
      session: { id: "s" },
      messages: [],
      attachments: [],
      usage: {},
    };
    invoke.mockResolvedValue(detail);
    const shown = await assistantApi.switchVersion({
      sessionId: "s",
      messageId: "m",
      expectedOwner: "owner",
      expectedGeneration: 2,
    });
    expect(invoke).toHaveBeenCalledWith("assistant_switch_version", {
      request: {
        sessionId: "s",
        messageId: "m",
        expectedOwner: "owner",
        expectedGeneration: 2,
      },
    });
    expect(shown).toBe(detail);
  });

  it("retries a reply with its own text, files and the reply it retries", async () => {
    invoke.mockResolvedValue({ runId: "r", operationId: "o" });
    const request = {
      sessionId: "s",
      runId: "r",
      operationId: "o",
      text: "Asked again",
      attachmentIds: [],
      retryMessageId: "failed",
    };
    await assistantApi.send(request);
    expect(invoke).toHaveBeenCalledWith("assistant_send", { request });
  });

  it("creates a chat session with only a model and the web search flag", async () => {
    invoke.mockResolvedValue({ id: "new" });
    await assistantApi.createSession({ modelId: "m", webSearch: false });
    expect(invoke).toHaveBeenCalledWith("assistant_create_session", {
      request: { modelId: "m", webSearch: false },
    });
  });

  it("opens the file picker for one conversation and returns the files it imported", async () => {
    const file = {
      id: "f1",
      name: "notes.pdf",
      mimeType: "application/pdf",
      sizeBytes: 2048,
      status: "ready",
      reason: null,
      previewUrl: null,
      details: null,
    };
    invoke.mockResolvedValue([file]);
    const files = await assistantApi.pickAttachments("s");
    expect(invoke).toHaveBeenCalledWith("assistant_pick_attachments", {
      sessionId: "s",
    });
    expect(files).toEqual([file]);
  });

  it("imports the files dropped on the window by their token, never by a path", async () => {
    const file = {
      id: "f2",
      name: "chart.png",
      mimeType: "image/png",
      sizeBytes: 12,
      status: "ready",
      reason: null,
      previewUrl: null,
      details: null,
    };
    invoke.mockResolvedValue([file]);
    const files = await assistantApi.importDrop({
      sessionId: "s",
      token: "token-1",
      expectedOwner: "owner",
      expectedGeneration: 4,
    });
    expect(invoke).toHaveBeenCalledWith("assistant_import_drop", {
      request: {
        sessionId: "s",
        token: "token-1",
        expectedOwner: "owner",
        expectedGeneration: 4,
      },
    });
    expect(files).toEqual([file]);
  });

  it("sends a pasted file as raw bytes with its authority and encoded name in headers", async () => {
    invoke.mockResolvedValue([]);
    const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47]);
    const pasted = fileOf(bytes, "中文 a+b.png", "image/png");
    await assistantApi.importPasted(pasted, {
      sessionId: "s",
      expectedOwner: "owner",
      expectedGeneration: 7,
    });
    expect(invoke).toHaveBeenCalledTimes(1);
    const [command, body, options] = invoke.mock.calls[0];
    expect(command).toBe("assistant_import_pasted");
    expect(body).toBeInstanceOf(Uint8Array);
    expect(Array.from(body as Uint8Array)).toEqual([0x89, 0x50, 0x4e, 0x47]);
    expect(options).toEqual({
      headers: {
        "x-session-id": "s",
        "x-expected-owner": "owner",
        "x-expected-generation": "7",
        "x-file-name": "%E4%B8%AD%E6%96%87%20a%2Bb.png",
      },
    });
  });

  it("claims no type for a pasted file: the backend reads it from the bytes", async () => {
    invoke.mockResolvedValue([]);
    await assistantApi.importPasted(
      fileOf(new Uint8Array([1]), "", "image/png"),
      { sessionId: "s", expectedOwner: "o", expectedGeneration: 0 },
    );
    const headers = invoke.mock.calls[0][2].headers as Record<string, string>;
    expect(Object.keys(headers).sort()).toEqual([
      "x-expected-generation",
      "x-expected-owner",
      "x-file-name",
      "x-session-id",
    ]);
    expect(headers["x-file-name"]).toBe("");
  });

  it("refuses a pasted file over 25 MB before reading it", async () => {
    const huge = new File([], "movie.mov");
    Object.defineProperty(huge, "size", { value: 25 * 1024 * 1024 + 1 });
    const read = vi.fn(async () => new ArrayBuffer(0));
    Object.defineProperty(huge, "arrayBuffer", { value: read });
    await expect(
      assistantApi.importPasted(huge, {
        sessionId: "s",
        expectedOwner: "o",
        expectedGeneration: 0,
      }),
    ).rejects.toBe("assistant_attachment_too_large");
    expect(read).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("reads a pasted file of exactly 25 MB", async () => {
    invoke.mockResolvedValue([]);
    const limit = new File([], "limit.bin");
    Object.defineProperty(limit, "size", { value: 25 * 1024 * 1024 });
    Object.defineProperty(limit, "arrayBuffer", {
      value: async () => new ArrayBuffer(4),
    });
    await assistantApi.importPasted(limit, {
      sessionId: "s",
      expectedOwner: "o",
      expectedGeneration: 0,
    });
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("sends no name for a pasted file whose name can't be encoded", async () => {
    invoke.mockResolvedValue([]);
    // A lone surrogate: encodeURIComponent throws a URIError for it. jsdom's
    // File repairs one in its constructor, so the name is set afterwards.
    const pasted = fileOf(new Uint8Array([1]), "name.png");
    Object.defineProperty(pasted, "name", { value: "bad\ud800name.png" });
    expect(() => encodeURIComponent(pasted.name)).toThrow(URIError);
    await assistantApi.importPasted(pasted, {
      sessionId: "s",
      expectedOwner: "o",
      expectedGeneration: 0,
    });
    const headers = invoke.mock.calls[0][2].headers as Record<string, string>;
    expect(headers["x-file-name"]).toBe("");
  });

  it("stops one run by session, run and operation", async () => {
    invoke.mockResolvedValue(undefined);
    await assistantApi.stop("s", "r", "o");
    expect(invoke).toHaveBeenCalledWith("assistant_stop", {
      sessionId: "s",
      runId: "r",
      operationId: "o",
    });
  });
});

describe("file drag and drop events", () => {
  beforeEach(() => {
    events.handlers.clear();
    events.off.mockClear();
  });

  it("reports whether a file drag is over the window", async () => {
    const seen: string[] = [];
    const off = await assistantApi.onDrag((state) => seen.push(state));
    events.handlers.get("assistant:drag")?.({ payload: { state: "enter" } });
    events.handlers.get("assistant:drag")?.({ payload: { state: "leave" } });
    expect(seen).toEqual(["enter", "leave"]);
    off();
    expect(events.off).toHaveBeenCalledTimes(1);
  });

  it("reports a drop by its token and file count only", async () => {
    const seen: unknown[] = [];
    await assistantApi.onDrop((drop) => seen.push(drop));
    events.handlers.get("assistant:drop")?.({
      payload: { token: "t", count: 3 },
    });
    expect(seen).toEqual([{ token: "t", count: 3 }]);
  });
});

describe("sign-in changes", () => {
  it("reports both auth events and stops listening to both", async () => {
    const seen: string[] = [];
    const off = await assistantApi.onAuthChange((state) => seen.push(state));
    events.handlers.get("ofox-auth-expired")?.();
    events.handlers.get("ofox-auth-restored")?.();
    expect(seen).toEqual(["expired", "restored"]);
    off();
    expect(events.off).toHaveBeenCalledTimes(2);
  });

  it.each([
    ["the restored", [true, false]],
    ["the expired", [false, true]],
  ])(
    "keeps no listener and reports it when %s listener cannot be registered",
    async (_, registered) => {
      const offs = registered.map(() => vi.fn());
      for (const [index, ok] of registered.entries())
        vi.mocked(listen).mockImplementationOnce(async () => {
          if (!ok) throw new Error("listen failed");
          return offs[index];
        });
      await expect(assistantApi.onAuthChange(() => undefined)).rejects.toThrow(
        "listen failed",
      );
      for (const [index, ok] of registered.entries())
        expect(offs[index]).toHaveBeenCalledTimes(ok ? 1 : 0);
    },
  );
});

describe("assistant error messages", () => {
  it.each([
    ["assistant_session_busy", "assistant.errors.sessionBusy"],
    [new Error("assistant_unknown_apex"), "assistant.errors.unknownApex"],
    [
      { code: "assistant_insufficient_balance", message: "402 credits" },
      "assistant.errors.insufficientBalance",
    ],
    [{ code: "assistant_rate_limited" }, "assistant.errors.rateLimited"],
    [{ code: "assistant_network_error" }, "assistant.errors.networkError"],
    [{ code: "assistant_key_rejected" }, "assistant.errors.keyRejected"],
    [
      "assistant_key_storage_unavailable",
      "assistant.errors.keyStorageUnavailable",
    ],
    [
      { code: "assistant_model_unavailable" },
      "assistant.errors.modelUnavailable",
    ],
    [
      { code: "assistant_model_request_failed" },
      "assistant.errors.modelRequestFailed",
    ],
    ["assistant_attachment_too_many", "assistant.errors.attachmentTooMany"],
    ["assistant_attachment_too_large", "assistant.errors.attachmentTooLarge"],
    [
      "assistant_attachment_total_too_large",
      "assistant.errors.attachmentTotalTooLarge",
    ],
    [
      "assistant_attachment_unsupported",
      "assistant.errors.attachmentUnsupported",
    ],
    [
      { code: "assistant_attachment_unreadable" },
      "assistant.errors.attachmentUnreadable",
    ],
    [
      "assistant_attachment_write_failed",
      "assistant.errors.attachmentWriteFailed",
    ],
    [
      "assistant_attachment_dialog_failed",
      "assistant.errors.attachmentDialogFailed",
    ],
    ["assistant_drop_unavailable", "assistant.errors.dropUnavailable"],
    [
      { code: "assistant_vision_model_required" },
      "assistant.errors.visionModelRequired",
    ],
    [
      { code: "assistant_search_outcome_unknown" },
      "assistant.errors.searchOutcomeUnknown",
    ],
  ])("maps %o to its own message", (error, key) => {
    expect(assistantErrorKey(error)).toBe(key);
  });

  it("reads an error event by its code and never by the provider's text", () => {
    expect(
      assistantErrorKey({
        code: "assistant_model_request_failed",
        message: "assistant_signing_out: upstream said so",
      }),
    ).toBe("assistant.errors.modelRequestFailed");
  });

  it.each([
    "assistant_brand_new_failure",
    "assistant_model_request_failed: assistant_signing_out",
    new Error("Provider unavailable"),
    { code: "assistant_something_else" },
    { message: "assistant_signing_out" },
    undefined,
  ])("falls back to the generic message for %o", (error) => {
    expect(assistantErrorKey(error)).toBe("assistant.errors.generic");
  });

  it("has every mapped message in every locale, and no message without a code", () => {
    const used = new Set([
      ...Object.values(ASSISTANT_ERROR_MESSAGES),
      "generic",
    ]);
    for (const [lang, data] of Object.entries(locales)) {
      expect(Object.keys(data.assistant.errors).sort(), lang).toEqual(
        [...used].sort(),
      );
    }
  });

  it.each([
    ["insufficientBalance", /top up/i],
    ["rateLimited", /wait/i],
    ["networkError", /connection/i],
    ["keyRejected", /sign (out|in)/i],
    ["attachmentTooMany", /up to 10 at a time/i],
    ["attachmentTooLarge", /up to 25 MB/i],
    ["attachmentTotalTooLarge", /new conversation/i],
    [
      "attachmentUnsupported",
      /images.*PDFs.*docx.*xlsx.*pptx.*UTF-8 text.*docx or PDF/i,
    ],
    ["attachmentUnreadable", /attach it again/i],
    ["dropUnavailable", /drop them again/i],
    // Also the alert for a sent turn's file, which can't be removed.
    [
      "attachmentUnavailable",
      /attach the file again.*send your message again/i,
    ],
    ["visionModelRequired", /choose a model that can read images/i],
    ["searchOutcomeUnknown", /may have been charged.*not repeated/i],
  ])("tells the user what to do about %s", (message, action) => {
    expect(
      en.assistant.errors[message as keyof typeof en.assistant.errors],
    ).toMatch(action);
  });
});

it("exports Markdown with the confirmed authority and returns native cancellation", async () => {
  invoke.mockResolvedValue(false);
  const request = {
    sessionId: "s",
    expectedOwner: "owner",
    expectedGeneration: 2,
    markdown: "# Chat",
  };
  expect(await assistantApi.exportMarkdown(request)).toBe(false);
  expect(invoke).toHaveBeenCalledWith("assistant_export_markdown", { request });
});

it("reads aggregate assistant usage through the native host", async () => {
  invoke.mockResolvedValue({
    estimatedCostUsd: "1.25",
    usageCoverage: "complete",
  });
  expect(await assistantApi.usageSummary()).toMatchObject({
    estimatedCostUsd: "1.25",
  });
  expect(invoke).toHaveBeenCalledWith("assistant_usage_summary");
});

it("binds backup and restore native dialogs to the confirmed account", async () => {
  const request = { expectedOwner: "owner", expectedGeneration: 3 };
  invoke.mockResolvedValueOnce(null).mockResolvedValueOnce(null);
  expect(await assistantApi.backup(request)).toBeNull();
  expect(await assistantApi.restore(request)).toBeNull();
  expect(invoke).toHaveBeenNthCalledWith(1, "assistant_backup", { request });
  expect(invoke).toHaveBeenNthCalledWith(2, "assistant_restore", { request });
});

it("passes on what a backup holds and what a restore did", async () => {
  const request = { expectedOwner: "owner", expectedGeneration: 3 };
  const restored = { imported: 2, skipped: 1, conflicts: 3, warning: null };
  invoke
    .mockResolvedValueOnce({ sessions: 4, skipped: 1 })
    .mockResolvedValueOnce(restored);
  expect(await assistantApi.backup(request)).toEqual({
    sessions: 4,
    skipped: 1,
  });
  expect(await assistantApi.restore(request)).toEqual(restored);
});
