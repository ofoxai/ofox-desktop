import { useTranslation } from "react-i18next";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { useRequestDetail } from "@/lib/query/usage";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import type { UsageSnapshotReceipt } from "@/types/usageQuery";

export function RequestDetailPanel({
  requestId,
  snapshotId,
  returnFocus,
  expectedReceipt,
  onClose,
}: {
  requestId: string;
  snapshotId?: string;
  returnFocus?: HTMLElement;
  expectedReceipt?: UsageSnapshotReceipt;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const result = useRequestDetail(requestId, snapshotId, expectedReceipt);
  const request = result.data;
  const imported = request?.dataSource && request.dataSource !== "proxy";
  const status =
    request && "observedStatusCode" in request
      ? request.observedStatusCode
      : imported
        ? null
        : request?.statusCode;
  const latency =
    request && "observedLatencyMs" in request
      ? request.observedLatencyMs
      : imported
        ? null
        : request?.latencyMs;
  const firstToken = request?.observedFirstTokenMs;
  const number = (value: number) => value.toLocaleString(i18n.language);
  const fields = request
    ? [
        [
          t("usage.time"),
          new Date(request.createdAt * 1000).toLocaleString(i18n.language),
        ],
        [t("usage.appType"), request.appType],
        [t("usage.provider"), request.providerName ?? request.providerId],
        [t("usage.model"), request.model],
        ...(request.requestModel
          ? [[t("usageWorkbench.requestModel"), request.requestModel]]
          : []),
        [t("usageWorkbench.source"), request.dataSource ?? "proxy"],
        [t("usageWorkbench.status"), status ?? t("usageWorkbench.notRecorded")],
        [t("usage.inputTokens"), number(request.inputTokens)],
        [t("usage.outputTokens"), number(request.outputTokens)],
        [t("usage.cacheReadTokens"), number(request.cacheReadTokens)],
        [t("usage.cacheCreationTokens"), number(request.cacheCreationTokens)],
        [t("usage.costMultiplier"), request.costMultiplier],
        [t("usageWorkbench.inputAmount"), `$${request.inputCostUsd}`],
        [t("usageWorkbench.outputAmount"), `$${request.outputCostUsd}`],
        [t("usageWorkbench.cacheReadAmount"), `$${request.cacheReadCostUsd}`],
        [t("usage.cacheCreationCost"), `$${request.cacheCreationCostUsd}`],
        [t("usageWorkbench.savedAmount"), `$${request.totalCostUsd}`],
        [
          t("usageWorkbench.currentEstimate"),
          formatUsageEstimate(request.currentEstimate),
        ],
        [
          t("usage.latency"),
          latency == null ? t("usageWorkbench.notRecorded") : `${latency} ms`,
        ],
        [
          t("usageWorkbench.firstToken"),
          firstToken == null
            ? t("usageWorkbench.notRecorded")
            : `${firstToken} ms`,
        ],
      ]
    : [];
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        className="max-w-2xl"
        aria-describedby={undefined}
        onCloseAutoFocus={(event) => {
          if (returnFocus?.isConnected) {
            event.preventDefault();
            returnFocus.focus();
          }
        }}
      >
        <DialogHeader>
          <div className="flex items-center justify-between gap-3">
            <DialogTitle>{t("usageWorkbench.detail")}</DialogTitle>
            <Button variant="ghost" onClick={onClose}>
              {t("usageWorkbench.close")}
            </Button>
          </div>
        </DialogHeader>
        <div className="max-h-[70vh] overflow-y-auto p-4">
          {result.isPending ? (
            <p role="status">{t("common.loading")}</p>
          ) : result.isError ? (
            <div role="alert" className="space-y-2 text-sub text-warn">
              <p>{t("usageWorkbench.detailFailed")}</p>
              <Button variant="ghost" onClick={() => void result.refetch()}>
                {t("usageWorkbench.retry")}
              </Button>
            </div>
          ) : !request ? (
            <p className="text-sub text-ink-muted">
              {t("usageWorkbench.notFound")}
            </p>
          ) : (
            <>
              <div className="mb-4 flex min-w-0 items-center gap-2">
                <code className="min-w-0 flex-1 break-all text-note text-ink-muted">
                  {requestId}
                </code>
                <CopyButton
                  text={requestId}
                  aria-label={t("usage.requestId")}
                />
              </div>
              <dl className="grid grid-cols-2 gap-x-5 gap-y-4 text-sub">
                {fields.map(([label, value]) => (
                  <div key={label} className="min-w-0">
                    <dt className="text-note text-ink-muted">{label}</dt>
                    <dd className="break-words tabular-nums text-ink">
                      {value}
                    </dd>
                  </div>
                ))}
              </dl>
              {request.currentEstimate?.additionalUnitsUnsupported && (
                <p className="mt-4 text-note text-ink-muted">
                  {t("usageWorkbench.unsupportedUnits")}
                </p>
              )}
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
