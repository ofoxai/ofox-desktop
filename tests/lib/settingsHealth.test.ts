import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { server } from "../msw/server";
import {
  getSettingsHealth,
  resetUnreadableSettings,
  type SettingsHealth,
} from "@/lib/api/settings";

const unreadable: SettingsHealth = {
  state: "unreadable",
  path: "/home/u/.ofox-desktop/settings.json",
  badCopy: "/home/u/.ofox-desktop/settings.json.bad-20261010-120000",
  detail: "64 bytes, every one zero",
};

describe("settings health API", () => {
  it("reads the health the backend reports", async () => {
    server.use(
      http.post("http://tauri.local/get_settings_health", () =>
        HttpResponse.json(unreadable),
      ),
    );

    expect(await getSettingsHealth()).toEqual(unreadable);
  });

  it("resets unreadable settings through the backend and returns the new health", async () => {
    const calls: string[] = [];
    server.use(
      http.post("http://tauri.local/reset_unreadable_settings", () => {
        calls.push("reset_unreadable_settings");
        return HttpResponse.json({
          state: "ok",
          path: unreadable.path,
          badCopy: null,
          detail: null,
        } satisfies SettingsHealth);
      }),
    );

    const health = await resetUnreadableSettings();

    expect(calls).toEqual(["reset_unreadable_settings"]);
    expect(health.state).toBe("ok");
  });
});
