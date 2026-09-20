import type { ComponentType } from "react";

import {
  ClaudeIcon,
  CodexIcon,
  GeminiIcon,
  HermesIcon,
  OpenClawIcon,
  OpenCodeIcon,
} from "@/components/BrandIcons";
import { TOOL_META } from "@/config/toolMeta";
import { cn } from "@/lib/utils";

type BrandIconComponent = ComponentType<{
  size?: number;
  className?: string;
}>;

/**
 * 单个工具在卡片/列表/弹窗里反复出现的"圆角徽章"——品牌色背景 + 居中
 * logo。在没有官方图标的工具上，回退到 TOOL_META.color + abbr 色块。
 *
 * 用法：
 *   <ToolBadge toolId="claude" size={40} />          // popover / 主列表
 *   <ToolBadge toolId="codex" size={28} />           // 工具选择器 / 紧凑场景
 *
 * 各家品牌色背景是按"品牌识别度 > 视觉一致性"挑的：
 *   - Codex (openai)   黑   — OpenAI 黑底白 logo
 *   - Claude           米橙 — Anthropic 米色品牌色
 *   - Gemini           白   — Gemini logo 本身彩色，背景留白最干净
 *   - OpenCode         浅灰 — pixel art logo 需要明度对比；浅灰避免过黑
 *   - OpenClaw         米色 — 红色卡通龙虾，米底突出主体
 *   - Hermes           米色 — 头像类图标，米底比纯白柔和
 *
 * 内边距统一走 padding（容器 size 的 1/5）——SVG 的 size 自然撑满剩余空间，
 * 不需要为每家硬编码不同 px。OpenCode 占比偏大的问题靠这个 padding 一并解决。
 */
const TOOL_ICON: Record<string, BrandIconComponent> = {
  claude: ClaudeIcon,
  codex: CodexIcon,
  gemini: GeminiIcon,
  opencode: OpenCodeIcon,
  openclaw: OpenClawIcon,
  hermes: HermesIcon,
};

/**
 * 每个工具的徽章背景 Tailwind class——按品牌色挑，色相饱和而非"浅淡"，
 * 这样在浅色 popover 里仍然有明显的卡片感。深色模式给一份相同色相但
 * 略低明度的版本兜底。
 */
const TOOL_BADGE_BG: Record<string, string> = {
  claude: "bg-[#E8DCC8] dark:bg-[#2D2419]",
  codex: "bg-neutral-900 dark:bg-neutral-900",
  // Gemini logo 是彩色，给暖米底让蓝/绿/红/黄四色更跳。
  gemini: "bg-[#E8DDC8] dark:bg-[#2A2419]",
  // OpenCode pixel-art monitor 黑+灰，给中性浅灰让线条干净。
  opencode: "bg-neutral-200 dark:bg-neutral-800",
  openclaw: "bg-[#FFD7BC] dark:bg-[#3A2418]",
  // Hermes 头像类图标，纯白底最干净——其它米色背景会让它"糊"在一起。
  // ring 让白底在浅色 popover 上仍有可见边界，不至于"消失"。
  hermes:
    "bg-white ring-1 ring-black/10 dark:bg-neutral-100 dark:ring-white/10",
};

/**
 * Codex 用的是 lobehub OpenAI 黑色单色 SVG，需要在深色背景上反色。
 * 这是 ToolBadge 内部对图标做的二次着色，与 BrandIcons.CodexIcon 自带的
 * `dark:brightness-0 dark:invert`（应对页面深色模式）不冲突——这里专门
 * 处理"徽章本身是深底"这一固定语境。
 */
const TOOL_ICON_FILTER: Record<string, string> = {
  codex: "[&_img]:brightness-0 [&_img]:invert",
};

interface ToolBadgeProps {
  toolId: string;
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
  size = 40,
  rounded = "lg",
  dimmed = false,
  className,
}: ToolBadgeProps) {
  const Icon = TOOL_ICON[toolId];
  const meta = TOOL_META[toolId];

  // 没有官方 logo（理论上 TOOL_META 已穷举，这里兜底）—— 用 abbr + 主题色块。
  if (!Icon) {
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
  // 单独给它一个更大的 padding 系数把比例拉回来。
  const paddingRatio = toolId === "opencode" ? 0.3 : 0.18;
  const padding = Math.max(4, Math.round(size * paddingRatio));
  const innerSize = size - padding * 2;

  return (
    <div
      style={{ width: size, height: size, padding }}
      className={cn(
        "flex shrink-0 items-center justify-center",
        ROUNDED_CLASS[rounded],
        dimmed
          ? "bg-neutral-200 dark:bg-neutral-800"
          : (TOOL_BADGE_BG[toolId] ?? "bg-muted"),
        // 仅在非 dimmed 时应用品牌特定滤镜——dimmed 下我们让整张 logo 降饱和
        !dimmed && TOOL_ICON_FILTER[toolId],
        dimmed && "opacity-50 grayscale",
        className,
      )}
    >
      <Icon size={innerSize} />
    </div>
  );
}
