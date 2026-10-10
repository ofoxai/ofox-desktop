import type { TFunction } from "i18next";
import { TOOL_META } from "@/config/toolMeta";

const PROXY_UNAVAILABLE = "LOCAL_PROXY_UNAVAILABLE|";
const TOOL_NOT_INSTALLED = "TOOL_NOT_INSTALLED|";

/**
 * 「打开」失败时的提示文案。后端用前缀标记已知原因：本机代理没开、CLI 没装
 * （Windows 上按此刻新开终端的 PATH 找不到）。
 */
export function launchErrorMessage(error: unknown, t: TFunction): string {
  const message = String(error);
  if (message.startsWith(PROXY_UNAVAILABLE)) {
    return t("toolLaunch.proxyUnavailable", {
      endpoint: message.slice(PROXY_UNAVAILABLE.length),
    });
  }
  if (message.startsWith(TOOL_NOT_INSTALLED)) {
    const toolId = message.slice(TOOL_NOT_INSTALLED.length);
    return t("toolLaunch.notInstalled", {
      tool: TOOL_META[toolId]?.label ?? toolId,
    });
  }
  return t("toolLaunch.failed");
}
