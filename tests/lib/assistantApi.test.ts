import { beforeEach, describe, expect, it, vi } from "vitest";
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
  it("lists one filter of sessions and names every damaged one with its code", async () => {
    invoke.mockResolvedValue({
      sessions: [{ id: "a", title: "Saved" }],
      unreadable: [
        { id: "b", error: "assistant_invalid_session" },
        { id: "c", code: "assistant_storage_unavailable" },
      ],
    });
    const list = await assistantApi.listSessions(false, "notes");
    expect(invoke).toHaveBeenCalledWith("assistant_list_sessions", {
      deleted: false,
      search: "notes",
    });
    expect(list.sessions).toEqual([{ id: "a", title: "Saved" }]);
    expect(list.unreadable).toEqual([
      { id: "b", code: "assistant_invalid_session" },
      { id: "c", code: "assistant_storage_unavailable" },
    ]);
  });

  it("treats a reply without unreadable rows as none", async () => {
    invoke.mockResolvedValue({ sessions: [] });
    expect((await assistantApi.listSessions()).unreadable).toEqual([]);
  });

  it("creates a chat session with only a model and the web search flag", async () => {
    invoke.mockResolvedValue({ id: "new" });
    await assistantApi.createSession({ modelId: "m", webSearch: false });
    expect(invoke).toHaveBeenCalledWith("assistant_create_session", {
      request: { modelId: "m", webSearch: false },
    });
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
});

describe("assistant error messages", () => {
  it.each([
    ["assistant_session_busy", "assistant.errors.sessionBusy"],
    [new Error("assistant_unknown_apex"), "assistant.errors.unknownApex"],
    [
      new Error("assistant_model_request_failed: assistant_signing_out"),
      "assistant.errors.signingOut",
    ],
    [
      "assistant_model_request_failed: 503 Service Unavailable",
      "assistant.errors.modelRequestFailed",
    ],
    [
      { code: "runtime_error", message: "upstream closed" },
      "assistant.errors.modelRequestFailed",
    ],
    [
      {
        code: "input_preparation_failed",
        message:
          "assistant_vision_model_required: This model cannot read images",
      },
      "assistant.errors.visionModelRequired",
    ],
  ])("maps %o to its own message", (error, key) => {
    expect(assistantErrorKey(error)).toBe(key);
  });

  it.each([
    "assistant_brand_new_failure",
    new Error("Provider unavailable"),
    { code: "assistant_something_else" },
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

  it("covers the codes the host is known to return", () => {
    for (const code of [
      "assistant_unknown_apex",
      "assistant_invalid_session",
      "assistant_sign_in_required",
      "assistant_signing_out",
      "assistant_shutting_down",
      "assistant_identity_changed",
      "assistant_session_busy",
      "assistant_session_not_found",
      "assistant_message_too_large",
      "assistant_title_too_long",
      "assistant_model_unavailable",
      "assistant_choose_model",
      "assistant_catalog_unavailable",
      "assistant_runtime_unavailable",
      "assistant_run_timeout",
      "assistant_storage_unavailable",
      "assistant_native_history_missing",
      "assistant_cancelled",
    ])
      expect(ASSISTANT_ERROR_MESSAGES, code).toHaveProperty(code);
  });
});
