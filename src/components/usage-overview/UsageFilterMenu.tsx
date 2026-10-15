import { useId, useRef, useState } from "react";
import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Input } from "@/components/ui/input";
import { MenuChevron, menuTriggerClassName } from "@/components/ui/menu-select";
import { useMenuTab } from "@/hooks/useMenuTab";

export function UsageFilterMenu({
  label,
  options,
  selected,
  onChange,
  disabled = false,
  pageSize,
}: {
  label: string;
  options: { value: string; label: string; title?: string }[];
  selected: string[];
  onChange: (values: string[]) => void;
  disabled?: boolean;
  pageSize?: number;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [pages, setPages] = useState(1);
  const id = useId();
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const menu = useMenuTab(() => setOpen(false));
  const matching = options.filter((option) =>
    `${option.value} ${option.label} ${option.title ?? ""}`
      .toLocaleLowerCase()
      .includes(search.toLocaleLowerCase()),
  );
  const shown = matching.slice(0, (pageSize ?? Infinity) * pages);
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) {
          setSearch("");
          setPages(1);
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
          className={menuTriggerClassName}
        >
          <span className="truncate">
            {label} ·{" "}
            {selected.length ? selected.length : t("usageWorkbench.all")}
          </span>
          <MenuChevron />
        </button>
      </PopoverTrigger>
      <PopoverContent
        className="w-[270px] p-2"
        onKeyDown={menu.onKeyDown}
        onCloseAutoFocus={menu.onCloseAutoFocus}
      >
        <Input
          aria-label={t("usageWorkbench.searchOptions", { name: label })}
          value={search}
          onChange={(event) => {
            setSearch(event.target.value);
            setPages(1);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              refs.current[0]?.focus();
            }
          }}
        />
        <button
          type="button"
          className="my-1 w-full rounded-ctl px-2 py-1 text-left text-note text-ink-muted hover:bg-pill"
          onClick={() => onChange([])}
        >
          {t("usageWorkbench.all")}
        </button>
        <div
          id={id}
          role="listbox"
          aria-label={label}
          aria-multiselectable="true"
          className="max-h-[250px] overflow-y-auto"
        >
          {shown.map((option, index) => (
            <button
              key={option.value}
              type="button"
              role="option"
              aria-selected={selected.includes(option.value)}
              ref={(node) => {
                refs.current[index] = node;
              }}
              className="flex w-full items-center gap-2 rounded-ctl px-2 py-1.5 text-left text-sub text-ink outline-none hover:bg-pill focus-visible:bg-pill focus-visible:ring-2 focus-visible:ring-brand/30"
              onClick={() =>
                onChange(
                  selected.includes(option.value)
                    ? selected.filter((id) => id !== option.value)
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
                {selected.includes(option.value) ? (
                  <Check className="h-3.5 w-3.5" />
                ) : null}
              </span>
              <span
                className="min-w-0 truncate"
                title={option.title ?? option.label}
              >
                {option.label}
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
      </PopoverContent>
    </Popover>
  );
}
