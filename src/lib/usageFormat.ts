/**
 * How the usage page and the tray put usage figures into words: token
 * counts, USD amounts, and when the OFox balance runs out.
 */

/** When the balance runs out: in whole days, or in hours within a day. */
export type RunsOutIn = { days: number } | { hours: number };

/**
 * `daysLeft` (from the balance trend) as whole days, or as hours when it is
 * less than a day. `null` for no estimate.
 */
export function runsOutIn(
  daysLeft: number | null | undefined,
): RunsOutIn | null {
  if (typeof daysLeft !== "number" || !Number.isFinite(daysLeft)) return null;
  if (daysLeft <= 0) return null;
  if (daysLeft >= 1) return { days: Math.round(daysLeft) };
  return { hours: Math.max(1, Math.round(Math.max(daysLeft, 0) * 24)) };
}
