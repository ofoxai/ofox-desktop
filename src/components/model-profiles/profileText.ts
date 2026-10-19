import { TOOL_META } from "@/config/toolMeta";
import type { ModelSelection } from "@/lib/api/modelSelection";

/** A profile target's selection as one line of text. */
export function selectionText(
  selection: ModelSelection | null,
  emptyLabel: string,
) {
  if (!selection) return emptyLabel;
  return selection.kind === "models"
    ? selection.models.join(", ")
    : selection.model || emptyLabel;
}

/** Codex and ChatGPT share one write target, named for both. */
export function targetLabel(target: string, shared: string) {
  return target === "codex" ? shared : (TOOL_META[target]?.label ?? target);
}

/** pi and dsh may run models outside the Ofox catalog: they take a typed id too. */
export const explicitTarget = (target: string) =>
  target === "pi" || target === "dsh";
