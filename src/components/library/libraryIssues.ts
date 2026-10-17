import type { McpCheckResult } from "@/lib/api/library";

export type LibraryNextAction =
  | { kind: "edit"; focusPath: string[] }
  | { kind: "nativeGuide" }
  | { kind: "rescan" }
  | { kind: "reconcile" }
  | { kind: "repreview" };
export interface LibraryIssue {
  messageKey: string;
  actionKey: string;
  action: LibraryNextAction;
}
/** This is a presentation projection, never new evidence or permission to write. */
export function checkIssue(result: McpCheckResult): LibraryIssue | undefined {
  if (["ok", "cancelled"].includes(result.state)) return;
  const edit = (field: string, key: string): LibraryIssue => ({
    messageKey: key,
    actionKey: key,
    action: { kind: "edit", focusPath: field ? [field] : [] },
  });
  if (result.reason === "programMissing") return edit("command", "command");
  if (result.reason === "cwdMissing") return edit("cwd", "directory");
  if (result.reason === "credentialMissing") return edit("", "credentials");
  if (
    ["runnerBlocked", "authRequired", "forbidden"].includes(
      result.reason ?? "",
    ) ||
    ["authRequired", "forbidden"].includes(result.state)
  )
    return {
      messageKey:
        result.reason === "runnerBlocked" ? "unsupported" : "authorization",
      actionKey: "nativeGuide",
      action: { kind: "nativeGuide" },
    };
  return;
}

export function batchIssue(status: string): LibraryIssue | undefined {
  if (["prepared", "needsReview", "partial"].includes(status))
    return {
      messageKey: "incomplete",
      actionKey: "reconcile",
      action: { kind: "reconcile" },
    };
  if (status === "cancelled")
    return {
      messageKey: "cancelled",
      actionKey: "repreview",
      action: { kind: "repreview" },
    };
  return;
}
