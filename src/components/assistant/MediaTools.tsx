import { forwardRef, type ReactNode } from "react";
import { Button, type ButtonProps } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Where a result will be, or why it is not: the size of the result. */
export const MEDIA_PLACEHOLDER =
  "flex flex-col items-center justify-center gap-2 rounded-list border border-line bg-card-2 px-4 text-center text-note text-ink-muted";

/**
 * A result's actions, shown only while the pointer or the keyboard focus is
 * on the result (an element with `group/media`): over a picture's corner, or
 * after a player that has no picture to sit on.
 */
export function MediaTools({
  inline,
  children,
}: {
  /** After the result instead of over its top right corner. */
  inline?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-1 opacity-0 transition-opacity duration-150 group-focus-within/media:opacity-100 group-hover/media:opacity-100 has-[[data-state=open]]:opacity-100",
        !inline && "absolute right-2 top-2",
      )}
    >
      {children}
    </div>
  );
}

/** One of a result's actions: an icon, named by its label. */
export const MediaToolButton = forwardRef<
  HTMLButtonElement,
  ButtonProps & { label: string }
>(({ label, className, ...props }, ref) => (
  <Button
    ref={ref}
    variant="icon"
    aria-label={label}
    title={label}
    className={cn(
      "h-7 w-7 bg-card/90 text-ink shadow-sm backdrop-blur hover:bg-card [&_svg]:h-3.5 [&_svg]:w-3.5",
      className,
    )}
    {...props}
  />
));
MediaToolButton.displayName = "MediaToolButton";
