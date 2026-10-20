import * as React from "react";
import { cn } from "@/lib/utils";

/** magpie's `.skeleton`: a pill-coloured block with a light sweeping across. */
export function Skeleton({
  className,
  ...props
}: React.HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      aria-hidden
      className={cn(
        "block animate-skeleton rounded-[5px] bg-[linear-gradient(90deg,rgb(var(--pill))_25%,rgb(var(--pill-hover))_45%,rgb(var(--pill))_65%)] bg-[length:240%_100%]",
        className,
      )}
      {...props}
    />
  );
}
