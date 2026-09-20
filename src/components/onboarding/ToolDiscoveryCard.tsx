import { useEffect } from "react";
import { motion, useAnimationControls } from "framer-motion";
import { ToolBadge } from "@/components/tools/ToolBadge";

/**
 * 工具卡的六态状态机。两个调用点共用：
 *   - onboarding/ToolDiscoveryPage（首次绑定）用 scanning / missing /
 *     installing / selected / unselected——所有候选都是"待绑"
 *   - console/AddToolsDialog（追加绑定）多一个 bound——已绑工具也在列表里
 *     展示，但只读
 *
 *   - scanning   首屏未知是否安装；不可交互，整卡 pulse + 虚线
 *   - missing    扫完发现未安装；灰显 + cursor-not-allowed，右下角"安装"按钮
 *   - installing 正在执行 install_tool；pulse 但不灰显，"安装中…" 版本文案
 *   - unselected 已安装、当前未选；点击翻 selected
 *   - selected   已安装并选中；点击翻 unselected
 *   - bound      已绑（仅 AddToolsDialog 用）；橙边 + 顶部绿勾 + 不可点
 *
 * 用 union 而不是 `detected+enabled+bound` 多 bool，是因为 6 态里有两个独立
 * 的"未知/进行中"语义（scanning / installing）——bool 组合表达不出。
 */
export type ToolStatus =
  | "scanning"
  | "missing"
  | "installing"
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
  /**
   * 可选——传入则在 missing 卡右下角渲染"安装"角标按钮，点击调本回调，**不**
   * 触发外层 onClick（stopPropagation）。不传则不渲染按钮（向后兼容
   * AddToolsDialog 里某些不该让用户装的工具，如未来 hermes 临时不可装时）。
   */
  onInstall?: () => void;
}

// relative：让顶部绿勾 badge 用 absolute 定位锚到这里
const BASE =
  "relative flex flex-col items-center gap-2 rounded-xl border-2 px-4 py-5 transition-colors duration-300";

const STATE_CLASSES: Record<ToolStatus, string> = {
  scanning:
    "border-dashed border-border/40 bg-muted/20 animate-pulse cursor-default",
  missing: "border-border/40 bg-muted/30 opacity-50 cursor-not-allowed",
  // installing：橙色虚线 + pulse，区别于 scanning 的"未知"灰色 pulse——这里
  // 是"正在被处理"，应该用品牌色而不是灰色。卡片不灰显，让用户看清楚是哪个
  // 工具在装。
  installing:
    "border-dashed border-orange-300 bg-orange-50/40 animate-pulse cursor-default dark:bg-orange-900/10 dark:border-orange-500/40",
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
  onInstall,
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

  // dimmed 控制 ToolBadge + label 的灰显——只 missing 真灰显（"装不上"）。
  // installing 时品牌色高亮，让用户感知"这个卡正在被处理"。scanning 也
  // 不再 dimmed badge——pulse 已经传达"未知"含义，badge 灰显反而显得这工具不存在。
  const dimmed = status === "missing";
  // 所有"未确定/不可手动操作"态都 disabled。
  const disabled =
    status === "scanning" ||
    status === "missing" ||
    status === "installing" ||
    status === "bound";
  const versionText =
    status === "scanning"
      ? "检测中…"
      : status === "installing"
        ? "安装中…"
        : (version ?? "未安装");

  // 外层 div 是 "卡片本体（motion.button）+ 角标按钮" 的共同定位锚——把
  // 角标渲染到 button **外部**有两个关键作用：
  //   1. button[disabled] 不会吞掉里面的点击事件（浏览器对 disabled button
  //      内的子元素 click 直接不派发，stopPropagation 也无解）
  //   2. missing 卡片整体 opacity-50 不会拉低按钮饱和度——按钮独立在
  //      "灰色蒙层"之外，颜色保持品牌橙原色
  return (
    <div className="relative">
      <motion.button
        type="button"
        onClick={onClick}
        disabled={disabled}
        animate={controls}
        whileTap={!disabled ? { scale: 0.97 } : undefined}
        className={`w-full ${BASE} ${STATE_CLASSES[status]}`}
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
      {status === "missing" && onInstall && (
        // 真 <button>——在 motion.button 外面，所以可以是嵌套 interactive
        // 元素，且不受外层 disabled / opacity-50 影响。z-10 保险锚到顶层。
        <button
          type="button"
          onClick={onInstall}
          className="absolute bottom-2 right-2 z-10 cursor-pointer rounded-md bg-orange-500 px-2 py-0.5 text-[10px] font-medium text-white shadow-sm hover:bg-orange-600"
        >
          安装
        </button>
      )}
    </div>
  );
}
