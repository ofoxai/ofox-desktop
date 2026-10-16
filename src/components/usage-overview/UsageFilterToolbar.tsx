import { useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { UsageFilterOption } from "./UsageFilterMenu";

export interface UsageFilterChip {
  key: string;
  name: string;
  value: string;
  title?: string;
  toolId?: string;
  remove: () => void;
}
export function UsageFilterToolbar({
  tools,
  selectedTools,
  onToolsChange,
  disabled,
  chips,
  onReset,
  children,
}: {
  tools: UsageFilterOption[];
  selectedTools: string[];
  onToolsChange: (ids: string[]) => void;
  disabled: boolean;
  chips: UsageFilterChip[];
  onReset: () => void;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const [focused, setFocused] = useState(0);
  const buttons = useRef<(HTMLButtonElement | null)[]>([]),
    chipButtons = useRef(new Map<string, HTMLButtonElement>());
  const rows = [{ value: "", label: t("usageFilters.allTools") }, ...tools];
  const restore = (next?: string) =>
    window.requestAnimationFrame(() => {
      const target = next ? chipButtons.current.get(next) : undefined;
      (target ?? buttons.current[0])?.focus({ preventScroll: true });
    });
  return (
    <div className="min-w-0 space-y-2">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <div
          role="group"
          aria-label={t("usageWorkbench.toolsFilter")}
          title={t("usageFilters.toolsHint")}
          className="flex min-w-[180px] flex-1 items-center overflow-x-auto rounded-[8px] bg-pill p-[2px] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        >
          {rows.map((row, index) => {
            const selected = row.value
              ? selectedTools.includes(row.value)
              : !selectedTools.length;
            return (
              <button
                key={row.value}
                type="button"
                aria-label={row.label}
                aria-pressed={selected}
                disabled={disabled}
                ref={(node) => {
                  buttons.current[index] = node;
                }}
                tabIndex={index === Math.min(focused, rows.length - 1) ? 0 : -1}
                onFocus={() => setFocused(index)}
                onClick={() =>
                  onToolsChange(
                    row.value
                      ? selected
                        ? selectedTools.filter((value) => value !== row.value)
                        : [...selectedTools, row.value].sort()
                      : [],
                  )
                }
                onKeyDown={(event) => {
                  const next =
                    event.key === "ArrowRight"
                      ? (index + 1) % rows.length
                      : event.key === "ArrowLeft"
                        ? (index - 1 + rows.length) % rows.length
                        : event.key === "Home"
                          ? 0
                          : event.key === "End"
                            ? rows.length - 1
                            : undefined;
                  if (next != null) {
                    event.preventDefault();
                    buttons.current[next]?.focus({ preventScroll: true });
                    buttons.current[next]?.scrollIntoView({
                      block: "nearest",
                      inline: "nearest",
                    });
                  }
                }}
                className={cn(
                  "inline-flex h-[24px] shrink-0 items-center gap-1.5 whitespace-nowrap rounded-[6px] px-2.5 text-ctl transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand/40 disabled:opacity-45",
                  selected
                    ? "bg-card text-ink shadow-[0_1px_2px_rgba(0,0,0,.14),0_0_0_.5px_rgba(0,0,0,.08)]"
                    : "text-ink-muted hover:text-ink",
                )}
              >
                {row.value && (
                  <ToolBadge toolId={row.value} size={14} variant="plain" />
                )}
                {row.label}
              </button>
            );
          })}
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          {children}
        </div>
      </div>
      {!!chips.length && (
        <div
          role="group"
          aria-label={t("usageFilters.activeFilters")}
          className="flex min-w-0 items-start gap-2"
        >
          <div className="flex max-h-[88px] min-w-0 flex-1 flex-wrap gap-1.5 overflow-y-auto">
            {chips.map((chip, index) => (
              <button
                key={chip.key}
                ref={(node) => {
                  if (node) chipButtons.current.set(chip.key, node);
                  else chipButtons.current.delete(chip.key);
                }}
                type="button"
                title={chip.title ?? chip.value}
                aria-label={t("usageFilters.removeFilter", {
                  name: chip.name,
                  value: chip.value,
                })}
                onClick={() => {
                  chip.remove();
                  restore(chips[index + 1]?.key ?? chips[index - 1]?.key);
                }}
                className="inline-flex h-[24px] max-w-[240px] items-center gap-1.5 rounded-[6px] border border-brand/20 bg-brand-soft/50 px-2 text-note text-brand-ink hover:bg-brand-soft focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand/40"
              >
                {chip.toolId && (
                  <ToolBadge toolId={chip.toolId} size={12} variant="plain" />
                )}
                <span className="min-w-0 truncate">{chip.value}</span>
                <X aria-hidden className="h-3 w-3 shrink-0" />
              </button>
            ))}
          </div>
          <Button
            variant="ghost"
            className="h-[24px] shrink-0 px-2 text-note"
            onClick={() => {
              onReset();
              restore();
            }}
          >
            {t("usageWorkbench.reset")}
          </Button>
        </div>
      )}
    </div>
  );
}
