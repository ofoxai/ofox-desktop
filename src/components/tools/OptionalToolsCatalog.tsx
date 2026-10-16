import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  INSTALL_CATALOG,
  getCatalogInstallCommand,
  type InstallCatalogEntry,
} from "@/config/installCatalog";
import {
  getInstallInventory,
  type InstallInventoryEntry,
} from "@/lib/api/installCatalog";
import { settingsApi } from "@/lib/api";
import {
  isCatalogOperationRunning,
  observeCatalogOperations,
} from "@/lib/api/catalogLifecycle";
import { ListCard, ListFold, ListRow } from "@/components/ui/list";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Input } from "@/components/ui/input";
import { toast } from "sonner";
import { RefreshCw } from "lucide-react";
import { ToolBadge } from "./ToolBadge";
import { CatalogLifecycleDialog } from "./CatalogLifecycleDialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export interface InstallCatalogHandle {
  refresh: () => Promise<void>;
}
function invalidateActiveCopies(rows: InstallInventoryEntry[]) {
  if (!isCatalogOperationRunning()) return rows;
  return rows.map((row) =>
    row.toolId === "copilot" && row.managedCopy
      ? {
          ...row,
          managedCopy: {
            ...row.managedCopy,
            state: "needsReview" as const,
            version: null,
          },
        }
      : row,
  );
}
export const OptionalToolsCatalog = forwardRef<
  InstallCatalogHandle,
  { boundTools: string[] }
>(function OptionalToolsCatalog({ boundTools }, ref) {
  const { t } = useTranslation();
  const [inventory, setInventory] = useState<InstallInventoryEntry[]>([]);
  const [installedOpen, setInstalledOpen] = useState(false);
  const [catalogOpen, setCatalogOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [error, setError] = useState(false);
  const [busy, setBusy] = useState(true);
  const [management, setManagement] = useState<InstallCatalogEntry | null>(
    null,
  );
  const [managementBusy, setManagementBusy] = useState(false);
  const managementTrigger = useRef<HTMLElement | null>(null);
  const section = useRef<HTMLElement | null>(null);
  const generation = useRef(0);
  const inFlight = useRef<Promise<void> | null>(null);
  const refreshFailed = useRef(false);
  const mutationVersion = useRef(0);
  const mutationFlight = useRef<Promise<void> | null>(null);
  const catalogMounted = useRef(true);
  const mountEpoch = useRef(0);
  const completionRefresh = useRef(new Map<string, Promise<void>>());
  const refresh = useCallback(() => {
    if (inFlight.current) return inFlight.current;
    const run = ++generation.current;
    setBusy(true);
    const promise = (async () => {
      try {
        const result = await getInstallInventory();
        if (run !== generation.current) return;
        setInventory(invalidateActiveCopies(result));
        setError(false);
        refreshFailed.current = false;
      } catch {
        if (run === generation.current) {
          setError(true);
          refreshFailed.current = true;
        }
      } finally {
        if (run === generation.current) setBusy(false);
      }
    })();
    inFlight.current = promise;
    void promise.finally(() => {
      if (inFlight.current === promise) inFlight.current = null;
    });
    return promise;
  }, []);
  const refreshAfterMutation = useCallback(() => {
    const epoch = mountEpoch.current;
    if (!catalogMounted.current)
      return Promise.reject(new Error("Catalog closed"));
    mutationVersion.current += 1;
    generation.current += 1;
    if (mutationFlight.current) return mutationFlight.current;
    const promise = (async () => {
      for (;;) {
        const pending = inFlight.current;
        if (pending) await pending;
        if (!catalogMounted.current || epoch !== mountEpoch.current)
          throw new Error("Catalog closed");
        const version = mutationVersion.current;
        await refresh();
        if (version !== mutationVersion.current) continue;
        if (refreshFailed.current) throw new Error("Catalog refresh failed");
        return;
      }
    })();
    mutationFlight.current = promise;
    const release = () => {
      if (mutationFlight.current === promise) mutationFlight.current = null;
    };
    void promise.then(release, release);
    return promise;
  }, [refresh]);
  const refreshForOperation = useCallback(
    (id?: string) => {
      if (id && completionRefresh.current.has(id))
        return completionRefresh.current.get(id)!;
      return refreshAfterMutation();
    },
    [refreshAfterMutation],
  );
  useImperativeHandle(ref, () => ({ refresh }), [refresh]);
  useEffect(() => {
    catalogMounted.current = true;
    mountEpoch.current += 1;
    const epoch = mountEpoch.current;
    const stop = observeCatalogOperations((event) => {
      if (!catalogMounted.current || epoch !== mountEpoch.current) return;
      if (event.phase === "start") {
        generation.current += 1;
        setInventory(invalidateActiveCopies);
      } else {
        const refresh = refreshAfterMutation();
        completionRefresh.current.set(event.operationId, refresh);
        if (completionRefresh.current.size > 20)
          completionRefresh.current.delete(
            completionRefresh.current.keys().next().value!,
          );
        void refresh.catch(() => undefined);
      }
    });
    void refresh();
    return () => {
      stop();
      completionRefresh.current.clear();
      catalogMounted.current = false;
      mountEpoch.current += 1;
      generation.current += 1;
      inFlight.current = null;
    };
  }, [refresh, refreshAfterMutation]);
  const byId = new Map(inventory.map((r) => [r.toolId, r]));
  const extras = INSTALL_CATALOG.filter((r) => !boundTools.includes(r.toolId));
  const hasCopy = (id: string) => {
    const info = byId.get(id);
    return (
      info?.status === "installed" || info?.managedCopy?.state === "managed"
    );
  };
  const installed = extras.filter((r) => hasCopy(r.toolId));
  const candidates = extras.filter((r) => !hasCopy(r.toolId));
  const filtered = candidates.filter((r) =>
    r.label.toLowerCase().includes(query.toLowerCase()),
  );
  const open = (url: string) =>
    void settingsApi
      .openExternal(url)
      .catch(() => toast.error(t("shell.openFailed")));
  const row = (entry: InstallCatalogEntry, isInstalled: boolean) => {
    const info = byId.get(entry.toolId);
    const status = info?.status ?? "unknown";
    const install = getCatalogInstallCommand(entry);
    const dependency = install?.dependency ?? entry.dependency;
    return (
      <ListRow
        key={entry.toolId}
        data-tool-id={entry.toolId}
        role="group"
        aria-label={entry.label}
        className="grid grid-cols-[32px_minmax(0,1fr)_auto] gap-x-3 gap-y-2 py-3 md:grid-cols-[32px_190px_minmax(0,1fr)_136px]"
      >
        <ToolBadge toolId={entry.toolId} variant="plain" size={32} />
        <div className="min-w-0">
          <p className="text-ui font-medium text-ink">{entry.label}</p>
          {info?.managedCopy && (
            <>
              <p
                className={
                  info.managedCopy.state === "managed"
                    ? "text-note text-ink-muted"
                    : "text-note text-warn"
                }
              >
                {info.managedCopy.state === "managed"
                  ? t("toolCatalog.managedCopy", {
                      version: info.managedCopy.version ?? "—",
                    })
                  : t("toolCatalog.managedCopyNeedsReview")}
              </p>
              <p
                className="truncate text-note text-ink-faint"
                title={info.managedCopy.path}
              >
                {info.managedCopy.path}
              </p>
            </>
          )}
          {(!info?.managedCopy || status !== "notInstalled") && (
            <p className="text-note text-ink-muted">
              {info?.managedCopy
                ? t("toolCatalog.externalCopy", {
                    status: t(`toolCatalog.${status}`),
                  })
                : t(`toolCatalog.${status}`)}
              {info?.version ? ` · ${info.version}` : ""}
            </p>
          )}
          {info?.error && (
            <p className="text-note text-warn">
              {t("toolCatalog.probeIncomplete")}
            </p>
          )}
          {info?.path && (
            <p className="truncate text-note text-ink-faint" title={info.path}>
              {info.path}
            </p>
          )}
          {!isInstalled && dependency && (
            <p className="text-note text-ink-faint">
              {t(`toolCatalog.dependencies.${dependency}`)}
            </p>
          )}
        </div>
        <div className="col-span-2 col-start-2 flex min-w-0 items-center gap-2 md:col-span-1 md:col-start-3 md:row-start-1">
          {!isInstalled && install && (
            <>
              <code className="min-w-0 flex-1 whitespace-pre-wrap break-words rounded-ctl bg-canvas px-2 py-1 text-note">
                {install.command}
              </code>
              <CopyButton
                text={install.command}
                aria-label={t("toolCatalog.copyCommand", { name: entry.label })}
              />
            </>
          )}
          {!isInstalled && !install && (
            <p className="text-note text-ink-muted">
              {t(
                entry.downloadOnly
                  ? "toolCatalog.downloadOnly"
                  : "toolCatalog.platformGuide",
              )}
            </p>
          )}
        </div>
        <div className="col-start-3 row-start-1 flex flex-wrap items-center justify-end gap-1.5 md:col-start-4">
          {info?.error && (
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => void refresh()}
              aria-label={t("toolCatalog.retryFor", { name: entry.label })}
              title={t("toolCatalog.retryFor", { name: entry.label })}
            >
              <RefreshCw aria-hidden className="h-3.5 w-3.5" />
            </Button>
          )}
          <Button
            variant="ghost"
            onClick={() => open(entry.officialUrl)}
            aria-label={t("toolCatalog.officialFor", { name: entry.label })}
          >
            {t("toolCatalog.official")}
          </Button>
          {entry.toolId === "copilot" && (
            <Button
              variant="secondary"
              aria-label={t("catalogLifecycle.manage", { name: entry.label })}
              onClick={(event) => {
                managementTrigger.current = event.currentTarget;
                setManagement(entry);
              }}
            >
              {t("catalogLifecycle.manageShort")}
            </Button>
          )}
        </div>
      </ListRow>
    );
  };
  return (
    <section
      ref={section}
      tabIndex={-1}
      className="mt-4 space-y-2"
      aria-label={t("toolCatalog.title")}
    >
      <Dialog
        open={!!management}
        onOpenChange={(open) => {
          if (!open && !managementBusy) setManagement(null);
        }}
      >
        <DialogContent
          className="max-w-2xl"
          onEscapeKeyDown={(event) => {
            if (managementBusy) event.preventDefault();
          }}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            (managementTrigger.current?.isConnected
              ? managementTrigger.current
              : section.current
            )?.focus();
          }}
        >
          <DialogHeader>
            <DialogTitle>
              {t("catalogLifecycle.title", { name: management?.label ?? "" })}
            </DialogTitle>
            <DialogDescription>
              {t("catalogLifecycle.preservesData")}
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 overflow-auto px-[18px] py-4">
            {management && (
              <CatalogLifecycleDialog
                entry={management}
                onBusyChange={setManagementBusy}
                onChanged={refreshForOperation}
              />
            )}
          </div>
          <DialogFooter>
            <Button
              variant="secondary"
              disabled={managementBusy}
              onClick={() => setManagement(null)}
            >
              {t("common.close")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      {(installedOpen || catalogOpen) && busy && (
        <p role="status" className="text-note text-ink-muted">
          {t("common.loading")}
        </p>
      )}
      {(installedOpen || catalogOpen) && error && (
        <p role="alert" className="text-note text-warn">
          {t("toolCatalog.detectFailed")}
        </p>
      )}
      <ListFold
        open={installedOpen}
        onOpenChange={setInstalledOpen}
        aria-controls="extra-installed-tools"
        note={installed.length}
      >
        {t("toolCatalog.installedFold")}
      </ListFold>
      {installedOpen && (
        <ListCard id="extra-installed-tools">
          {installed.length ? (
            installed.map((r) => row(r, true))
          ) : busy || error ? null : (
            <p className="p-3 text-sub text-ink-muted">
              {t("toolCatalog.noInstalled")}
            </p>
          )}
        </ListCard>
      )}
      <ListFold
        open={catalogOpen}
        onOpenChange={setCatalogOpen}
        aria-controls="extra-install-catalog"
        note={candidates.length}
      >
        {t("toolCatalog.installFold")}
      </ListFold>
      {catalogOpen && (
        <div id="extra-install-catalog" className="space-y-2">
          <p className="text-note text-ink-muted">
            {t("toolCatalog.manualOnly")}
          </p>
          <div className="flex items-center gap-2">
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("toolCatalog.search")}
              aria-label={t("toolCatalog.search")}
            />
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => void refresh()}
            >
              {t(busy ? "common.loading" : "toolCatalog.refresh")}
            </Button>
          </div>
          <ListCard>
            {filtered.length ? (
              filtered.map((r) => row(r, false))
            ) : (
              <p className="p-3 text-sub text-ink-muted">
                {t("toolCatalog.noMatches")}
              </p>
            )}
          </ListCard>
        </div>
      )}
    </section>
  );
});
