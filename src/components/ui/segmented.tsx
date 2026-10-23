import * as React from "react";
import { motion, useReducedMotion } from "framer-motion";
import { cn } from "@/lib/utils";

export interface SegmentedItem<T extends string = string> {
  value: T;
  label: React.ReactNode;
}

export interface SegmentedNavItem<T extends string = string>
  extends SegmentedItem<T> {
  /** id of the panel this tab shows, for `aria-controls`. */
  controls?: string;
}

// The thumb glides between segments on magpie's soft curve.
const GLIDE = { duration: 0.3, ease: [0.2, 0.8, 0.2, 1] } as const;

/** Where a key moves within `count` segments from `index`, wrapping. */
function stepFor(
  key: string,
  index: number,
  count: number,
  vertical: boolean,
): number | undefined {
  const back = key === "ArrowLeft" || (vertical && key === "ArrowUp");
  const ahead = key === "ArrowRight" || (vertical && key === "ArrowDown");
  if (back) return (index - 1 + count) % count;
  if (ahead) return (index + 1) % count;
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  return undefined;
}

/**
 * Roving focus: the checked segment (or the first, when none is) is the one
 * stop in the tab order; the arrows, Home and End move focus and pick the
 * segment reached, as magpie's tabs do.
 */
function useRoving<T extends string>(
  values: T[],
  value: T,
  onChange: (value: T) => void,
  vertical: boolean,
) {
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const current = values.indexOf(value);
  const tabStop = current >= 0 ? current : 0;

  const onKeyDown = (event: React.KeyboardEvent, index: number) => {
    const next = stepFor(event.key, index, values.length, vertical);
    if (next === undefined) return;
    event.preventDefault();
    refs.current[next]?.focus();
    if (values[next] !== value) onChange(values[next]);
  };

  const bind = (index: number) => ({
    ref: (node: HTMLButtonElement | null) => {
      refs.current[index] = node;
    },
    tabIndex: index === tabStop ? 0 : -1,
    onKeyDown: (event: React.KeyboardEvent) => onKeyDown(event, index),
  });

  return bind;
}

/**
 * The thumb under the checked segment. It carries a shared `layoutId`, so it
 * slides from the old segment to the new one; under reduced motion it simply
 * appears in place.
 */
function Thumb({ group, className }: { group: string; className: string }) {
  const reduce = useReducedMotion();
  return (
    <motion.span
      aria-hidden
      data-seg-thumb=""
      layoutId={reduce ? undefined : `${group}-thumb`}
      transition={GLIDE}
      className={cn("pointer-events-none absolute inset-0", className)}
    />
  );
}

export interface SegmentedNavProps<T extends string> {
  items: ReadonlyArray<SegmentedNavItem<T>>;
  value: T;
  onChange: (value: T) => void;
  "aria-label"?: string;
  className?: string;
}

/**
 * magpie's header `.seg`: page tabs with no track, only a white thumb
 * under the open one.
 */
export function SegmentedNav<T extends string>({
  items,
  value,
  onChange,
  "aria-label": ariaLabel,
  className,
}: SegmentedNavProps<T>) {
  const group = React.useId();
  const bind = useRoving(
    items.map((item) => item.value),
    value,
    onChange,
    false,
  );
  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      aria-orientation="horizontal"
      className={cn("inline-flex rounded-[8px] p-[2px]", className)}
    >
      {items.map((item, index) => {
        const on = item.value === value;
        return (
          <button
            key={item.value}
            type="button"
            role="tab"
            aria-selected={on}
            aria-controls={item.controls}
            onClick={() => {
              if (!on) onChange(item.value);
            }}
            {...bind(index)}
            className={cn(
              "relative h-6 whitespace-nowrap rounded-[6px] px-3 text-nav text-ink-ctl transition-[color,transform] duration-200 ease-soft active:scale-[.96]",
              on ? "text-ink" : "hover:text-ink-2",
            )}
          >
            {on && (
              <Thumb
                group={group}
                className="rounded-[6px] bg-seg-thumb shadow-[0_1px_2px_rgba(0,0,0,.14),0_0_0_.5px_rgba(0,0,0,.08)]"
              />
            )}
            <span className="relative z-[1]">{item.label}</span>
          </button>
        );
      })}
    </div>
  );
}

export interface SegmentedControlProps<T extends string> {
  options: ReadonlyArray<SegmentedItem<T>>;
  value: T;
  onChange: (value: T) => void;
  /** Optional editor action when the currently selected option is clicked. */
  onReselect?: (value: T) => void;
  "aria-label": string;
  disabled?: boolean;
  className?: string;
}

/** magpie's `.segs`: a choice of a few values on a pill track. */
export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  onReselect,
  "aria-label": ariaLabel,
  disabled = false,
  className,
}: SegmentedControlProps<T>) {
  const group = React.useId();
  const bind = useRoving(
    options.map((option) => option.value),
    value,
    onChange,
    true,
  );
  return (
    <div
      role="radiogroup"
      aria-label={ariaLabel}
      aria-disabled={disabled || undefined}
      className={cn(
        "relative inline-flex self-start rounded-ctl bg-pill p-[2px]",
        disabled && "opacity-55",
        className,
      )}
    >
      {options.map((option, index) => {
        const on = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={on}
            disabled={disabled}
            onClick={() => {
              if (on) onReselect?.(option.value);
              else onChange(option.value);
            }}
            {...bind(index)}
            className={cn(
              "relative h-[22px] whitespace-nowrap rounded-[5px] px-2.5 text-ctl text-ink-muted transition-[color,transform] duration-200 ease-soft active:scale-[.96] disabled:pointer-events-none",
              on ? "text-ink" : "hover:text-ink-2",
            )}
          >
            {on && (
              <Thumb
                group={group}
                className="rounded-[5px] bg-card shadow-[0_1px_2px_rgba(0,0,0,.12),0_0_0_.5px_rgba(0,0,0,.06)]"
              />
            )}
            <span className="relative z-[1]">{option.label}</span>
          </button>
        );
      })}
    </div>
  );
}
