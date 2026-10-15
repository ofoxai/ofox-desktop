import { useTranslation } from "react-i18next";
import type { InstructionSet, LibraryData } from "@/lib/api/library";
import { ListCard, ListHead, ListRow } from "@/components/ui/list";
import { Button } from "@/components/ui/button";
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
            <ListRow key={target.toolId} className="flex-wrap items-start">
              <div className="min-w-0 flex-1 space-y-1 text-ctl">
                <p className="font-medium">
                  {tool} ·{" "}
                  {t(`library.status.${target.status}`, {
                    defaultValue: t("library.unknown"),
                  })}
                </p>
                <p className="break-all text-note text-ink-muted">
                  {target.path}
                </p>
                <p className="text-note text-ink-muted">
                  {t("library.instructionCounts", {
                    own: count(target.userContentLines),
                    managed: count(target.managedContentLines),
                  })}
                </p>
                <p className="break-words text-note">
                  {t("library.instructionSelection", {
                    desired: name(target.desiredResourceId),
                    readable,
                  })}
                </p>
                {target.lastAppliedRevision != null && (
                  <p className="text-note text-ink-muted">
                    {t("library.lastManagedRevision", {
                      revision: target.lastAppliedRevision,
                    })}
                  </p>
                )}
                {target.status === "shared" && (
                  <p className="text-note text-ink-muted">
                    {t("library.sharedReadable")}
                  </p>
                )}
                {target.readers && target.readers.length > 1 && (
                  <p className="text-note text-ink-muted">
                    {t("library.readers", {
                      tools: target.readers
                        .map((reader) => toolLabel(reader, t))
                        .join(", "),
                    })}
                  </p>
                )}
                {!target.available && (
                  <p className="text-note text-ink-muted">
                    {t("library.nativeCliRequired")}
                  </p>
                )}
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
