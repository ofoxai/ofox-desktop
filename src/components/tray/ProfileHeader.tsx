export default function ProfileHeader() {
  return (
    <div className="flex items-center gap-2.5 px-3.5 pt-3 pb-2.5">
      {/* 头像 */}
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-orange-500 text-sm font-bold text-white">
        O
      </div>

      {/* 用户信息 */}
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-semibold leading-tight text-foreground">
          Lu Mirae
        </div>
        <div className="truncate text-[11px] text-muted-foreground">
          lu@miraecho.jp &middot; Pro 计划
        </div>
      </div>

      {/* 连接状态 */}
      <div className="flex items-center gap-1 text-[11px] text-green-600 dark:text-green-400">
        <span className="h-1.5 w-1.5 rounded-full bg-green-500" />
        已连接
      </div>
    </div>
  );
}
