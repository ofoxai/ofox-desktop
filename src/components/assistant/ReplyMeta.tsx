import { useRef, useState } from "react";
import { AssistantRequestDetails } from "./AssistantRequestDetails";
import { Info } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AssistantMessage, AssistantModel } from "@/lib/api/assistant";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { fmtTokens } from "@/lib/format";
import {
  fmtDuration,
  fmtUsd,
  replyDuration,
  usageBreakdown,
} from "@/utils/assistantUsage";

/** A model as the picker names it; the id of one the catalog no longer lists. */
function modelName(models: ReadonlyArray<AssistantModel>, id: string) {
  const model = models.find((item) => item.id === id);
  return {
    label: model ? model.shortName || model.name : id,
    full: model?.name ?? id,
  };
}

/**
 * `model · 14:32 ⓘ` under a reply: the model it was asked of, when it ended
 * (when it began, for a reply saved without an end), and a popover with what
 * the run used and cost, line by line as ofox-agent shows it (an estimate at
 * the prices frozen when each request was sent), and how long it took. `onOpenChange` tells the row
 * the popover is open, so the row stays in view while the pointer is on it.
 */
export function ReplyMeta({
  message,
  models,
  onOpenChange,
}: {
  message: AssistantMessage;
  models: ReadonlyArray<AssistantModel>;
  onOpenChange?: (open: boolean) => void;
}) {
  const { t, i18n } = useTranslation();
  const metaTrigger = useRef<HTMLButtonElement>(null);
  const [requestsOpen, setRequestsOpen] = useState(false);
  const requests = message.usage?.requests ?? [];
  const model = message.modelId ? modelName(models, message.modelId) : null;
  const at = message.completedAt ?? message.createdAt;
  const date = new Date(at);
  const duration = replyDuration(message);
  // A reply that has ended: its cost is settled.
  const breakdown = message.usage ? usageBreakdown(message.usage) : null;
  const costShown =
    breakdown !== null &&
    (breakdown.lines.length > 0 ||
      breakdown.search !== null ||
      breakdown.generations.length > 0 ||
      breakdown.total !== null ||
      (message.usage?.modelRequests ?? 0) > 0);
  return (
    <>
      <span className="flex min-w-0 items-center gap-1 text-note text-ink-ctl">
        {model && (
          <>
            <span className="min-w-0 truncate" title={model.full}>
              {model.label}
            </span>
            <span aria-hidden>·</span>
          </>
        )}
        <time
          dateTime={date.toISOString()}
          title={date.toLocaleString(i18n.language)}
          className="shrink-0 tabular-nums"
        >
          {date.toLocaleTimeString(i18n.language, {
            hour: "2-digit",
            minute: "2-digit",
          })}
        </time>
      </span>
      {(costShown || duration !== null || requests.length > 0) && (
        <Popover onOpenChange={onOpenChange}>
          <PopoverTrigger asChild>
            <Button
              ref={metaTrigger}
              variant="icon"
              className="h-6 w-6 shrink-0"
              aria-label={t("assistant.replyDetails")}
              title={t("assistant.replyDetails")}
            >
              <Info aria-hidden className="h-3.5 w-3.5" />
            </Button>
          </PopoverTrigger>
          <PopoverContent
            align="start"
            aria-label={t("assistant.replyDetails")}
            className="w-72 max-w-[calc(100vw-32px)] overflow-hidden p-0 text-note text-ink-2"
          >
            {message.modelId && (
              <p className="break-all border-b border-line px-3 py-2 text-ink-muted">
                {message.modelId}
              </p>
            )}
            {costShown && breakdown && (
              <>
                {(breakdown.lines.length > 0 || breakdown.search) && (
                  <div className="space-y-1 px-3 py-2">
                    {breakdown.chat !== undefined && (
                      <div
                        data-usage-line
                        className="flex items-baseline gap-3 font-medium text-ink"
                      >
                        <span className="flex-1">
                          {t("assistant.usageLines.chat")}
                        </span>
                        <span className="w-20 text-right tabular-nums">
                          {fmtUsd(breakdown.chat)}
                        </span>
                      </div>
                    )}
                    {breakdown.lines.map((line) => (
                      <div
                        key={line.key}
                        data-usage-line
                        className="flex items-baseline gap-3"
                      >
                        <span className="flex-1">
                          {t(`assistant.usageLines.${line.key}`)}
                        </span>
                        <span className="tabular-nums text-ink">
                          {fmtTokens(line.tokens)}
                        </span>
                        <span className="w-20 text-right tabular-nums text-ink-muted">
                          {fmtUsd(line.usd)}
                        </span>
                      </div>
                    ))}
                    {breakdown.search && (
                      <div
                        data-usage-line
                        className="flex items-baseline gap-3"
                      >
                        <span className="flex-1">
                          {t("assistant.usageLines.search", {
                            count: breakdown.search.count,
                          })}
                        </span>
                        <span className="w-20 text-right tabular-nums text-ink-muted">
                          {fmtUsd(breakdown.search.usd)}
                        </span>
                      </div>
                    )}
                  </div>
                )}
                {breakdown.generations.length > 0 && (
                  <div className="space-y-1 border-t border-line px-3 py-2">
                    {breakdown.generations.map((line) => (
                      <div
                        key={line.key}
                        data-usage-line
                        className="flex items-baseline gap-3"
                      >
                        <span className="min-w-0 flex-1 truncate">
                          {[
                            t(`assistant.generation.kinds.${line.kind}`, {
                              defaultValue: line.kind,
                            }),
                            line.modelId
                              ? modelName(models, line.modelId).label
                              : "",
                          ]
                            .filter(Boolean)
                            .join(" · ")}
                        </span>
                        <span className="w-20 text-right tabular-nums text-ink-muted">
                          {fmtUsd(line.usd)}
                        </span>
                      </div>
                    ))}
                  </div>
                )}
                <div className="flex items-baseline justify-between gap-3 border-t border-line bg-surface-tertiary px-3 py-2 text-ink">
                  <span>{t("assistant.usageLines.total")}</span>
                  <span className="tabular-nums">
                    {breakdown.total === null
                      ? t("assistant.costUnavailable")
                      : fmtUsd(breakdown.total)}
                  </span>
                </div>
              </>
            )}
            {(duration !== null || requests.length > 0) && (
              <div className="space-y-1 border-t border-line px-3 py-2 first:border-t-0">
                {duration !== null && (
                  <p>
                    {t("assistant.replyDuration", {
                      duration: fmtDuration(duration, i18n.language),
                    })}
                  </p>
                )}
                {requests.length > 0 && (
                  <Button
                    variant="link"
                    className="h-auto px-0 text-note"
                    onClick={() => setRequestsOpen(true)}
                  >
                    {t("assistant.requestRecords")}
                  </Button>
                )}
              </div>
            )}
          </PopoverContent>
        </Popover>
      )}
      <AssistantRequestDetails
        requests={requests}
        open={requestsOpen}
        onOpenChange={setRequestsOpen}
        returnFocusRef={metaTrigger}
      />
    </>
  );
}
