import * as React from "react";
import { Command as CommandPrimitive } from "cmdk";
import { Check, ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useMenuTab } from "@/hooks/useMenuTab";
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
  /**
   * Compact detail at the right of the label, such as a model's context and
   * prices. The option then keeps to one line: the label is cut short first.
   */
  meta?: string;
  /** The option's tooltip, for the detail its row leaves out. */
  title?: string;
  /**
   * Options that share a `group` sit under one heading, `groupLabel`, in the
   * order the groups first appear; options with none stay where they are.
   */
  group?: string;
  groupLabel?: string;
  disabled?: boolean;
}

export interface MenuSelectProps<T extends string> {
  value: T;
  /** Preferred highlight when the current value belongs to a different group. */
  highlightValue?: T;
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
  /** The trigger's tooltip, such as the full name `valueLabel` shortens. */
  triggerTitle?: string;
  /** Classes for the menu, such as a larger `max-h-*`. */
  menuClassName?: string;
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
        `${option.label} ${option.note ?? ""} ${option.value} ${option.groupLabel ?? ""}`
          .toLocaleLowerCase()
          .includes(q),
    );
}

/** The key of the first option that can be picked, or "" for none. */
function firstKey<T extends string>(items: Indexed<T>[]): string {
  const first = items.find(({ option }) => !option.disabled);
  return first ? itemKey(first.index) : "";
}

interface Section<T extends string> {
  /** Undefined for options outside any group. */
  group?: string;
  label?: string;
  items: Indexed<T>[];
}

/**
 * The options as the menu draws them: each group once, at the place of its
 * first option, and every ungrouped option on its own.
 */
function toSections<T extends string>(items: Indexed<T>[]): Section<T>[] {
  const sections: Section<T>[] = [];
  const byGroup = new Map<string, Section<T>>();
  for (const item of items) {
    const { group, groupLabel } = item.option;
    if (group === undefined) {
      sections.push({ items: [item] });
      continue;
    }
    let section = byGroup.get(group);
    if (!section) {
      section = { group, label: groupLabel ?? group, items: [] };
      byGroup.set(group, section);
      sections.push(section);
    }
    section.items.push(item);
  }
  return sections;
}

/**
 * Scroll `list` so `item` is in its middle (`center`) or just inside it
 * (`nearest`). From offsets, so the menu's opening animation (a scale) does
 * not skew it; the list is the items' offset parent.
 */
function scrollIntoList(
  list: HTMLElement,
  item: HTMLElement,
  block: "center" | "nearest",
) {
  const top = item.offsetTop;
  const bottom = top + item.offsetHeight;
  if (block === "center") {
    list.scrollTop = top - (list.clientHeight - item.offsetHeight) / 2;
  } else if (top < list.scrollTop) {
    list.scrollTop = top;
  } else if (bottom > list.scrollTop + list.clientHeight) {
    list.scrollTop = bottom - list.clientHeight;
  }
}

/** What a query leaves, in the order the menu draws it (and arrows through it). */
function inMenuOrder<T extends string>(
  options: ReadonlyArray<MenuSelectOption<T>>,
  query: string,
): Indexed<T>[] {
  return toSections(filterOptions(options, query)).flatMap(
    (section) => section.items,
  );
}

/**
 * The app's own dropdown (magpie's proto menu): a trigger that reads as a
 * combobox and a frosted menu of ticked options. ↑/↓ wrap, Home/End jump,
 * Enter picks, Escape closes and hands focus back to the trigger. Inside a
 * dialog the menu portals into that dialog (see `DialogContainerContext`).
 */
export function MenuSelect<T extends string>({
  value,
  highlightValue,
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
  triggerTitle,
  menuClassName,
}: MenuSelectProps<T>) {
  const { t } = useTranslation();
  const [open, setOpen] = React.useState(false);
  const menuTab = useMenuTab(() => setOpen(false));
  const [query, setQuery] = React.useState("");
  // The highlighted option's key. Kept here rather than in cmdk: cmdk only
  // records which element is highlighted (for aria-activedescendant) when a
  // key or the pointer moves it, never when it picks the first option itself.
  const [active, setActive] = React.useState("");
  const [activeId, setActiveId] = React.useState<string>();
  const [listId, setListId] = React.useState<string>();
  const listRef = React.useRef<HTMLDivElement>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const headRef = React.useRef<HTMLDivElement>(null);
  const itemRefs = React.useRef(new Map<string, HTMLDivElement>());
  // Where the list goes the next time the highlight is followed: the chosen
  // option to the middle when the menu opens; back to the top, where the
  // first match is, after the query changes. Otherwise cmdk keeps the
  // highlighted option in view as the arrows move it.
  const pendingScroll = React.useRef<"center" | "top" | null>(null);

  const chosen = options.find((option) => option.value === value);
  const sections = React.useMemo(
    () => toSections(filterOptions(options, query)),
    [options, query],
  );

  const handleOpenChange = (next: boolean) => {
    if (next) {
      pendingScroll.current = "center";
      setQuery("");
      // Both searchable and plain menus open on the current selection.
      const at = options.findIndex(
        (option) => option.value === value && !option.disabled,
      );
      setActive(at >= 0 ? itemKey(at) : firstKey(inMenuOrder(options, "")));
    }
    setOpen(next);
  };

  const handleQuery = (next: string) => {
    pendingScroll.current = "top";
    setQuery(next);
    setActive(firstKey(inMenuOrder(options, next)));
  };

  // Without a filter, focus rides on the highlighted option itself, as in
  // magpie's menus, so a screen reader names each option as it is reached
  // (the list as a fallback, when no option can be picked). With one, focus
  // stays in the field, which points at the option instead.
  const followHighlight = React.useCallback(() => {
    const item = itemRefs.current.get(active);
    const list = listRef.current;
    setListId(list?.id);
    // The list mounts a render after the menu opens: until then the scroll
    // waits.
    if (list && pendingScroll.current) {
      const block = pendingScroll.current === "center" ? "center" : "nearest";
      if (pendingScroll.current === "top") list.scrollTop = 0;
      pendingScroll.current = null;
      if (item) scrollIntoList(list, item, block);
    }
    if (searchable) {
      setActiveId(item?.id);
    } else {
      (item ?? list)?.focus({ preventScroll: true });
    }
  }, [active, searchable]);

  // The popup's content mounts a render after `open` turns true, so its
  // first focus comes from onOpenAutoFocus below; this follows the moves.
  React.useLayoutEffect(() => {
    if (open) followHighlight();
  }, [open, followHighlight, query]);

  React.useLayoutEffect(() => {
    if (
      open &&
      !inMenuOrder(options, query).some(
        ({ index }) => itemKey(index) === active,
      )
    ) {
      setActive(firstKey(inMenuOrder(options, query)));
    }
    if (open) followHighlight();
  }, [open, options, query, active, followHighlight]);

  React.useLayoutEffect(() => {
    if (!open) return;
    setQuery("");
    const preferred = options.findIndex(
      (option) =>
        !option.disabled &&
        option.value ===
          (options.some((o) => o.value === value) ? value : highlightValue),
    );
    setActive(
      preferred >= 0 ? itemKey(preferred) : firstKey(inMenuOrder(options, "")),
    );
    // A different kind or refreshed catalog starts its own search/highlight.
  }, [options, highlightValue]);

  const choose = (option: MenuSelectOption<T>) => {
    setOpen(false);
    if (option.value !== value) onChange(option.value);
  };

  const renderItem = ({ option, index }: Indexed<T>) => {
    const checked = option.value === value;
    const title = (
      <span className="truncate text-nav font-medium leading-4">
        {option.label}
      </span>
    );
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
        title={option.title}
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
          {option.meta ? (
            <span className="flex min-w-0 items-baseline gap-2">
              {title}
              <span className="ml-auto flex-none text-note leading-4 tabular-nums text-ink-ctl">
                {option.meta}
              </span>
            </span>
          ) : (
            title
          )}
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
  };

  return (
    <Popover open={open} onOpenChange={handleOpenChange} modal={false}>
      <PopoverTrigger asChild>
        <button
          type="button"
          ref={menuTab.triggerRef}
          aria-controls={open ? listId : undefined}
          id={id}
          role="combobox"
          aria-label={ariaLabel}
          aria-expanded={open}
          aria-haspopup="listbox"
          title={triggerTitle}
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
        onKeyDownCapture={(event) => {
          const headStop = headRef.current?.querySelector<HTMLElement>(
            'button[tabindex="0"]:not(:disabled), input:not(:disabled)',
          );
          if (event.key === "Tab" && searchable && headStop) {
            if (event.shiftKey && event.target === inputRef.current) {
              event.preventDefault();
              event.stopPropagation();
              headStop.focus();
              return;
            }
            if (
              !event.shiftKey &&
              headRef.current?.contains(event.target as Node)
            ) {
              event.preventDefault();
              event.stopPropagation();
              inputRef.current?.focus();
              return;
            }
          }
          menuTab.onKeyDown(event);
        }}
        onCloseAutoFocus={menuTab.onCloseAutoFocus}
        // A listbox popup, not a dialog: the trigger says so with
        // aria-haspopup, and the list inside carries the options.
        role="presentation"
        side="bottom"
        align={align}
        sideOffset={5}
        collisionPadding={8}
        style={{ width }}
        className={cn(
          "flex max-h-[min(360px,70vh)] max-w-[calc(100vw-16px)] flex-col overflow-hidden border border-line",
          menuClassName,
        )}
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
            <div
              ref={headRef}
              className="px-2 pb-[5px] pt-1.5 text-note text-ink-muted"
            >
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
            className="relative min-h-0 flex-1 overflow-y-auto overscroll-contain outline-none focus-visible:outline-none [&_[cmdk-list-sizer]]:grid [&_[cmdk-list-sizer]]:gap-px"
          >
            <CommandPrimitive.Empty className="px-2 pb-1.5 pt-2 text-note text-ink-muted">
              {t("common.noMatches")}
            </CommandPrimitive.Empty>
            {sections.map((section) => {
              if (section.group === undefined)
                return renderItem(section.items[0]);
              return (
                <CommandPrimitive.Group
                  key={`group-${section.group}`}
                  heading={section.label}
                  className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:pb-0.5 [&_[cmdk-group-heading]]:pt-2 [&_[cmdk-group-heading]]:text-note [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:text-ink-ctl [&_[cmdk-group-items]]:grid [&_[cmdk-group-items]]:gap-px"
                >
                  {section.items.map(renderItem)}
                </CommandPrimitive.Group>
              );
            })}
          </CommandPrimitive.List>
        </CommandPrimitive>
      </PopoverContent>
    </Popover>
  );
}
