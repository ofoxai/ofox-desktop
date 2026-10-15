import { TOOL_LOGOS } from "@/config/toolLogos";
import { TOOL_META } from "@/config/toolMeta";
import { cn } from "@/lib/utils";

/**
 * 每个工具的徽章背景 Tailwind class——按品牌色挑，色相饱和而非"浅淡"，
 * 这样在浅色 popover 里仍然有明显的卡片感。深色模式给一份相同色相但
 * 略低明度的版本兜底。
 */
const TOOL_BADGE_BG: Record<string, string> = {
  dsh: "bg-white dark:bg-neutral-100",
  pi: "bg-orange-200 dark:bg-orange-950",
  claude: "bg-[#E8DCC8] dark:bg-[#2D2419]",
  codex: "bg-transparent",
  // ChatGPT 品牌绿（近 OpenAI 官网 accent），让"Codex vs ChatGPT"两张
  // OpenAI 牌卡片一眼分得开。
  chatgpt: "bg-[#10A37F] dark:bg-[#0E8266]",
  // Gemini logo 是彩色，给暖米底让蓝/绿/红/黄四色更跳。
  gemini: "bg-[#E8DDC8] dark:bg-[#2A2419]",
  // OpenCode pixel-art monitor 黑+灰，给中性浅灰让线条干净。
  opencode: "bg-neutral-200 dark:bg-neutral-800",
  openclaw: "bg-[#FFD7BC] dark:bg-[#3A2418]",
  hermes: "bg-neutral-100 dark:bg-neutral-800",
  // WorkBuddy 官方图标已自带圆形渐变背景，不再叠加额外色块。
  workbuddy: "bg-transparent",
};

interface ToolBadgeProps {
  toolId: string;
  variant?: "badge" | "plain";
  /** 徽章正方形边长（px）。默认 40，适合 popover / 主列表。 */
  size?: number;
  /** 圆角半径风格——`"md"` (8px) / `"lg"` (12px) / `"xl"` (16px)。 */
  rounded?: "md" | "lg" | "xl";
  /**
   * 工具不可用 / 未检测到时的灰化态——onboarding 和 AddToolsDialog 在用，
   * 表达"虽然界面里列出来了，但当前机器上没装"。视觉上把品牌色统一替换
   * 为中性灰，logo 降饱和。
   */
  dimmed?: boolean;
  className?: string;
}

const ROUNDED_CLASS: Record<NonNullable<ToolBadgeProps["rounded"]>, string> = {
  md: "rounded-md",
  lg: "rounded-lg",
  xl: "rounded-xl",
};

export function ToolBadge({
  toolId,
  variant = "badge",
  size = 40,
  rounded = "lg",
  dimmed = false,
  className,
}: ToolBadgeProps) {
  const logo = TOOL_LOGOS[toolId];
  const meta = TOOL_META[toolId];

  // Preserve a fallback for unknown future IDs; every catalog entry has a logo.
  if (!logo) {
    return (
      <div
        style={{ width: size, height: size }}
        className={cn(
          "flex shrink-0 items-center justify-center font-bold text-white",
          ROUNDED_CLASS[rounded],
          dimmed ? "bg-gray-400" : (meta?.color ?? "bg-muted-foreground"),
          className,
        )}
      >
        <span style={{ fontSize: Math.max(10, Math.floor(size * 0.3)) }}>
          {meta?.abbr ?? toolId.substring(0, 2).toUpperCase()}
        </span>
      </div>
    );
  }

  // SVG 在 padding 内自然撑满；不同 logo 的"自带 viewBox 留白"差异
  // 由统一的 padding 抹平，肉眼看上去比例一致。
  //
  // OpenCode 的 SVG 是 pixel-art monitor，本身没有 viewBox 留白，监视器
  // 矩形几乎贴到边——视觉上比其它"自带 padding"的 lobehub logo 大一圈。
  // WorkBuddy 的官方圆形图标则自带完整背景，需要铺满徽章。
  const paddingRatio =
    toolId === "opencode" ? 0.3 : toolId === "workbuddy" ? 0 : 0.18;
  const padding =
    variant === "plain" || toolId === "workbuddy"
      ? 0
      : Math.max(4, Math.round(size * paddingRatio));
  const innerSize = size - padding * 2;

  return (
    <div
      style={{ width: size, height: size, padding }}
      className={cn(
        "flex shrink-0 items-center justify-center",
        ROUNDED_CLASS[rounded],
        variant === "badge" &&
          (dimmed
            ? "bg-neutral-200 dark:bg-neutral-800"
            : (TOOL_BADGE_BG[toolId] ?? "bg-transparent")),
        dimmed && "opacity-50 grayscale",
        className,
      )}
    >
      <img
        src={logo.src}
        width={innerSize}
        height={innerSize}
        alt={logo.label}
        loading="lazy"
        className={cn(
          "h-full w-full object-contain",
          logo.darkTreatment === "white" && "dark:brightness-0 dark:invert",
          logo.darkTreatment === "invert" && "dark:invert",
          toolId === "chatgpt" &&
            variant === "badge" &&
            !dimmed &&
            "brightness-0 invert",
        )}
      />
    </div>
  );
}
