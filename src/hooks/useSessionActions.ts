import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { sessionsApi } from "@/lib/api/sessions";
import { copyText } from "@/lib/clipboard";

const launchingKeys = new Set<string>();
/** Shared by list and reader so switching views cannot submit two launches. */
export function useSessionResume(sessionKey: string) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [command, setCommand] = useState<string>();
  const [feedback, setFeedback] = useState<string>();
  const launch = async () => {
    if (launchingKeys.has(sessionKey)) return;
    launchingKeys.add(sessionKey);
    setBusy(true);
    setCommand(undefined);
    setFeedback(undefined);
    try {
      let result;
      try {
        result = await sessionsApi.resume(sessionKey, true);
      } catch {
        result = await sessionsApi.resume(sessionKey, false);
      }
      if (!result.launched) {
        setCommand(result.command);
        try {
          await copyText(result.command);
          setFeedback(t("sessionExperience.resumeFallback"));
          toast.success(t("sessionCenter.commandCopied"));
        } catch {
          setFeedback(t("sessionExperience.resumeCopyFailed"));
        }
      }
    } catch (error) {
      toast.error(String(error));
    } finally {
      launchingKeys.delete(sessionKey);
      setBusy(false);
    }
  };
  return { busy, command, feedback, launch };
}

export function useSessionExport(sessionKey: string) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const running = useRef(false);
  const exportFile = async () => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    try {
      const preview = await sessionsApi.exportMarkdown(sessionKey);
      const destination = await save({
        defaultPath: preview.filename,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (!destination) return;
      await sessionsApi.exportMarkdown(sessionKey, destination);
      toast.success(t("sessionCenter.exported"));
    } catch (error) {
      toast.error(String(error));
    } finally {
      running.current = false;
      setBusy(false);
    }
  };
  return { busy, exportFile };
}
