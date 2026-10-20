import type { TFunction } from "i18next";
import type { EffortState } from "@/lib/api/modelSelection";

/** Weakest first, as magpie ranks them; a level outside a tool's list sorts by this. */
const EFFORT_ORDER = [
  "off",
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
];

/**
 * The slider's stops: the tool's default first, then its levels. A level set
 * outside Ofox that the tool's list lacks gets a stop of its own, so the
 * slider can show it and move off it.
 */
export function effortStops(
  effort: EffortState,
  value: string | null,
): (string | null)[] {
  const stops: (string | null)[] = [null, ...effort.levels];
  if (value === null || effort.levels.includes(value)) return stops;
  const rank = EFFORT_ORDER.indexOf(value);
  const at =
    rank < 0
      ? 1
      : 1 +
        effort.levels.filter((level) => EFFORT_ORDER.indexOf(level) < rank)
          .length;
  stops.splice(at, 0, value);
  return stops;
}

/** A stop as words: 默认（中） for the default where the tool's is known. */
export function effortName(
  value: string | null,
  effort: EffortState,
  t: TFunction,
): string {
  if (value === null)
    return effort.defaultLevel
      ? t("effort.defaultTakes", {
          level: t(`effort.levels.${effort.defaultLevel}`),
        })
      : t("effort.default");
  return t(`effort.levels.${value}`, { defaultValue: value });
}
