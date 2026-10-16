import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  libraryApi,
  type LibraryInventory,
  type LibraryKind,
  type InventoryImport,
} from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ListCard, ListRow } from "@/components/ui/list";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { libraryError, toolLabel } from "./libraryText";

export function LibraryInventoryView({
  kind,
  epoch,
  onChanged,
  onBusyChange,
  onScanned,
  selectionHint,
}: {
  kind: LibraryKind;
  epoch: number;
  onChanged: () => Promise<void>;
  onBusyChange: (busy: boolean) => void;
  onScanned?: (inventory: LibraryInventory) => void;
  selectionHint?: { name: string; variant?: string | null; tools: string[] };
}) {
  const { t } = useTranslation();
  const [inventory, setInventory] = useState<LibraryInventory>();
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [result, setResult] = useState<InventoryImport>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  const running = useRef(false);
  const loaded = useRef(false);
  const callbacks = useRef({ onChanged, onBusyChange, onScanned });
  callbacks.current = { onChanged, onBusyChange, onScanned };
  const reportedBusy = useRef(false);
  const ticket = useRef<string>();
  const discarding = useRef(new Set<string>());
  function notifyBusy(value: boolean) {
    if (reportedBusy.current === value) return;
    reportedBusy.current = value;
    callbacks.current.onBusyChange(value);
  }
  function discard(ticketId: string) {
    if (discarding.current.has(ticketId)) return;
    discarding.current.add(ticketId);
    void libraryApi
      .discardInventory(ticketId)
      .catch(() => undefined)
      .finally(() => discarding.current.delete(ticketId));
  }
  useEffect(() => {
    alive.current = true;
    if (running.current) notifyBusy(true);
    return () => {
      alive.current = false;
      notifyBusy(false);
      if (ticket.current) discard(ticket.current);
    };
  }, []);
  async function scan() {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    notifyBusy(true);
    setError("");
    try {
      const data = await libraryApi.inventory(kind);
      if (alive.current) {
        if (ticket.current && ticket.current !== data.ticketId)
          discard(ticket.current);
        ticket.current = data.ticketId;
        setInventory(data);
        callbacks.current.onScanned?.(data);
        setSelected(
          new Set(
            selectionHint
              ? data.entries
                  .filter(
                    (entry) =>
                      entry.name === selectionHint.name &&
                      (entry.variant ?? null) ===
                        (selectionHint.variant ?? null) &&
                      JSON.stringify(
                        [
                          ...new Set(
                            entry.sources.map((source) => source.toolId),
                          ),
                        ].sort(),
                      ) ===
                        JSON.stringify(
                          [...new Set(selectionHint.tools)].sort(),
                        ),
                  )
                  .map((entry) => entry.entryId)
              : [],
          ),
        );
        setResult(undefined);
      } else discard(data.ticketId);
    } catch (e) {
      if (alive.current) setError(libraryError(e, t));
    } finally {
      running.current = false;
      if (alive.current) {
        setBusy(false);
        notifyBusy(false);
      }
    }
  }
  useEffect(() => {
    if (!loaded.current) {
      loaded.current = true;
      void scan();
    }
  }, []);
  async function importSelected() {
    if (running.current || !inventory || !selected.size) return;
    running.current = true;
    setBusy(true);
    notifyBusy(true);
    setError("");
    try {
      const receipt = await libraryApi.importInventory(
        inventory.ticketId,
        [...selected],
        epoch,
      );
      if (alive.current) {
        setResult(receipt);
        setSelected(new Set());
        await callbacks.current.onChanged();
      }
    } catch (e) {
      if (alive.current) setError(libraryError(e, t));
    } finally {
      running.current = false;
      if (ticket.current) discard(ticket.current);
      ticket.current = undefined;
      if (alive.current) {
        // Import consumes a scan ticket. A second import requires a fresh scan.
        setInventory(undefined);
        setSelected(new Set());
        setBusy(false);
        notifyBusy(false);
      }
    }
  }
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-ctl text-ink-muted">
          {t("library.workbench.importDrafts")}
        </p>
        <Button variant="secondary" disabled={busy} onClick={() => void scan()}>
          {t("library.workbench.rescan")}
        </Button>
      </div>
      {busy && (
        <p role="status" className="text-ctl text-ink-muted">
          {t("library.loading")}
        </p>
      )}
      {error && (
        <p role="alert" className="text-ctl text-danger">
          {error}
        </p>
      )}
      {inventory?.sources
        .filter((source) => !["ready", "missing"].includes(source.state))
        .map((source) => (
          <p key={source.sourceId} role="status" className="text-ctl text-warn">
            {toolLabel(source.toolId, t)} ·{" "}
            {t(`library.workbench.source.${source.label}`, {
              defaultValue: t("library.workbench.nativeSource"),
            })}
            :{" "}
            {source.reason
              ? libraryError(source.reason, t)
              : t("library.inventorySourceUnreadable")}
          </p>
        ))}
      {inventory && (
        <ListCard>
          {inventory.entries.map((entry) => (
            <ListRow key={entry.entryId} className="gap-3">
              <Checkbox
                aria-label={t("library.workbench.selectNamed", {
                  name: entry.name,
                })}
                checked={selected.has(entry.entryId)}
                disabled={busy}
                onCheckedChange={(checked) =>
                  setSelected((previous) => {
                    const next = new Set(previous);
                    if (checked) next.add(entry.entryId);
                    else next.delete(entry.entryId);
                    return next;
                  })
                }
              />
              <div className="min-w-0 flex-1">
                <p className="truncate text-ui font-medium" title={entry.name}>
                  {entry.name}
                </p>
                <p className="text-note text-ink-muted">
                  {entry.matchedResourceIds.length
                    ? t("library.workbench.alreadyLibrary")
                    : entry.conflict
                      ? t("library.workbench.importVariant")
                      : t("library.workbench.draftOnly")}
                  {entry.variant ? ` · ${entry.variant}` : ""}
                </p>
              </div>
              <div className="flex flex-wrap gap-1">
                {Array.from(
                  new Set(entry.sources.map((source) => source.toolId)),
                ).map((tool) => (
                  <span key={tool} title={toolLabel(tool, t)}>
                    <ToolBadge toolId={tool} size={22} variant="plain" />
                  </span>
                ))}
              </div>
            </ListRow>
          ))}
          {!inventory.entries.length && (
            <ListRow className="text-ctl text-ink-muted">
              {t("library.workbench.noNativeResources")}
            </ListRow>
          )}
        </ListCard>
      )}
      {result && (
        <div role="status" className="space-y-1 text-ctl">
          <p>{t("library.workbench.imported", { count: result.imported })}</p>
          {result.items
            .filter((item) => !["imported", "existing"].includes(item.status))
            .map((item) => (
              <p key={item.entryId} className="text-warn">
                {item.name}:{" "}
                {item.reason
                  ? libraryError(item.reason, t)
                  : t("library.inventoryChanged")}
              </p>
            ))}
        </div>
      )}
      <Button
        disabled={busy || !selected.size}
        onClick={() => void importSelected()}
      >
        {t("library.workbench.importSelected", { count: selected.size })}
      </Button>
    </div>
  );
}
