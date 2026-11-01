import type { RefObject } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { AssistantRequestRecord } from "@/lib/api/assistant";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogClose,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

/** Saved facts only: opening this view performs no model or billing request. */
export function AssistantRequestDetails({
  requests,
  open,
  onOpenChange,
  returnFocusRef,
}: {
  requests: AssistantRequestRecord[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  returnFocusRef?: RefObject<HTMLElement>;
}) {
  const { t } = useTranslation();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="flex max-h-[85dvh] max-w-xl flex-col overflow-hidden"
        onCloseAutoFocus={(event) => {
          if (returnFocusRef?.current?.isConnected) {
            event.preventDefault();
            returnFocusRef.current.focus();
          }
        }}
      >
        <DialogHeader>
          <DialogTitle>{t("assistant.requestRecords")}</DialogTitle>
          <DialogDescription>
            {t("assistant.requestChargeUnknown")}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 space-y-3 overflow-y-auto px-4 py-3 text-note">
          {requests.length === 0 && <p>{t("assistant.requestRecordsEmpty")}</p>}
          {open &&
            requests.map((request, index) => (
              <section
                key={index}
                className="min-w-0 space-y-2 rounded-lg border border-line p-3"
              >
                <p className="break-words font-medium text-ink">
                  <span>
                    {request.modelId ?? t("assistant.requestUnknown")}
                  </span>{" "}
                  · <span>{request.apex ?? t("assistant.requestUnknown")}</span>
                </p>
                <p className="text-ink-2">
                  {t(`assistant.requestKind.${request.kind}`, {
                    defaultValue: request.kind,
                  })}{" "}
                  ·{" "}
                  {t(`assistant.requestStatus.${request.status}`, {
                    defaultValue: t("assistant.requestUnknown"),
                  })}
                  {request.httpStatus ? ` · HTTP ${request.httpStatus}` : ""}
                </p>
                <div className="min-w-0 text-ink-2">
                  <p>{t("assistant.requestIdentifier")}</p>
                  <p className="break-all font-mono select-text">
                    {request.requestId ?? t("assistant.requestNotReturned")}
                  </p>
                  {request.requestIdSource && (
                    <p className="break-all text-ink-muted">
                      {request.requestIdSource}
                    </p>
                  )}
                </div>
                {request.jobId && (
                  <p className="break-all text-ink-2">
                    {t("assistant.requestJobId")}: {request.jobId}
                  </p>
                )}
                {request.estimatedCostUsd != null && (
                  <p>
                    {t("assistant.requestEstimate", {
                      cost: request.estimatedCostUsd,
                    })}
                  </p>
                )}
                {request.reportedCostUsd != null && (
                  <p>
                    {t("assistant.requestReportedCost", {
                      cost: request.reportedCostUsd,
                    })}
                  </p>
                )}
                <details className="min-w-0">
                  <summary className="cursor-pointer rounded text-ink-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                    {t("assistant.requestFacts")}
                  </summary>
                  <pre className="mt-2 max-h-60 overflow-auto whitespace-pre-wrap break-all rounded bg-surface-tertiary p-2 text-note text-ink-2 select-text">
                    {JSON.stringify(
                      {
                        usage: request.usage ?? null,
                        priceSnapshot: request.priceSnapshot ?? null,
                      },
                      null,
                      2,
                    )}
                  </pre>
                </details>
              </section>
            ))}
        </div>
        <DialogFooter>
          <DialogClose asChild>
            <Button variant="secondary">{t("common.close")}</Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
