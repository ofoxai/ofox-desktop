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
  type InstallCatalogEntry,
} from "@/config/installCatalog";
import {
  getInstallInventory,
  type InstallInventoryEntry,
} from "@/lib/api/installCatalog";
import { settingsApi } from "@/lib/api";
import { ListCard, ListFold, ListRow } from "@/components/ui/list";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Input } from "@/components/ui/input";
import { toast } from "sonner";
import { ToolBadge } from "./ToolBadge";

export interface InstallCatalogHandle {
  refresh: () => Promise<void>;
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
  const generation = useRef(0);
  const inFlight = useRef<Promise<void> | null>(null);
  const refresh = useCallback(() => {
    if (inFlight.current) return inFlight.current;
    const run = ++generation.current;
    setBusy(true);
    const promise = (async () => {
      try {
        const result = await getInstallInventory();
        if (run !== generation.current) return;
        setInventory(result);
        setError(false);
      } catch {
        if (run === generation.current) setError(true);
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
  useImperativeHandle(ref, () => ({ refresh }), [refresh]);
  useEffect(() => {
    void refresh();
    return () => {
      generation.current += 1;
      inFlight.current = null;
    };
  }, [refresh]);
  const byId = new Map(inventory.map((r) => [r.toolId, r]));
  const extras = INSTALL_CATALOG.filter((r) => !boundTools.includes(r.toolId));
  const installed = extras.filter(
    (r) => byId.get(r.toolId)?.status === "installed",
  );
  const candidates = extras.filter(
    (r) => byId.get(r.toolId)?.status !== "installed",
  );
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
    return (
      <ListRow
        key={entry.toolId}
        data-tool-id={entry.toolId}
        className="flex-wrap"
      >
        <ToolBadge toolId={entry.toolId} />
        <div className="min-w-0 flex-1 basis-48">
          <p className="text-ui font-medium text-ink">{entry.label}</p>
          <p className="text-note text-ink-muted">
            {t(`toolCatalog.${status}`)}
            {info?.version ? ` · ${info.version}` : ""}
          </p>
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
          {!isInstalled && entry.dependency && (
            <p className="text-note text-ink-faint">
              {t(`toolCatalog.dependencies.${entry.dependency}`)}
            </p>
          )}
        </div>
        {!isInstalled && entry.command && (
          <div className="flex min-w-0 flex-1 basis-72 items-center gap-2">
            <code className="min-w-0 flex-1 whitespace-pre-wrap break-words rounded-ctl bg-canvas px-2 py-1 text-note">
              {entry.command}
            </code>
            <CopyButton
              text={entry.command}
              aria-label={t("toolCatalog.copyCommand", { name: entry.label })}
            />
          </div>
        )}
        {info?.error && (
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => void refresh()}
            aria-label={t("toolCatalog.retryFor", { name: entry.label })}
          >
            {t("toolCatalog.refresh")}
          </Button>
        )}
        <Button
          variant="ghost"
          onClick={() => open(entry.officialUrl)}
          aria-label={t("toolCatalog.officialFor", { name: entry.label })}
        >
          {t("toolCatalog.official")}
        </Button>
      </ListRow>
    );
  };
  return (
    <section className="mt-4 space-y-2" aria-label={t("toolCatalog.title")}>
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
