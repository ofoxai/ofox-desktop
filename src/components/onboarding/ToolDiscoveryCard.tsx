import { useEffect } from "react";
import { motion, useAnimationControls } from "framer-motion";
import { ToolBadge } from "@/components/tools/ToolBadge";

/**
 * 工具卡的五态状态机。两个调用点共用：
 *   - onboarding/ToolDiscoveryPage（首次绑定）只用 scanning / missing /
 *     selected / unselected——所有候选都是"待绑"
 *   - console/AddToolsDialog（追加绑定）多一个 bound——已绑工具也在列表里
 *     展示，但只读
 *
 *   - scanning   首屏未知是否安装；不可交互，整卡 pulse + 虚线
 *   - missing    扫完发现未安装；灰显 + cursor-not-allowed
 *   - unselected 已安装、当前未选；点击翻 selected
 *   - selected   已安装并选中；点击翻 unselected
 *   - bound      已绑（仅 AddToolsDialog 用）；橙边 + 顶部绿勾 + 不可点
 *
 * 用 union 而不是 `detected+enabled+bound` 多 bool，是因为 5 态里有一个独立
 * 的"扫描中"语义——bool 组合表达不出"detected 还未确定"的状态。
 */
export type ToolStatus =
  | "scanning"
  | "missing"
  | "selected"
  | "unselected"
  | "bound";

interface ToolDiscoveryCardProps {
  toolId: string;
  label: string;
  version: string | null;
  status: ToolStatus;
  /**
   * 父组件每次"自动 stagger 翻 selected"时递增 1；变化时本卡播一次 scale
   * 反馈关键帧 [1, 1.04, 1]。手动点击切换 selected ⇄ unselected 时这个值
   * 不变，所以不会触发关键帧——手动反馈靠 whileTap + CSS transition-colors。
   *
   * 用 tick 计数器而不是 `bool` 是为了让"连点两次自动选中"也能各触发一次动画
   * （bool 在 true→true 时 useEffect 不会重跑）。
   */
  autoSelectTick: number;
  onClick: () => void;
}

// relative：让顶部绿勾 badge 用 absolute 定位锚到这里
const BASE =
  "relative flex flex-col items-center gap-2 rounded-xl border-2 px-4 py-5 transition-colors duration-300";

const STATE_CLASSES: Record<ToolStatus, string> = {
  scanning:
    "border-dashed border-border/40 bg-muted/20 animate-pulse cursor-default",
  missing:
    "border-border/40 bg-muted/30 opacity-50 cursor-not-allowed",
  unselected:
    "border-border bg-white hover:border-orange-200 dark:bg-neutral-800 cursor-pointer",
  selected:
    "border-orange-400 bg-white shadow-sm shadow-orange-100 dark:bg-neutral-800 dark:shadow-orange-900/20 cursor-pointer",
  // bound 视觉上和 selected 几乎一样——同一张橙边卡——靠顶部绿勾区分。
  // cursor-default + button[disabled] 拦截点击。
  bound:
    "border-orange-400 bg-white shadow-sm shadow-orange-100 dark:bg-neutral-800 dark:shadow-orange-900/20 cursor-default",
};

export function ToolDiscoveryCard({
  toolId,
  label,
  version,
  status,
  autoSelectTick,
  onClick,
}: ToolDiscoveryCardProps) {
  const controls = useAnimationControls();

  // 自动 stagger 翻 selected 时,播一次缩放反馈。条件里同时校验 status 是
  // selected——避免父组件因为别的原因（理论上不会发生）递增 tick 时也触发
  // 动画。
  useEffect(() => {
    if (autoSelectTick > 0 && status === "selected") {
      void controls.start({
        scale: [1, 1.04, 1],
        transition: { duration: 0.24, ease: "easeOut" },
      });
    }
  }, [autoSelectTick, status, controls]);

  const dimmed = status === "scanning" || status === "missing";
  // bound 也 disabled——已绑工具在添加面板里只读，点击无意义。
  const disabled =
    status === "scanning" || status === "missing" || status === "bound";
  const versionText =
    status === "scanning" ? "检测中…" : version ?? "未安装";

  return (
    <motion.button
      type="button"
      onClick={onClick}
      disabled={disabled}
      animate={controls}
      whileTap={!disabled ? { scale: 0.97 } : undefined}
      className={`${BASE} ${STATE_CLASSES[status]}`}
    >
      {status === "bound" && (
        // 顶部居中绿勾——位置压在卡片顶边上（-top-2），让"已绑定"在一眼扫
        // 视时就能跟"选中"区分开。SVG 用 viewBox 内绘制对勾，避免依赖额外
        // icon 包。aria-hidden + 父按钮已 disabled，无需额外 a11y 文案。
        <span
          aria-hidden="true"
          className="absolute -top-2 left-1/2 flex h-5 w-5 -translate-x-1/2 items-center justify-center rounded-full bg-green-500 text-white shadow-sm ring-2 ring-white dark:ring-neutral-800"
        >
          <svg
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
            className="h-3 w-3"
          >
            <path d="M3 8.5l3.2 3.2L13 5" />
          </svg>
        </span>
      )}
      <ToolBadge toolId={toolId} size={40} rounded="xl" dimmed={dimmed} />
      <span
        className={`text-sm font-medium ${
          dimmed ? "text-muted-foreground" : "text-foreground"
        }`}
      >
        {label}
      </span>
      <span className="text-[11px] text-muted-foreground">{versionText}</span>
    </motion.button>
  );
}
