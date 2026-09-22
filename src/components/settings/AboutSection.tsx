import { useCallback, useEffect, useState } from "react";
import { Copy, ExternalLink, Info, Loader2, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ToolUpdatesPanel } from "./ToolUpdatesPanel";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { getVersion } from "@tauri-apps/api/app";
import { settingsApi } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { motion } from "framer-motion";
import appIcon from "@/assets/icons/app-icon.png";
import { isWindows } from "@/lib/platform";

interface AboutSectionProps {
  isPortable: boolean;
}

const ONE_CLICK_INSTALL_COMMANDS = `# Claude Code (Native install - recommended)
curl -fsSL https://claude.ai/install.sh | bash
# Codex
npm i -g @openai/codex@latest
# Gemini CLI
npm i -g @google/gemini-cli@latest
# OpenCode
curl -fsSL https://opencode.ai/install | bash`;

export function AboutSection({ isPortable }: AboutSectionProps) {
  // ... (use hooks as before) ...
  const { t } = useTranslation();
  const [version, setVersion] = useState<string | null>(null);
  const [isLoadingVersion, setIsLoadingVersion] = useState(true);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  useEffect(() => {
    let active = true;
    getVersion()
      .then((value) => {
        if (active) setVersion(value);
      })
      .catch(() => undefined)
      .finally(() => {
        if (active) setIsLoadingVersion(false);
      });
    return () => {
      active = false;
    };
  }, []);

  // ... (handlers like handleOpenReleaseNotes, handleCheckUpdate) ...

  const handleOpenReleaseNotes = useCallback(async () => {
    try {
      // Ofox Desktop 的下载/版本页（自托管），不再指向上游 GitHub releases。
      await settingsApi.openExternal("https://ofox.ai/download");
    } catch (error) {
      console.error("[AboutSection] Failed to open release notes", error);
      toast.error(t("settings.openReleaseNotesFailed"));
    }
  }, [t]);

  const handleCheckUpdate = useCallback(async () => {
    // Ofox Desktop 自托管更新：拉 R2 的 latest.json 比对版本。本应用不做自动
    // 安装，发现新版只提示并引导用户去下载页手动安装。
    setIsCheckingUpdate(true);
    try {
      const result = await settingsApi.checkUpdates();

      if (!result.hasUpdate) {
        toast.success(t("settings.upToDate"), { closeButton: true });
        return;
      }

      const target = result.downloadUrl;
      toast.info(
        t("settings.updateAvailable", { version: result.latestVersion }),
        {
          description: result.notes || undefined,
          duration: 12000,
          closeButton: true,
          action: target
            ? {
                label: t("settings.goDownload"),
                onClick: () => {
                  settingsApi.openExternal(target).catch((error) => {
                    console.error(
                      "[AboutSection] open download url failed",
                      error,
                    );
                    toast.error(t("settings.openReleaseNotesFailed"));
                  });
                },
              }
            : undefined,
        },
      );
    } catch (error) {
      console.error("[AboutSection] Check update failed", error);
      toast.error(t("settings.checkUpdateFailed"));
    } finally {
      setIsCheckingUpdate(false);
    }
  }, [t]);

  const handleCopyInstallCommands = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(ONE_CLICK_INSTALL_COMMANDS);
      toast.success(t("settings.installCommandsCopied"), { closeButton: true });
    } catch (error) {
      console.error("[AboutSection] Failed to copy install commands", error);
      toast.error(t("settings.installCommandsCopyFailed"));
    }
  }, [t]);

  const displayVersion = version ?? t("common.unknown");

  return (
    <motion.section
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3 }}
      className="space-y-6"
    >
      <header className="space-y-1">
        <h3 className="text-sm font-medium">{t("common.about")}</h3>
        <p className="text-xs text-muted-foreground">
          {t("settings.aboutHint")}
        </p>
      </header>

      <motion.div
        initial={{ opacity: 0, scale: 0.98 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.3, delay: 0.1 }}
        className="rounded-xl border border-border bg-gradient-to-br from-card/80 to-card/40 p-6 space-y-5 shadow-sm"
      >
        <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <img src={appIcon} alt="Ofox Desktop" className="h-5 w-5" />
              <h4 className="text-lg font-semibold text-foreground">
                Ofox Desktop
              </h4>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant="outline" className="gap-1.5 bg-background/80">
                <span className="text-muted-foreground">
                  {t("common.version")}
                </span>
                {isLoadingVersion ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : (
                  <span className="font-medium">{`v${displayVersion}`}</span>
                )}
              </Badge>
              {isPortable && (
                <Badge variant="secondary" className="gap-1.5">
                  <Info className="h-3 w-3" />
                  {t("settings.portableMode")}
                </Badge>
              )}
            </div>
          </div>

          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={handleOpenReleaseNotes}
              className="h-8 gap-1.5 text-xs"
            >
              <ExternalLink className="h-3.5 w-3.5" />
              {t("settings.releaseNotes")}
            </Button>
            <Button
              type="button"
              size="sm"
              onClick={handleCheckUpdate}
              disabled={isCheckingUpdate}
              className="h-8 gap-1.5 text-xs"
            >
              {isCheckingUpdate ? (
                <>
                  <RefreshCw className="h-3.5 w-3.5 animate-spin" />
                  {t("settings.checking")}
                </>
              ) : (
                <>
                  <RefreshCw className="h-3.5 w-3.5" />
                  {t("settings.checkForUpdates")}
                </>
              )}
            </Button>
          </div>
        </div>
      </motion.div>

      <ToolUpdatesPanel />

      {!isWindows() && (
        <motion.div
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.3, delay: 0.3 }}
          className="space-y-3"
        >
          <h3 className="text-sm font-medium px-1">
            {t("settings.oneClickInstall")}
          </h3>
          <div className="rounded-xl border border-border bg-gradient-to-br from-card/80 to-card/40 p-4 space-y-3 shadow-sm">
            <div className="flex items-center justify-between gap-2">
              <p className="text-xs text-muted-foreground">
                {t("settings.oneClickInstallHint")}
              </p>
              <Button
                size="sm"
                variant="outline"
                onClick={handleCopyInstallCommands}
                className="h-7 gap-1.5 text-xs"
              >
                <Copy className="h-3.5 w-3.5" />
                {t("common.copy")}
              </Button>
            </div>
            <pre className="text-xs font-mono bg-background/80 px-3 py-2.5 rounded-lg border border-border/60 overflow-x-auto">
              {ONE_CLICK_INSTALL_COMMANDS}
            </pre>
          </div>
        </motion.div>
      )}
    </motion.section>
  );
}
