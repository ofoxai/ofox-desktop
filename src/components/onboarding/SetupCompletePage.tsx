import { CheckCircle2 } from "lucide-react";

interface SetupCompletePageProps {
  boundCount: number;
  onOpenConsole: () => void;
}

export default function SetupCompletePage({
  boundCount,
  onOpenConsole,
}: SetupCompletePageProps) {
  return (
    <div className="flex h-screen w-full items-center justify-center bg-gradient-to-br from-orange-50/80 via-white to-orange-50/40 dark:from-neutral-950 dark:via-neutral-900 dark:to-neutral-950">
      <div className="flex w-full max-w-xl flex-col items-center px-8">
        {/* Success Icon */}
        <div className="mb-6 flex h-20 w-20 items-center justify-center rounded-[22px] bg-emerald-500 text-4xl text-white shadow-lg shadow-emerald-200 dark:shadow-emerald-900/30">
          <CheckCircle2 className="h-10 w-10" strokeWidth={2.5} />
        </div>

        {/* Title */}
        <h1 className="mb-3 text-3xl font-bold text-foreground">
          全部设置完成
        </h1>
        <p className="mb-2 text-center text-sm leading-relaxed text-muted-foreground">
          已为 {boundCount} 个工具绑定 Ofox。打开 Claude Code、Cursor
          或其它工具就能直接使用。
        </p>
        <p className="mb-8 text-center text-sm text-muted-foreground">
          菜单栏的 <span className="font-semibold text-orange-500">Ofox 图标</span>{" "}
          可以随时查看余额和用量。
        </p>

        {/* Stats */}
        <div className="mb-8 grid w-full grid-cols-2 gap-4">
          <div className="flex flex-col items-start rounded-xl border border-border px-4 py-3">
            <span className="text-[11px] text-muted-foreground">
              工具已绑定
            </span>
            <span className="text-2xl font-bold text-foreground">
              {boundCount}
            </span>
          </div>
          <div className="flex flex-col items-start rounded-xl border border-border px-4 py-3">
            <span className="text-[11px] text-muted-foreground">可用模型</span>
            <span className="text-2xl font-bold text-foreground">100+</span>
          </div>
          {/* 「赠送额度 $5.00」卡片暂时隐藏 —— 实际赠送策略未定，避免给用户硬编码承诺 */}
        </div>

        {/* Actions */}
        <div className="flex w-full max-w-sm items-center justify-center">
          <button
            onClick={onOpenConsole}
            className="flex-1 rounded-xl bg-gradient-to-r from-orange-400 to-orange-500 px-6 py-3 text-base font-semibold text-white shadow-md shadow-orange-200 transition-all hover:from-orange-500 hover:to-orange-600 dark:shadow-orange-900/20"
          >
            打开控制台 →
          </button>
        </div>

        {/* Tip */}
        <p className="mt-6 text-[12px] text-muted-foreground/60">
          💡 在终端运行 claude 或 cursor 立即试试
        </p>
      </div>
    </div>
  );
}
