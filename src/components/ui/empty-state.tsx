import * as React from "react";
import { cn } from "@/lib/utils";

/** magpie's `.empty-state`: a centred title in ink, why in muted, and a way on. */
export function EmptyState({
  title,
  children,
  action,
  icon,
  className,
}: {
  title: React.ReactNode;
  children?: React.ReactNode;
  action?: React.ReactNode;
  icon?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "px-5 py-9 text-center text-ui leading-[1.5] text-ink-muted",
        className,
      )}
    >
      {icon && (
        <div aria-hidden className="mb-2 flex justify-center text-ink-faint">
          {icon}
        </div>
      )}
      <b className="mb-1 block font-medium text-ink">{title}</b>
      {children}
      {action && <div className="mt-3 flex justify-center">{action}</div>}
    </div>
  );
}
