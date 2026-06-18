import { useCallback, useState } from "react";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useOfoxAuth } from "@/hooks/useOfoxAuth";
import { ofoxSetApex } from "@/lib/api/ofoxApex";
import type { OfoxApex } from "@/lib/ofoxUrls";

interface OfoxApexSwitchProps {
  /**
   * Tailwind size token for the select trigger. Default `"h-8 w-[140px]"`,
   * 适合设置弹窗里的 Row 控件；登录页头部用 `"h-7 w-[120px]"` 更紧凑。
   */
  triggerClassName?: string;
}

const APEX_OPTIONS: ReadonlyArray<{ value: OfoxApex; label: string }> = [
  { value: "ofox.ai", label: "海外 · ofox.ai" },
  { value: "ofox.io", label: "国内 · ofox.io" },
];

const APEX_LABEL: Record<OfoxApex, string> = {
  "ofox.ai": "海外 · ofox.ai",
  "ofox.io": "国内 · ofox.io",
};

/**
 * OFox 区域（apex）切换器——登录页 + 设置弹窗共用。
 *
 * 行为：
 *   - 未登录态切换：直接调 `ofoxSetApex`（后端的同值 no-op 兜底，万一用户
 *     点了同一个选项也不会触发副作用）。
 *   - 已登录态切换：先弹 ConfirmDialog，确认后才发请求。后端在 `set_apex`
 *     里会原子地完成 logout + reseed providers + emit reauth-requested，
 *     用户会被自动跳到 LoginPage。
 *
 * 失败处理：toast 报错，UI state 不动（pending 选项不会被锁定为已选）。
 *
 * 此组件**只**负责"切 apex"这件事；登录页 / 设置弹窗各自决定把它放在哪里、
 * 配什么标签、占多大空间。
 */
export function OfoxApexSwitch({
  triggerClassName = "h-8 w-[140px] text-[13px]",
}: OfoxApexSwitchProps) {
  const { apex, refetch } = useOfoxApex();
  const { isActive } = useOfoxAuth();

  const [pendingTarget, setPendingTarget] = useState<OfoxApex | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const performSwitch = useCallback(
    async (next: OfoxApex) => {
      setSubmitting(true);
      try {
        await ofoxSetApex(next);
        // 后端会 emit `ofox-apex-changed`，hook 自动 setApex；这里 refetch
        // 兜底覆盖事件丢失的极端情形（dev 下 emit 偶尔来不及）。
        await refetch();
        toast.success(`已切换到 ${APEX_LABEL[next]}`);
      } catch (e) {
        console.error("[OfoxApexSwitch] ofoxSetApex failed", e);
        toast.error("切换区域失败，请稍后重试");
      } finally {
        setSubmitting(false);
        setPendingTarget(null);
      }
    },
    [refetch],
  );

  const handleSelectChange = useCallback(
    (rawValue: string) => {
      if (rawValue !== "ofox.ai" && rawValue !== "ofox.io") return;
      const next = rawValue as OfoxApex;
      if (next === apex) return; // 同值 no-op
      if (isActive) {
        // 已登录：弹确认对话框
        setPendingTarget(next);
      } else {
        // 未登录：直接切
        void performSwitch(next);
      }
    },
    [apex, isActive, performSwitch],
  );

  const handleConfirm = useCallback(() => {
    if (pendingTarget) void performSwitch(pendingTarget);
  }, [pendingTarget, performSwitch]);

  const handleCancel = useCallback(() => {
    setPendingTarget(null);
  }, []);

  return (
    <>
      <Select
        value={apex}
        onValueChange={handleSelectChange}
        disabled={submitting}
      >
        <SelectTrigger className={triggerClassName}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {APEX_OPTIONS.map((opt) => (
            <SelectItem key={opt.value} value={opt.value}>
              {opt.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>

      <ConfirmDialog
        isOpen={pendingTarget !== null}
        variant="info"
        title="切换区域需要重新登录"
        message={
          pendingTarget
            ? `当前会话将被清除，您需要在 ${APEX_LABEL[pendingTarget]} 重新授权。\n` +
              `已绑定工具的 API Key 不会丢失，但工具会暂时不可用，直到您完成新的登录。`
            : ""
        }
        confirmText="继续切换"
        cancelText="保留当前区域"
        onConfirm={handleConfirm}
        onCancel={handleCancel}
      />
    </>
  );
}
