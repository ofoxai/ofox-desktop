import * as React from "react";
import { cn } from "@/lib/utils";

export type InputProps = React.InputHTMLAttributes<HTMLInputElement>;

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, ...props }, ref) => {
    return (
      <input
        type={type}
        className={cn(
          "flex h-[26px] w-full rounded-ctl border border-line bg-card px-2.5 text-ctl text-foreground outline-none transition-[border-color,box-shadow] duration-150 ease-soft file:border-0 file:bg-transparent file:text-ctl file:font-medium file:text-foreground placeholder:text-ink-faint focus:border-brand focus:ring-[3px] focus:ring-brand/20 focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
        ref={ref}
        {...props}
      />
    );
  },
);
Input.displayName = "Input";

export { Input };
