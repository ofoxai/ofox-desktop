import { expect, it, vi } from "vitest";
const invoke = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
import { libraryApi } from "@/lib/api/library";
it("includes the opened epoch in MCP saves rather than only the resource revision", async () => {
  await libraryApi.saveMcp("server", "Draft", 3, [], 44);
  expect(invoke).toHaveBeenCalledWith("save_library_mcp", {
    resourceId: "server",
    name: "Draft",
    expectedRevision: 3,
    edits: [],
    expectedEpoch: 44,
  });
});
