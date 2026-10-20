import type { TFunction } from "i18next";
import { TOOL_META } from "@/config/toolMeta";
import type { RequestLog } from "@/types/usage";

const LOCAL_SESSION_TOOLS: Record<string, string> = {
  session_log: "claude",
  codex_session: "codex",
  gemini_session: "gemini",
  opencode_session: "opencode",
  workbuddy_session: "workbuddy",
  pi_session: "pi",
  dsh_session: "dsh",
  openclaw_session: "openclaw",
  hermes_session: "hermes",
};

// Hermes keeps per-session, per-model totals rather than per-request rows.
const AGGREGATE_SOURCES = new Set(["hermes_session"]);

export function requestSourcePresentation(
  source: string | undefined,
  t: TFunction,
) {
  const tool =
    source && Object.prototype.hasOwnProperty.call(LOCAL_SESSION_TOOLS, source)
      ? LOCAL_SESSION_TOOLS[source]
      : undefined;
  if (tool) {
    const label = t("usageWorkbench.localSessionImport");
    const toolName = TOOL_META[tool].label;
    return {
      label,
      filterLabel: `${toolName} · ${label}`,
      description: t(
        AGGREGATE_SOURCES.has(source ?? "")
          ? "usageWorkbench.localSessionAggregateHelp"
          : "usageWorkbench.localSessionSourceHelp",
        { tool: toolName },
      ),
    };
  }
  const measuredSource = source == null || source === "proxy";
  const label = t(
    measuredSource
      ? "usageWorkbench.localRequestRecord"
      : "usageWorkbench.otherSourceRecord",
  );
  return {
    label,
    filterLabel: label,
    description: t(
      measuredSource
        ? "usageWorkbench.localRequestSourceHelp"
        : "usageWorkbench.otherSourceHelp",
    ),
  };
}

const nonnegative = (value: number | null | undefined) =>
  typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : null;

/** Explicit null beats compatibility columns, even on historical proxy rows. */
export function requestMeasurements(request: RequestLog) {
  const legacyMeasured =
    request.dataSource == null || request.dataSource === "proxy";
  const code =
    "observedStatusCode" in request
      ? request.observedStatusCode
      : legacyMeasured
        ? request.statusCode
        : null;
  return {
    status:
      typeof code === "number" &&
      Number.isInteger(code) &&
      code >= 100 &&
      code <= 599
        ? code
        : null,
    latency: nonnegative(
      "observedLatencyMs" in request
        ? request.observedLatencyMs
        : legacyMeasured
          ? request.latencyMs
          : null,
    ),
    // firstTokenMs is the first SSE event, not proof of generated content.
    firstToken: nonnegative(request.observedFirstTokenMs),
  };
}
