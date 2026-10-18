import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { LibraryInventory } from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { ListCard, ListFold, ListRow } from "@/components/ui/list";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { inventoryEntryDescription, toolLabel } from "./libraryText";
import { LibraryResourceIcon } from "./LibraryResourceIcon";

type Entry = LibraryInventory["entries"][number];
export function nativeEntryMatches(
  entry: Entry,
  inventory: LibraryInventory,
  search: string,
  tool: string,
) {
  const text = [
    entry.name,
    entry.summary,
    entry.displaySummary,
    entry.serviceHost,
    ...entry.sources.map((source) =>
      toolLabel(source.toolId, (() => "") as never),
    ),
    ...entry.sources.map(
      (source) =>
        inventory.sources.find((value) => value.sourceId === source.sourceId)
          ?.displayPath,
    ),
  ]
    .join(" ")
    .toLocaleLowerCase();
  return (
    text.includes(search.trim().toLocaleLowerCase()) &&
    (tool === "all" || entry.sources.some((source) => source.toolId === tool))
  );
}
export function LibraryNativeSources({
  inventory,
  loading,
  failed,
  search,
  tool,
  status,
  busy,
  onRescan,
  onImport,
  onSource,
  onLocate,
  onCompare,
}: {
  inventory?: LibraryInventory;
  loading: boolean;
  failed: boolean;
  search: string;
  tool: string;
  status: string;
  busy: boolean;
  onRescan: () => void;
  onImport: (entry: Entry, trigger: HTMLElement) => void;
  onSource: (entry: Entry, sourceRef: string, trigger: HTMLElement) => void;
  onLocate: (entry: Entry, id: string) => void;
  onCompare: (entries: Entry[], trigger: HTMLElement) => void;
}) {
  const { t } = useTranslation();
  const [showMatched, setShowMatched] = useState(false);
  const entries = (inventory?.entries ?? []).filter(
    (entry) =>
      nativeEntryMatches(entry, inventory!, search, tool) &&
      status !== "applied" &&
      (status !== "attention" || entry.conflict),
  );
  const incomplete = inventory?.sources.some(
    (source) => !["ready", "missing"].includes(source.state),
  );
  const filtering = !!search.trim() || tool !== "all" || status !== "all";
  const unmatched = entries.filter((entry) => !entry.matchedResourceIds.length),
    matched = entries.filter((entry) => entry.matchedResourceIds.length);
  const groups = new Map<string, Entry[]>();
  for (const entry of unmatched) {
    const key = entry.name.toLocaleLowerCase();
    groups.set(key, [...(groups.get(key) ?? []), entry]);
  }
  function row(entry: Entry) {
    return (
      <ListRow key={entry.entryId} className="flex-wrap gap-3 py-3">
        <LibraryResourceIcon
          kind={entry.kind}
          recipeId={entry.iconId ?? undefined}
          className="h-9 w-9"
        />
        <div className="min-w-0 flex-1 basis-48">
          <p className="truncate text-ui font-medium" title={entry.name}>
            {entry.name}
          </p>
          <p
            className="truncate text-note text-ink-muted"
            title={
              entry.displaySummary ||
              entry.summary ||
              entry.serviceHost ||
              inventoryEntryDescription(inventory!, entry, t)
            }
          >
            {entry.displaySummary ||
              entry.summary ||
              entry.serviceHost ||
              inventoryEntryDescription(inventory!, entry, t)}
          </p>
        </div>
        <div
          className="flex flex-wrap gap-1"
          aria-label={t("library.workbench2.sourceDetails")}
        >
          {entry.sources.map((reference, index) => {
            const source = inventory!.sources.find(
              (source) =>
                source.sourceId === reference.sourceId &&
                source.toolId === reference.toolId,
            );
            return (
              <Button
                key={`${reference.sourceId}:${index}`}
                size="icon"
                variant="ghost"
                aria-label={t("library.workbench2.sourceNamed", {
                  name: entry.name,
                  tool: toolLabel(reference.toolId, t),
                })}
                title={t(
                  source?.sourceRef
                    ? "library.workbench2.observedHint"
                    : "library.workbench2.unsupported",
                )}
                disabled={busy}
                aria-disabled={!source?.sourceRef}
                aria-description={
                  !source?.sourceRef
                    ? t("library.workbench2.unsupported")
                    : undefined
                }
                onClick={(event) =>
                  source?.sourceRef &&
                  onSource(entry, source.sourceRef, event.currentTarget)
                }
              >
                <ToolBadge
                  toolId={reference.toolId}
                  size={22}
                  variant="plain"
                />
              </Button>
            );
          })}
        </div>
        {entry.matchedResourceIds.length ? (
          entry.matchedResourceIds.map((id) => (
            <Button
              key={id}
              variant="ghost"
              aria-label={t("library.workbench2.locateNamed", {
                name: entry.name,
              })}
              onClick={() => onLocate(entry, id)}
            >
              {t("library.interaction.viewResource")}
            </Button>
          ))
        ) : (
          <Button
            variant="secondary"
            disabled={busy}
            aria-label={t("library.interaction.collectNamed", {
              name: entry.name,
            })}
            onClick={(event) => onImport(entry, event.currentTarget)}
          >
            {t("library.interaction.collect")}
          </Button>
        )}
      </ListRow>
    );
  }
  return (
    <section
      role="region"
      aria-label={t("library.workbench2.native")}
      className="space-y-2"
    >
      <div className="flex flex-wrap items-center justify-between gap-2 px-1 pt-2">
        <h3 className="text-ctl font-medium">
          {t("library.workbench2.native")}
        </h3>
        <Button variant="ghost" disabled={loading || busy} onClick={onRescan}>
          {t("library.workbench2.scan")}
        </Button>
      </div>
      {loading && (
        <p role="status" className="px-1 text-note text-ink-muted">
          {t("library.workbench2.scanning")}
        </p>
      )}
      {failed && (
        <p role="status" className="px-1 text-note text-warn">
          {t("library.workbench2.scanFailed")}
        </p>
      )}
      {inventory && (
        <>
          {(unmatched.length > 0 ||
            (!matched.length && (!incomplete || filtering))) && (
            <ListCard>
              {[...groups].map(([name, variants]) =>
                variants.length === 1 ? (
                  row(variants[0])
                ) : (
                  <div key={name}>
                    <details open={!!search}>
                      <summary className="cursor-pointer px-4 py-2 text-ctl font-medium">
                        {t("library.workbench2.variants", {
                          name: variants[0].name,
                          count: variants.length,
                        })}
                      </summary>
                      {variants.map(row)}
                      <div className="px-4 pb-2">
                        <Button
                          variant="ghost"
                          onClick={(event) =>
                            onCompare(variants, event.currentTarget)
                          }
                        >
                          {t("library.workbench2.compare")}
                        </Button>
                      </div>
                    </details>
                  </div>
                ),
              )}
              {!unmatched.length && (
                <ListRow className="text-note text-ink-muted">
                  {t(
                    filtering
                      ? "library.noMatch"
                      : "library.workbench.noNativeResources",
                  )}
                </ListRow>
              )}
            </ListCard>
          )}
          {matched.length > 0 && (
            <>
              <ListFold
                open={showMatched || !!search}
                onOpenChange={setShowMatched}
              >
                {t("library.workbench2.matched", { count: matched.length })}
              </ListFold>
              {(showMatched || search) && (
                <ListCard>{matched.map(row)}</ListCard>
              )}
            </>
          )}
          {inventory.sources
            .filter((source) => !["ready", "missing"].includes(source.state))
            .map((source) => (
              <p
                key={source.sourceId}
                role="status"
                className="px-1 text-note text-warn"
              >
                {toolLabel(source.toolId, t)} ·{" "}
                {t(`library.workbench.source.${source.label}`, {
                  defaultValue: t("library.workbench.nativeSource"),
                })}{" "}
                · {t("library.workbench2.sourceUnconfirmed")}
              </p>
            ))}
        </>
      )}
    </section>
  );
}
