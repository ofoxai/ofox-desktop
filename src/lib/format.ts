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
export function fmtBytes(value: number): string {
  for (const [scale, unit] of [
    [1e9, "GB"],
    [1e6, "MB"],
    [1e3, "KB"],
  ] as const) {
    if (value >= scale) return `${Number((value / scale).toFixed(1))} ${unit}`;
  }
  return `${value} B`;
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
