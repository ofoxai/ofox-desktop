import { useState, useEffect, useRef, useCallback } from "react";
import { CheckCircle2, KeyRound, RotateCcw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { isMac } from "@/lib/platform";
import { toast } from "sonner";
import { settingsApi } from "@/lib/api";
import {
  ofoxStartLogin,
  ofoxPollForToken,
  ofoxGetAuthStatus,
  ofoxRetryKeychain,
  type OfoxDeviceCodeResponse,
  type OfoxUserInfo,
} from "@/lib/api/ofoxAuth";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { ofoxPrivacyUrl, ofoxRegisterUrl, ofoxTermsUrl } from "@/lib/ofoxUrls";

type LoginState = "idle" | "polling" | "success" | "error";

interface LoginPageProps {
  onLoginSuccess: (user: OfoxUserInfo) => void;
}

export default function LoginPage({ onLoginSuccess }: LoginPageProps) {
  const { t } = useTranslation();
  const FEATURES = [
    t("uiCopy.featureDetect"),
    t("uiCopy.featureModels"),
    t("uiCopy.featureUsage"),
  ];
  // 上次读取钥匙串被用户拒绝：登录其实还在，重新授权就能恢复，不必重新登录。
  const [keychainDenied, setKeychainDenied] = useState(false);
  const [keychainRetrying, setKeychainRetrying] = useState(false);
  useEffect(() => {
    ofoxGetAuthStatus()
      .then((status) => setKeychainDenied(Boolean(status.keychain_denied)))
      .catch(() => undefined);
  }, []);
  const retryKeychain = async () => {
    setKeychainRetrying(true);
    try {
      const status = await ofoxRetryKeychain();
      if (status.state === "active" && status.user) {
        onLoginSuccess(status.user);
        return;
      }
      setKeychainDenied(Boolean(status.keychain_denied));
      toast.error(t("keychainDenied.stillDenied"));
    } catch (e) {
      toast.error(String(e));
    } finally {
      setKeychainRetrying(false);
    }
  };
  const { apex } = useOfoxApex();
  const [loginState, setLoginState] = useState<LoginState>("idle");
  const [deviceCode, setDeviceCode] = useState<OfoxDeviceCodeResponse | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intervalRef = useRef(5);
  // Counts consecutive *unknown* errors during polling (i.e. errors not
  // matching the three documented OAuth codes). Three in a row trips us
  // out of the silent-spin trap and tells the user something's wrong.
  const unknownErrorCountRef = useRef(0);
  // Tracks whether the "我已完成授权" check is in flight, so we can
  // disable that one button without affecting the others.
  const [checkingStatus, setCheckingStatus] = useState(false);
  // Whether to surface the "重新生成验证码" escape hatch. We hide it by
  // default to keep the polling card uncluttered — a user actively waiting
  // for the browser shouldn't be tempted to invalidate their fresh code.
  // Only after the user clicks "我已完成授权" and the check fails do we
  // reveal it: at that point regenerating is the right next move because
  // either the device code is genuinely consumed/expired, or something
  // upstream is wrong and a fresh code is the cheapest reset.
  const [showRegenerate, setShowRegenerate] = useState(false);

  const stopPolling = useCallback(() => {
    if (pollingRef.current) {
      clearInterval(pollingRef.current);
      pollingRef.current = null;
    }
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  }, []);

  useEffect(() => {
    return () => stopPolling();
  }, [stopPolling]);

  const startLogin = async () => {
    setError(null);
    setLoginState("idle");
    stopPolling();
    unknownErrorCountRef.current = 0;
    setShowRegenerate(false);

    try {
      const resp = await ofoxStartLogin();
      setDeviceCode(resp);
      setLoginState("polling");
      intervalRef.current = Math.max((resp.interval || 5) + 3, 8);

      // Copy user_code to clipboard
      try {
        await navigator.clipboard.writeText(resp.user_code);
        setCopied(true);
        setTimeout(() => setCopied(false), 3000);
      } catch {
        // ignore clipboard error
      }

      // Open verification URL
      const url = resp.verification_uri_complete || resp.verification_uri;
      await settingsApi.openExternal(url);

      // Start polling
      const expiresAt = Date.now() + resp.expires_in * 1000;

      pollingRef.current = setInterval(async () => {
        if (Date.now() > expiresAt) {
          stopPolling();
          setLoginState("error");
          setError(t("uiCopy.codeExpired"));
          return;
        }

        try {
          const user = await ofoxPollForToken(resp.device_code);
          if (user) {
            stopPolling();
            setLoginState("success");
            onLoginSuccess(user);
          }
          // null = still pending, keep polling
          unknownErrorCountRef.current = 0;
        } catch (e) {
          const msg = String(e);
          if (msg.includes("slow_down")) {
            intervalRef.current += 5;
            // Restart with new interval
            stopPolling();
            pollingRef.current = setInterval(async () => {
              try {
                const user = await ofoxPollForToken(resp.device_code);
                if (user) {
                  stopPolling();
                  setLoginState("success");
                  onLoginSuccess(user);
                }
              } catch {
                // ignore poll errors during retry
              }
            }, intervalRef.current * 1000);
          } else if (
            msg.includes("access_denied") ||
            msg.includes("expired_token")
          ) {
            stopPolling();
            setLoginState("error");
            setError(
              msg.includes("access_denied")
                ? t("uiCopy.authDenied")
                : t("uiCopy.codeExpired"),
            );
          } else {
            // Anything we don't recognize — bad network, OFox backend
            // unreachable, malformed upstream response, an `invalid_grant`
            // because the device_code was already consumed by a successful
            // (but not-fully-propagated) poll, etc. Pre-fix this branch
            // was a no-op, so the user spun forever with no feedback.
            // Log + count; after 3 consecutive unknown failures, assume
            // we're stuck and surface an error so the rescue buttons
            // (regenerate code / "我已完成授权") become the obvious path.
            unknownErrorCountRef.current += 1;
            console.warn(
              `[LoginPage] poll error (${unknownErrorCountRef.current}/3):`,
              msg,
            );
            if (unknownErrorCountRef.current >= 3) {
              stopPolling();
              setLoginState("error");
              setError(t("uiCopy.authError"));
            }
          }
        }
      }, intervalRef.current * 1000);

      // Auto-expire timeout
      timeoutRef.current = setTimeout(() => {
        stopPolling();
        if (loginState === "polling") {
          setLoginState("error");
          setError(t("uiCopy.codeExpired"));
        }
      }, resp.expires_in * 1000);
    } catch (e) {
      setLoginState("error");
      setError(String(e));
    }
  };

  const copyUserCode = async () => {
    if (!deviceCode) return;
    try {
      await navigator.clipboard.writeText(deviceCode.user_code);
      setCopied(true);
      setTimeout(() => setCopied(false), 3000);
    } catch {
      // ignore
    }
  };

  /**
   * Manual rescue — "我已完成授权".
   *
   * Most useful when the silent-spin trap hit before the 3-strike threshold:
   * the backend may already be `Active` (tokens on disk, set_state ran)
   * but the poll loop hasn't seen a successful return because of the
   * `/openapi/me` failure path (now fixed in `ofox_auth.rs`, but kept as
   * a belt-and-suspenders here for older deployments and for genuine
   * "user gave up before the next poll fired" scenarios).
   *
   * Reads `ofoxGetAuthStatus()` directly. If active, we synthesize a
   * minimal `OfoxUserInfo` from whatever the backend already cached and
   * call `onLoginSuccess`, mirroring what the polling success path does.
   */
  const handleAlreadyAuthorized = useCallback(async () => {
    if (checkingStatus) return;
    setCheckingStatus(true);
    try {
      const status = await ofoxGetAuthStatus();
      if (status.state === "active") {
        stopPolling();
        setLoginState("success");
        onLoginSuccess(
          status.user ?? {
            email: null,
            name: null,
            org_id: null,
            avatar_url: null,
          },
        );
      } else {
        // Failed: surface the regenerate hatch now. The user has explicitly
        // told us "I'm done in the browser" — if the backend disagrees, the
        // device code is most likely stale, and continuing to wait silently
        // is the worst option. A fresh code is one click away.
        setShowRegenerate(true);
        toast.info(t("uiCopy.authPending"));
      }
    } catch (e) {
      console.error("[LoginPage] handleAlreadyAuthorized failed", e);
      // Network/IPC failure — same UX: offer the regenerate path so the
      // user isn't stuck if the backend is temporarily unreachable.
      setShowRegenerate(true);
      toast.error(t("uiCopy.authCheckFailed"));
    } finally {
      setCheckingStatus(false);
    }
  }, [checkingStatus, onLoginSuccess, stopPolling]);

  /**
   * Manual rescue — "重新生成验证码".
   *
   * Hard reset of the device-code flow. Re-runs `startLogin()` which:
   *   1. clears polling timers,
   *   2. resets `unknownErrorCountRef`,
   *   3. asks the backend for a fresh device code (the previous one is
   *      orphaned server-side; OFox lets it expire naturally),
   *   4. opens the browser tab again and starts polling from scratch.
   */
  const handleRegenerate = useCallback(async () => {
    await startLogin();
  }, []);

  return (
    <div className="relative flex h-screen w-full flex-col bg-canvas">
      {/* Title bar drag region — same 40px strip ConsolePage uses, so the
          window stays movable while the user is on this page. Without this
          there is literally no draggable surface on the LoginPage and the
          window gets stuck wherever it first opened. Windows and Linux have
          a native title bar to drag instead. */}
      {isMac() && (
        <div className="h-10 shrink-0" data-tauri-drag-region="true" />
      )}

      {/* Apex 切换器在 onboarding 阶段不显示：未授权用户从 OFox 官网下载客户端时
          已经决定了走 .io / .ai，登录前再暴露选择只会把 device flow 引到错误的
          IDP。需要切换的用户登录后可以在设置弹窗里改。 */}

      <div className="flex min-h-0 w-full flex-1 overflow-y-auto">
        <div className="m-auto flex w-full max-w-xl flex-col items-center px-8 py-2">
          {keychainDenied && (
            <div
              role="alert"
              className="mb-6 flex w-full items-start gap-3 rounded-xl border border-brand/30 bg-warn-soft p-4 text-sm "
            >
              <KeyRound className="mt-0.5 h-4 w-4 shrink-0 text-brand" />
              <div className="flex-1 space-y-3">
                <p className="leading-relaxed">{t("keychainDenied.message")}</p>
                <button
                  type="button"
                  onClick={() => void retryKeychain()}
                  disabled={keychainRetrying}
                  className="rounded-md bg-brand px-3 py-1.5 text-xs font-medium text-brand-on hover:bg-brand disabled:opacity-60"
                >
                  {t(
                    keychainRetrying
                      ? "keychainDenied.retrying"
                      : "keychainDenied.retry",
                  )}
                </button>
              </div>
            </div>
          )}
          {/* Logo */}
          <div className="mb-6 flex h-20 w-20 items-center justify-center rounded-[22px] bg-brand text-4xl font-bold text-brand-on shadow-lg ">
            O
          </div>

          {/* Title */}
          <h1 className="mb-2 text-3xl font-bold text-ink">
            {t("uiCopy.welcome")}
          </h1>
          <p className="mb-8 text-center text-sm leading-relaxed text-ink-muted">
            {t("uiCopy.welcomeLead")}
            <br />
            {t("uiCopy.welcomeHint")}
          </p>

          {/* Features (only show when idle) */}
          {loginState === "idle" && (
            <div className="mb-8 grid w-full grid-cols-2 gap-x-6 gap-y-3">
              {FEATURES.map((feature) => (
                <div key={feature} className="flex items-start gap-2">
                  <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-green-500" />
                  <span className="text-[13px] leading-snug text-ink/80">
                    {feature}
                  </span>
                </div>
              ))}
            </div>
          )}

          {/* Device Code Display (polling state) */}
          {loginState === "polling" && deviceCode && (
            <div className="mb-8 flex flex-col items-center gap-3">
              <p className="text-sm text-ink-muted">{t("uiCopy.enterCode")}</p>
              <button
                onClick={copyUserCode}
                className="rounded-xl border-2 border-dashed border-brand/30 bg-warn-soft px-8 py-4 text-3xl font-mono font-bold tracking-[0.3em] text-brand transition-colors hover:border-brand/30 hover:bg-warn-soft "
              >
                {deviceCode.user_code}
              </button>
              <p className="text-xs text-ink-muted">
                {copied ? t("uiCopy.copied") : t("uiCopy.clickCopy")}
              </p>
              <div className="flex items-center gap-2 text-sm text-ink-muted">
                <div className="h-4 w-4 animate-spin rounded-full border-2 border-brand/30 border-t-transparent" />
                {t("uiCopy.waitingAuth")}
              </div>

              {/* Rescue actions — visible only while polling so users have an
               *  escape hatch when something goes wrong silently (e.g. the
               *  /openapi/me failure used to leave them stuck forever).
               *  Default visible: only "我已完成授权" — keeps the card calm
               *  and avoids tempting users to invalidate a perfectly good
               *  device code. The "重新生成" button only appears AFTER the
               *  user clicks "我已完成授权" and the check fails — at that
               *  point regenerating is the obvious next move. */}
              <div className="mt-2 flex items-center gap-2 text-[12px]">
                <button
                  type="button"
                  onClick={handleAlreadyAuthorized}
                  disabled={checkingStatus}
                  className="rounded-md border border-line bg-background px-3 py-1.5 font-medium text-ink transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {checkingStatus
                    ? t("uiCopy.checkingAuth")
                    : t("uiCopy.authorized")}
                </button>
                {showRegenerate && (
                  <button
                    type="button"
                    onClick={handleRegenerate}
                    className="inline-flex items-center gap-1 rounded-md border border-line bg-background px-3 py-1.5 font-medium text-ink transition-colors hover:bg-accent"
                  >
                    <RotateCcw className="h-3 w-3" />
                    {t("uiCopy.regenerate")}
                  </button>
                )}
              </div>
            </div>
          )}

          {/* Error */}
          {error && (
            <div className="mb-4 w-full rounded-lg bg-danger-soft px-4 py-2 text-sm text-danger ">
              {error}
            </div>
          )}

          {/* Primary CTA */}
          <button
            onClick={startLogin}
            disabled={loginState === "polling"}
            className="mb-4 w-full max-w-sm rounded-xl bg-brand px-6 py-3 text-base font-semibold text-brand-on shadow-md transition-all hover:shadow-lg disabled:opacity-50 "
          >
            {loginState === "polling"
              ? t("uiCopy.waitingBrowser")
              : loginState === "error"
                ? t("uiCopy.reauthArrow")
                : t("uiCopy.loginArrow")}
          </button>

          {/* Secondary Actions */}
          <div className="mb-6 flex items-center gap-3 text-[13px]">
            <span className="text-ink-muted">{t("uiCopy.notRegistered")}</span>
            <button
              onClick={() => settingsApi.openExternal(ofoxRegisterUrl(apex))}
              className="text-brand hover:text-brand hover:underline"
            >
              {t("uiCopy.register")}
            </button>
          </div>

          {/* Footer */}
          <p className="text-[11px] text-ink-muted/60">
            {t("uiCopy.agreement")}{" "}
            <button
              onClick={() => settingsApi.openExternal(ofoxTermsUrl(apex))}
              className="underline hover:text-ink-muted"
            >
              {t("uiCopy.terms")}
            </button>{" "}
            {t("uiCopy.and")}{" "}
            <button
              onClick={() => settingsApi.openExternal(ofoxPrivacyUrl(apex))}
              className="underline hover:text-ink-muted"
            >
              {t("uiCopy.privacy")}
            </button>
          </p>
        </div>
      </div>
    </div>
  );
}
