import { SkillApprovalCard } from "./SkillResultCards";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  AssistantGenerationApproval,
  AssistantSkillApproval,
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

/**
 * The composer's `权限：默认 ▾`: how much the assistant may do on this
 * computer without asking. Full auto asks first.
 */
export function PermissionControl({
  mode,
  disabled,
  onMode,
}: {
  mode: AssistantPermissionMode;
  disabled: boolean;
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
  return (
    <>
      <MenuSelect<AssistantPermissionMode>
        aria-label={t("assistant.local.permission")}
        value={mode}
        disabled={disabled || busy}
        onChange={(next) => {
          if (next === "fullAuto") setConfirmFull(true);
          else void choose(next);
        }}
        options={MODES.map((value) => ({
          value,
          label: t(`assistant.local.modes.${value}`),
          note: t(`assistant.local.modeNotes.${value}`),
        }))}
        valueLabel={t("assistant.local.permissionValue", {
          mode: t(`assistant.local.modes.${mode}`),
        })}
        width={300}
        align="start"
        triggerClassName={cn(
          "h-[27px] border-transparent bg-transparent px-2 text-ink-2 hover:bg-transparent hover:text-ink",
          mode === "fullAuto" && "text-warn",
        )}
      />
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
    </>
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
  if (
    (approval.category === "skillInstall" ||
      approval.category === "skillSave") &&
    typeof approval.details === "object"
  )
    return (
      <SkillApprovalCard
        details={approval.details as AssistantSkillApproval}
        install={approval.category === "skillInstall"}
        busy={busy}
        onRespond={respond}
      />
    );
  if (
    approval.category === "generation" &&
    typeof approval.details === "object"
  )
    return (
      <GenerationApproval
        details={approval.details as AssistantGenerationApproval}
        busy={busy}
        allowSession={approval.allowSession}
        onRespond={respond}
      />
    );
  const overwrites = approval.overwrites ?? [];
  const mcpDetails =
    approval.category === "mcp" &&
    typeof approval.details === "object" &&
    "tool" in approval.details
      ? approval.details
      : null;
  return (
    <div
      role="group"
      aria-label={t("assistant.local.approval")}
      className="mt-2 space-y-2 rounded-list border border-line bg-card-2 px-3 py-2 text-ctl"
    >
      <p className="font-medium text-ink">
        {approval.category === "mcp"
          ? t("assistant.mcp.approvalTitle")
          : t(`assistant.local.operations.${approval.name}`, {
              defaultValue: approval.name,
            })}
      </p>
      {mcpDetails && (
        <p className="break-words text-note text-ink-2">
          {mcpDetails.server ?? "MCP"} · {mcpDetails.tool}
        </p>
      )}
      {approval.category === "mcp" && (
        <p className="text-note text-ink-muted">
          {t("assistant.mcp.approval")}
        </p>
      )}
      {mcpDetails && typeof approval.argumentsPreview === "string" && (
        <details className="text-note text-ink-muted">
          <summary className="cursor-default select-none">
            {t("assistant.mcp.argumentsPreview")}
          </summary>
          <pre className="mt-1 max-h-36 overflow-auto whitespace-pre-wrap break-words rounded-ctl bg-pill px-2 py-1 font-mono text-ink-2">
            {approval.argumentsPreview}
          </pre>
        </details>
      )}
      {mcpDetails && approval.argumentsTruncated === true && (
        <p className="text-note text-ink-muted">
          {t("assistant.mcp.argumentsTruncated")}
        </p>
      )}
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
      {typeof approval.details === "string" && approval.details && (
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

/** A paid video the model is about to make: what, and what it costs. */
function GenerationApproval({
  details,
  busy,
  allowSession,
  onRespond,
}: {
  details: AssistantGenerationApproval;
  busy: boolean;
  allowSession: boolean;
  onRespond: (choice: AssistantApprovalChoice) => void;
}) {
  const { t } = useTranslation();
  const materials = details.materials;
  const files = materials?.firstFrame
    ? t(
        materials.lastFrame
          ? "assistant.generation.firstLastFrames"
          : "assistant.generation.firstFrame",
      )
    : materials?.references?.length
      ? t("assistant.generation.references", {
          count: materials.references.length,
        })
      : "";
  const line = [
    details.model,
    details.duration
      ? t("assistant.video.seconds", { count: details.duration })
      : "",
    details.resolution,
    details.aspectRatio,
    details.generateAudio === undefined
      ? ""
      : details.generateAudio
        ? t("assistant.generation.withSound")
        : t("assistant.generation.silent"),
    files,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <div
      role="group"
      aria-label={t("assistant.generation.confirmTitle")}
      className="mt-2 space-y-2 rounded-list border border-line bg-card-2 px-3 py-2 text-ctl"
    >
      <p className="font-medium text-ink">
        {t("assistant.generation.confirmTitle")}
      </p>
      <p className="text-note text-ink-2">{line}</p>
      <p className="font-medium tabular-nums text-ink">
        {details.estimatedCostUsd
          ? t("assistant.generation.estimate", {
              cost: details.estimatedCostUsd,
            })
          : t("assistant.generation.estimateUnknown")}
      </p>
      {details.prompt && (
        <details className="text-note text-ink-muted">
          <summary className="cursor-default select-none">
            {t("assistant.generation.prompt")}
          </summary>
          <p className="mt-1 whitespace-pre-wrap break-words text-ink-2">
            {details.prompt}
          </p>
        </details>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          variant="secondary"
          disabled={busy}
          onClick={() => onRespond("once")}
        >
          {t("assistant.generation.generate")}
        </Button>
        {allowSession && (
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => onRespond("session")}
          >
            {t("assistant.generation.allowSession")}
          </Button>
        )}
        <Button
          variant="ghost"
          disabled={busy}
          onClick={() => onRespond("deny")}
        >
          {t("assistant.generation.decline")}
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
