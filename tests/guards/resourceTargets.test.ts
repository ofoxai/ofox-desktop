// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { resourceClientIds } from "../../scripts/resource-targets.mjs";
import { TOOL_ORDER } from "@/config/toolMeta";
import {
  RESOURCE_TARGET_IDS,
  resourceTargetId,
} from "@/config/resourceTargets";
import { skillColumns } from "@/components/library/skillCells";
import { MCP_TABLE_TOOLS } from "@/components/library/mcpTableCells";

describe("resource target catalogue parity", () => {
  it("gives every bindable client one MCP and Skills target in the Rust registry", () => {
    const rust = readFileSync(
      new URL(
        "../../src-tauri/src/services/library/extension_targets.rs",
        import.meta.url,
      ),
      "utf8",
    );
    const targetIds = [...rust.matchAll(/id: "([a-z-]+)"/g)].map(
      (match) => match[1],
    );
    expect(new Set(targetIds)).toEqual(new Set(RESOURCE_TARGET_IDS));
    expect(TOOL_ORDER).toHaveLength(10);
    expect(resourceClientIds).toEqual(TOOL_ORDER);
    expect(RESOURCE_TARGET_IDS).toHaveLength(9);
    expect(MCP_TABLE_TOOLS).toEqual(RESOURCE_TARGET_IDS);
    expect(
      skillColumns(
        targetIds.map((id) => ({
          id,
          label: id,
          detected: true,
          readsShared: false,
          scopeKnown: true,
        })),
        [],
      ),
    ).toEqual(RESOURCE_TARGET_IDS);
    expect(resourceTargetId("chatgpt")).toBe("codex");
  });
});
