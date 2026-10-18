import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type {
  InstructionSet,
  McpDefinition,
  McpCheckResult,
  SkillDefinition,
  LibraryKind,
  ResourceTarget,
} from "@/lib/api/library";
import { InstructionEditor, SkillDetails } from "./ResourceEditors";
import { McpCheck } from "./McpCheck";
import { toolLabel } from "./libraryText";
import type { Resource } from "./LibrarySection";
import type { DraftStore } from "./useDraftState";
import type { LibraryEditorController } from "./useLibraryInline";
import type { LibraryNextAction } from "./libraryIssues";

export function LibraryInlineDetails({
  kind,
  row,
  store,
  epoch,
  stale,
  busy,
  targets,
  onSave,
  onEditor,
  onBusy,
  onCheck,
  onEdit,
  onNextAction,
  routingKey,
  onCollapse,
}: {
  routingKey: string;
  onCollapse: () => void;
  kind: LibraryKind;
  row: Resource;
  store: DraftStore;
  epoch: number;
  stale: boolean;
  busy: boolean;
  targets: ResourceTarget[];
  onSave: (set: InstructionSet) => Promise<boolean>;
  onEditor?: (controller: LibraryEditorController) => void;
  onBusy: (busy: boolean) => void;
  onCheck: (result: McpCheckResult, time: number) => void;
  onEdit: (trigger: HTMLElement) => void;
  onNextAction: (action: LibraryNextAction, trigger: HTMLElement) => void;
}) {
  const { t } = useTranslation();
  return (
    <section
      id={`library-inline-${row.id}`}
      role="region"
      aria-label={t("library.interaction.detailsNamed", { name: row.name })}
      className="space-y-3 border-t border-line-2 bg-card-2 px-4 py-4 min-[1000px]:pl-[62px]"
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.defaultPrevented) {
          event.stopPropagation();
          event.preventDefault();
          onCollapse();
        }
      }}
    >
      {stale && (
        <p role="alert" className="text-ctl text-warn">
          {t("library.revisionChanged")}
        </p>
      )}
      {kind === "instructions" ? (
        <InstructionEditor
          store={store}
          initial={row as InstructionSet}
          busy={busy || stale}
          onSave={(set) => onSave(set)}
          onController={onEditor}
        />
      ) : kind === "skills" ? (
        <SkillDetails
          store={store}
          skill={row as SkillDefinition}
          epoch={epoch}
          inline
        />
      ) : (
        <>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-ctl text-ink-2">
              {(row as McpDefinition).displaySummary ||
                (row as McpDefinition).serviceHost ||
                t(
                  (row as McpDefinition).transport === "stdio"
                    ? "library.workbench.localConnection"
                    : "library.workbench.remoteConnection",
                )}
            </p>
            <Button
              variant="secondary"
              disabled={busy || stale}
              onClick={(event) => onEdit(event.currentTarget)}
            >
              {t("library.edit")}
            </Button>
          </div>
          <p className="text-note text-ink-muted">
            {t("library.interaction.appliedTools", {
              tools:
                targets
                  .filter(
                    (target) => target.status === "applied" && target.desired,
                  )
                  .map((target) => toolLabel(target.toolId, t))
                  .join("、") || t("library.interaction.noneApplied"),
            })}
          </p>
          <McpCheck
            scopeKey={routingKey}
            store={store}
            server={row as McpDefinition}
            epoch={epoch}
            readOnly={busy || stale}
            onBusyChange={onBusy}
            onResult={onCheck}
            onNextAction={onNextAction}
          />
        </>
      )}
    </section>
  );
}
