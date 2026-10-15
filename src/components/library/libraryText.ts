import type { TFunction } from "i18next";
export function libraryError(error: unknown, t: TFunction) {
  const key = String(error).match(/\blibrary\.[A-Za-z]+\b/)?.[0];
  return key
    ? t(key, { defaultValue: t("library.failed") })
    : t("library.failed");
}
export function toolLabel(tool: string, t: TFunction) {
  return tool === "shared"
    ? t("library.sharedDirectory")
    : ({
        claude: "Claude Code",
        codex: "Codex",
        gemini: "Gemini CLI",
        opencode: "OpenCode",
        hermes: "Hermes",
      }[tool] ?? tool);
}
