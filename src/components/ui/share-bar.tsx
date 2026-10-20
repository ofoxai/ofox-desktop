import { cn } from "@/lib/utils";

/** magpie's `.share`: a 4px track filled to `value`, a share from 0 to 1. */
export function ShareBar({
  value,
  className,
}: {
  value: number;
  className?: string;
}) {
  const share = Number.isFinite(value) ? Math.min(Math.max(value, 0), 1) : 0;
  return (
    <div
      aria-hidden
      className={cn("h-1 overflow-hidden rounded-[2px] bg-pill", className)}
    >
      <i
        className="block h-full rounded-[2px] bg-brand opacity-70"
        style={{ width: `${Math.round(share * 1000) / 10}%` }}
      />
    </div>
  );
}
