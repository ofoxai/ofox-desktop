import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Wrench } from "lucide-react";
import type {
  AssistantApprovalChoice,
  AssistantLocalChange,
  AssistantPermissionMode,
  AssistantRunEvent,
} from "@/lib/api/assistant";
import { Button } from "@/components/ui/button";
import { MenuSelect } from "@/components/ui/menu-select";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { cn } from "@/lib/utils";

export type LocalApproval = Extract<
  AssistantRunEvent,
  { type: "approval" }
>["data"];

export interface LocalPlanStep {
  step: string;
  status: "pending" | "in_progress" | "completed";
}

const MODES = ["default", "autoEdit", "fullAuto"] as const;
type LocalMenuChoice = AssistantPermissionMode | "off";

/**
 * The composer's local configuration switch. Off, a toggle like Web search;
 * on, `本机配置 · 默认 ▾`: the name turns it off, the menu picks how much the
 * assistant may do without asking (or turns it off). Full auto asks first.
 */
export function LocalConfigControl({
  on,
  mode,
  disabled,
  onToggle,
  onMode,
}: {
  on: boolean;
  mode: AssistantPermissionMode;
  disabled: boolean;
  onToggle: (on: boolean) => void;
  onMode: (
    mode: AssistantPermissionMode,
    confirmed?: boolean,
  ) => Promise<boolean>;
}) {
  const { t } = useTranslation();
  const [confirmFull, setConfirmFull] = useState(false);
  const [busy, setBusy] = useState(false);
  const choose = async (next: AssistantPermissionMode, confirmed?: boolean) => {
    setBusy(true);
    try {
      return await onMode(next, confirmed);
    } finally {
      setBusy(false);
    }
  };
  // One wrapper on and off, so the toggle keeps its focus when pressed.
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center rounded-ctl",
        on && "bg-brand-soft text-brand-ink",
      )}
    >
      <Button
        variant="ghost"
        aria-pressed={on}
        disabled={disabled}
        onClick={() => onToggle(!on)}
        className={cn(
          "shrink-0 px-2",
          on && "pr-1 text-brand-ink hover:bg-transparent hover:text-ink",
        )}
      >
        <Wrench aria-hidden className="h-3.5 w-3.5" />
        {t("assistant.local.title")}
      </Button>
      {on && (
        <>
          <span aria-hidden className="opacity-60">
            ·
          </span>
          <MenuSelect<LocalMenuChoice>
            aria-label={t("assistant.local.permission")}
            value={mode}
            disabled={disabled || busy}
            onChange={(next) => {
              if (next === "off") onToggle(false);
              else if (next === "fullAuto") setConfirmFull(true);
              else void choose(next);
            }}
            options={[
              ...MODES.map((value) => ({
                value,
                label: t(`assistant.local.modes.${value}`),
                note: t(`assistant.local.modeNotes.${value}`),
                group: "modes",
                groupLabel: t("assistant.local.permission"),
              })),
              { value: "off" as const, label: t("assistant.local.turnOff") },
            ]}
            valueLabel={t(`assistant.local.modes.${mode}`)}
            width={300}
            align="start"
            triggerClassName="h-[27px] border-transparent bg-transparent pl-1 pr-2 text-brand-ink hover:bg-transparent hover:text-ink data-[state=open]:bg-transparent data-[state=open]:text-ink"
          />
        </>
      )}
      <ConfirmDialog
        isOpen={confirmFull}
        title={t("assistant.local.fullTitle")}
        message={t("assistant.local.fullMessage")}
        confirmText={t("assistant.local.fullConfirm")}
        busy={busy}
        onCancel={() => setConfirmFull(false)}
        onConfirm={() =>
          void choose("fullAuto", true).then((ok) => {
            if (ok) setConfirmFull(false);
          })
        }
      />
    </span>
  );
}

/**
 * Inside the reply a local configuration run is writing: what it is doing
 * now, a warning from the host, and its plan. Gone once the run ends.
 */
export function LocalTaskProgress({
  intent,
  warning,
  plan,
}: {
  intent: string | null;
  warning: string | null;
  plan: LocalPlanStep[];
}) {
  const { t } = useTranslation();
  if (!intent && !warning && plan.length === 0) return null;
  return (
    <div className="mt-2 space-y-1 text-note text-ink-muted">
      {intent && <p>{intent}</p>}
      {warning && (
        <p role="status" className="text-warn">
          {t(`assistant.local.warnings.${warning}`)}
        </p>
      )}
      {plan.length > 0 && (
        <ol aria-label={t("assistant.local.plan")} className="space-y-0.5">
          {plan.map((step, index) => (
            <li
              key={index}
              className={
                step.status === "completed"
                  ? "text-ink-faint"
                  : step.status === "in_progress"
                    ? "text-ink-2"
                    : undefined
              }
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
    </div>
  );
}

/**
 * An operation waiting for the user, at the end of the transcript: what it
 * is, the edited fields it would set back, its arguments (folded), and the
 * three answers. It goes once answered or when its run ends.
 */
export function LocalApprovalCard({
  approval,
  onRespond,
}: {
  approval: LocalApproval;
  onRespond: (id: string, choice: AssistantApprovalChoice) => Promise<boolean>;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const respond = (choice: AssistantApprovalChoice) => {
    setBusy(true);
    void onRespond(approval.approvalId, choice).finally(() => setBusy(false));
  };
  const overwrites = approval.overwrites ?? [];
  return (
    <div
      role="group"
      aria-label={t("assistant.local.approval")}
      className="mt-2 space-y-2 rounded-list border border-line bg-card-2 px-3 py-2 text-ctl"
    >
      <p className="font-medium text-ink">
        {t(`assistant.local.operations.${approval.name}`, {
          defaultValue: approval.name,
        })}
      </p>
      {overwrites.length > 0 && (
        <div className="text-note text-ink-2">
          <p>{t("assistant.local.overwrites")}</p>
          <ul className="mt-1 list-disc space-y-0.5 break-all pl-4">
            {overwrites.map((field) => (
              <li key={field}>{field}</li>
            ))}
          </ul>
        </div>
      )}
      {approval.details && (
        <details className="text-note text-ink-muted">
          <summary className="cursor-default select-none">
            {t("assistant.local.details")}
          </summary>
          <pre className="mt-1 max-h-36 overflow-auto whitespace-pre-wrap break-words rounded-ctl bg-pill px-2 py-1 font-mono text-ink-2">
            {approval.details}
          </pre>
        </details>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          variant="secondary"
          disabled={busy}
          onClick={() => respond("once")}
        >
          {t("assistant.local.allowOnce")}
        </Button>
        {approval.allowSession && (
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => respond("session")}
          >
            {t("assistant.local.allowSession")}
          </Button>
        )}
        <Button variant="ghost" disabled={busy} onClick={() => respond("deny")}>
          {t("assistant.local.deny")}
        </Button>
      </div>
    </div>
  );
}

/**
 * Under the last reply of a local configuration conversation: `改动 N 个文件
 * · 还原` while some change is not restored. The count opens the file list.
 */
export function LocalChangesLine({
  changes,
  disabled,
  onRestore,
}: {
  changes: AssistantLocalChange[];
  disabled: boolean;
  onRestore: () => Promise<boolean>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const pending = changes.filter(
    (change) => change.changed && !change.restored,
  );
  if (pending.length === 0) return null;
  const files = [...new Set(pending.flatMap((change) => change.paths))];
  return (
    <div className="mt-1 text-note text-ink-muted">
      <div className="flex items-center gap-1">
        <Button
          variant="link"
          aria-expanded={open}
          className="h-auto px-0 text-note text-ink-muted"
          onClick={() => setOpen((value) => !value)}
        >
          {t("assistant.local.changed", { count: files.length })}
        </Button>
        <span aria-hidden>·</span>
        <Button
          variant="link"
          aria-label={t("assistant.local.restoreAll")}
          disabled={disabled || busy}
          className="h-auto px-0 text-note"
          onClick={() => {
            setBusy(true);
            void onRestore().finally(() => setBusy(false));
          }}
        >
          {t("assistant.local.restore")}
        </Button>
      </div>
      {open && (
        <ul className="mt-1 max-h-32 space-y-0.5 overflow-auto break-all">
          {files.map((path) => (
            <li key={path}>{path}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
