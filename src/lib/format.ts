/** Compact decimal units shared by the session and usage pages. */
export function fmtTokens(value: number): string {
  for (const [scale, unit] of [
    [1e9, "B"],
    [1e6, "M"],
    [1e3, "K"],
  ] as const) {
    if (Math.abs(value) >= scale)
      return `${Number((value / scale).toFixed(1))}${unit}`;
  }
  return String(value);
}
/** A session's token count as magpie writes it (`fmtN`): three figures. */
export function fmtTokenCount(value: number): string {
  if (value >= 1e9) return `${(value / 1e9).toFixed(2)}B`;
  if (value >= 1e7) return `${Math.round(value / 1e6)}M`;
  if (value >= 1e6) return `${(value / 1e6).toFixed(1)}M`;
  if (value >= 1e5) return `${Math.round(value / 1e3)}K`;
  if (value >= 1e3) return `${(value / 1e3).toFixed(1)}K`;
  return String(value);
}
function bytesIn(base: number, value: number): string {
  for (const [power, unit] of [
    [3, "GB"],
    [2, "MB"],
    [1, "KB"],
  ] as const) {
    const scale = base ** power;
    if (value < scale) continue;
    const n = value / scale;
    return `${n < 10 ? n.toFixed(1) : Math.round(n)} ${unit}`;
  }
  return `${value} B`;
}
/** Bytes as magpie writes them: base 1000, one decimal only below 10. */
export function fmtBytes(value: number): string {
  return bytesIn(1000, value);
}
/**
 * Bytes in base 1024, as limits given in binary megabytes are written (the
 * assistant's 25 MB per file): a file at the limit reads as the limit.
 */
export function fmtBinaryBytes(value: number): string {
  return bytesIn(1024, value);
}
export function fmtCost(value?: number | null): string {
  if (value == null || !Number.isFinite(value)) return "—";
  if (value > 0 && value < 0.001) return "<$0.001";
  return `≈$${value.toFixed(value < 1 ? 3 : value < 100 ? 2 : 0)}`;
}
export function fmtAgo(
  value: number | undefined,
  language: string,
  now = Date.now(),
): string {
  if (value == null) return "";
  const seconds = (value - now) / 1000;
  const format = new Intl.RelativeTimeFormat(language, { numeric: "auto" });
  for (const [size, unit] of [
    [86400, "day"],
    [3600, "hour"],
    [60, "minute"],
  ] as const) {
    if (Math.abs(seconds) >= size)
      return format.format(Math.round(seconds / size), unit);
  }
  return format.format(Math.round(seconds), "second");
}
