import * as React from "react";
import { Command as CommandPrimitive } from "cmdk";
import { Check, ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";

export interface MenuSelectOption<T extends string = string> {
  value: T;
  label: string;
  /** A second, muted line under the label. */
  note?: string;
  /** A faint figure at the right, such as how many items the option holds. */
  count?: React.ReactNode;
  /** Drawn at 13px before the label, in the menu and on the trigger. */
  icon?: React.ReactNode;
  disabled?: boolean;
}

export interface MenuSelectProps<T extends string> {
  value: T;
  /** Called only when the user picks an option other than `value`. */
  onChange: (value: T) => void;
  options: ReadonlyArray<MenuSelectOption<T>>;
  /** Shown on the trigger while no option holds `value`. */
  placeholder?: string;
  /** Adds a filter field that matches label, note and value as you type. */
  searchable?: boolean;
  searchPlaceholder?: string;
  /** A small muted line at the top of the menu. */
  head?: React.ReactNode;
  /** Menu width in px. */
  width?: number;
  align?: "start" | "end";
  triggerClassName?: string;
  "aria-label": string;
  id?: string;
  disabled?: boolean;
  /** Replaces the chosen option's label on the trigger. */
  valueLabel?: string;
}

/**
 * magpie's `.sess-pick`: a 26px bordered control with a chevron that turns
 * over while its menu is open. Shared with the other pickers that open a
 * menu (ModelPicker) so every dropdown trigger looks the same.
 */
export const menuTriggerClassName =
  "group inline-flex h-[26px] max-w-[180px] flex-none cursor-default items-center gap-1 rounded-ctl border border-line bg-card pl-[9px] pr-[7px] text-ctl text-ink-ctl transition-colors duration-150 ease-soft hover:bg-pill data-[state=open]:bg-pill disabled:pointer-events-none disabled:opacity-50";

export function MenuChevron({ className }: { className?: string }) {
  return (
    <ChevronDown
      aria-hidden
      strokeWidth={2}
      className={cn(
        "h-3 w-3 flex-none opacity-70 transition-transform duration-150 ease-soft group-data-[state=open]:rotate-180 motion-reduce:transition-none",
        className,
      )}
    />
  );
}

const ICON_BOX =
  "grid h-4 w-[13px] flex-none place-items-center [&>img]:h-[13px] [&>img]:w-[13px] [&>svg]:h-[13px] [&>svg]:w-[13px]";

// cmdk keys items by a string it trims and treats "" as none, so items carry
// their position in `options` rather than the caller's value.
const itemKey = (index: number) => `option-${index}`;

function matches(option: MenuSelectOption<string>, query: string): boolean {
  return `${option.label} ${option.note ?? ""} ${option.value}`
    .toLocaleLowerCase()
    .includes(query);
}

/**
 * The app's own dropdown (magpie's proto menu): a trigger that reads as a
 * combobox and a frosted menu of ticked options. ↑/↓ wrap, Home/End jump,
 * Enter picks, Escape closes and hands focus back to the trigger. Inside a
 * dialog the menu portals into that dialog (see `DialogContainerContext`).
 */
export function MenuSelect<T extends string>({
  value,
  onChange,
  options,
  placeholder,
  searchable = false,
  searchPlaceholder,
  head,
  width = 262,
  align = "start",
  triggerClassName,
  "aria-label": ariaLabel,
  id,
  disabled = false,
  valueLabel,
}: MenuSelectProps<T>) {
  const { t } = useTranslation();
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [active, setActive] = React.useState("");
  const listRef = React.useRef<HTMLDivElement>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);

  const chosen = options.find((option) => option.value === value);
  const shown = React.useMemo(() => {
    const q = query.trim().toLocaleLowerCase();
    return options
      .map((option, index) => ({ option, index }))
      .filter(({ option }) => !q || matches(option, q));
  }, [options, query]);

  const handleOpenChange = (next: boolean) => {
    if (next) {
      // Open on the chosen option, as magpie does, else the first one free.
      setQuery("");
      const at = options.findIndex(
        (option) => option.value === value && !option.disabled,
      );
      const first = options.findIndex((option) => !option.disabled);
      const start = at >= 0 ? at : first;
      setActive(start >= 0 ? itemKey(start) : "");
    }
    setOpen(next);
  };

  const choose = (option: MenuSelectOption<T>) => {
    setOpen(false);
    if (option.value !== value) onChange(option.value);
  };

  return (
    <Popover open={open} onOpenChange={handleOpenChange} modal={false}>
      <PopoverTrigger asChild>
        <button
          type="button"
          id={id}
          role="combobox"
          aria-label={ariaLabel}
          aria-expanded={open}
          aria-haspopup="listbox"
          disabled={disabled}
          onKeyDown={(event) => {
            if (
              !open &&
              (event.key === "ArrowDown" || event.key === "ArrowUp")
            ) {
              event.preventDefault();
              handleOpenChange(true);
            }
          }}
          className={cn(
            menuTriggerClassName,
            chosen && "text-ink",
            triggerClassName,
          )}
        >
          {chosen?.icon && (
            <span aria-hidden className={ICON_BOX}>
              {chosen.icon}
            </span>
          )}
          <span className="min-w-0 flex-1 truncate text-left">
            {valueLabel ?? chosen?.label ?? placeholder}
          </span>
          {chosen?.count != null && (
            <span className="flex-none text-micro tabular-nums text-ink-faint">
              {chosen.count}
            </span>
          )}
          <MenuChevron />
        </button>
      </PopoverTrigger>
      <PopoverContent
        // A listbox popup, not a dialog: the trigger says so with
        // aria-haspopup, and the list inside carries the options.
        role="presentation"
        side="bottom"
        align={align}
        sideOffset={5}
        collisionPadding={8}
        style={{ width }}
        className="flex max-h-[min(360px,70vh)] max-w-[calc(100vw-16px)] flex-col overflow-hidden border border-line"
        onOpenAutoFocus={(event) => {
          // cmdk listens for keys on its root, so focus has to start inside
          // it: on the filter, or on the list (which tracks the highlight
          // through aria-activedescendant).
          event.preventDefault();
          (searchable ? inputRef.current : listRef.current)?.focus({
            preventScroll: true,
          });
        }}
      >
        <CommandPrimitive
          shouldFilter={false}
          loop
          value={active}
          onValueChange={setActive}
          label={ariaLabel}
          className="flex min-h-0 flex-1 flex-col outline-none"
        >
          {head && (
            <div className="px-2 pb-[5px] pt-1.5 text-note text-ink-muted">
              {head}
            </div>
          )}
          {searchable && (
            <div className="pb-1.5 pt-1">
              <CommandPrimitive.Input
                ref={inputRef}
                value={query}
                onValueChange={setQuery}
                placeholder={searchPlaceholder}
                aria-label={searchPlaceholder ?? ariaLabel}
                className="block h-7 w-full rounded-ctl border border-line bg-pill px-[9px] text-nav text-ink outline-none transition-colors duration-150 ease-soft placeholder:text-ink-faint focus:border-brand focus-visible:outline-none"
              />
            </div>
          )}
          <CommandPrimitive.List
            ref={listRef}
            className="min-h-0 flex-1 overflow-y-auto overscroll-contain outline-none focus-visible:outline-none [&_[cmdk-list-sizer]]:grid [&_[cmdk-list-sizer]]:gap-px"
          >
            <CommandPrimitive.Empty className="px-2 pb-1.5 pt-2 text-note text-ink-muted">
              {t("common.noMatches")}
            </CommandPrimitive.Empty>
            {shown.map(({ option, index }) => {
              const checked = option.value === value;
              return (
                <CommandPrimitive.Item
                  key={option.value}
                  value={itemKey(index)}
                  disabled={option.disabled}
                  onSelect={() => choose(option)}
                  data-checked={checked || undefined}
                  className="flex w-full cursor-default select-none items-start gap-[7px] rounded-ctl px-2 pb-[7px] pt-1.5 text-left text-ink outline-none transition-colors duration-100 data-[disabled=true]:opacity-45 data-[selected=true]:bg-pill"
                >
                  <span
                    aria-hidden
                    className="grid h-4 w-3.5 flex-none place-items-center text-ink"
                  >
                    {checked && <Check className="h-3 w-3" strokeWidth={1.9} />}
                  </span>
                  {option.icon && (
                    <span aria-hidden className={ICON_BOX}>
                      {option.icon}
                    </span>
                  )}
                  <span className="grid min-w-0 flex-1 gap-px">
                    <span className="truncate text-nav font-medium leading-4">
                      {option.label}
                    </span>
                    {option.note && (
                      <span className="truncate text-note leading-[15px] text-ink-muted">
                        {option.note}
                      </span>
                    )}
                  </span>
                  {option.count != null && (
                    <span className="flex-none pl-2 text-note leading-4 tabular-nums text-ink-faint">
                      {option.count}
                    </span>
                  )}
                </CommandPrimitive.Item>
              );
            })}
          </CommandPrimitive.List>
        </CommandPrimitive>
      </PopoverContent>
    </Popover>
  );
}
