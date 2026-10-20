import { cn } from "@/lib/utils";

export interface StackedBarPoint {
  /** The bucket's start (an hour, a day); passed to the formatters. */
  start: string | number;
  /** The lower, lighter part (e.g. input tokens). */
  a: number;
  /** The upper part in full brand (e.g. output tokens). */
  b: number;
}

export interface StackedBarsProps {
  series: ReadonlyArray<StackedBarPoint>;
  /** Text under the first, middle and last bars. */
  formatLabel?: (start: StackedBarPoint["start"]) => string;
  /** Each bar's tooltip. */
  formatTitle?: (point: StackedBarPoint) => string;
  /** Names the chart for screen readers; without it the bars are hidden. */
  "aria-label"?: string;
  className?: string;
}

const pct = (value: number, peak: number) =>
  `${Math.round((1000 * Math.max(value, 0)) / peak) / 10}%`;

/** The bars and their few labels: first, middle and last. */
function labelled(count: number): Set<number> {
  if (count === 0) return new Set();
  return new Set([0, Math.floor((count - 1) / 2), count - 1]);
}

/**
 * magpie's usage chart: one 72px column per bucket, `b` in brand stacked on
 * `a` at .38, all scaled to the tallest. Plain divs, a `title` per bar.
 */
export function StackedBars({
  series,
  formatLabel = String,
  formatTitle = (point) => `${formatLabel(point.start)} · ${point.a + point.b}`,
  "aria-label": ariaLabel,
  className,
}: StackedBarsProps) {
  const peak = Math.max(1, ...series.map((point) => point.a + point.b));
  const shown = labelled(series.length);
  return (
    <div
      className={cn("min-w-0", className)}
      role={ariaLabel ? "img" : undefined}
      aria-label={ariaLabel}
      aria-hidden={ariaLabel ? undefined : true}
    >
      <div className="flex h-[72px] items-end gap-[3px] border-b border-line-2">
        {series.map((point, index) => (
          <div
            key={index}
            title={formatTitle(point)}
            className="flex h-full min-w-[2px] flex-1 basis-0 flex-col justify-end overflow-hidden rounded-t-[3px] hover:bg-pill"
          >
            <i
              className="block flex-none bg-brand"
              style={{ height: pct(point.b, peak) }}
            />
            <i
              className="block flex-none bg-brand opacity-[.38]"
              style={{ height: pct(point.a, peak) }}
            />
          </div>
        ))}
      </div>
      <div className="mt-[5px] flex justify-between gap-2 text-micro tabular-nums text-ink-muted">
        {series.map((point, index) =>
          shown.has(index) ? (
            <span key={index} className="whitespace-nowrap">
              {formatLabel(point.start)}
            </span>
          ) : null,
        )}
      </div>
    </div>
  );
}
