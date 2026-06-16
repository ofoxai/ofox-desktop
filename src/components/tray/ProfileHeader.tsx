import type { OfoxAuthStatus } from "@/lib/api/ofoxAuth";

interface ProfileHeaderProps {
  status: OfoxAuthStatus | null;
}

/**
 * Avatar + name + email + connection chip.
 *
 * The connection chip is the user's at-a-glance signal: green dot for an
 * active session, amber for "expired, needs re-login", muted for "not signed
 * in". Names/emails fall back to the cached user info even when expired so
 * the user can still recognize which account they were logged into.
 */
export default function ProfileHeader({ status }: ProfileHeaderProps) {
  const user = status?.user ?? null;
  const state = status?.state ?? "loggedout";

  // First letter of name or email for the avatar; default to "O" (Ofox).
  const avatarChar =
    user?.name?.trim()?.[0]?.toUpperCase() ??
    user?.email?.trim()?.[0]?.toUpperCase() ??
    "O";

  const displayName = user?.name?.trim() || user?.email || "未登录";
  const subline = (() => {
    if (state === "active") {
      return user?.email ?? "";
    }
    if (state === "expired") {
      return user?.email ? `${user.email} · 登录已过期` : "登录已过期";
    }
    return "请登录 Ofox 账号";
  })();

  const chip = (() => {
    if (state === "active") {
      return (
        <div className="flex items-center gap-1 text-[11px] text-green-600 dark:text-green-400">
          <span className="h-1.5 w-1.5 rounded-full bg-green-500" />
          已连接
        </div>
      );
    }
    if (state === "expired") {
      return (
        <div className="flex items-center gap-1 text-[11px] text-amber-600 dark:text-amber-400">
          <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
          已过期
        </div>
      );
    }
    return (
      <div className="flex items-center gap-1 text-[11px] text-muted-foreground">
        <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/60" />
        未登录
      </div>
    );
  })();

  return (
    <div className="flex items-center gap-2.5 px-3.5 pt-3 pb-2.5">
      {/* 头像 */}
      <div
        className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-sm font-bold text-white ${
          state === "active"
            ? "bg-orange-500"
            : state === "expired"
              ? "bg-amber-500"
              : "bg-muted-foreground/60"
        }`}
      >
        {avatarChar}
      </div>

      {/* 用户信息 */}
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-semibold leading-tight text-foreground">
          {displayName}
        </div>
        <div className="truncate text-[11px] text-muted-foreground">
          {subline}
        </div>
      </div>

      {chip}
    </div>
  );
}
