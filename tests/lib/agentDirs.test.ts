import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { server } from "../msw/server";
import { getAgentDirs, type AgentDir } from "@/lib/api/agentDirs";

const dirs: AgentDir[] = [
  {
    tool: "codex",
    dir: "/data/codex",
    source: "loginShell",
    envVar: "CODEX_HOME",
  },
  {
    tool: "claude",
    dir: "/home/u/.claude",
    source: "default",
    envVar: "CLAUDE_CONFIG_DIR",
  },
];

describe("agent directories API", () => {
  it("reads the directories and their sources from the backend", async () => {
    server.use(
      http.post("http://tauri.local/get_agent_dirs", () =>
        HttpResponse.json(dirs),
      ),
    );

    expect(await getAgentDirs()).toEqual(dirs);
  });
});
