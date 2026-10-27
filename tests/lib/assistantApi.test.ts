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
  ])("tells the user what to do about %s", (message, action) => {
    expect(
      en.assistant.errors[message as keyof typeof en.assistant.errors],
    ).toMatch(action);
  });
});
