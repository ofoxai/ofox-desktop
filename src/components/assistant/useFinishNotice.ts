import { useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { useOptionalShell } from "@/components/shell/ShellContext";

/** How a run ended, as its `run_end` says. */
export type FinishStatus = "completed" | "stopped" | "error" | "queued";

/** A run of a conversation that just ended. */
export interface FinishedRun {
  sessionId: string;
  status: FinishStatus;
}

/**
 * How to tell the user that a reply finished: a system notification while
 * the window is in the background, a notice while they look at another page
 * or conversation, nothing while it is on screen. A stopped run is never
 * told: the user stopped it, or it ended for them (a model change, the
 * trash, a sign-out). A failed one is, as a finished one is: either way the
 * user who waits for it has something to look at.
 */
export function finishNotice({
  status,
  enabled,
  focused,
  onAssistantPage,
  openSessionId,
  sessionId,
}: {
  status: FinishStatus;
  /** The user's switch. */
  enabled: boolean;
  /** The window has the focus. */
  focused: boolean;
  /** The app shows the assistant page. */
  onAssistantPage: boolean;
  /** The conversation the assistant page shows; null for none. */
  openSessionId: string | null;
  sessionId: string;
}): "system" | "toast" | null {
  if (!enabled || (status !== "completed" && status !== "error")) return null;
  if (!focused) return "system";
  if (!onAssistantPage || openSessionId !== sessionId) return "toast";
  return null;
}

/** Whether the system lets the app notify, asking the user if need be. */
async function notificationsAllowed() {
  try {
    if (await isPermissionGranted()) return true;
    return (await requestPermission()) === "granted";
  } catch (error) {
    console.warn("[Assistant] notification permission is unavailable", error);
    return false;
  }
}

interface FinishNoticeOptions {
  /** The user's switch. */
  enabled: boolean;
  /** Changes when the account or authority changes. */
  authority: () => number;
  /** The conversation the assistant page shows; null for none. */
  openSessionId: string | null;
  /** A conversation's title as the user knows it; empty for untitled. */
  titleOf: (sessionId: string) => string;
  /** Show a conversation in the assistant page. */
  open: (sessionId: string) => void;
}

/**
 * Tells the user that a run finished (see `finishNotice`), with the
 * conversation's title only, never the reply. The system's permission is
 * checked once, at the first notification, never before.
 */
export function useFinishNotice(options: FinishNoticeOptions) {
  const { t } = useTranslation();
  const shell = useOptionalShell();
  const latest = useRef({ options, shell, t });
  latest.current = { options, shell, t };
  const permission = useRef<Promise<boolean> | null>(null);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  return useCallback((run: FinishedRun) => {
    const { options, shell, t } = latest.current;
    const authority = options.authority();
    const current = () =>
      mounted.current &&
      latest.current.options.enabled &&
      latest.current.options.authority() === authority;
    if (!current()) return;
    const kind = finishNotice({
      status: run.status,
      enabled: options.enabled,
      focused: document.hasFocus(),
      // A page rendered without the console's frame is all there is.
      onAssistantPage: !shell || shell.active === "assistant",
      openSessionId: options.openSessionId,
      sessionId: run.sessionId,
    });
    if (!kind) return;
    const title = options.titleOf(run.sessionId) || t("assistant.untitled");
    const body = t(
      run.status === "error"
        ? "assistant.finishNotice.failed"
        : "assistant.finishNotice.completed",
    );
    if (kind === "toast") {
      toast(title, {
        description: body,
        action: {
          label: t("assistant.finishNotice.view"),
          onClick: () => {
            if (!current()) return;
            latest.current.shell?.setActive("assistant");
            latest.current.options.open(run.sessionId);
          },
        },
      });
      return;
    }
    permission.current ??= notificationsAllowed();
    void permission.current.then((allowed) => {
      if (!allowed || !current()) return;
      try {
        sendNotification({ title, body });
      } catch (error) {
        console.warn("[Assistant] the notification could not be shown", error);
      }
    });
  }, []);
}
