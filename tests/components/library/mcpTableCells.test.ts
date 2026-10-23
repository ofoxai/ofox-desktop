import { describe, expect, it } from "vitest";
import type {
  LibraryInventory,
  McpDefinition,
  ResourceTarget,
} from "@/lib/api/library";
import { MCP_TABLE_TOOLS, mcpCell } from "@/components/library/mcpTableCells";

const target = (toolId: string, extra: Partial<ResourceTarget> = {}) => ({
  toolId,
  path: `${toolId}.json`,
  targetGroupId: toolId,
  available: true,
  discoveryStatus: "ready",
  status: "unassigned",
  desired: false,
  ...extra,
});
const row = (
  targets: ResourceTarget[],
  extra: Partial<McpDefinition> = {},
): McpDefinition => ({
  id: "context7",
  name: "context7",
  revision: 1,
  transport: "http",
  credentialState: "nativeEnvironmentRequired",
  fields: [],
  targets,
  ...extra,
});
const inventory = (
  entries: Partial<LibraryInventory["entries"][number]>[],
): LibraryInventory => ({
  ticketId: "t",
  kind: "mcp",
  expiresAtMs: Date.now() + 60_000,
  sources: [],
  entries: entries.map((entry, index) => ({
    entryId: `e${index}`,
    kind: "mcp",
    name: "context7",
    sources: [],
    matchedResourceIds: [],
    conflict: false,
    ...entry,
  })),
});

describe("mcpCell", () => {
  it("lists the MCP-capable bindable agents in the tools page order", () => {
    expect(MCP_TABLE_TOOLS).toEqual([
      "claude",
      "codex",
      "opencode",
      "openclaw",
      "hermes",
      "gemini",
      "workbuddy",
      "dsh",
      "pi",
    ]);
  });
  it("is checked and toggleable once Ofox applied it, or while it is assigned", () => {
    expect(
      mcpCell(
        row([target("codex", { desired: true, status: "applied" })]),
        "codex",
      ),
    ).toMatchObject({ state: "on", checked: true, toggleable: true });
    expect(
      mcpCell(
        row([target("codex", { desired: true, status: "pending" })]),
        "codex",
      ),
    ).toMatchObject({ state: "pending", checked: true, toggleable: true });
  });
  it("counts an identical entry already in the tool as checked", () => {
    const scan = inventory([
      {
        matchedResourceIds: ["context7"],
        sources: [{ sourceId: "s", toolId: "codex" }],
      },
    ]);
    expect(mcpCell(row([target("codex")]), "codex", scan)).toMatchObject({
      state: "present",
      checked: true,
      toggleable: true,
    });
  });
  it("leaves an entry the tool switched off unchecked and untouchable", () => {
    const scan = inventory([
      {
        matchedResourceIds: ["context7"],
        sources: [{ sourceId: "s", toolId: "codex", disabled: true }],
      },
    ]);
    expect(mcpCell(row([target("codex")]), "codex", scan)).toMatchObject({
      state: "presentOff",
      checked: false,
      toggleable: false,
    });
  });
  it("refuses a tool holding a different entry under the same name", () => {
    const scan = inventory([
      { name: "Context7", sources: [{ sourceId: "s", toolId: "codex" }] },
    ]);
    expect(mcpCell(row([target("codex")]), "codex", scan)).toMatchObject({
      state: "conflict",
      toggleable: false,
    });
    expect(mcpCell(row([target("claude")]), "claude", scan).state).toBe("off");
  });
  it("explains tools that are missing, unsupported, drifted, or app-owned", () => {
    expect(
      mcpCell(row([target("gemini", { available: false })]), "gemini").state,
    ).toBe("unavailable");
    expect(
      mcpCell(
        row([
          target("codex", { disabledReason: "library.transportUnsupported" }),
        ]),
        "codex",
      ),
    ).toMatchObject({
      state: "unsupported",
      reason: "library.transportUnsupported",
    });
    expect(
      mcpCell(
        row([target("codex", { desired: true, status: "conflict" })]),
        "codex",
      ),
    ).toMatchObject({ state: "conflict", checked: true, toggleable: false });
    expect(
      mcpCell(row([target("codex")], { appOwned: true }), "codex").state,
    ).toBe("appOwned");
    expect(mcpCell(row([]), "codex").state).toBe("unsupported");
  });
  it("does not mistake an old Pi config file for native MCP support", () => {
    const scan = inventory([
      {
        matchedResourceIds: ["context7"],
        sources: [{ sourceId: "pi", toolId: "pi" }],
      },
    ]);
    expect(
      mcpCell(
        row([
          target("pi", {
            available: false,
            discoveryStatus: "unsupported",
            disabledReason: "library.piMcpVersion",
          }),
        ]),
        "pi",
        scan,
      ),
    ).toMatchObject({
      state: "unsupported",
      checked: false,
      toggleable: false,
      reason: "library.piMcpVersion",
    });
  });
});
