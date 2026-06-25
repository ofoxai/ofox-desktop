import { UserAvatar } from "@/components/UserAvatar";
import type { OfoxAuthStatus } from "@/lib/api/ofoxAuth";
import { ofoxAvatarUrl } from "@/lib/ofoxUrls";

interface ProfileHeaderProps {
  status: OfoxAuthStatus | null;
}

/**
 * 头像 + 名称 + 邮箱/状态说明。
 *
 * 头像走主窗口同款 `UserAvatar`——有 `avatar_url` 走真实图片，没有则
 * fallback 到首字母圆。这一组件本身已经负责图片加载失败的 onError 兜底。
 *
 * 不再展示"已连接/已过期/未登录" chip——连接状态由 AuthExpiredBanner 在异常态时
 * 显式提醒；正常态用户看到自己的账号就足够了。
 */
export default function ProfileHeader({ status }: ProfileHeaderProps) {
  const user = status?.user ?? null;
  const state = status?.state ?? "loggedout";

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

  return (
    <div className="flex items-center gap-2.5 px-3.5 pt-3 pb-2.5">
      <UserAvatar
        avatarUrl={ofoxAvatarUrl(user?.avatar_url)}
        name={user?.name}
        email={user?.email}
        className="h-8 w-8"
        fallbackTextClassName="text-sm"
      />

      {/* 用户信息 */}
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-semibold leading-tight text-foreground">
          {displayName}
        </div>
        <div className="truncate text-[11px] text-muted-foreground">
          {subline}
        </div>
      </div>
    </div>
  );
}
