import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// The assistant's thinking-level table, built from the Pi SDK's own model
// data: for each reasoning model of an Ofox vendor Pi knows, the levels Pi
// offers (its getSupportedThinkingLevels), the map from a level to what the
// provider is sent, and the Anthropic compat that adaptive thinking needs.
// The host embeds the table; the runtime gets a model's map with a run.
//
//   node scripts/assistant-thinking-levels.mjs   rewrites the table
//
// assistant-runtime/test/thinking-levels.test.mjs regenerates it in memory
// and fails when the committed copy differs.

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const piAi = path.join(
  root,
  "assistant-runtime/node_modules/@earendil-works/pi-ai",
);
export const tablePath = path.join(
  root,
  "src-tauri/src/services/assistant/thinking_levels.json",
);

/** Ofox vendor (a model id's prefix) → Pi provider data file. */
const PROVIDERS = {
  anthropic: "anthropic",
  deepseek: "deepseek",
  google: "google",
  minimax: "minimax",
  moonshotai: "moonshotai",
  openai: "openai",
  "x-ai": "xai",
  "z-ai": "zai",
};

/** The Anthropic compat fields adaptive thinking reads. */
const ANTHROPIC_COMPAT = ["forceAdaptiveThinking", "supportsTemperature"];

/**
 * A model's key in the table: `vendor/name`, lowercase. Anthropic writes
 * versions with hyphens where Ofox writes dots (claude-haiku-5.5 is Pi's
 * claude-haiku-5-5). The host normalises Ofox ids the same way.
 */
export function tableKey(vendor, name) {
  const lower = name.toLowerCase();
  return `${vendor}/${vendor === "anthropic" ? lower.replaceAll(".", "-") : lower}`;
}

/** `value` with every object's keys sorted, so the output is deterministic. */
function sorted(value) {
  if (Array.isArray(value)) return value.map(sorted);
  if (value && typeof value === "object")
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, sorted(value[key])]),
    );
  return value;
}

async function entries() {
  const { getSupportedThinkingLevels } = await import(
    pathToFileURL(path.join(piAi, "dist/models.js")).href
  );
  const models = {};
  for (const [vendor, provider] of Object.entries(PROVIDERS)) {
    const data = JSON.parse(
      await readFile(
        path.join(piAi, "dist/providers/data", `${provider}.json`),
        "utf8",
      ),
    );
    for (const [api, byKey] of Object.entries(data))
      for (const model of Object.values(byKey)) {
        if ((model.type ?? "chat") !== "chat" || model.reasoning !== true)
          continue;
        const key = tableKey(vendor, model.id);
        if (key in models)
          throw new Error(`Two Pi models share the table key ${key}`);
        const entry = { levels: getSupportedThinkingLevels(model) };
        if (model.thinkingLevelMap)
          entry.thinkingLevelMap = model.thinkingLevelMap;
        const compat = Object.fromEntries(
          ANTHROPIC_COMPAT.filter(
            (field) => model.compat?.[field] !== undefined,
          ).map((field) => [field, model.compat[field]]),
        );
        if (api === "anthropic-messages" && Object.keys(compat).length)
          entry.anthropicCompat = compat;
        models[key] = sorted(entry);
      }
  }
  return models;
}

/** The table's text: one model per line, keys sorted. */
export async function generateThinkingLevels() {
  const { version } = JSON.parse(
    await readFile(path.join(piAi, "package.json"), "utf8"),
  );
  const models = await entries();
  const lines = Object.keys(models)
    .sort()
    .map((key) => `    ${JSON.stringify(key)}: ${JSON.stringify(models[key])}`);
  return `{\n  "piAi": ${JSON.stringify(version)},\n  "models": {\n${lines.join(",\n")}\n  }\n}\n`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await writeFile(tablePath, await generateThinkingLevels());
  console.log(`Wrote ${path.relative(root, tablePath)}`);
}
