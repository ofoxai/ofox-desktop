import { useTranslation } from "react-i18next";
import type { InstructionSet, LibraryData } from "@/lib/api/library";
import { ListCard, ListHead, ListRow } from "@/components/ui/list";
import { Button } from "@/components/ui/button";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { toolLabel } from "./libraryText";

export function InstructionOverview({
  data,
  busy,
  onManage,
}: {
  data: LibraryData;
  busy: boolean;
  onManage: (set: InstructionSet, event: React.MouseEvent<HTMLElement>) => void;
}) {
  const { t } = useTranslation();
  const sets = new Map(data.instructions?.map((set) => [set.id, set]));
  const name = (id?: string | null) =>
    id
      ? (sets.get(id)?.name ?? t("library.definitionMissing", { id }))
      : t("library.noInstructionSet");
  const count = (value?: number | null) =>
    typeof value === "number" && Number.isSafeInteger(value) && value >= 0
      ? t("library.contentLines", { count: value })
      : t("library.unknown");
  return (
    <section
      aria-label={t("library.instructionOverview")}
      className="space-y-2"
    >
      <ListCard>
        <ListHead>{t("library.instructionOverview")}</ListHead>
        {data.targets?.map((target) => {
          const tool = toolLabel(target.toolId, t);
          const selected =
            target.desiredResourceId ?? target.readableResourceId;
          const set = selected ? sets.get(selected) : undefined;
          const unreadable = ["conflict", "unreadable"].includes(
            target.nativeState ?? "",
          );
          const readable = unreadable
            ? t("library.unknown")
            : target.readableResourceId
              ? name(target.readableResourceId)
              : ["unmanaged", "missing"].includes(target.nativeState ?? "")
                ? t("library.noInstructionSet")
                : t("library.unknown");
          return (
            <ListRow
              key={target.toolId}
              className="flex-wrap items-start gap-3 py-3"
            >
              <ToolBadge toolId={target.toolId} size={28} rounded="md" />
              <div className="min-w-0 flex-1 space-y-1 text-ctl">
                <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                  <span className="font-medium text-ink">{tool}</span>
                  <span className="rounded bg-pill px-1.5 py-0.5 text-note text-ink-muted">
                    {t(`library.status.${target.status}`, {
                      defaultValue: t("library.unknown"),
                    })}
                  </span>
                  <span className="text-note text-ink-muted">
                    {t("library.instructionExperience.ownSummary", {
                      lines: count(target.userContentLines),
                    })}
                  </span>
                </div>
                <p className="break-words text-note text-ink-muted">
                  {target.desiredResourceId &&
                  target.desiredResourceId === target.readableResourceId &&
                  !unreadable
                    ? t("library.interaction.currentInstruction", {
                        name: readable,
                      })
                    : t("library.instructionSelection", {
                        desired: name(target.desiredResourceId),
                        readable,
                      })}
                </p>
                {target.status === "shared" && (
                  <p className="text-note text-ink-muted">
                    {t("library.sharedReadable")}
                  </p>
                )}
                {!target.available && (
                  <p className="text-note text-ink-muted">
                    {t("library.nativeCliRequired")}
                  </p>
                )}
                <details className="pt-1 text-note text-ink-muted">
                  <summary className="w-fit cursor-pointer rounded outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-brand">
                    {t("library.instructionExperience.technicalDetails")}
                  </summary>
                  <div className="mt-2 space-y-1 border-l border-line-2 pl-3">
                    <p className="break-all font-mono">{target.path}</p>
                    <p>
                      {t("library.instructionCounts", {
                        own: count(target.userContentLines),
                        managed: count(target.managedContentLines),
                      })}
                    </p>
                    {target.lastAppliedRevision != null && (
                      <p>
                        {t("library.lastManagedRevision", {
                          revision: target.lastAppliedRevision,
                        })}
                      </p>
                    )}
                    {target.readers && target.readers.length > 1 && (
                      <p>
                        {t("library.readers", {
                          tools: target.readers
                            .map((reader) => toolLabel(reader, t))
                            .join(", "),
                        })}
                      </p>
                    )}
                  </div>
                </details>
              </div>
              {set && (
                <Button
                  variant="secondary"
                  disabled={busy}
                  aria-label={t("library.manageToolInstructions", { tool })}
                  onClick={(event) => onManage(set, event)}
                >
                  {t("library.assign")}
                </Button>
              )}
            </ListRow>
          );
        })}
      </ListCard>
      {data.detachedTargets?.map((target) => (
        <p
          role="status"
          key={target.path}
          className="break-all text-note text-warn"
        >
          {t("library.detachedInstruction", {
            tools: target.tools.map((tool) => toolLabel(tool, t)).join(", "),
            name: name(target.resourceId),
            path: target.path,
          })}
        </p>
      ))}
    </section>
  );
}
