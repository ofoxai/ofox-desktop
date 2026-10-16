import { useId, useRef, useState } from "react";
import { Check, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { SearchInput } from "@/components/ui/search-input";
import { MenuChevron, menuTriggerClassName } from "@/components/ui/menu-select";
import { useMenuTab } from "@/hooks/useMenuTab";
import { cn } from "@/lib/utils";

export interface UsageFilterOption {
  value: string;
  label: string;
  title?: string;
}
export function UsageFilterMenu({
  label,
  allLabel,
  options,
  selected,
  onChange,
  disabled = false,
  pageSize,
}: {
  label: string;
  allLabel?: string;
  options: UsageFilterOption[];
  selected: string[];
  onChange: (values: string[]) => void;
  disabled?: boolean;
  pageSize?: number;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false),
    [search, setSearch] = useState(""),
    [pages, setPages] = useState(1);
  const id = useId(),
    refs = useRef<(HTMLButtonElement | null)[]>([]),
    input = useRef<HTMLInputElement>(null);
  // Pin only the initially selected options. Toggling does not move the focused row.
  const selectedOnOpen = useRef(new Set(selected));
  const menu = useMenuTab(() => setOpen(false));
  const selectedOptions = selected.map(
    (value) =>
      options.find((option) => option.value === value) ?? {
        value,
        label: value,
      },
  );
  const summary = selectedOptions.map((option) => option.label).join(", ");
  const clearSelection = () => {
    onChange([]);
    // Clear becomes disabled after a controlled update. Keep the next
    // keyboard action inside the still-open menu on a stable focus target.
    input.current?.focus({ preventScroll: true });
  };
  const needle = search.trim().toLocaleLowerCase();
  const matching = options
    .filter((option) =>
      `${option.value} ${option.label} ${option.title ?? ""}`
        .toLocaleLowerCase()
        .includes(needle),
    )
    .sort(
      (a, b) =>
        Number(selectedOnOpen.current.has(b.value)) -
        Number(selectedOnOpen.current.has(a.value)),
    );
  const shown = matching.slice(0, (pageSize ?? Infinity) * pages);
  const trigger = selectedOptions.length
    ? `${selectedOptions[0].label}${selectedOptions.length > 1 ? ` +${selectedOptions.length - 1}` : ""}`
    : (allLabel ?? `${label} · ${t("usageWorkbench.all")}`);
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) {
          setSearch("");
          setPages(1);
          selectedOnOpen.current = new Set(selected);
        }
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          ref={menu.triggerRef}
          role="combobox"
          aria-label={label}
          aria-controls={open ? id : undefined}
          aria-expanded={open}
          aria-haspopup="listbox"
          disabled={disabled}
          title={selected.length ? summary : (allLabel ?? label)}
          className={cn(
            menuTriggerClassName,
            "min-w-0 max-w-[180px]",
            selected.length > 0 &&
              "border-brand/30 bg-brand-soft/50 text-brand-ink",
          )}
        >
          <span className="min-w-0 flex-1 truncate text-left">{trigger}</span>
          <MenuChevron />
        </button>
      </PopoverTrigger>
      <PopoverContent
        className="flex max-h-[min(420px,var(--radix-popover-content-available-height))] w-[340px] max-w-[calc(100vw-24px)] flex-col overflow-hidden p-0"
        onKeyDown={(event) => {
          if (event.key !== "Tab") return;
          const current = document.activeElement;
          const targets = Array.from(
            event.currentTarget.querySelectorAll<HTMLElement>(
              "button, input, [tabindex]",
            ),
          ).filter(
            (node) =>
              !node.matches(":disabled") &&
              (node.tabIndex >= 0 || node === current),
          );
          const index = targets.indexOf(current as HTMLElement);
          const next = targets[index + (event.shiftKey ? -1 : 1)];
          if (next) {
            event.preventDefault();
            // Radix's looping FocusScope handles this same event after the
            // child handler. Move focus after it has checked the original node.
            queueMicrotask(() => next.focus({ preventScroll: true }));
          } else menu.onKeyDown(event);
        }}
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          input.current?.focus({ preventScroll: true });
        }}
        onCloseAutoFocus={menu.onCloseAutoFocus}
      >
        <div className="sticky top-0 z-[1] shrink-0 border-b border-line-2 bg-pop-bg p-2">
          <SearchInput
            ref={input}
            className="!w-full"
            aria-label={t("usageWorkbench.searchOptions", { name: label })}
            placeholder={t("usageWorkbench.searchOptions", { name: label })}
            value={search}
            onChange={(value) => {
              setSearch(value);
              setPages(1);
            }}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                refs.current[
                  event.key === "ArrowDown" ? 0 : shown.length - 1
                ]?.focus();
              }
            }}
          />
          <div className="mt-2 flex items-center justify-between gap-2 text-note">
            <span className="text-ink-muted">
              {t("usageFilters.selectedCount", {
                selected: selected.length,
                total: options.length,
              })}
            </span>
            <button
              type="button"
              disabled={!selected.length}
              aria-label={t("usageFilters.clearMenu", { name: label })}
              onClick={clearSelection}
              className="inline-flex shrink-0 items-center gap-1 rounded-ctl px-1.5 py-0.5 text-ink-muted hover:bg-pill disabled:opacity-40"
            >
              <X aria-hidden className="h-3 w-3" />
              {t("common.clear")}
            </button>
          </div>
          {selected.length > 0 && (
            <p
              className="mt-1 truncate text-note text-brand-ink"
              title={summary}
            >
              {summary}
            </p>
          )}
        </div>
        <div className="min-h-0 overflow-y-auto p-1">
          <button
            type="button"
            aria-pressed={!selected.length}
            className={cn(
              "mb-1 flex w-full items-center gap-2 rounded-ctl px-2 py-1.5 text-left text-sub",
              !selected.length
                ? "bg-brand-soft/50 text-brand-ink"
                : "text-ink-muted hover:bg-pill",
            )}
            onClick={clearSelection}
          >
            <span aria-hidden className="w-3.5 shrink-0">
              {!selected.length && <Check className="h-3.5 w-3.5" />}
            </span>
            {allLabel ?? t("usageWorkbench.all")}
          </button>
          <div
            id={id}
            role="listbox"
            aria-label={label}
            aria-multiselectable="true"
          >
            {shown.map((option, index) => (
              <button
                key={option.value}
                type="button"
                role="option"
                tabIndex={index === 0 ? 0 : -1}
                aria-label={option.label}
                aria-selected={selected.includes(option.value)}
                ref={(node) => {
                  refs.current[index] = node;
                }}
                className={cn(
                  "flex w-full items-center gap-2 rounded-ctl px-2 py-1.5 text-left text-sub outline-none hover:bg-pill focus-visible:ring-2 focus-visible:ring-brand/30",
                  selected.includes(option.value)
                    ? "bg-brand-soft/50 text-brand-ink"
                    : "text-ink",
                )}
                onClick={() =>
                  onChange(
                    selected.includes(option.value)
                      ? selected.filter((value) => value !== option.value)
                      : [...selected, option.value].sort(),
                  )
                }
                onKeyDown={(event) => {
                  const next =
                    event.key === "ArrowDown"
                      ? (index + 1) % shown.length
                      : event.key === "ArrowUp"
                        ? (index - 1 + shown.length) % shown.length
                        : event.key === "Home"
                          ? 0
                          : event.key === "End"
                            ? shown.length - 1
                            : undefined;
                  if (next !== undefined) {
                    event.preventDefault();
                    refs.current[next]?.focus();
                  }
                }}
              >
                <span aria-hidden className="w-3.5 shrink-0">
                  {selected.includes(option.value) && (
                    <Check className="h-3.5 w-3.5" />
                  )}
                </span>
                <span
                  className="min-w-0 flex-1"
                  title={option.title ?? option.label}
                >
                  <span className="block truncate">{option.label}</span>
                  {option.title && option.title !== option.label && (
                    <span className="block truncate font-mono text-micro text-ink-faint">
                      {option.title}
                    </span>
                  )}
                </span>
              </button>
            ))}
            {!shown.length && (
              <p className="p-2 text-note text-ink-muted">
                {t("usageWorkbench.noOptions")}
              </p>
            )}
          </div>
          {matching.length > shown.length && (
            <button
              type="button"
              className="mt-1 w-full rounded-ctl px-2 py-1.5 text-left text-note text-ink-muted hover:bg-pill focus-visible:ring-2 focus-visible:ring-brand/30"
              onClick={() => setPages((value) => value + 1)}
            >
              {t("usageSessions.showMore", {
                count: matching.length - shown.length,
              })}
            </button>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
