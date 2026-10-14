import { CheckCircle2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { isMac } from "@/lib/platform";
import { TOOL_META } from "@/config/toolMeta";
import { Button } from "@/components/ui/button";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { ListCard, ListRow } from "@/components/ui/list";
interface SetupCompletePageProps {
  boundTools: string[];
  onOpenConsole: () => void;
}
export default function SetupCompletePage({
  boundTools,
  onOpenConsole,
}: SetupCompletePageProps) {
  const { t } = useTranslation();
  const tools = [...new Set(boundTools)];
  return (
    <div className="flex h-screen w-full overflow-y-auto bg-canvas">
      <div className="m-auto flex w-full max-w-xl flex-col items-center px-8 py-8">
        <div className="mb-6 grid h-16 w-16 place-items-center rounded-dialog bg-ok-soft text-ok">
          <CheckCircle2 className="h-9 w-9" />
        </div>
        <h1 className="mb-3 text-2xl font-semibold text-ink">
          {t("setupComplete.title")}
        </h1>
        <p className="mb-2 text-center text-ui text-ink-muted">
          {t("setupComplete.bound", { count: tools.length })}
        </p>
        <p className="mb-6 text-center text-ui text-ink-muted">
          {t(isMac() ? "setupComplete.macHint" : "setupComplete.trayHint")}
        </p>
        <ListCard className="mb-6 w-full">
          {tools.map((id) => (
            <ListRow key={id}>
              <ToolBadge toolId={id} size={24} />
              <span className="text-ui text-ink">
                {TOOL_META[id]?.label ?? id}
              </span>
              <CheckCircle2 aria-hidden className="ml-auto h-4 w-4 text-ok" />
            </ListRow>
          ))}
        </ListCard>
        <Button variant="primary" onClick={onOpenConsole}>
          {t("setupComplete.open")}
        </Button>
      </div>
    </div>
  );
}
