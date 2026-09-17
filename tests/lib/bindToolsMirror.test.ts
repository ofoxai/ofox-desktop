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

    server.use(
      http.post(`${TAURI_ENDPOINT}/get_settings`, () =>
        HttpResponse.json(baseSettings),
      ),
      http.post(`${TAURI_ENDPOINT}/save_settings`, async ({ request }) => {
        const body = (await request.json()) as { settings: unknown };
        savedPayload = body.settings as Record<string, unknown>;
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
    // Spread should preserve every other field
    expect(
      (savedPayload as unknown as { showInTray?: boolean })?.showInTray,
    ).toBe(true);

    // Emit hook fired
    expect(emitMock).toHaveBeenCalledWith("ofox-prefs-updated");
  });

  it("unbindTool drops the tool from boundTools", async () => {
    localStorage.setItem(
      "bound_tools",
      JSON.stringify(["claude", "codex"]),
    );

    let savedPayload: Record<string, unknown> | null = null;

    server.use(
      http.post(`${TAURI_ENDPOINT}/get_settings`, () =>
        HttpResponse.json({
          ...baseSettings,
          boundTools: ["claude", "codex"],
        }),
      ),
      http.post(`${TAURI_ENDPOINT}/save_settings`, async ({ request }) => {
        const body = (await request.json()) as { settings: unknown };
        savedPayload = body.settings as Record<string, unknown>;
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
});
