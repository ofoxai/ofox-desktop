export function updateCompatibilityIssue(error: unknown): {
  key: "update.requiresMacOS" | "update.compatibilityUnknown";
  version?: string;
} | null {
  const code = error instanceof Error ? error.message : error;
  if (typeof code !== "string") return null;
  const minimum = /^updateRequiresMacOS:(\d+\.\d+(?:\.\d+)?)$/.exec(code);
  if (minimum) return { key: "update.requiresMacOS", version: minimum[1] };
  if (code === "updateCompatibilityUnknown" || code === "invalidUpdateManifest")
    return { key: "update.compatibilityUnknown" };
  return null;
}
