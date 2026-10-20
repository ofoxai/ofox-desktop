import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { MenuSelect } from "@/components/ui/menu-select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  requestMeasurements,
  requestSourcePresentation,
} from "@/lib/usageRequestEvidence";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { UsageSourceAlert } from "./UsageSourceAlert";
import { UsageBars } from "./UsageBars";
import { usageIsUnavailable } from "@/lib/usageWorkbench";
import { UsageRequestMeasurements } from "./UsageRequestMeasurements";
import { RequestDetailPanel } from "@/components/usage/RequestDetailPanel";
import { queryClient } from "@/lib/query/queryClient";
import { usageApi } from "@/lib/api/usage";
import { fmtTokens } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import type {
  UsageQueryV1,
  UsageRequestFilters,
  UsageSnapshotReceipt,
  UsageFilterOptions,
} from "@/types/usageQuery";

export function UsageRequests({
  query,
  apex,
  active,
  sourceIds,
  updateVersion,
  canExport,
  onLatest,
  onOptions,
  recordBasis = "effective",
  onEffectiveRecords,
  onSavedHistory,
}: {
  query: UsageQueryV1;
  apex: string;
  active: boolean;
  sourceIds: string[];
  updateVersion: number;
  canExport: boolean;
  onLatest: () => void;
  onOptions?: (options: UsageFilterOptions) => void;
  recordBasis?: UsageRequestFilters["recordBasis"];
  onEffectiveRecords?: () => void;
  onSavedHistory?: (query: UsageQueryV1) => void;
}) {
  const { t, i18n } = useTranslation();
  const [draft, setDraft] = useState("");
  const [filters, setFilters] = useState<UsageRequestFilters>({});
  const [generation, setGeneration] = useState(0);
  const requestFilters = useMemo(
    () => ({
      ...filters,
      ...(recordBasis === "savedHistory" ? { recordBasis } : {}),
    }),
    [filters, recordBasis],
  );
  const scope = JSON.stringify([apex, query, requestFilters, generation]);
  const searchInput = useRef<HTMLInputElement>(null);
  const auditReturn = useRef<HTMLButtonElement>(null);
  const restoreSearch = useRef(false);
  useLayoutEffect(() => {
    if (!active) return;
    if (recordBasis === "savedHistory") auditReturn.current?.focus();
    else if (restoreSearch.current) {
      searchInput.current?.focus();
      restoreSearch.current = false;
    }
  }, [active, recordBasis]);
  const [paging, setPaging] = useState<{
    scope: string;
    page: number;
    snapshotId?: string;
    receipt?: UsageSnapshotReceipt;
  }>({ scope, page: 0 });
  const sameScope = paging.scope === scope;
  const page = sameScope ? paging.page : 0;
  const snapshotId = sameScope ? paging.snapshotId : undefined;
  const expectedReceipt = sameScope ? paging.receipt : undefined;
  const result = useQuery(
    {
      queryKey: ["usage", "workbench-logs", scope, page],
      queryFn: () =>
        usageApi.getRequestLogs({}, page, 20, {
          query,
          expectedApex: apex,
          requestFilters,
          ...(snapshotId ? { snapshotId } : {}),
          ...(expectedReceipt ? { expectedReceipt } : {}),
        }),
      enabled: active && (page === 0 || !!snapshotId),
      staleTime: Infinity,
      gcTime: 0,
      retry: false,
      refetchOnWindowFocus: false,
    },
    queryClient,
  );
  useEffect(() => {
    if (result.data?.snapshotId)
      setPaging((previous) =>
        previous.scope === scope &&
        previous.snapshotId === result.data!.snapshotId
          ? previous
          : {
              scope,
              page,
              snapshotId: result.data!.snapshotId,
              receipt: {
                queryFingerprint: result.data!.queryFingerprint!,
                snapshotVersion: result.data!.snapshotVersion!,
              },
            },
      );
  }, [result.data, scope, page]);
  useEffect(() => {
    if (result.data?.options) onOptions?.(result.data.options);
  }, [result.data, onOptions]);
  const [seenUpdate, setSeenUpdate] = useState(updateVersion);
  const [selected, setSelected] = useState<{
    id: string;
    snapshotId?: string;
    trigger: HTMLElement;
    apex: string;
    receipt: UsageSnapshotReceipt;
  }>();
  const [exportOpen, setExportOpen] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exportError, setExportError] = useState(false);
  const scopeOwner = useMemo(() => ({ scope, active }), [scope, active]);
  const exportOwner = useRef<object | null>(scopeOwner);
  exportOwner.current = scopeOwner;
  useEffect(() => {
    setExportOpen(false);
    setExporting(false);
    setExportError(false);
  }, [scopeOwner]);
  useEffect(
    () => () => {
      exportOwner.current = null;
    },
    [],
  );
  useEffect(() => setSelected(undefined), [apex]);
  useEffect(() => setSelected(undefined), [recordBasis]);
  const data = result.data;
  const pages = Math.max(1, Math.ceil((data?.total ?? 0) / 20));
  const summary = data?.summary;
  const noUsage = usageIsUnavailable(data);
  const latest = () => {
    setSeenUpdate(updateVersion);
    setGeneration((value) => value + 1);
    onLatest();
  };
  const exportCsv = async () => {
    if (!data?.snapshotId || exporting) return;
    const owner = scopeOwner;
    const frozen = data;
    setExporting(true);
    setExportError(false);
    try {
      const frozenReceipt = {
        queryFingerprint: frozen.queryFingerprint!,
        snapshotVersion: frozen.snapshotVersion!,
      };
      const preview = await usageApi.exportRequestLogsCsv(
        frozen.snapshotId!,
        undefined,
        frozenReceipt,
      );
      if (exportOwner.current !== owner) return;
      if (
        preview.queryFingerprint !== frozen.queryFingerprint ||
        preview.snapshotVersion !== frozen.snapshotVersion ||
        preview.recordCount !== frozen.total
      )
        throw new Error("usage.queryMismatch");
      const destination = await save({
        defaultPath: preview.fileName,
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (!destination || exportOwner.current !== owner) return;
      await usageApi.exportRequestLogsCsv(
        frozen.snapshotId!,
        destination,
        frozenReceipt,
      );
      if (exportOwner.current === owner) setExportOpen(false);
    } catch {
      if (exportOwner.current === owner) setExportError(true);
    } finally {
      if (exportOwner.current === owner) setExporting(false);
    }
  };
  return (
    <section
      aria-label={t("usageWorkbench.requestConditions")}
      className="space-y-3"
    >
      {recordBasis === "savedHistory" && (
        <div
          role="status"
          className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-line bg-card px-3 py-2 text-note text-ink-muted"
        >
          <p className="min-w-0 flex-1">
            {t("usageExperience.savedHistoryNotice")}
          </p>
          {onEffectiveRecords && (
            <Button
              ref={auditReturn}
              variant="secondary"
              onClick={() => {
                restoreSearch.current = true;
                onEffectiveRecords();
              }}
            >
              {t("usageExperience.effectiveRecords")}
            </Button>
          )}
        </div>
      )}
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          setFilters((value) => ({
            ...value,
            search: draft.trim() || undefined,
          }));
        }}
      >
        <Input
          ref={searchInput}
          aria-label={t("usageWorkbench.searchRequests")}
          placeholder={t("usageWorkbench.searchRequests")}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          className="min-w-[180px] max-w-[320px] flex-1"
        />
        <MenuSelect
          aria-label={t("usageWorkbench.source")}
          value={filters.sourceIds?.[0] ?? "all"}
          onChange={(value) =>
            setFilters((previous) => ({
              ...previous,
              sourceIds: value === "all" ? [] : [value],
            }))
          }
          options={[
            { value: "all", label: t("usageWorkbench.all") },
            ...sourceIds.map((value) => {
              const source = requestSourcePresentation(value, t);
              return {
                value,
                label: source.filterLabel,
                note: source.description,
              };
            }),
          ]}
        />
        <MenuSelect
          aria-label={t("usageWorkbench.status")}
          value={
            filters.statusCode != null
              ? String(filters.statusCode)
              : (filters.statusKind ?? "all")
          }
          onChange={(value) =>
            setFilters((previous) => ({
              ...previous,
              statusCode: /^\d+$/.test(value) ? Number(value) : undefined,
              statusKind:
                value === "recorded" || value === "unrecorded"
                  ? value
                  : undefined,
            }))
          }
          options={[
            { value: "all", label: t("usageWorkbench.allStatus") },
            { value: "recorded", label: t("usageWorkbench.recordedStatus") },
            {
              value: "unrecorded",
              label: t("usageWorkbench.unrecordedStatus"),
              note: t("usageWorkbench.noMeasuredStatusNote"),
            },
            ...[200, 400, 401, 403, 429, 500, 502, 503].map((code) => ({
              value: String(code),
              label: `HTTP ${code}`,
            })),
          ]}
        />
        <Button type="submit" variant="secondary">
          {t("usageWorkbench.applyFilters")}
        </Button>
        <Button
          type="button"
          variant="ghost"
          onClick={() => {
            setFilters({});
            setDraft("");
          }}
        >
          {t("usageWorkbench.reset")}
        </Button>
      </form>
      {updateVersion > seenUpdate && (
        <div
          role="status"
          className="flex flex-wrap items-center gap-2 rounded-card bg-pill px-3 py-2 text-note text-ink-muted"
        >
          <span>{t("usageWorkbench.newData")}</span>
          <Button variant="ghost" onClick={latest}>
            {t("usageWorkbench.loadLatest")}
          </Button>
        </div>
      )}
      <UsageSourceAlert pricing={data?.pricingCoverage} />
      {recordBasis === "effective" &&
        data?.sourceCoverage?.legacyHistoryAvailable &&
        data.appliedQuery &&
        onSavedHistory && (
          <Button
            variant="ghost"
            className="h-6 w-fit px-2 text-note"
            onClick={() => onSavedHistory(data.appliedQuery!)}
          >
            {t("usageExperience.savedHistory")}
          </Button>
        )}
      {summary && (
        <div className="flex flex-wrap gap-x-5 gap-y-1 rounded-card border border-line bg-card p-3 text-sub tabular-nums">
          <span>
            {t("usageWorkbench.records")}:{" "}
            {noUsage ? "—" : summary.requests.toLocaleString(i18n.language)}
          </span>
          <span>Tokens: {noUsage ? "—" : fmtTokens(summary.totalTokens)}</span>
          <span>
            {t("usageWorkbench.currentEstimate")}:{" "}
            {noUsage ? "—" : formatUsageEstimate(summary.estimate)}
          </span>
        </div>
      )}
      {data?.trend && !noUsage && (
        <section aria-label={t("usageWorkbench.requestTrend")}>
          <UsageBars overview={data.trend} />
        </section>
      )}
      {data?.measurements && (
        <UsageRequestMeasurements
          measurements={data.measurements}
          total={data.total}
        />
      )}
      {data && (!data.trend || !data.measurements) && (
        <p className="text-note text-ink-muted">
          {t("usageWorkbench.requestAnalyticsUnavailable")}
        </p>
      )}
      {result.isPending ? (
        <p role="status" className="p-6 text-center text-sub text-ink-muted">
          {t("common.loading")}
        </p>
      ) : result.isError ? (
        <div
          role="alert"
          className="space-y-2 rounded-card border border-warn/30 bg-warn-soft p-3 text-sub text-warn"
        >
          <p>
            {t(
              recordBasis === "savedHistory" &&
                String(result.error).includes("usage.queryUnsupported")
                ? "usageExperience.savedHistoryUnavailable"
                : "usageWorkbench.requestFailed",
            )}
          </p>
          <Button variant="ghost" onClick={() => void result.refetch()}>
            {t("usageWorkbench.retry")}
          </Button>
          <Button variant="ghost" onClick={latest}>
            {t("usageWorkbench.loadLatest")}
          </Button>
        </div>
      ) : (
        <>
          <TooltipProvider delayDuration={250}>
            <div className="overflow-hidden rounded-list border border-line bg-card">
              <div className="grid grid-cols-[100px_minmax(0,1fr)_110px_110px] gap-3 border-b border-line-2 px-3 py-2 text-note text-ink-muted">
                <span>{t("usageWorkbench.date")}</span>
                <span>{t("usage.model")}</span>
                <span className="text-right">Tokens</span>
                <span className="text-right">
                  {t("usageWorkbench.currentEstimate")}
                </span>
              </div>
              {!data?.data.length ? (
                <p className="p-7 text-center text-sub text-ink-muted">
                  {t(
                    noUsage
                      ? "usagePage.unavailable"
                      : data?.coverage?.state === "unsupported"
                        ? "usageWorkbench.queryUnavailable"
                        : "usageWorkbench.noRequests",
                  )}
                </p>
              ) : (
                data.data.map((record) => {
                  const source = requestSourcePresentation(
                    record.dataSource,
                    t,
                  );
                  const status = requestMeasurements(record).status;
                  const time = new Date(
                    record.createdAtMs ?? record.createdAt * 1000,
                  ).toLocaleString(i18n.language, {
                    month: "2-digit",
                    day: "2-digit",
                    hour: "2-digit",
                    minute: "2-digit",
                  });
                  return (
                    <Tooltip key={record.requestId}>
                      <TooltipTrigger asChild>
                        <button
                          type="button"
                          aria-label={t("usageWorkbench.viewRequestRecord", {
                            model: record.model,
                            time,
                          })}
                          className="grid w-full grid-cols-[100px_minmax(0,1fr)_110px_110px] items-center gap-3 border-b border-line-2 px-3 py-3 text-left last:border-b-0 hover:bg-pill focus-visible:relative focus-visible:z-[1] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand/50"
                          onClick={(event) =>
                            setSelected({
                              id: record.requestId,
                              snapshotId: data.snapshotId,
                              trigger: event.currentTarget,
                              apex,
                              receipt: {
                                queryFingerprint: data.queryFingerprint!,
                                snapshotVersion: data.snapshotVersion!,
                              },
                            })
                          }
                        >
                          <span className="text-note tabular-nums text-ink-muted">
                            {time}
                          </span>
                          <span className="flex min-w-0 items-center gap-2">
                            <ToolBadge toolId={record.appType} size={22} />
                            <span className="min-w-0">
                              <span
                                className="block truncate text-sub text-ink"
                                title={record.model}
                              >
                                {record.model}
                              </span>
                              <span
                                className="block truncate text-note text-ink-muted"
                                title={source.description}
                              >
                                {source.label}
                                {status != null && (
                                  <span
                                    className={
                                      status >= 400 ? "text-warn" : undefined
                                    }
                                  >
                                    {" · HTTP "}
                                    {status}
                                  </span>
                                )}
                              </span>
                            </span>
                          </span>
                          <span className="text-right tabular-nums">
                            <span className="block text-sub text-ink">
                              {fmtTokens(
                                record.inputTokens + record.outputTokens,
                              )}
                            </span>
                            <span className="block text-note text-ink-muted">
                              {fmtTokens(record.inputTokens)} /{" "}
                              {fmtTokens(record.outputTokens)}
                            </span>
                          </span>
                          <span
                            className="text-right text-sub tabular-nums text-ink"
                            title={
                              record.currentEstimate?.knownCostUsd ?? undefined
                            }
                          >
                            {formatUsageEstimate(record.currentEstimate)}
                          </span>
                        </button>
                      </TooltipTrigger>
                      <TooltipContent>{source.description}</TooltipContent>
                    </Tooltip>
                  );
                })
              )}
            </div>
          </TooltipProvider>
          <div className="flex flex-wrap items-center justify-between gap-2 text-note text-ink-muted">
            <span>
              {t("usageWorkbench.totalRecords", { count: data?.total ?? 0 })}
            </span>
            <div className="flex items-center gap-2">
              <Button
                variant="ghost"
                aria-label={t("usageWorkbench.previous")}
                disabled={page === 0}
                onClick={() =>
                  setPaging({
                    scope,
                    page: page - 1,
                    snapshotId: data?.snapshotId,
                    receipt: expectedReceipt,
                  })
                }
              >
                ←
              </Button>
              <span>{t("usageWorkbench.page", { page: page + 1, pages })}</span>
              <Button
                variant="ghost"
                aria-label={t("usageWorkbench.next")}
                disabled={page + 1 >= pages || !data?.snapshotId}
                onClick={() =>
                  setPaging({
                    scope,
                    page: page + 1,
                    snapshotId: data?.snapshotId,
                    receipt: expectedReceipt,
                  })
                }
              >
                →
              </Button>
              <Button
                variant="secondary"
                disabled={!canExport || !data?.total || !data?.snapshotId}
                onClick={() => {
                  setExportError(false);
                  setExportOpen(true);
                }}
              >
                {t("usageWorkbench.export")}
              </Button>
            </div>
          </div>
        </>
      )}
      {active && selected && selected.apex === apex && (
        <RequestDetailPanel
          requestId={selected.id}
          snapshotId={selected.snapshotId}
          expectedReceipt={selected.receipt}
          returnFocus={selected.trigger}
          onClose={() => setSelected(undefined)}
        />
      )}
      <Dialog
        open={active && exportOpen}
        onOpenChange={(open) => {
          if (!exporting) setExportOpen(open);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("usageWorkbench.exportTitle")}</DialogTitle>
            <DialogDescription>
              {t("usageWorkbench.exportDescription", {
                count: data?.total ?? 0,
              })}
            </DialogDescription>
          </DialogHeader>
          {exportError && (
            <p role="alert" className="px-4 py-2 text-sub text-warn">
              {t("usageWorkbench.exportFailed")}
            </p>
          )}
          <DialogFooter>
            <Button
              variant="ghost"
              disabled={exporting}
              onClick={() => setExportOpen(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button disabled={exporting} onClick={() => void exportCsv()}>
              {t("usageWorkbench.download")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
