import type { TFunction } from "i18next";
import type { LibraryInventory } from "@/lib/api/library";

export function inventorySourceLabel(
  inventory: LibraryInventory,
  entry: LibraryInventory["entries"][number],
  t: TFunction,
) {
  const fallback = t("library.workbench.nativeSource");
  const labels = entry.sources.map((reference) => {
    const source = inventory.sources.find(
      (source) =>
        source.sourceId === reference.sourceId &&
        source.toolId === reference.toolId,
    );
    return source
      ? t(`library.workbench.source.${source.label}`, {
          defaultValue: fallback,
        })
      : fallback;
  });
  return [...new Set(labels.length ? labels : [fallback])].join(" · ");
}

export function inventoryEntryDescription(
  inventory: LibraryInventory,
  entry: LibraryInventory["entries"][number],
  t: TFunction,
) {
  const state = entry.matchedResourceIds.length
    ? t("library.workbench.alreadyLibrary")
    : entry.conflict
      ? t("library.workbench.importVariant")
      : t("library.workbench.draftOnly");
  return [inventorySourceLabel(inventory, entry, t), state]
    .filter(Boolean)
    .join(" · ");
}

export function libraryError(error: unknown, t: TFunction) {
  // The backend appends GitHub's reset time as `@<unix seconds>`.
  const limited = String(error).match(
    /\blibrary\.githubRateLimited(?:@(\d+))?/,
  );
  if (limited)
    return limited[1]
      ? t("library.githubRateLimitedUntil", {
          time: new Date(Number(limited[1]) * 1000).toLocaleTimeString([], {
            hour: "2-digit",
            minute: "2-digit",
          }),
        })
      : t("library.githubRateLimited");
  const key = String(error).match(/\blibrary\.[A-Za-z]+\b/)?.[0];
  return key
    ? t(key, { defaultValue: t("library.failed") })
    : t("library.failed");
}
export function toolLabel(tool: string, t: TFunction) {
  if (tool === "ofox-assistant") return t("library.assistantTarget.label");
  return tool === "shared"
    ? t("library.sharedDirectory")
    : ({
        claude: "Claude Code",
        codex: "Codex",
        chatgpt: "ChatGPT",
        openclaw: "OpenClaw",
        workbuddy: "WorkBuddy",
        dsh: "DeepSeek Harness",
        pi: "Pi",
        copilot: "GitHub Copilot CLI",
        gemini: "Gemini CLI",
        opencode: "OpenCode",
        hermes: "Hermes",
      }[tool] ?? tool);
}

export function libraryCommandUnavailable(reason: unknown) {
  const message = reason instanceof Error ? reason.message : String(reason);
  return /(?:unknown|missing|not found).*command|command.*(?:not found|unknown|does not exist)/i.test(
    message,
  );
}
