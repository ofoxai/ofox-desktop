import { useTranslation } from "react-i18next";
import { MenuSelect } from "@/components/ui/menu-select";
import { SearchInput } from "@/components/ui/search-input";
import { Button } from "@/components/ui/button";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { TOOL_META } from "@/config/toolMeta";
import type { ManagedSession } from "@/lib/api/sessions";
import { agentCounts } from "./model";
import { SegmentedControl } from "@/components/ui/segmented";
import type { ReactNode } from "react";
export function SessionsHeader({
  sessions,
  agent,
  setAgent,
  search,
  setSearch,
  trash,
  setTrash,
  trashCount,
  searchMode = "metadata",
  setSearchMode,
  onStartSearch,
  projectPicker,
}: {
  sessions: ManagedSession[];
  agent: string;
  setAgent: (s: string) => void;
  search: string;
  setSearch: (s: string) => void;
  trash: boolean;
  setTrash: (v: boolean) => void;
  trashCount: number;
  searchMode?: "metadata" | "content";
  setSearchMode?: (mode: "metadata" | "content") => void;
  onStartSearch?: () => void;
  projectPicker?: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <div className="mb-3 flex flex-wrap items-center gap-2">
      <MenuSelect
        aria-label={t("sessionCenter.provider")}
        value={agent}
        onChange={setAgent}
        options={[
          {
            value: "all",
            label: t("sessionCenter.allTools"),
            count: sessions.length,
          },
          ...agentCounts(sessions).map(([value, count]) => ({
            value,
            count,
            label: TOOL_META[value]?.label ?? value,
            icon: <ToolBadge toolId={value} size={13} />,
          })),
        ]}
      />
      {projectPicker}
      <span className="flex-1" />
      {!trash && setSearchMode && (
        <SegmentedControl
          aria-label={t("sessionExperience.searchMode")}
          value={searchMode}
          onChange={setSearchMode}
          options={[
            { value: "metadata", label: t("sessionExperience.metadata") },
            { value: "content", label: t("sessionExperience.content") },
          ]}
        />
      )}
      <SearchInput
        value={search}
        onChange={setSearch}
        className="w-[200px]"
        placeholder={t(
          searchMode === "content"
            ? "sessionExperience.searchPlaceholder"
            : "sessionCenter.filter",
        )}
        onKeyDown={(event) => {
          if (searchMode === "content" && event.key === "Enter") {
            event.preventDefault();
            onStartSearch?.();
          }
        }}
      />
      <Button
        variant="ghost"
        aria-pressed={trash}
        onClick={() => setTrash(!trash)}
      >
        {t("sessionCenter.trash")} · {trashCount}
      </Button>
    </div>
  );
}
