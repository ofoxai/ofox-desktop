import * as React from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";

/** magpie's `.list`: one card holding rows, each split from the next by a hairline. */
export const ListCard = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn(
      "flex-none divide-y divide-line-2 overflow-hidden rounded-list border border-line bg-card",
      className,
    )}
    {...props}
  />
));
ListCard.displayName = "ListCard";

// A row in a wrapper inside the card (a fold, a page of rows) still gets the
// hairline from the row before it, as magpie's `.row + .row` does.
const SIBLING_LINE = "[*+&]:border-t [*+&]:border-line-2";

/** magpie's window `.row`: at least 48px, icon/text/actions 12px apart. */
export const ListRow = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn(
      "flex min-h-12 items-center gap-3 py-[7px] pl-[14px] pr-3",
      SIBLING_LINE,
      className,
    )}
    {...props}
  />
));
ListRow.displayName = "ListRow";

/** A short bar over a list's rows: a select-all, a count, bulk actions. */
export const ListHead = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn(
      "flex min-h-[34px] items-center gap-2 py-1 pl-[14px] pr-3 text-sub text-ink-muted",
      SIBLING_LINE,
      className,
    )}
    {...props}
  />
));
ListHead.displayName = "ListHead";

export interface ListFoldProps
  extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "onChange"> {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** A faint figure after the label, such as how many rows are folded. */
  note?: React.ReactNode;
}

/**
 * magpie's `.fold`: a quiet disclosure button whose chevron points down while
 * what it folds is shown. The caller renders the folded content.
 */
export const ListFold = React.forwardRef<HTMLButtonElement, ListFoldProps>(
  (
    { open, onOpenChange, note, className, children, onClick, ...props },
    ref,
  ) => (
    <button
      ref={ref}
      type="button"
      aria-expanded={open}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented) onOpenChange(!open);
      }}
      className={cn(
        "group -ml-1 inline-flex items-center gap-[5px] rounded-[6px] py-[3px] pl-1 pr-1.5 text-ctl text-ink-faint transition-colors duration-150 hover:bg-pill hover:text-ink-muted",
        className,
      )}
      {...props}
    >
      <ChevronRight
        aria-hidden
        strokeWidth={2}
        className="h-3 w-3 flex-none transition-transform duration-150 ease-out group-aria-expanded:rotate-90 motion-reduce:transition-none"
      />
      {children}
      {note != null && <span className="tabular-nums">{note}</span>}
    </button>
  ),
);
ListFold.displayName = "ListFold";
