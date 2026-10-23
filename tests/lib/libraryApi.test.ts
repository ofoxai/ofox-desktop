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
it("confirms the resource pack using camelCase frozen fingerprint, revision and epoch", async () => {
  await libraryApi.restorePack(
    "/sandbox/drafts.zip",
    { fingerprint: "hash-before-confirm", revision: 7, epoch: 11, items: [] },
    "copy",
  );
  expect(invoke).toHaveBeenCalledWith("restore_library_pack", {
    path: "/sandbox/drafts.zip",
    expectedFingerprint: "hash-before-confirm",
    expectedRevision: 7,
    expectedEpoch: 11,
    conflicts: "copy",
  });
});
it("uses a dedicated selected-export command so an older backend cannot silently export every resource", async () => {
  await libraryApi.exportSelected("mcp", ["selected-server"]);
  expect(invoke).toHaveBeenCalledWith("export_library_selection", {
    selection: { kind: "mcp", resourceIds: ["selected-server"] },
  });
  expect(invoke).not.toHaveBeenCalledWith(
    "export_library_pack",
    expect.anything(),
  );
});

it("fences explicit Skills replacement and keep decisions with the opened revision and epoch", async () => {
  await libraryApi.previewSkillReplacement("skill", "codex", 7, 19);
  expect(invoke).toHaveBeenCalledWith("preview_library_skill_replacement", {
    resourceId: "skill",
    tool: "codex",
    expectedRevision: 7,
    expectedEpoch: 19,
  });
  await libraryApi.keepSkillNative("skill", "codex", 7, 19);
  expect(invoke).toHaveBeenCalledWith("keep_library_skill_native", {
    resourceId: "skill",
    tool: "codex",
    expectedRevision: 7,
    expectedEpoch: 19,
  });
});
