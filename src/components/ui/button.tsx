import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

// magpie's controls: one brand action, quiet bordered secondaries, text
// buttons that gain a pill on hover. Every press gives a little.
const primary = "bg-brand font-medium text-brand-on hover:brightness-[1.08]";
const secondary =
  "border border-line bg-card text-ink-2 shadow-[0_1px_1px_rgba(0,0,0,.04)] hover:border-ink-muted hover:bg-card-2 hover:text-ink";
const danger =
  "bg-danger font-medium text-destructive-foreground hover:brightness-[1.08]";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-ctl text-ctl transition-[background-color,color,border-color,box-shadow,filter,opacity,transform] duration-150 ease-soft active:scale-[.96] disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        primary,
        secondary,
        ghost: "text-ink-ctl hover:bg-pill hover:text-ink",
        danger,
        dangerGhost: "text-ink-ctl hover:bg-danger-soft hover:text-danger",
        pill: "bg-pill font-medium text-ink-2 hover:bg-pill-hover hover:text-ink",
        icon: "text-ink-muted hover:bg-pill hover:text-ink",
        copy: "text-ink-muted hover:bg-pill-hover hover:text-ink",
        link: "text-brand-ink underline-offset-4 hover:underline",
        // Older names, kept so existing screens compile while they are
        // restyled one by one.
        default: primary,
        outline: secondary,
        destructive: danger,
        mcp: primary,
      },
      size: {
        default: "h-[27px] px-2.5",
        sm: "h-6 px-2",
        lg: "h-8 px-3.5 text-ui",
        icon: "h-[26px] w-[26px] p-0",
      },
    },
    // These shapes are part of the variant: they win over any size.
    compoundVariants: [
      { variant: "pill", class: "h-6 rounded-full px-2.5 text-sub" },
      { variant: "icon", class: "h-[26px] w-[26px] rounded-ctl p-0" },
      { variant: "copy", class: "h-[22px] w-[22px] rounded-[5px] p-0" },
    ],
    defaultVariants: {
      variant: "primary",
      size: "default",
    },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        className={cn(buttonVariants({ variant, size, className }))}
        ref={ref}
        {...props}
      />
    );
  },
);
Button.displayName = "Button";

export { Button, buttonVariants };
