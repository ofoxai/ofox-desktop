import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  getSettingsHealth,
  resetUnreadableSettings,
  SETTINGS_HEALTH_EVENT,
  type SettingsHealth,
} from "@/lib/api/settings";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export function SettingsHealthBanner() {
  const { t } = useTranslation();
  const [health, setHealth] = useState<SettingsHealth | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const resetting = useRef(false);
  useEffect(() => {
    let active = true;
    let revision = 0;
    let off: (() => void) | undefined;
    // Subscribe first, so a recovery event cannot be lost during the initial read.
    void listen<SettingsHealth>(SETTINGS_HEALTH_EVENT, (event) => {
      revision += 1;
      if (active) {
        setHealth(event.payload);
        setDismissed(false);
      }
    })
      .then((stop) => {
        if (!active) {
          stop();
          return;
        }
        off = stop;
        const started = revision;
        void getSettingsHealth()
          .then((value) => {
            if (active && revision === started) setHealth(value);
          })
          .catch((cause) => {
            if (active) setError(String(cause));
          });
      })
      .catch((cause) => {
        if (active) setError(String(cause));
      });
    return () => {
      active = false;
      off?.();
    };
  }, []);
  const reset = async () => {
    if (resetting.current) return;
    resetting.current = true;
    setBusy(true);
    setError(null);
    try {
      setHealth(await resetUnreadableSettings());
      setConfirming(false);
    } catch (cause) {
      setError(String(cause));
    } finally {
      resetting.current = false;
      setBusy(false);
    }
  };
  if ((!health || health.state === "ok") && !error) return null;
  if (health?.state === "recovered" && dismissed && !error) return null;
  return (
    <>
      <div
        role="alert"
        className={cn(
          "mb-3 space-y-2 rounded-card border px-3 py-2 text-ctl",
          health?.state === "unreadable"
            ? "border-danger/25 bg-danger-soft text-danger"
            : "border-warn/25 bg-warn-soft text-warn",
        )}
      >
        {health && health.state !== "ok" && (
          <p className="font-medium">{t(`settingsHealth.${health.state}`)}</p>
        )}
        {health?.badCopy && (
          <p className="break-all">
            {t("settingsHealth.badCopy", { path: health.badCopy })}
          </p>
        )}
        {health?.detail && <p className="break-words">{health.detail}</p>}
        {error && <p className="break-words">{error}</p>}
        {health?.state === "unreadable" && (
          <Button variant="secondary" onClick={() => setConfirming(true)}>
            {t("settingsHealth.reset")}
          </Button>
        )}
        {health?.state === "recovered" && (
          <Button variant="ghost" onClick={() => setDismissed(true)}>
            {t("common.close")}
          </Button>
        )}
      </div>
      <ConfirmDialog
        isOpen={confirming}
        title={t("settingsHealth.confirmTitle")}
        message={t("settingsHealth.confirmMessage")}
        confirmText={t("settingsHealth.confirm")}
        busy={busy}
        onConfirm={() => void reset()}
        onCancel={() => setConfirming(false)}
      />
    </>
  );
}
