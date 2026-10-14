import * as React from "react";
import * as PopoverPrimitive from "@radix-ui/react-popover";
import { useDialogContainer } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";

const Popover = PopoverPrimitive.Root;
const PopoverTrigger = PopoverPrimitive.Trigger;
const PopoverAnchor = PopoverPrimitive.Anchor;

const PopoverContent = React.forwardRef<
  React.ComponentRef<typeof PopoverPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Content> & {
    /**
     * Where Radix should portal the popover into. Defaults to the enclosing
     * dialog's content (see `DialogContainerContext`), else `document.body`.
     *
     * Radix Dialog uses `react-remove-scroll` keyed on the dialog's content
     * node as the only "scroll shard", so a popover portaled to body from a
     * dialog has its wheel / touch-scroll events swallowed. Inside the
     * dialog's content node it can scroll its `CommandList` normally.
     */
    container?: React.ComponentProps<
      typeof PopoverPrimitive.Portal
    >["container"];
  }
>(
  (
    { className, align = "start", sideOffset = 4, container, ...props },
    ref,
  ) => {
    const dialogContainer = useDialogContainer();
    return (
      <PopoverPrimitive.Portal container={container ?? dialogContainer}>
        <PopoverPrimitive.Content
          ref={ref}
          align={align}
          sideOffset={sideOffset}
          collisionBoundary={dialogContainer ?? undefined}
          collisionPadding={8}
          className={cn(
            "z-50 origin-[var(--radix-popover-content-transform-origin)] animate-pop-in rounded-pop bg-pop-bg p-1 text-popover-foreground shadow-pop outline-none backdrop-blur-xl data-[side=top]:[--pop-dy:6px]",
            className,
          )}
          {...props}
        />
      </PopoverPrimitive.Portal>
    );
  },
);
PopoverContent.displayName = PopoverPrimitive.Content.displayName;

export { Popover, PopoverTrigger, PopoverContent, PopoverAnchor };
