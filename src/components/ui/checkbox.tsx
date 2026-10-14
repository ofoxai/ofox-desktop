import * as React from "react";
import * as CheckboxPrimitive from "@radix-ui/react-checkbox";
import { Check, Minus } from "lucide-react";

import { cn } from "@/lib/utils";

/** The app's own checkbox: a 14px box, brand when checked. Pass
 *  `checked="indeterminate"` for a parent of a partial selection; it reads as
 *  `aria-checked="mixed"` and shows a dash. */
const Checkbox = React.forwardRef<
  React.ElementRef<typeof CheckboxPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof CheckboxPrimitive.Root>
>(({ className, ...props }, ref) => (
  <CheckboxPrimitive.Root
    ref={ref}
    className={cn(
      "peer grid h-3.5 w-3.5 shrink-0 place-content-center rounded-[4px] border border-ink-faint bg-card text-brand-on transition-colors duration-150 ease-soft hover:border-ink-muted disabled:cursor-not-allowed disabled:opacity-45 data-[state=checked]:border-brand data-[state=checked]:bg-brand data-[state=indeterminate]:border-brand data-[state=indeterminate]:bg-brand",
      className,
    )}
    {...props}
  >
    <CheckboxPrimitive.Indicator className="grid place-content-center text-current">
      {props.checked === "indeterminate" ? (
        <Minus className="h-2.5 w-2.5" strokeWidth={3} />
      ) : (
        <Check className="h-2.5 w-2.5" strokeWidth={3} />
      )}
    </CheckboxPrimitive.Indicator>
  </CheckboxPrimitive.Root>
));
Checkbox.displayName = CheckboxPrimitive.Root.displayName;

export { Checkbox };
