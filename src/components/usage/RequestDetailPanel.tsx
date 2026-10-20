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
import { TOOL_META } from "@/config/toolMeta";
import {
  requestMeasurements,
  requestSourcePresentation,
} from "@/lib/usageRequestEvidence";
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
  const evidence = request ? requestMeasurements(request) : null;
  const source = requestSourcePresentation(request?.dataSource, t);
  const number = (value: number) => value.toLocaleString(i18n.language);
  const money = (value: string | null | undefined) =>
    value == null ? t("usageWorkbench.notProvided") : `$${value}`;
  const savedAmount = request
    ? request.recordedCostUsd === undefined
      ? request.totalCostUsd
      : request.recordedCostUsd
    : null;
  const missingTiming = evidence
    ? [
        ...(evidence.latency == null ? [t("usageWorkbench.elapsed")] : []),
        ...(evidence.firstToken == null
          ? [t("usageWorkbench.firstToken")]
          : []),
      ]
    : [];
  const fields =
    request && evidence
      ? [
          [
            t("usage.time"),
            new Date(
              request.createdAtMs ?? request.createdAt * 1000,
            ).toLocaleString(i18n.language),
          ],
          [
            t("usage.appType"),
            TOOL_META[request.appType]?.label ?? t("usageWorkbench.otherTool"),
          ],
          [t("usage.model"), request.model],
          [t("usageWorkbench.source"), source.label],
          ...(request.requestModel
            ? [[t("usageWorkbench.requestModel"), request.requestModel]]
            : []),
          ...(evidence.status != null
            ? [[t("usageWorkbench.status"), `HTTP ${evidence.status}`]]
            : []),
          [t("usage.inputTokens"), number(request.inputTokens)],
          [t("usage.outputTokens"), number(request.outputTokens)],
          [t("usage.cacheReadTokens"), number(request.cacheReadTokens)],
          [t("usage.cacheCreationTokens"), number(request.cacheCreationTokens)],
          [
            t("usageWorkbench.currentEstimate"),
            formatUsageEstimate(request.currentEstimate),
          ],
          ...(evidence.latency != null
            ? [[t("usageWorkbench.elapsed"), `${number(evidence.latency)} ms`]]
            : []),
          ...(evidence.firstToken != null
            ? [
                [
                  t("usageWorkbench.firstToken"),
                  `${number(evidence.firstToken)} ms`,
                ],
              ]
            : []),
        ]
      : [];
  const technicalFields = request
    ? [
        [
          t("usageWorkbench.sourceId"),
          request.dataSource ?? t("usageWorkbench.notProvided"),
        ],
        [t("usage.appType"), request.appType],
        [t("usage.provider"), request.providerName ?? request.providerId],
        [t("usage.costMultiplier"), request.costMultiplier],
        ...(request.recordedCostUsd === null
          ? []
          : [
              [t("usageWorkbench.inputAmount"), money(request.inputCostUsd)],
              [t("usageWorkbench.outputAmount"), money(request.outputCostUsd)],
              [
                t("usageWorkbench.cacheReadAmount"),
                money(request.cacheReadCostUsd),
              ],
              [
                t("usage.cacheCreationCost"),
                money(request.cacheCreationCostUsd),
              ],
            ]),
        [t("usageWorkbench.savedAmount"), money(savedAmount)],
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
              <p className="mt-3 text-note text-ink-muted">
                {source.description}
              </p>
              <p className="mt-1 text-note text-ink-muted">
                {t("usageWorkbench.estimateNotBill")}
              </p>
              {(evidence?.status == null || missingTiming.length > 0) && (
                <div className="mt-3 space-y-1 rounded-lg bg-pill px-3 py-2 text-note text-ink-muted">
                  {evidence?.status == null && (
                    <p>{t("usageWorkbench.originalStatusMissing")}</p>
                  )}
                  {missingTiming.length > 0 && (
                    <p>
                      {t("usageWorkbench.originalMeasurementsMissing", {
                        fields: missingTiming.join(
                          t("usageWorkbench.measurementSeparator"),
                        ),
                      })}
                    </p>
                  )}
                </div>
              )}
              {request.dataSource === "opencode_session" &&
                evidence?.latency != null && (
                  <p className="mt-2 text-note text-ink-muted">
                    {t("usageWorkbench.nativeElapsedHelp")}
                  </p>
                )}
              <details className="mt-4 border-t border-line-2 pt-3">
                <summary className="w-fit cursor-pointer rounded-sm text-sub text-ink-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand/50">
                  {t("usageWorkbench.technicalDetails")}
                </summary>
                <div className="mt-3 flex min-w-0 items-center gap-2">
                  <code className="min-w-0 flex-1 break-all text-note text-ink-muted">
                    {requestId}
                  </code>
                  <CopyButton
                    text={requestId}
                    aria-label={t("usage.requestId")}
                  />
                </div>
                <dl className="mt-3 grid grid-cols-2 gap-x-5 gap-y-3 text-sub">
                  {technicalFields.map(([label, value]) => (
                    <div key={label} className="min-w-0">
                      <dt className="text-note text-ink-muted">{label}</dt>
                      <dd className="break-words tabular-nums text-ink">
                        {value}
                      </dd>
                    </div>
                  ))}
                </dl>
                <p className="mt-2 text-note text-ink-muted">
                  {t("usageWorkbench.savedAmountHelp")}
                </p>
              </details>
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
