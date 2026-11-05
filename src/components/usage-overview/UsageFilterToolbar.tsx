import { useRef, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { Button } from "@/components/ui/button";
import { UsageFilterMenu, type UsageFilterOption } from "./UsageFilterMenu";

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
  leading,
  trailing,
  children,
}: {
  tools: UsageFilterOption[];
  selectedTools: string[];
  onToolsChange: (ids: string[]) => void;
  disabled: boolean;
  chips: UsageFilterChip[];
  onReset: () => void;
  /** Before the tools menu on the same row (the period). */
  leading?: ReactNode;
  /** At the row's right end (the estimate). */
  trailing?: ReactNode;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const toolsMenu = useRef<HTMLDivElement>(null),
    chipButtons = useRef(new Map<string, HTMLButtonElement>());
  const restore = (next?: string) =>
    window.requestAnimationFrame(() => {
      const target = next ? chipButtons.current.get(next) : undefined;
      (
        target ??
        toolsMenu.current?.querySelector<HTMLElement>('[role="combobox"]')
      )?.focus({ preventScroll: true });
    });
  return (
    <div className="min-w-0 space-y-2">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        {leading}
        <div ref={toolsMenu} title={t("usageFilters.toolsHint")}>
          <UsageFilterMenu
            label={t("usageWorkbench.toolsFilter")}
            allLabel={t("usageFilters.allTools")}
            options={tools.map((tool) => ({
              ...tool,
              icon: <ToolBadge toolId={tool.value} size={14} variant="plain" />,
            }))}
            selected={selectedTools}
            onChange={onToolsChange}
            disabled={disabled}
          />
        </div>
        <div
          role="group"
          aria-label={t("usageFilters.otherFilters")}
          className="flex min-w-0 flex-wrap items-center gap-2"
        >
          {children}
        </div>
        {trailing && <div className="ml-auto flex-none">{trailing}</div>}
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
