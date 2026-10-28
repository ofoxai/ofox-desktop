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
  handlers: new Map<string, () => void>(),
  off: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: () => void) => {
    events.handlers.set(name, handler);
    return events.off;
  }),
}));

const locales = { en, ja, zh } as Record<
  string,
  { assistant: { errors: Record<string, string> } }
>;

beforeEach(() => {
  invoke.mockReset();
});

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
  invoke.mockResolvedValueOnce(false).mockResolvedValueOnce(null);
  expect(await assistantApi.backup(request)).toBe(false);
  expect(await assistantApi.restore(request)).toBeNull();
  expect(invoke).toHaveBeenNthCalledWith(1, "assistant_backup", { request });
  expect(invoke).toHaveBeenNthCalledWith(2, "assistant_restore", { request });
});
