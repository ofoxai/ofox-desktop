import { describe, expect, it, vi, beforeEach } from "vitest";
import { http, HttpResponse } from "msw";

import { server } from "../msw/server";

// Mock the @tauri-apps/api/event emit so the test can observe calls.
const emitMock = vi.fn();
vi.mock("@tauri-apps/api/event", async () => {
  const actual =
    (await vi.importActual<typeof import("../msw/tauriMocks")>(
      "../msw/tauriMocks",
    )) ?? {};
  return {
    ...((actual as unknown as Record<string, unknown>) || {}),
    emit: (...args: unknown[]) => emitMock(...args),
    listen: async () => () => {},
  };
});

// Mock toolMeta so bindTools doesn't pull in unrelated paths.
vi.mock("@/config/toolMeta", () => ({
  BOUND_TOOLS_STORAGE_KEY: "bound_tools",
  PROXY_SUPPORTED_TOOLS: ["claude", "codex", "gemini"],
}));

vi.mock("@/lib/api/proxy", () => ({
  proxyApi: {
    setProxyTakeoverForApp: vi.fn(async () => undefined),
  },
}));

const TAURI_ENDPOINT = "http://tauri.local";

const baseSettings = {
  showInTray: true,
  minimizeToTrayOnClose: true,
  webdavSync: undefined,
};

describe("bindTools.mirrorBoundToolsToSettings", () => {
  beforeEach(() => {
    emitMock.mockClear();
    localStorage.clear();
  });

  it("bindTools writes localStorage AND mirrors boundTools to settings", async () => {
    let savedPayload: Record<string, unknown> | null = null;
    let fullSettingsReads = 0;

    server.use(
      http.post(`${TAURI_ENDPOINT}/get_settings`, () => {
        fullSettingsReads += 1;
        return HttpResponse.json(baseSettings);
      }),
      http.post(`${TAURI_ENDPOINT}/save_bound_tools`, async ({ request }) => {
        savedPayload = (await request.json()) as Record<string, unknown>;
        return HttpResponse.json(true);
      }),
      // bindTools calls ensureDefaultModel() after each bind, which reads the
      // active model first. Without a stub, MSW passes the request through
      // (setupTests.ts uses onUnhandledRequest: "warn"), the call to
      // tauri.local hangs, and the test dies at the 5s timeout. A non-empty
      // value makes ensureDefaultModel return early.
      http.post(`${TAURI_ENDPOINT}/get_active_ofox_model`, () =>
        HttpResponse.json("ofox-default"),
      ),
      // Stub the actual bind invocation — we only care about the mirror path.
      http.post(`${TAURI_ENDPOINT}/ofox_bind_tool`, () =>
        HttpResponse.json(undefined),
      ),
    );

    const { bindTools } = await import("@/lib/bindTools");
    await bindTools(["claude", "codex"]);

    // Mirror is fire-and-forget; settle pending microtasks.
    await new Promise((r) => setTimeout(r, 30));

    expect(localStorage.getItem("bound_tools")).toBe(
      JSON.stringify(["claude", "codex"]),
    );
    expect(savedPayload).not.toBeNull();
    expect(
      (savedPayload as unknown as { boundTools?: unknown })?.boundTools,
    ).toEqual(["claude", "codex"]);
    // No full snapshot can reset a concurrently changed region or key record.
    expect(savedPayload).toEqual({ boundTools: ["claude", "codex"] });
    expect(fullSettingsReads).toBe(0);

    // Emit hook fired
    expect(emitMock).toHaveBeenCalledWith("ofox-prefs-updated");
  });

  it("unbindTool drops the tool from boundTools", async () => {
    localStorage.setItem("bound_tools", JSON.stringify(["claude", "codex"]));

    let savedPayload: Record<string, unknown> | null = null;

    server.use(
      http.post(`${TAURI_ENDPOINT}/get_settings`, () =>
        HttpResponse.json({
          ...baseSettings,
          boundTools: ["claude", "codex"],
        }),
      ),
      http.post(`${TAURI_ENDPOINT}/save_bound_tools`, async ({ request }) => {
        savedPayload = (await request.json()) as Record<string, unknown>;
        return HttpResponse.json(true);
      }),
      http.post(`${TAURI_ENDPOINT}/ofox_unbind_tool`, () =>
        HttpResponse.json(undefined),
      ),
    );

    // Re-import after the localStorage seeding so module has it on first run.
    vi.resetModules();
    const { unbindTool } = await import("@/lib/bindTools");
    await unbindTool("codex");
    await new Promise((r) => setTimeout(r, 30));

    expect(localStorage.getItem("bound_tools")).toBe(
      JSON.stringify(["claude"]),
    );
    expect(
      (savedPayload as unknown as { boundTools?: unknown })?.boundTools,
    ).toEqual(["claude"]);
  });

  it("unbindTool keeps the tool bound when the backend rejects", async () => {
    localStorage.setItem("bound_tools", JSON.stringify(["claude", "codex"]));
    server.use(
      http.post(`${TAURI_ENDPOINT}/ofox_unbind_tool`, () =>
        HttpResponse.text("restore failed", { status: 500 }),
      ),
    );

    vi.resetModules();
    const { unbindTool } = await import("@/lib/bindTools");
    await expect(unbindTool("codex")).rejects.toThrow("restore failed");

    expect(localStorage.getItem("bound_tools")).toBe(
      JSON.stringify(["claude", "codex"]),
    );
  });

  it("unbindTool tells the backend which tools stay bound and returns the report", async () => {
    localStorage.setItem(
      "bound_tools",
      JSON.stringify(["claude", "codex", "chatgpt"]),
    );
    let request: Record<string, unknown> | null = null;
    const report = { tool: "codex", sharedKeptBy: ["chatgpt"] };
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_settings`, () =>
        HttpResponse.json(baseSettings),
      ),
      http.post(`${TAURI_ENDPOINT}/save_bound_tools`, () =>
        HttpResponse.json(true),
      ),
      http.post(
        `${TAURI_ENDPOINT}/ofox_unbind_tool`,
        async ({ request: r }) => {
          request = (await r.json()) as Record<string, unknown>;
          return HttpResponse.json(report);
        },
      ),
    );

    vi.resetModules();
    const { unbindTool } = await import("@/lib/bindTools");
    await expect(unbindTool("codex")).resolves.toEqual(report);

    expect(request).toEqual({
      app: "codex",
      stillBound: ["claude", "chatgpt"],
    });
    expect(localStorage.getItem("bound_tools")).toBe(
      JSON.stringify(["claude", "chatgpt"]),
    );
  });

  it("bindTools binds chatgpt through the backend so ~/.codex is configured", async () => {
    const bound: string[] = [];
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_settings`, () =>
        HttpResponse.json(baseSettings),
      ),
      http.post(`${TAURI_ENDPOINT}/save_bound_tools`, () =>
        HttpResponse.json(true),
      ),
      http.post(`${TAURI_ENDPOINT}/get_active_ofox_model`, () =>
        HttpResponse.json("ofox-default"),
      ),
      http.post(`${TAURI_ENDPOINT}/ofox_bind_tool`, async ({ request }) => {
        bound.push(((await request.json()) as { app: string }).app);
        return HttpResponse.json(undefined);
      }),
    );

    vi.resetModules();
    const { bindTools } = await import("@/lib/bindTools");
    await bindTools(["chatgpt"]);

    expect(bound).toEqual(["chatgpt"]);
  });

  it("adds only new bindings and never rewrites a retained tool's deleted configuration", async () => {
    localStorage.setItem("bound_tools", JSON.stringify(["workbuddy", "codex"]));
    const bound: string[] = [];
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_settings`, () =>
        HttpResponse.json(baseSettings),
      ),
      http.post(`${TAURI_ENDPOINT}/save_bound_tools`, () =>
        HttpResponse.json(true),
      ),
      http.post(`${TAURI_ENDPOINT}/get_active_ofox_model`, () =>
        HttpResponse.json("saved-model"),
      ),
      http.post(`${TAURI_ENDPOINT}/ofox_bind_tool`, async ({ request }) => {
        bound.push(((await request.json()) as { app: string }).app);
        return HttpResponse.json(undefined);
      }),
    );
    const { bindTools } = await import("@/lib/bindTools");
    const retained = await bindTools([
      "workbuddy",
      "codex",
      "claude",
      "claude",
    ]);
    expect(bound).toEqual(["claude"]);
    expect(retained).toEqual(["workbuddy", "codex", "claude"]);
    expect(JSON.parse(localStorage.getItem("bound_tools")!)).toEqual(retained);
  });

  it("retains every existing binding when adding a new tool fails", async () => {
    localStorage.setItem(
      "bound_tools",
      JSON.stringify(["workbuddy", "chatgpt"]),
    );
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_settings`, () =>
        HttpResponse.json(baseSettings),
      ),
      http.post(`${TAURI_ENDPOINT}/save_bound_tools`, () =>
        HttpResponse.json(true),
      ),
      http.post(`${TAURI_ENDPOINT}/ofox_bind_tool`, () =>
        HttpResponse.text("binding failed", { status: 500 }),
      ),
    );
    const { bindTools } = await import("@/lib/bindTools");
    expect(await bindTools(["codex"])).toEqual(["workbuddy", "chatgpt"]);
    expect(JSON.parse(localStorage.getItem("bound_tools")!)).toEqual([
      "workbuddy",
      "chatgpt",
    ]);
  });

  it("retains the UI binding when backend record cleanup reports a failure", async () => {
    localStorage.setItem("bound_tools", JSON.stringify(["codex", "chatgpt"]));
    server.use(
      http.post(`${TAURI_ENDPOINT}/ofox_unbind_tool`, () =>
        HttpResponse.json({
          tool: "codex",
          warnings: [{ code: "recordCleanupFailed" }],
        }),
      ),
    );
    const { unbindTool } = await import("@/lib/bindTools");
    await expect(unbindTool("codex")).rejects.toThrow();
    expect(JSON.parse(localStorage.getItem("bound_tools")!)).toEqual([
      "codex",
      "chatgpt",
    ]);
  });
});
