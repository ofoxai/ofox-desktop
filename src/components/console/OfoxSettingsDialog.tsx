import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { Loader2, X } from "lucide-react";
import { emit } from "@tauri-apps/api/event";

import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { settingsApi } from "@/lib/api";
import { ofoxLogout } from "@/lib/api/ofoxAuth";
import { useOfoxAuth } from "@/hooks/useOfoxAuth";
import { getCurrentVersion } from "@/lib/updater";

/**
 * OFox-flavored settings dialog launched from the bottom-left "设置" button on
 * `ConsolePage`. The legacy cc-switch settings page (`SettingsPage.tsx`) still
 * exists and is reachable from the menu bar — this dialog deliberately exposes
 * a much narrower surface scoped to what an OFox end user actually needs:
 *
 *   · Account     — show identity, allow logout
 *   · Preferences — auto-launch, low-balance threshold, tool health-check freq
 *   · About       — app version + check-for-updates entry point
 *
 * Things deliberately omitted:
 *   · "在浏览器中打开设置" — we don't have an external dashboard for these
 *     prefs yet, and the link in the design mock would otherwise mislead.
 *   · "只显示菜单栏图标 / Dock 图标" — explicitly crossed out in the spec.
 *   · "高级 · 自定义供应商" — design left it cut off; landing in a follow-up.
 *   · "原始配置备份" — deferred until OFox owns its own ~/.ofox/backups/.
 *
 * The "检查更新" button intentionally does NOT call cc-switch's existing
 * tauri-plugin-updater flow — per spec we'll wire a separate OFox-native
 * scheme later. Today it just opens GitHub Releases via openExternal so the
 * button has a reasonable fallback action instead of being inert.
 */

// ─── Preference storage ──────────────────────────────────────────────────
//
// Two new prefs (low-balance threshold + health-check interval) live in
// localStorage with the `ofox:` prefix. Backend wiring is intentionally
// deferred — the toggles surface the user's intent today; the consumers
// (balance banner, tool health probe) will read the same keys when they
// land. Default values picked to match the design mock.

const PREF_LOW_BAL_ENABLED = "ofox:settings:lowBalanceEnabled";
const PREF_LOW_BAL_THRESHOLD = "ofox:settings:lowBalanceThreshold";
const PREF_HEALTH_INTERVAL = "ofox:settings:healthCheckInterval";

type HealthInterval = "off" | "1h" | "6h" | "24h";

const HEALTH_INTERVAL_LABELS: Record<HealthInterval, string> = {
  off: "关闭",
  "1h": "每 1 小时",
  "6h": "每 6 小时",
  "24h": "每 24 小时",
};

function readNumberPref(key: string, fallback: number): number {
  const raw = localStorage.getItem(key);
  if (!raw) return fallback;
  const n = Number(raw);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

function readBoolPref(key: string, fallback: boolean): boolean {
  const raw = localStorage.getItem(key);
  if (raw === null) return fallback;
  return raw === "1" || raw === "true";
}

function readHealthInterval(fallback: HealthInterval): HealthInterval {
  const raw = localStorage.getItem(PREF_HEALTH_INTERVAL);
  if (raw === "off" || raw === "1h" || raw === "6h" || raw === "24h") {
    return raw;
  }
  return fallback;
}

// ─── Component ───────────────────────────────────────────────────────────

interface OfoxSettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export default function OfoxSettingsDialog({
  open,
  onOpenChange,
}: OfoxSettingsDialogProps) {
  const { status, refetch } = useOfoxAuth();
  const user = status?.user ?? null;

  // ── Auto-launch (sourced from backend so the toggle reflects reality even
  // if it was changed elsewhere — e.g. macOS System Settings). Reads on open;
  // writes optimistically with a rollback on backend failure.
  const [autoLaunch, setAutoLaunch] = useState<boolean | null>(null);

  // ── Preferences (localStorage-backed, see above)
  const [lowBalEnabled, setLowBalEnabled] = useState(() =>
    readBoolPref(PREF_LOW_BAL_ENABLED, true),
  );
  const [lowBalThreshold, setLowBalThreshold] = useState<number>(() =>
    readNumberPref(PREF_LOW_BAL_THRESHOLD, 10),
  );
  const [healthInterval, setHealthInterval] = useState<HealthInterval>(() =>
    readHealthInterval("6h"),
  );

  // ── Logout / version state
  const [loggingOut, setLoggingOut] = useState(false);
  const [version, setVersion] = useState<string | null>(null);

  // Re-read on every open: auto-launch can drift if changed outside the app,
  // and the auth status may have refreshed since dialog was last opened.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    (async () => {
      try {
        const enabled = await settingsApi.getAutoLaunchStatus();
        if (!cancelled) setAutoLaunch(enabled);
      } catch (e) {
        console.error("[OfoxSettingsDialog] getAutoLaunchStatus failed", e);
        if (!cancelled) setAutoLaunch(false);
      }
      try {
        const v = await getCurrentVersion();
        if (!cancelled) setVersion(v);
      } catch (e) {
        console.error("[OfoxSettingsDialog] getCurrentVersion failed", e);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open]);

  // ── Handlers ───────────────────────────────────────────────────────────

  const handleAutoLaunchToggle = useCallback(
    async (next: boolean) => {
      const prev = autoLaunch;
      setAutoLaunch(next); // optimistic
      try {
        await settingsApi.setAutoLaunch(next);
      } catch (e) {
        console.error("[OfoxSettingsDialog] setAutoLaunch failed", e);
        setAutoLaunch(prev ?? false);
        toast.error("开机自动启动设置失败，请稍后再试");
      }
    },
    [autoLaunch],
  );

  const handleLowBalToggle = useCallback((next: boolean) => {
    setLowBalEnabled(next);
    localStorage.setItem(PREF_LOW_BAL_ENABLED, next ? "1" : "0");
  }, []);

  const handleLowBalThresholdChange = useCallback((raw: string) => {
    // Strip non-digits — the input also has type="number" but Safari accepts
    // letters anyway. Empty input collapses to 0 visually but we don't
    // persist 0 (would silence the alert entirely without the toggle).
    const cleaned = raw.replace(/[^\d]/g, "");
    const n = cleaned === "" ? 0 : Number(cleaned);
    setLowBalThreshold(n);
    if (n > 0) {
      localStorage.setItem(PREF_LOW_BAL_THRESHOLD, String(n));
    }
  }, []);

  const handleHealthIntervalChange = useCallback((next: string) => {
    if (next === "off" || next === "1h" || next === "6h" || next === "24h") {
      setHealthInterval(next);
      localStorage.setItem(PREF_HEALTH_INTERVAL, next);
    }
  }, []);

  const handleLogout = useCallback(async () => {
    if (loggingOut) return;
    setLoggingOut(true);
    try {
      await ofoxLogout();
      await refetch();
      toast.success("已退出登录");
      // Close the settings dialog first so the LoginPage isn't fighting the
      // dialog overlay for focus when the route flips.
      onOpenChange(false);
      // The backend's `ofox_logout` only mutates auth state — it does NOT
      // emit `ofox-auth-expired` (that event is reserved for refresh-token
      // failures). MainApp keys the Console↔Onboarding swap off auth events,
      // so without a nudge here the user would stay on Console with stale
      // user info until next launch. Reuse the same event the tray's "重新
      // 登录" button uses (`ofox-reauth-requested`) — MainApp already
      // listens for it and routes to the LoginPage.
      await emit("ofox-reauth-requested");
    } catch (e) {
      console.error("[OfoxSettingsDialog] ofoxLogout failed", e);
      toast.error("退出登录失败，请稍后重试");
    } finally {
      setLoggingOut(false);
    }
  }, [loggingOut, refetch, onOpenChange]);

  const handleCheckUpdate = useCallback(async () => {
    // Per spec: don't reuse cc-switch's tauri-plugin-updater here — the OFox
    // update channel will land later. As a no-op-friendly placeholder we open
    // the public releases page so the button has a meaningful action today.
    try {
      await settingsApi.openExternal(
        "https://github.com/farion1231/ofox-switch/releases/latest",
      );
    } catch (e) {
      console.error("[OfoxSettingsDialog] openExternal failed", e);
      toast.error("无法打开更新页面");
    }
  }, []);

  // ── Render ─────────────────────────────────────────────────────────────

  const userInitial = (user?.name || user?.email || "U")
    .charAt(0)
    .toUpperCase();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="max-w-[560px] gap-0 overflow-hidden rounded-2xl p-0"
        zIndex="nested"
      >
        {/* Header — custom layout (no bottom border, larger padding) so it
            matches the floating-card feel of the spec instead of the standard
            DialogHeader divider. */}
        <div className="flex items-center justify-between px-6 pt-5 pb-3">
          <DialogTitle className="text-[18px] font-semibold tracking-tight">
            设置
          </DialogTitle>
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            aria-label="关闭"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Scrollable body — capped at 70vh so the footer stays visible on
            short windows; sections separate via gap-4 to mirror the mock. */}
        <div className="flex max-h-[70vh] flex-col gap-4 overflow-y-auto px-6 pb-4">
          {/* ─── 账户 ─────────────────────────────────────────────────── */}
          <SectionCard title="账户">
            <div className="flex items-center justify-between px-4 py-3">
              <div className="flex min-w-0 items-center gap-3">
                <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-purple-200 text-base font-bold text-purple-700">
                  {userInitial}
                </div>
                <div className="min-w-0">
                  <div className="truncate text-[14px] font-semibold text-foreground">
                    {user?.name ?? "未登录"}
                  </div>
                  <div className="truncate text-[12px] text-muted-foreground">
                    {user?.email ?? ""}
                  </div>
                </div>
              </div>
              <button
                type="button"
                onClick={handleLogout}
                disabled={loggingOut || !user}
                className="inline-flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-[12px] font-medium text-foreground transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
              >
                {loggingOut && <Loader2 className="h-3 w-3 animate-spin" />}
                退出登录
              </button>
            </div>
          </SectionCard>

          {/* ─── 偏好 ─────────────────────────────────────────────────── */}
          <SectionCard title="偏好">
            {/* Auto-launch */}
            <Row
              label="开机自动启动"
              control={
                <OrangeSwitch
                  checked={autoLaunch === true}
                  // While the initial fetch is in-flight we still show the
                  // switch (defaulted off) but disable it — flipping it now
                  // would race the read-back.
                  disabled={autoLaunch === null}
                  onCheckedChange={handleAutoLaunchToggle}
                />
              }
            />

            {/* Low-balance alert */}
            <Row
              label="低余额提醒"
              hint="余额低于阈值时通知"
              control={
                <div className="flex items-center gap-2">
                  <div className="relative">
                    <span className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-[13px] text-muted-foreground">
                      $
                    </span>
                    <input
                      type="number"
                      inputMode="numeric"
                      min={1}
                      value={lowBalThreshold || ""}
                      onChange={(e) =>
                        handleLowBalThresholdChange(e.target.value)
                      }
                      disabled={!lowBalEnabled}
                      className="h-7 w-[72px] rounded-md border border-border bg-background pl-5 pr-2 text-right text-[13px] font-medium text-foreground outline-none transition-colors focus:border-orange-400 disabled:cursor-not-allowed disabled:opacity-50"
                    />
                  </div>
                  <OrangeSwitch
                    checked={lowBalEnabled}
                    onCheckedChange={handleLowBalToggle}
                  />
                </div>
              }
            />

            {/* Health-check frequency */}
            <Row
              label="工具健康检查"
              hint="定期对每个工具发送测试请求"
              control={
                <Select
                  value={healthInterval}
                  onValueChange={handleHealthIntervalChange}
                >
                  <SelectTrigger className="h-8 w-[112px] text-[13px]">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {(
                      Object.keys(HEALTH_INTERVAL_LABELS) as HealthInterval[]
                    ).map((k) => (
                      <SelectItem key={k} value={k}>
                        {HEALTH_INTERVAL_LABELS[k]}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              }
              isLast
            />
          </SectionCard>

          {/* ─── 关于 ─────────────────────────────────────────────────── */}
          <SectionCard title="关于">
            <div className="flex items-center justify-between px-4 py-3">
              <div className="min-w-0">
                <div className="text-[14px] font-semibold text-foreground">
                  Ofox Desktop {version ? `v${version}` : ""}
                </div>
                <div className="text-[12px] text-muted-foreground">
                  最新版本
                </div>
              </div>
              <button
                type="button"
                onClick={handleCheckUpdate}
                className="rounded-lg border border-border px-3 py-1.5 text-[12px] font-medium text-foreground transition-colors hover:bg-accent"
              >
                检查更新
              </button>
            </div>
          </SectionCard>
        </div>

        {/* Footer — only the primary "完成" action; the spec's "在浏览器中
            打开设置" link is intentionally omitted (see top-of-file comment). */}
        <div className="flex items-center justify-end border-t border-border-default bg-muted/20 px-6 py-3">
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            className="rounded-lg bg-orange-500 px-5 py-1.5 text-[13px] font-medium text-white transition-colors hover:bg-orange-600"
          >
            完成
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

// ─── Internal layout primitives ────────────────────────────────────────

interface SectionCardProps {
  title: string;
  children: React.ReactNode;
}

/** Grouped card with a muted header strip, mirroring the mock's "账户 / 偏好 /
 *  关于" group containers. */
function SectionCard({ title, children }: SectionCardProps) {
  return (
    <section className="overflow-hidden rounded-xl border border-border bg-background">
      <div className="border-b border-border bg-muted/40 px-4 py-2">
        <h3 className="text-[12px] font-medium uppercase tracking-wider text-muted-foreground">
          {title}
        </h3>
      </div>
      <div className="divide-y divide-border">{children}</div>
    </section>
  );
}

interface RowProps {
  label: string;
  hint?: string;
  control: React.ReactNode;
  /** Suppresses the bottom divider — for the last row in a section. The
   *  parent's `divide-y` already does this for non-last rows, so callers
   *  generally don't need to set it; provided as escape hatch. */
  isLast?: boolean;
}

function Row({ label, hint, control, isLast }: RowProps) {
  // `divide-y` on the parent draws between rows; nothing to do for isLast.
  void isLast;
  return (
    <div className="flex items-center justify-between px-4 py-3">
      <div className="min-w-0">
        <div className="text-[14px] font-medium text-foreground">{label}</div>
        {hint && (
          <div className="text-[12px] text-muted-foreground">{hint}</div>
        )}
      </div>
      <div className="flex shrink-0 items-center">{control}</div>
    </div>
  );
}

/** Switch with the spec's orange "on" tint. The shared `<Switch>` defaults to
 *  emerald — overriding via className keeps the rest of the codebase's
 *  emerald switches intact. */
function OrangeSwitch(props: React.ComponentProps<typeof Switch>) {
  return (
    <Switch
      {...props}
      className={`data-[state=checked]:bg-orange-500 dark:data-[state=checked]:bg-orange-500 ${
        props.className ?? ""
      }`}
    />
  );
}
