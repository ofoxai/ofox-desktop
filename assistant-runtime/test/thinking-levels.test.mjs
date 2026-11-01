import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import {
  generateThinkingLevels,
  tableKey,
  tablePath,
} from "../../scripts/assistant-thinking-levels.mjs";

test("the host's thinking-level table is what the pinned Pi SDK's model data gives", async () => {
  assert.equal(
    await readFile(tablePath, "utf8"),
    await generateThinkingLevels(),
    "stale: run `node scripts/assistant-thinking-levels.mjs`",
  );
  const table = JSON.parse(await readFile(tablePath, "utf8"));
  const runtime = JSON.parse(
    await readFile(new URL("../package.json", import.meta.url), "utf8"),
  );
  assert.equal(table.piAi, runtime.dependencies["@earendil-works/pi-ai"]);
});

test("table keys follow Ofox ids: lowercase, Anthropic versions with hyphens", async () => {
  assert.equal(
    tableKey("anthropic", "claude-haiku-5.5"),
    "anthropic/claude-haiku-5-5",
  );
  assert.equal(tableKey("minimax", "MiniMax-M3"), "minimax/minimax-m3");
  assert.equal(
    tableKey("google", "gemini-3.1-flash-lite"),
    "google/gemini-3.1-flash-lite",
  );
  const { models } = JSON.parse(await readFile(tablePath, "utf8"));
  // Pi's rule: a level its map marks null is not offered; xhigh and max only
  // when mapped.
  assert.deepEqual(models["anthropic/claude-haiku-5-5"], {
    anthropicCompat: {
      forceAdaptiveThinking: true,
      supportsTemperature: false,
    },
    levels: ["off", "low", "medium", "high", "xhigh", "max"],
    thinkingLevelMap: {
      high: "high",
      low: "low",
      max: "max",
      medium: "medium",
      minimal: null,
      xhigh: "xhigh",
    },
  });
  assert.deepEqual(models["google/gemini-3.1-flash-lite"].levels, [
    "minimal",
    "low",
    "medium",
    "high",
  ]);
  assert.equal(models["openai/gpt-6-luna"].thinkingLevelMap.off, "none");
  // Only reasoning chat models are listed.
  assert.equal(models["openai/gpt-4o"], undefined);
});
