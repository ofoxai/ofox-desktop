import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ListFold } from "@/components/ui/list";
/**
 * The fold under a session with the subagents it started, drawn as sessions
 * of their own (Magpie sessions client.js `subagentsBox`, MIT).
 */
export function SessionSubagents({
  count,
  nested,
  open,
  onOpenChange,
  children,
}: {
  count: number;
  /** Drawn inside another session's fold, which sets it in already. */
  nested: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <div className={nested ? "pb-1.5 pl-6 pr-3" : "pb-1.5 pl-[30px] pr-3"}>
      <ListFold open={open} onOpenChange={onOpenChange} className="text-note">
        {t("sessionCenter.subagents", { count })}
      </ListFold>
      {open && (
        <div className="mt-0.5 flex flex-col divide-y divide-line-2 overflow-hidden rounded-[9px] border border-line-2 bg-card">
          {children}
        </div>
      )}
    </div>
  );
}
