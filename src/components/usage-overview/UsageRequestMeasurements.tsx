import { useTranslation } from "react-i18next";
import type { UsageRequestMeasurements as Measurements } from "@/types/usage";

export function UsageRequestMeasurements({
  measurements,
  total,
}: {
  measurements: Measurements;
  total: number;
}) {
  const { t, i18n } = useTranslation();
  const number = (value: number) =>
    value.toLocaleString(i18n.language, { maximumFractionDigits: 2 });
  const measured = (value: number | null, unit: string) =>
    value == null
      ? t("usageWorkbench.notRecorded")
      : `${number(value)} ${unit}`;
  const items = [
    {
      label: t("usageWorkbench.failureRate"),
      value:
        measurements.failureRate == null
          ? t("usageWorkbench.notRecorded")
          : measurements.failureRate.toLocaleString(i18n.language, {
              style: "percent",
              maximumFractionDigits: 2,
            }),
      samples: measurements.statusSamples,
    },
    {
      label: t("usageWorkbench.avgLatency"),
      value: measured(measurements.avgLatencyMs, "ms"),
      samples: measurements.latencySamples,
    },
    {
      label: t("usageWorkbench.avgFirstToken"),
      value: measured(measurements.avgFirstTokenMs, "ms"),
      samples: measurements.firstTokenSamples,
    },
    {
      label: t("usageWorkbench.generationSpeed"),
      value: measured(measurements.avgGenerationTokensPerSecond, "Tokens/s"),
      samples: measurements.generationSpeedSamples,
    },
  ];
  return (
    <section
      aria-label={t("usageWorkbench.measurements")}
      className="rounded-list border border-line bg-card p-3.5"
    >
      <dl className="grid grid-cols-2 gap-4 md:grid-cols-4">
        {items.map((item) => (
          <div key={item.label} className="min-w-0">
            <dt className="text-note text-ink-muted">{item.label}</dt>
            <dd className="mt-1 break-words text-sub font-medium tabular-nums text-ink">
              {item.value}
            </dd>
            <dd className="mt-1 text-note tabular-nums text-ink-faint">
              {t("usageWorkbench.measuredSamples", {
                count: item.samples,
                total: total.toLocaleString(i18n.language),
              })}
            </dd>
          </div>
        ))}
      </dl>
      <p className="mt-3 text-note text-ink-muted">
        {t("usageWorkbench.measurementBasis")}
      </p>
    </section>
  );
}
