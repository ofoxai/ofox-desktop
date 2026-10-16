import type { TFunction } from "i18next";

export function formatActivity(
  ms: number | null | undefined,
  t: TFunction,
): string {
  if (ms == null || !Number.isFinite(ms)) return "—";
  if (ms > 0 && ms < 1000) return t("usageExperience.lessThanSecond");
  if (ms >= 1000 && ms < 60000)
    return t("usageExperience.seconds", { value: Math.floor(ms / 1000) });
  const minutes = Math.floor(ms / 60000);
  return minutes < 60
    ? t("usageExperience.minutes", { value: Number((ms / 60000).toFixed(1)) })
    : t("usageExperience.hoursMinutes", {
        hours: Math.floor(minutes / 60),
        minutes: minutes % 60,
      });
}

export const heatColors = [
  "bg-pill",
  "bg-brand/15",
  "bg-brand/30",
  "bg-brand/50",
  "bg-brand/75",
  "bg-brand",
];
export function heatColor(value: number | null, max: number): string {
  if (value == null) return "border border-dashed border-line bg-canvas";
  return heatColors[
    value <= 0
      ? 0
      : Math.max(1, Math.min(5, Math.ceil((value / Math.max(1, max)) * 5)))
  ];
}
