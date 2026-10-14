import * as React from "react";
import { cn } from "@/lib/utils";

export interface KpiItem {
  label: React.ReactNode;
  value: React.ReactNode;
  sub?: React.ReactNode;
  /** A tooltip for the whole tile, for the detail that does not fit. */
  title?: string;
}

/**
 * magpie's `.stats`: a row of figures in one 12px frame, the tiles joined by
 * 1px gaps. Each tile is the figure, an uppercase label under it, and an
 * optional small line.
 */
export function KpiStrip({
  items,
  className,
}: {
  items: ReadonlyArray<KpiItem>;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "grid flex-none gap-px overflow-hidden rounded-list border border-line bg-line",
        className,
      )}
      style={{
        gridTemplateColumns: `repeat(${Math.max(items.length, 1)}, minmax(0, 1fr))`,
      }}
    >
      {items.map((item, index) => (
        <div
          key={index}
          title={item.title}
          className="flex min-w-0 flex-col bg-card px-[14px] pb-[11px] pt-3"
        >
          <b className="truncate whitespace-nowrap text-kpi font-semibold leading-[1.15] tracking-[-.01em] tabular-nums text-ink">
            {item.value}
          </b>
          <span className="mt-[3px] text-note font-semibold uppercase tracking-[.05em] text-ink-muted">
            {item.label}
          </span>
          {item.sub != null && (
            <small className="mt-[3px] truncate whitespace-nowrap text-note text-ink-muted">
              {item.sub}
            </small>
          )}
        </div>
      ))}
    </div>
  );
}
