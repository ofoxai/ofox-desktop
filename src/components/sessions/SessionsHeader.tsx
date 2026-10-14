import { useTranslation } from "react-i18next";
import { MenuSelect } from "@/components/ui/menu-select";
import { SearchInput } from "@/components/ui/search-input";
import { Button } from "@/components/ui/button";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { TOOL_META } from "@/config/toolMeta";
import type { ManagedSession } from "@/lib/api/sessions";
import { agentCounts } from "./model";
export function SessionsHeader({
  sessions,
  agent,
  setAgent,
  search,
  setSearch,
  trash,
  setTrash,
  trashCount,
}: {
  sessions: ManagedSession[];
  agent: string;
  setAgent: (s: string) => void;
  search: string;
  setSearch: (s: string) => void;
  trash: boolean;
  setTrash: (v: boolean) => void;
  trashCount: number;
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
      <span className="flex-1" />
      <SearchInput
        value={search}
        onChange={setSearch}
        className="w-[200px]"
        placeholder={t("sessionCenter.filter")}
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
