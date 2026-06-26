import { ArrowUpCircle, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import { settingsApi } from "@/lib/api";
import { useUpdate } from "@/contexts/UpdateContext";

/**
 * 托盘弹窗顶部的"发现新版本"横幅。
 *
 * 显示条件由父组件 `TrayPopoverApp` 控制（`hasUpdate && !isDismissed`），
 * 这里只负责渲染与两个动作：
 * - 前往下载：打开下载页（直链或通用下载页），随后隐藏弹窗（参考 BottomMenu）。
 * - 跳过：调 `useUpdate().dismissUpdate()`，写 localStorage 并广播事件，主窗口
 *   顶栏徽标会同步消失。
 *
 * 本应用不做自动安装，只引导用户去下载页手动更新。
 */
export default function UpdateBanner() {
  const { t } = useTranslation();
  const { updateInfo, dismissUpdate } = useUpdate();
  const version = updateInfo?.availableVersion ?? "";
  const target = updateInfo?.downloadUrl;

  const handleDownload = async () => {
    if (target) {
      try {
        await settingsApi.openExternal(target);
      } catch (e) {
        console.error("[UpdateBanner] open download url failed", e);
      }
    }
    try {
      await getCurrentWindow().hide();
    } catch (e) {
      console.error("[UpdateBanner] hide popover failed", e);
    }
  };

  return (
    <div className="mx-3.5 flex items-center gap-2.5 rounded-lg border border-emerald-300 bg-gradient-to-l from-emerald-100 to-emerald-50 px-2.5 py-2 dark:border-emerald-500/30 dark:from-emerald-600/25 dark:to-emerald-500/5">
      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-emerald-500 text-white">
        <ArrowUpCircle className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[11px] font-semibold text-emerald-800 dark:text-emerald-300">
          {t("update.bannerTitle", { defaultValue: "发现新版本" })}
          {version ? ` v${version}` : ""}
        </div>
        <div className="text-[10px] leading-tight text-emerald-700/80 dark:text-emerald-400/70">
          {t("update.bannerHint", { defaultValue: "前往下载页获取最新版本" })}
        </div>
      </div>
      <button
        onClick={() => void handleDownload()}
        className="shrink-0 rounded-md bg-emerald-500 px-2.5 py-0.5 text-[11px] font-medium text-white hover:bg-emerald-600"
      >
        {t("settings.goDownload", { defaultValue: "前往下载" })}
      </button>
      <button
        onClick={() => dismissUpdate()}
        title={t("update.skipThisVersion", { defaultValue: "跳过此版本" })}
        aria-label={t("update.skipThisVersion", { defaultValue: "跳过此版本" })}
        className="shrink-0 rounded p-0.5 text-emerald-700/60 hover:bg-emerald-500/10 hover:text-emerald-800 dark:text-emerald-400/60 dark:hover:text-emerald-300"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
