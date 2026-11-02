import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import type {
  AssistantApprovalChoice,
  AssistantPermissionMode,
  AssistantLocalChange,
} from "@/lib/api/assistant";
export interface LocalApproval {
  approvalId: string;
  name: string;
  details: string;
  allowSession: boolean;
}
export interface LocalPlanStep {
  step: string;
  status: "pending" | "in_progress" | "completed";
}
export function AssistantLocalPanel({
  mode,
  locked,
  approvals,
  changes,
  plan,
  intent,
  warning,
  cost,
  onPermission,
  onApproval,
  onRestore,
}: {
  mode: AssistantPermissionMode;
  locked: boolean;
  approvals: LocalApproval[];
  changes: AssistantLocalChange[];
  plan: LocalPlanStep[];
  intent: string | null;
  warning: string | null;
  cost: number;
  onPermission: (
    mode: AssistantPermissionMode,
    confirmed?: boolean,
  ) => Promise<boolean>;
  onApproval: (id: string, choice: AssistantApprovalChoice) => Promise<boolean>;
  onRestore: () => Promise<boolean>;
}) {
  const { t } = useTranslation();
  const [confirmFull, setConfirmFull] = useState(false);
  const [busy, setBusy] = useState(false);
  const perform = async (action: () => Promise<boolean>) => {
    setBusy(true);
    try {
      await action();
    } finally {
      setBusy(false);
    }
  };
  return (
    <section
      aria-label={t("assistant.local.title")}
      className="space-y-2 border-b border-line px-4 py-2 text-note"
    >
      <div className="flex flex-wrap items-center gap-3">
        <Select
          value={mode}
          disabled={locked || busy}
          onValueChange={(value: AssistantPermissionMode) => {
            if (value === "fullAccess") setConfirmFull(true);
            else void perform(() => onPermission(value));
          }}
        >
          <SelectTrigger
            aria-label={t("assistant.local.permission")}
            className="w-auto min-w-36"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {(["readOnly", "ask", "autoConfig", "fullAccess"] as const).map(
              (value) => (
                <SelectItem key={value} value={value}>
                  {t(`assistant.local.modes.${value}`)}
                </SelectItem>
              ),
            )}
          </SelectContent>
        </Select>
        <span>{t("assistant.local.roundCost", { cost: cost.toFixed(4) })}</span>
      </div>
      {intent && (
        <p role="status" className="text-ink-muted">
          {intent}
        </p>
      )}
      {warning && (
        <p role="status" className="text-warn">
          {t(`assistant.local.warnings.${warning}`)}
        </p>
      )}
      {plan.length > 0 && (
        <ol aria-label={t("assistant.local.plan")} className="space-y-1">
          {plan.map((step, index) => (
            <li
              key={index}
              className={step.status === "completed" ? "text-ink-muted" : ""}
            >
              <span aria-label={t(`assistant.local.steps.${step.status}`)}>
                {step.status === "completed"
                  ? "✓"
                  : step.status === "in_progress"
                    ? "→"
                    : "○"}
              </span>{" "}
              {step.step}
            </li>
          ))}
        </ol>
      )}
      {approvals.map((approval) => (
        <div
          key={approval.approvalId}
          role="group"
          aria-label={t("assistant.local.approval")}
          className="space-y-2 rounded border border-line p-3"
        >
          <p>
            {t(`assistant.local.operations.${approval.name}`, {
              defaultValue: approval.name,
            })}
          </p>
          <pre className="max-h-36 overflow-auto whitespace-pre-wrap break-words font-sans">
            {approval.details}
          </pre>
          <div className="flex flex-wrap gap-2">
            {(
              [
                "once",
                ...(approval.allowSession ? ["session" as const] : []),
                "deny",
              ] as const
            ).map((choice) => (
              <Button
                key={choice}
                variant={choice === "deny" ? "ghost" : "outline"}
                disabled={busy}
                onClick={() =>
                  void perform(() => onApproval(approval.approvalId, choice))
                }
              >
                {t(
                  `assistant.local.${choice === "once" ? "allowOnce" : choice === "session" ? "allowSession" : "deny"}`,
                )}
              </Button>
            ))}
          </div>
        </div>
      ))}
      {changes.length > 0 && (
        <details>
          <summary>
            {t("assistant.local.changes", {
              count: changes.filter((c) => c.changed && !c.restored).length,
            })}
          </summary>
          <ul className="my-2 max-h-32 overflow-auto">
            {changes.map((change) => (
              <li key={change.id} className="break-all">
                {change.restored ? "✓ " : ""}
                {change.paths.join(", ")}
              </li>
            ))}
          </ul>
          <Button
            variant="outline"
            disabled={
              locked || busy || !changes.some((c) => !c.restored && c.changed)
            }
            onClick={() => void perform(onRestore)}
          >
            {t("assistant.local.restore")}
          </Button>
        </details>
      )}
      <ConfirmDialog
        isOpen={confirmFull}
        title={t("assistant.local.fullTitle")}
        message={t("assistant.local.fullMessage")}
        busy={busy}
        onCancel={() => setConfirmFull(false)}
        onConfirm={() =>
          void perform(async () => {
            const ok = await onPermission("fullAccess", true);
            if (ok) setConfirmFull(false);
            return ok;
          })
        }
      />
    </section>
  );
}
