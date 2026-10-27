import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  assistantApi,
  type AssistantEvent,
  type AssistantUsageSummary,
} from "@/lib/api/assistant";
import { Button } from "@/components/ui/button";

/** Independent first-party source; never folded into CLI/Pi session scans. */
export function AssistantUsageCard({
  identity,
  active,
}: {
  identity: string;
  active: boolean;
}) {
  const { t, i18n } = useTranslation();
  const [result, setResult] = useState<{
    identity: string;
    data: AssistantUsageSummary;
  } | null>(null);
  const [error, setError] = useState(false);
  const [busy, setBusy] = useState(false);
  const sequence = useRef(0);
  const refresh = useCallback(async () => {
    if (!active || !identity) return;
    const request = ++sequence.current;
    setBusy(true);
    setError(false);
    try {
      const data = await assistantApi.usage();
      if (request === sequence.current) setResult({ identity, data });
    } catch {
      if (request === sequence.current) {
        setResult(null);
        setError(true);
      }
    } finally {
      if (request === sequence.current) setBusy(false);
    }
  }, [identity, active]);
  useEffect(() => {
    void refresh();
    return () => {
      sequence.current += 1;
    };
  }, [refresh]);
  useEffect(() => {
    if (!active || !identity) return;
    let disposed = false;
    const cleanup: (() => void)[] = [];
    for (const name of [
      "ofox-auth-expired",
      "ofox-auth-restored",
      "assistant:event",
    ]) {
      void listen<AssistantEvent>(name, ({ payload }) => {
        if (disposed) return;
        if (name === "ofox-auth-expired") {
          sequence.current += 1;
          setResult(null);
          setBusy(false);
          return;
        }
        if (name === "ofox-auth-restored" || payload?.type === "run_end")
          void refresh();
      })
        .then((off) => (disposed ? off() : cleanup.push(off)))
        .catch(() => undefined);
    }
    return () => {
      disposed = true;
      cleanup.forEach((off) => off());
    };
  }, [active, identity, refresh]);
  if (!identity) return null;
  const data = result?.identity === identity ? result.data : null;
  const number = (value: number | undefined) =>
    value == null ? "—" : value.toLocaleString(i18n.language);
  const money = (value: string | null | undefined) =>
    value == null ? t("assistant.usageUnavailable") : `$${value}`;
  const metrics = data
    ? [
        [t("assistant.inputTokens"), number(data.inputTokens)],
        [t("assistant.outputTokens"), number(data.outputTokens)],
        [t("assistant.modelEstimate"), money(data.estimatedCostUsd)],
        [t("assistant.searchCalls"), number(data.searchCount)],
        [t("assistant.searchEstimate"), money(data.searchEstimatedCostUsd)],
      ]
    : [];
  return (
    <section
      aria-label={t("assistant.usageTitle")}
      className="rounded-card border border-line bg-card p-4"
    >
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-ui font-semibold">{t("assistant.usageTitle")}</h2>
          <p className="mt-1 text-note text-ink-muted">
            {t("assistant.usageScope")}
          </p>
        </div>
        <Button
          variant="icon"
          aria-label={t("assistant.refreshUsage")}
          disabled={busy || !active}
          onClick={() => void refresh()}
        >
          <RefreshCw
            className={busy ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"}
          />
        </Button>
      </div>
      {error ? (
        <p role="status" className="mt-3 text-ctl text-ink-muted">
          {t("assistant.usageFailed")}
        </p>
      ) : data ? (
        <>
          {data.usageCoverage === "incomplete" && (
            <p role="status" className="mt-3 text-note text-ink-muted">
              {t("assistant.usageIncomplete")}
            </p>
          )}
          <dl className="mt-4 grid grid-cols-2 gap-4 min-[900px]:grid-cols-5">
            {metrics.map(([label, value]) => (
              <div key={label}>
                <dt className="text-note text-ink-muted">{label}</dt>
                <dd className="mt-1 text-lg font-semibold tabular-nums">
                  {value}
                </dd>
              </div>
            ))}
          </dl>
          <p className="mt-3 text-note text-ink-muted">
            {t("assistant.usageConversations", {
              sessions: data.sessionCount,
              runs: data.runCount,
            })}{" "}
            ·{" "}
            {data.billedCostUsd == null
              ? t("assistant.actualChargeUnavailable")
              : t("assistant.billedCost", { cost: data.billedCostUsd })}
          </p>
        </>
      ) : (
        <p role="status" className="mt-3 text-ctl text-ink-muted">
          {t("assistant.loading")}
        </p>
      )}
    </section>
  );
}
