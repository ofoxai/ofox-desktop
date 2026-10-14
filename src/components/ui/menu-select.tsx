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

type Indexed<T extends string> = { option: MenuSelectOption<T>; index: number };

function filterOptions<T extends string>(
  options: ReadonlyArray<MenuSelectOption<T>>,
  query: string,
): Indexed<T>[] {
  const q = query.trim().toLocaleLowerCase();
  return options
    .map((option, index) => ({ option, index }))
    .filter(
      ({ option }) =>
        !q ||
        `${option.label} ${option.note ?? ""} ${option.value}`
          .toLocaleLowerCase()
          .includes(q),
    );
}

/** The key of the first option that can be picked, or "" for none. */
function firstKey<T extends string>(items: Indexed<T>[]): string {
  const first = items.find(({ option }) => !option.disabled);
  return first ? itemKey(first.index) : "";
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
  // The highlighted option's key. Kept here rather than in cmdk: cmdk only
  // records which element is highlighted (for aria-activedescendant) when a
  // key or the pointer moves it, never when it picks the first option itself.
  const [active, setActive] = React.useState("");
  const [activeId, setActiveId] = React.useState<string>();
  const [listId, setListId] = React.useState<string>();
  const listRef = React.useRef<HTMLDivElement>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const itemRefs = React.useRef(new Map<string, HTMLDivElement>());

  const chosen = options.find((option) => option.value === value);
  const shown = React.useMemo(
    () => filterOptions(options, query),
    [options, query],
  );

  const handleOpenChange = (next: boolean) => {
    if (next) {
      setQuery("");
      // A menu opens on the chosen option, as magpie's does; a filter on the
      // first option, which Enter takes.
      const at = options.findIndex(
        (option) => option.value === value && !option.disabled,
      );
      const all = filterOptions(options, "");
      setActive(!searchable && at >= 0 ? itemKey(at) : firstKey(all));
    }
    setOpen(next);
  };

  const handleQuery = (next: string) => {
    setQuery(next);
    setActive(firstKey(filterOptions(options, next)));
  };

  // Without a filter, focus rides on the highlighted option itself, as in
  // magpie's menus, so a screen reader names each option as it is reached
  // (the list as a fallback, when no option can be picked). With one, focus
  // stays in the field, which points at the option instead.
  const followHighlight = React.useCallback(() => {
    const item = itemRefs.current.get(active);
    if (searchable) {
      setActiveId(item?.id);
      setListId(listRef.current?.id);
    } else {
      (item ?? listRef.current)?.focus({ preventScroll: true });
    }
  }, [active, searchable]);

  // The popup's content mounts a render after `open` turns true, so its
  // first focus comes from onOpenAutoFocus below; this follows the moves.
  React.useLayoutEffect(() => {
    if (open) followHighlight();
  }, [open, followHighlight, shown]);

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
          // it: on the filter, else on the highlighted option.
          event.preventDefault();
          if (searchable) inputRef.current?.focus({ preventScroll: true });
          followHighlight();
        }}
      >
        <CommandPrimitive
          shouldFilter={false}
          loop
          value={active}
          onValueChange={setActive}
          className="flex min-h-0 flex-1 flex-col outline-none"
        >
          {head && (
            <div className="px-2 pb-[5px] pt-1.5 text-note text-ink-muted">
              {head}
            </div>
          )}
          {searchable && (
            <div className="pb-1.5 pt-1">
              {/* A plain field rather than cmdk's Input, so it can point at
                  the highlighted option (see `active`). Its keys still reach
                  cmdk's root, which moves the highlight. */}
              <input
                ref={inputRef}
                type="text"
                role="combobox"
                aria-expanded
                aria-autocomplete="list"
                aria-controls={listId}
                aria-activedescendant={activeId}
                aria-label={searchPlaceholder ?? ariaLabel}
                autoComplete="off"
                spellCheck={false}
                value={query}
                onChange={(event) => handleQuery(event.target.value)}
                placeholder={searchPlaceholder}
                className="block h-7 w-full rounded-ctl border border-line bg-pill px-[9px] text-nav text-ink outline-none transition-colors duration-150 ease-soft placeholder:text-ink-faint focus:border-brand focus-visible:outline-none"
              />
            </div>
          )}
          <CommandPrimitive.List
            ref={listRef}
            label={ariaLabel}
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
                  ref={(node) => {
                    if (node) itemRefs.current.set(itemKey(index), node);
                    else itemRefs.current.delete(itemKey(index));
                  }}
                  tabIndex={-1}
                  value={itemKey(index)}
                  disabled={option.disabled}
                  onSelect={() => choose(option)}
                  data-checked={checked || undefined}
                  className="flex w-full cursor-default select-none items-start gap-[7px] rounded-ctl px-2 pb-[7px] pt-1.5 text-left text-ink outline-none transition-colors duration-100 focus-visible:outline-none data-[disabled=true]:opacity-45 data-[selected=true]:bg-pill"
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
