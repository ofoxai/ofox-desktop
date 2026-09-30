import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { server } from "../msw/server";
import {
  manageToolApi,
  managedToolId,
  TOOL_PROTOCOL,
} from "@/lib/api/manageTool";

describe("ChatGPT desktop Codex configuration", () => {
  it("uses Codex's config for every model-management command", async () => {
    const calls: Array<{ command: string; app: string }> = [];
    const responses: Record<string, string | object | null> = {
      get_tool_config_file_path: "/home/test/.codex/config.toml",
      get_active_ofox_model: "openai/gpt-6-luna",
      set_active_ofox_model: null,
      check_ofox_model_compatibility: {
        app: "codex",
        model: "openai/gpt-6-luna",
        protocol: "responses",
        status: "compatible",
        source: "catalog",
        reason: null,
      },
      ofox_ping_model: {
        success: true,
        latencyMs: 1,
        statusCode: 200,
        error: null,
      },
    };
    for (const [command, response] of Object.entries(responses)) {
      server.use(
        http.post(`http://tauri.local/${command}`, async ({ request }) => {
          const body = (await request.json()) as { app: string };
          calls.push({ command, app: body.app });
          return HttpResponse.json(response);
        }),
      );
    }

    expect(TOOL_PROTOCOL.chatgpt).toBe("openai");
    expect(managedToolId("chatgpt")).toBe("codex");
    expect(managedToolId("claude")).toBe("claude");
    expect(await manageToolApi.getConfigFilePath("chatgpt")).toBe(
      "/home/test/.codex/config.toml",
    );
    expect(await manageToolApi.getActiveModel("chatgpt")).toBe(
      "openai/gpt-6-luna",
    );
    await manageToolApi.setActiveModel("chatgpt", "openai/gpt-6-luna");
    await manageToolApi.checkCompatibility("chatgpt", "openai/gpt-6-luna");
    await manageToolApi.pingModel("chatgpt", "openai/gpt-6-luna");
    expect(calls).toHaveLength(5);
    expect(calls.map(({ app }) => app)).toEqual(Array(5).fill("codex"));
  });
});
