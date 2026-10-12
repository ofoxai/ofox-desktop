import type { TFunction } from "i18next";
import { toast } from "sonner";
import { TOOL_META } from "@/config/toolMeta";
import type { KeyReconcileReport, LogoutReport } from "@/lib/api/ofoxAuth";

function toolLabels(ids: string[], t: TFunction): string {
  return ids
    .map((id) => TOOL_META[id]?.label ?? id)
    .join(t("accountKeys.listSeparator"));
}

/** 退出登录后说明已绑定的工具暂停了；服务器上没停掉的 key 请用户去控制台删。 */
export function announceLogout(report: LogoutReport, t: TFunction) {
  toast.success(
    report.stopped.length > 0
      ? t("accountKeys.loggedOutPaused")
      : t("accountKeys.loggedOut"),
  );
  if (report.notRevoked.length > 0) {
    toast.warning(
      t("accountKeys.notRevoked", {
        tools: toolLabels(report.notRevoked, t),
      }),
      { duration: 15_000 },
    );
  }
}

/** 登录后说明哪些工具改用了当前账号的额度、哪些没能自动换。 */
export function announceKeyReconcile(
  report: KeyReconcileReport,
  t: TFunction,
  account: string | null | undefined,
) {
  if (report.switched.length > 0) {
    toast.success(
      t(account ? "accountKeys.switched" : "accountKeys.switchedToCurrent", {
        tools: toolLabels(report.switched, t),
        account,
      }),
    );
  }
  for (const failure of report.failed) {
    toast.warning(
      t("accountKeys.switchFailed", {
        tool: toolLabels([failure.tool], t),
        message: failure.message,
      }),
      { duration: 15_000 },
    );
  }
}
