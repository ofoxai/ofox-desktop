import { useTranslation } from "react-i18next";
import { SlidersHorizontal } from "lucide-react";
import type {
  AssistantToolCapabilities,
  AssistantPermission,
} from "@/lib/api/assistant";
import { TOOL_META } from "@/config/toolMeta";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogClose,
  DialogFooter,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
export function AssistantCapabilities({
  capabilities,
  mode,
}: {
  capabilities: AssistantToolCapabilities;
  mode: AssistantPermission;
}) {
  const { t } = useTranslation();
  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button
          variant="ghost"
          className="h-auto min-h-[27px] w-full justify-start whitespace-normal py-1 text-left text-note"
        >
          <SlidersHorizontal aria-hidden className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 break-words">
            {t("assistant.capabilities.title")}
          </span>
        </Button>
      </DialogTrigger>
      <DialogContent className="flex max-h-[85vh] max-w-3xl flex-col overflow-hidden">
        <DialogHeader className="shrink-0">
          <DialogTitle>{t("assistant.capabilities.title")}</DialogTitle>
          <DialogDescription>
            {t("assistant.capabilities.description")}
          </DialogDescription>
        </DialogHeader>
        <p className="px-[18px] pt-4 text-note">
          {t("assistant.permission." + mode)} ·{" "}
          {t("assistant.capabilities.command")}:{" "}
          {t(
            "assistant.capabilities.support." +
              capabilities.arbitraryCommand[mode],
          )}
        </p>
        <div className="mx-[18px] my-4 min-h-0 overflow-auto">
          <table className="w-full text-left text-note">
            <thead>
              <tr>
                <th className="p-2">{t("assistant.capabilities.tool")}</th>
                <th className="p-2">{t("assistant.capabilities.action")}</th>
                <th className="p-2">
                  {t("assistant.capabilities.supportTitle")}
                </th>
              </tr>
            </thead>
            <tbody>
              {capabilities.tools.flatMap((tool) =>
                tool.actions.map((action) => (
                  <tr
                    key={tool.tool + action.action}
                    className="border-t border-line"
                  >
                    <td className="p-2 align-top">
                      {TOOL_META[tool.tool]?.label ?? tool.tool}
                    </td>
                    <td className="p-2">
                      <span>
                        {t("assistant.capabilities.actions." + action.action)}
                      </span>
                      <p className="mt-1 text-ink-muted">
                        {t("assistant.capabilities.notes." + action.note)}
                      </p>
                    </td>
                    <td className="p-2 align-top">
                      {t(
                        "assistant.capabilities.support." +
                          action.supportByMode[mode],
                      )}
                    </td>
                  </tr>
                )),
              )}
            </tbody>
          </table>
        </div>
        <DialogFooter className="shrink-0">
          <DialogClose asChild>
            <Button variant="secondary">{t("common.close")}</Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
