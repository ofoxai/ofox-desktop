import { useState, useEffect, useRef, useCallback } from "react";
import { CheckCircle2 } from "lucide-react";
import { settingsApi } from "@/lib/api";
import {
  ofoxStartLogin,
  ofoxPollForToken,
  type OfoxDeviceCodeResponse,
  type OfoxUserInfo,
} from "@/lib/api/ofoxAuth";

const FEATURES = [
  "自动检测本机 AI 工具，一键接入",
  "Claude / GPT / Gemini 等 100+ 模型",
  "统一用量与账单，菜单栏随时查看",
  "支持第三方 API 并存，高级设置可调",
];

type LoginState = "idle" | "polling" | "success" | "error";

interface LoginPageProps {
  onLoginSuccess: (user: OfoxUserInfo) => void;
}

export default function LoginPage({ onLoginSuccess }: LoginPageProps) {
  const [loginState, setLoginState] = useState<LoginState>("idle");
  const [deviceCode, setDeviceCode] =
    useState<OfoxDeviceCodeResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intervalRef = useRef(5);

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
          setError("验证码已过期，请重试");
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
                ? "授权被拒绝"
                : "验证码已过期，请重试",
            );
          }
        }
      }, intervalRef.current * 1000);

      // Auto-expire timeout
      timeoutRef.current = setTimeout(() => {
        stopPolling();
        if (loginState === "polling") {
          setLoginState("error");
          setError("验证码已过期，请重试");
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

  return (
    <div className="flex h-screen w-full items-center justify-center bg-gradient-to-br from-orange-50/80 via-white to-orange-50/40 dark:from-neutral-950 dark:via-neutral-900 dark:to-neutral-950">
      <div className="flex w-full max-w-xl flex-col items-center px-8">
        {/* Logo */}
        <div className="mb-6 flex h-20 w-20 items-center justify-center rounded-[22px] bg-gradient-to-br from-orange-400 to-orange-600 text-4xl font-bold text-white shadow-lg shadow-orange-200 dark:shadow-orange-900/30">
          O
        </div>

        {/* Title */}
        <h1 className="mb-2 text-3xl font-bold text-foreground">
          欢迎使用 Ofox
        </h1>
        <p className="mb-8 text-center text-sm leading-relaxed text-muted-foreground">
          为本机所有 AI 编程工具接入 Ofox 统一 API。
          <br />
          告别配置文件、API Key 和多账单。
        </p>

        {/* Features (only show when idle) */}
        {loginState === "idle" && (
          <div className="mb-8 grid w-full grid-cols-2 gap-x-6 gap-y-3">
            {FEATURES.map((feature) => (
              <div key={feature} className="flex items-start gap-2">
                <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-green-500" />
                <span className="text-[13px] leading-snug text-foreground/80">
                  {feature}
                </span>
              </div>
            ))}
          </div>
        )}

        {/* Device Code Display (polling state) */}
        {loginState === "polling" && deviceCode && (
          <div className="mb-8 flex flex-col items-center gap-3">
            <p className="text-sm text-muted-foreground">
              请在浏览器中输入以下验证码
            </p>
            <button
              onClick={copyUserCode}
              className="rounded-xl border-2 border-dashed border-orange-300 bg-orange-50 px-8 py-4 text-3xl font-mono font-bold tracking-[0.3em] text-orange-600 transition-colors hover:border-orange-400 hover:bg-orange-100 dark:border-orange-500/30 dark:bg-orange-950/30 dark:text-orange-400"
            >
              {deviceCode.user_code}
            </button>
            <p className="text-xs text-muted-foreground">
              {copied ? "已复制到剪贴板 ✓" : "点击复制"}
            </p>
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <div className="h-4 w-4 animate-spin rounded-full border-2 border-orange-400 border-t-transparent" />
              等待授权中...
            </div>
          </div>
        )}

        {/* Error */}
        {error && (
          <div className="mb-4 w-full rounded-lg bg-red-50 px-4 py-2 text-sm text-red-600 dark:bg-red-950/50 dark:text-red-400">
            {error}
          </div>
        )}

        {/* Primary CTA */}
        <button
          onClick={startLogin}
          disabled={loginState === "polling"}
          className="mb-4 w-full max-w-sm rounded-xl bg-gradient-to-r from-orange-400 to-orange-500 px-6 py-3 text-base font-semibold text-white shadow-md shadow-orange-200 transition-all hover:from-orange-500 hover:to-orange-600 hover:shadow-lg disabled:opacity-50 dark:shadow-orange-900/20"
        >
          {loginState === "polling"
            ? "等待浏览器授权..."
            : loginState === "error"
              ? "重新登录 →"
              : "使用 Ofox 账户登录 →"}
        </button>

        {/* Secondary Actions */}
        <div className="mb-6 flex items-center gap-3 text-[13px]">
          <span className="text-muted-foreground">未注册？</span>
          <button
            onClick={() =>
              settingsApi.openExternal("https://app.ofox.ai/register")
            }
            className="text-orange-500 hover:text-orange-600 hover:underline"
          >
            免费注册
          </button>
        </div>

        {/* Footer */}
        <p className="text-[11px] text-muted-foreground/60">
          继续登录即表示同意{" "}
          <button
            onClick={() =>
              settingsApi.openExternal("https://ofox.ai/terms")
            }
            className="underline hover:text-muted-foreground"
          >
            服务条款
          </button>{" "}
          与{" "}
          <button
            onClick={() =>
              settingsApi.openExternal("https://ofox.ai/privacy")
            }
            className="underline hover:text-muted-foreground"
          >
            隐私政策
          </button>
        </p>
      </div>
    </div>
  );
}
