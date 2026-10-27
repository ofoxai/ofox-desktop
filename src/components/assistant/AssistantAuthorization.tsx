import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ofoxStartLogin,
  ofoxPollForToken,
  ofoxCancelLogin,
  type OfoxDeviceCodeResponse,
} from "@/lib/api/ofoxAuth";
import { settingsApi } from "@/lib/api/settings";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";

/** Independent grant flow. Never calls the global sign-out/onboarding handler. */
export function AssistantAuthorization({
  onAuthorized,
}: {
  onAuthorized: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [code, setCode] = useState<OfoxDeviceCodeResponse | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!open) return;
    let current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let device: string | undefined;
    setCode(null);
    setError("");
    void (async () => {
      try {
        const result = await ofoxStartLogin(true);
        device = result.device_code;
        if (!current) {
          await ofoxCancelLogin(device);
          return;
        }
        setCode(result);
        const expires = Date.now() + Math.min(result.expires_in, 86_400) * 1000;
        let interval = Math.max(result.interval, 5) * 1000;
        const poll = async () => {
          if (!current) return;
          try {
            if (Date.now() >= expires) throw new Error("expired_token");
            const user = await ofoxPollForToken(result.device_code);
            if (!current) return;
            if (user) {
              setOpen(false);
              await onAuthorized();
              return;
            }
          } catch (reason) {
            if (!current) return;
            const message =
              reason instanceof Error ? reason.message : String(reason);
            if (message === "slow_down") interval += 5000;
            else {
              setError(message);
              return;
            }
          }
          timer = setTimeout(() => void poll(), interval);
        };
        timer = setTimeout(() => void poll(), interval);
      } catch (reason) {
        if (current)
          setError(reason instanceof Error ? reason.message : String(reason));
      }
    })();
    return () => {
      current = false;
      clearTimeout(timer);
      if (device) void ofoxCancelLogin(device).catch(() => undefined);
    };
  }, [open, attempt, onAuthorized]);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="secondary">
          {t("assistant.authorization.title")}
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("assistant.authorization.title")}</DialogTitle>
          <DialogDescription>
            {t("assistant.authorization.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4 px-[18px] py-4">
          {code && (
            <>
              <p className="select-all text-center font-mono text-xl">
                {code.user_code}
              </p>
              <Button
                onClick={() =>
                  void settingsApi
                    .openExternal(
                      code.verification_uri_complete || code.verification_uri,
                    )
                    .catch(() => setError("open_failed"))
                }
              >
                {t("assistant.authorization.open")}
              </Button>
            </>
          )}
          {error ? (
            <>
              <p role="alert" className="text-ui text-danger">
                {t("assistant.authorization.failed")}{" "}
                {t(error, { defaultValue: "" })}
              </p>
              <Button
                variant="secondary"
                onClick={() => setAttempt((value) => value + 1)}
              >
                {t("assistant.retry")}
              </Button>
            </>
          ) : (
            <p role="status" className="text-note text-ink-muted">
              {t("assistant.authorization.waiting")}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            {t("common.cancel")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
