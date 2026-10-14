import {
  useCallback,
  useState,
  type ComponentType,
  type SVGProps,
} from "react";
import { Compass, Flag, Globe2 } from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { MenuSelect, type MenuSelectOption } from "@/components/ui/menu-select";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useOfoxAuth } from "@/hooks/useOfoxAuth";
import { ofoxSetApex, ofoxSetApexAuto } from "@/lib/api/ofoxApex";
import type { OfoxApex } from "@/lib/ofoxUrls";

type LucideIcon = ComponentType<SVGProps<SVGSVGElement>>;

interface OfoxApexSwitchProps {
  /** 额外的 trigger class（宽度等）；默认是 MenuSelect 的 26px 外观。 */
  triggerClassName?: string;
}

/** 下拉里的「自动检测」项：不锁定，每次启动按网络重新探测。 */
export const AUTO_SELECTION = "auto";
export type ApexSelection = typeof AUTO_SELECTION | OfoxApex;

const APEX_LABEL: Record<OfoxApex, string> = {
  "ofox.ai": "ofox.ai",
  "ofox.io": "ofox.io",
};

const APEX_ICON: Record<OfoxApex, LucideIcon> = {
  "ofox.ai": Globe2,
  "ofox.io": Flag,
};

export type SelectionPlan =
  | { kind: "noop" }
  /** 从「自动」改成手动选当前这个区域：只锁定，不清会话。 */
  | { kind: "pin"; apex: OfoxApex }
  | { kind: "switch"; apex: OfoxApex; confirm: boolean }
  | { kind: "auto"; confirm: boolean };

/** 用户在下拉里选了 `selection` 之后该做什么。已登录时切换要先确认。 */
export function planSelection(
  selection: ApexSelection,
  state: { apex: OfoxApex; pinned: boolean; isActive: boolean },
): SelectionPlan {
  if (selection === AUTO_SELECTION) {
    return state.pinned
      ? { kind: "auto", confirm: state.isActive }
      : { kind: "noop" };
  }
  if (selection === state.apex) {
    return state.pinned ? { kind: "noop" } : { kind: "pin", apex: selection };
  }
  return { kind: "switch", apex: selection, confirm: state.isActive };
}

function isSelection(value: string): value is ApexSelection {
  return value === AUTO_SELECTION || value === "ofox.ai" || value === "ofox.io";
}

/**
 * OFox 区域（apex）切换器——登录页 + 设置弹窗共用。
 *
 * 三个选项：自动检测（默认，每次启动按出口 IP 选）、ofox.ai、ofox.io。手动选
 * 一个区域就锁定它，启动探测不再改；选「自动检测」解除锁定并立刻探测一次。
 *
 * 行为：
 *   - 未登录态：直接调后端。
 *   - 已登录态：切换区域或改回自动都可能清会话，先弹 ConfirmDialog。
 *   - 从自动改成手动选当前区域：只锁定，不清会话，不弹窗。
 *
 * 失败处理：toast 报错，UI state 不动（pending 选项不会被锁定为已选）。
 */
export function OfoxApexSwitch({ triggerClassName }: OfoxApexSwitchProps) {
  const { t } = useTranslation();
  const { apex, pinned, refetch } = useOfoxApex();
  const { isActive } = useOfoxAuth();

  const [pendingPlan, setPendingPlan] = useState<SelectionPlan | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const performPlan = useCallback(
    async (plan: SelectionPlan) => {
      setSubmitting(true);
      try {
        if (plan.kind === "pin" || plan.kind === "switch") {
          const workBuddySynced = await ofoxSetApex(plan.apex);
          // 后端会 emit `ofox-apex-changed`，hook 自动刷新；这里 refetch
          // 兜底覆盖事件丢失的极端情形（dev 下 emit 偶尔来不及）。
          await refetch();
          toast.success(
            t(
              plan.kind === "pin" ? "apexSwitch.pinned" : "apexSwitch.switched",
              {
                apex: APEX_LABEL[plan.apex],
              },
            ),
          );
          if (plan.kind === "switch" && !workBuddySynced) {
            toast.warning(t("modelCompatibility.workBuddyEndpointSyncFailed"));
          }
        } else if (plan.kind === "auto") {
          const result = await ofoxSetApexAuto();
          await refetch();
          const label = APEX_LABEL[result.apex];
          if (result.detected) {
            toast.success(t("apexSwitch.autoDetected", { apex: label }));
          } else {
            toast.warning(t("apexSwitch.autoUndetected", { apex: label }));
          }
        }
      } catch (e) {
        console.error("[OfoxApexSwitch] apex change failed", e);
        toast.error(t("apexSwitch.switchFailed"));
      } finally {
        setSubmitting(false);
        setPendingPlan(null);
      }
    },
    [refetch, t],
  );

  const handleSelectChange = useCallback(
    (rawValue: string) => {
      if (!isSelection(rawValue)) return;
      const plan = planSelection(rawValue, { apex, pinned, isActive });
      if (plan.kind === "noop") return;
      if ("confirm" in plan && plan.confirm) {
        setPendingPlan(plan);
      } else {
        void performPlan(plan);
      }
    },
    [apex, pinned, isActive, performPlan],
  );

  const handleConfirm = useCallback(() => {
    if (pendingPlan) void performPlan(pendingPlan);
  }, [pendingPlan, performPlan]);

  const handleCancel = useCallback(() => {
    setPendingPlan(null);
  }, []);

  const options: MenuSelectOption<ApexSelection>[] = [
    {
      value: AUTO_SELECTION,
      label: t("apexSwitch.auto"),
      icon: <Compass />,
    },
    ...(["ofox.ai", "ofox.io"] as const).map((value) => {
      const Icon = APEX_ICON[value];
      return { value, label: APEX_LABEL[value], icon: <Icon /> };
    }),
  ];
  // 自动模式下 trigger 显示「自动 · 当前探测到的区域」，菜单里仍是「自动检测」。
  const triggerLabel = pinned
    ? undefined
    : t("apexSwitch.autoLabel", { apex: APEX_LABEL[apex] });
  const confirmMessage =
    pendingPlan?.kind === "switch"
      ? t("apexSwitch.confirmMessage", { apex: APEX_LABEL[pendingPlan.apex] })
      : pendingPlan?.kind === "auto"
        ? t("apexSwitch.confirmAutoMessage", { apex: APEX_LABEL[apex] })
        : "";

  return (
    <>
      <MenuSelect<ApexSelection>
        aria-label={t("apexSwitch.label")}
        value={pinned ? apex : AUTO_SELECTION}
        onChange={handleSelectChange}
        options={options}
        valueLabel={triggerLabel}
        disabled={submitting}
        width={200}
        align="end"
        triggerClassName={triggerClassName}
      />

      <ConfirmDialog
        isOpen={pendingPlan !== null}
        variant="info"
        title={t("apexSwitch.confirmTitle")}
        message={confirmMessage}
        confirmText={t("apexSwitch.confirm")}
        cancelText={t("apexSwitch.cancel")}
        onConfirm={handleConfirm}
        onCancel={handleCancel}
      />
    </>
  );
}
