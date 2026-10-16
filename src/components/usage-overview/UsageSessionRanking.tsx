import { useEffect, useRef, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SegmentedControl } from "@/components/ui/segmented";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { getSessionAnalyticsPage } from "@/lib/api/usageOverview";
import { fmtTokens } from "@/lib/format";
import { formatActivity } from "@/lib/usageInsights";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import type {
  AnalyticsPage,
  AnalyticsSession,
  AnalyticsSort,
  SessionAnalytics,
} from "@/types/sessionAnalytics";

type Context = Pick<SessionAnalytics, "appliedQuery" | "snapshotVersion">;
type OpenRow = (
  row: AnalyticsSession,
  event: MouseEvent<HTMLButtonElement>,
  context: Context,
) => void;
export function UsageSessionRanking({
  data,
  active,
  onSelect,
}: {
  data: SessionAnalytics;
  active: boolean;
  onSelect: OpenRow;
}) {
  const { t } = useTranslation();
  const [sort, setSort] = useState<AnalyticsSort>("tokens");
  const [frozen, setFrozen] = useState<SessionAnalytics>();
  const current = frozen ?? data;
  const [page, setPage] = useState(0);
  const [draft, setDraft] = useState("");
  const [search, setSearch] = useState("");
  const [response, setResponse] = useState<AnalyticsPage>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (!active) setFrozen(undefined);
  }, [active]);
  useEffect(() => {
    if (!frozen?.analyticsSnapshotId || !active) return;
    let stale = false;
    setLoading(true);
    setError(false);
    setResponse(undefined);
    void getSessionAnalyticsPage({
      analyticsSnapshotId: frozen.analyticsSnapshotId,
      expectedReceipt: frozen,
      sortBy: sort,
      page,
      pageSize: 25,
      search,
    })
      .then((result) => {
        if (!stale && mounted.current) setResponse(result);
      })
      .catch(() => {
        if (!stale && mounted.current) setError(true);
      })
      .finally(() => {
        if (!stale && mounted.current) setLoading(false);
      });
    return () => {
      stale = true;
    };
  }, [frozen, sort, page, search, active]);
  const choices = (["tokens", "cost", "active"] as const).map((value) => ({
    value,
    label: t(`usageExperience.${value}`),
  }));
  const rows =
    data.compactTop?.[sort] ?? (sort === "tokens" ? data.topSessions : []);
  const renderRow = (row: AnalyticsSession, context: Context) => (
    <button
      key={row.sessionKey}
      type="button"
      aria-label={t("usageSessions.view", { title: row.title })}
      onClick={(event) => onSelect(row, event, context)}
      className="flex w-full min-w-0 items-center gap-2 rounded-md px-1.5 py-1 text-left hover:bg-pill focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand/50"
    >
      <ToolBadge toolId={row.toolId} size={20} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sub font-medium" title={row.title}>
          {row.title}
        </span>
        <span
          className="block truncate text-micro text-ink-muted"
          title={row.projectPath ?? row.projectLabel ?? undefined}
        >
          {row.projectLabel ||
            (row.projectId === "__unknown_project__"
              ? t("usageSessions.unknownProject")
              : row.projectId || t("usageSessions.projectUnparsed"))}{" "}
          ·{" "}
          {row.models
            .map((model) => model.model)
            .slice(0, 2)
            .join(", ") || "—"}
        </span>
      </span>
      <span
        className="max-w-[34%] shrink-0 truncate text-right text-sub tabular-nums"
        title={
          sort === "cost" ? (row.estimate.knownCostUsd ?? undefined) : undefined
        }
      >
        {sort === "active"
          ? formatActivity(row.activeMs, t)
          : sort === "cost"
            ? formatUsageEstimate(row.estimate)
            : row.tokens
              ? fmtTokens(row.tokens.totalTokens)
              : "—"}
      </span>
    </button>
  );
  return (
    <section
      className="flex min-w-0 flex-col overflow-hidden rounded-list border border-line bg-card px-3.5 pb-2.5 pt-3"
      aria-label={t("usageExperience.rankTitle")}
    >
      <div className="mb-2 flex min-h-5 flex-wrap items-center justify-between gap-2">
        <h2 className="text-note font-medium text-ink-muted">
          {t("usageExperience.rankTitle")}
        </h2>
        <SegmentedControl
          aria-label={t("usageExperience.sortLabel")}
          value={sort}
          onChange={setSort}
          options={choices}
        />
      </div>
      {rows.length ? (
        rows.slice(0, 6).map((row) => renderRow(row, data))
      ) : (
        <p className="py-5 text-sub text-ink-muted">
          {t(
            data.compactTop
              ? data.totals.sessionCount
                ? "usageExperience.rankNoSamples"
                : "usageSessions.empty"
              : "usageExperience.rankUnavailable",
          )}
        </p>
      )}
      {sort === "cost" && (
        <p className="pt-1.5 text-micro text-ink-faint">
          {t("usageExperience.costRankHint")}
        </p>
      )}
      {data.pagingUnavailableReason && (
        <p className="py-1.5 text-micro text-ink-muted">
          {t("usageExperience.pagingLimited")}
        </p>
      )}
      <Dialog
        open={active && !!frozen}
        onOpenChange={(open) => {
          if (!open) setFrozen(undefined);
        }}
      >
        {data.analyticsSnapshotId && (
          <DialogTrigger asChild>
            <Button
              variant="ghost"
              className="mt-1 self-start px-1 text-note"
              onClick={() => {
                setPage(0);
                setSearch("");
                setDraft("");
                setFrozen(data);
              }}
            >
              {t("usageExperience.viewAll")}
            </Button>
          </DialogTrigger>
        )}
        <DialogContent
          aria-describedby={undefined}
          className="max-w-3xl"
          onCloseAutoFocus={(event) => {
            if (!active) event.preventDefault();
          }}
        >
          <DialogHeader>
            <div className="flex items-center justify-between gap-3">
              <DialogTitle>{t("usageExperience.allSessions")}</DialogTitle>
              <Button variant="ghost" onClick={() => setFrozen(undefined)}>
                {t("usageWorkbench.close")}
              </Button>
            </div>
          </DialogHeader>
          <div className="min-h-0 overflow-y-auto space-y-3 p-4">
            <p className="text-note text-ink-muted">
              {t("usageExperience.snapshot")}
            </p>
            <SegmentedControl
              aria-label={t("usageExperience.sortLabel")}
              value={sort}
              onChange={(value) => {
                setSort(value);
                setPage(0);
              }}
              options={choices}
            />
            <form
              className="flex gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                setPage(0);
                setSearch(draft.trim());
              }}
            >
              <Input
                value={draft}
                maxLength={256}
                onChange={(event) => setDraft(event.target.value)}
                aria-label={t("usageExperience.listSearch")}
                placeholder={t("usageExperience.listSearch")}
              />
              <Button type="submit" variant="secondary">
                {t("common.search")}
              </Button>
            </form>
            <p className="text-note text-ink-faint">
              {t("usageExperience.listSearchHint")}
            </p>
            {loading && <p role="status">{t("common.loading")}</p>}
            {error && (
              <p role="alert" className="text-warn">
                {t("usageExperience.expired")}
              </p>
            )}
            {!loading && !error && response && (
              <>
                <div className="max-h-[50vh] overflow-auto rounded-lg border border-line-2">
                  {response.data.length ? (
                    response.data.map((row) => renderRow(row, current))
                  ) : (
                    <p className="p-5 text-sub text-ink-muted">
                      {t("usageSessions.empty")}
                    </p>
                  )}
                </div>
                <div className="flex items-center justify-between gap-2">
                  <Button
                    variant="secondary"
                    disabled={page === 0}
                    onClick={() => setPage((value) => value - 1)}
                  >
                    {t("usageExperience.previous")}
                  </Button>
                  <p className="text-note text-ink-muted">
                    {t("usageExperience.page", {
                      page: page + 1,
                      total: response.total,
                    })}
                  </p>
                  <Button
                    variant="secondary"
                    disabled={(page + 1) * 25 >= response.total}
                    onClick={() => setPage((value) => value + 1)}
                  >
                    {t("usageExperience.next")}
                  </Button>
                </div>
              </>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </section>
  );
}
