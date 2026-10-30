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
import {
  fmtDuration,
  replyDuration,
  usageSummary,
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
 * the run used and cost and how long it took. `onOpenChange` tells the row
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
  const usage = usageSummary(message.usage ?? undefined, true, t);
  const details = [
    usage,
    duration !== null
      ? t("assistant.replyDuration", {
          duration: fmtDuration(duration, i18n.language),
        })
      : null,
  ].filter((line): line is string => line !== null);
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
      {(details.length > 0 || requests.length > 0) && (
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
            className="w-auto max-w-[min(320px,calc(100vw-32px))] space-y-1 px-3 py-2 text-note text-ink-2"
          >
            {details.map((line) => (
              <p key={line}>{line}</p>
            ))}
            {requests.length > 0 && (
              <Button
                variant="link"
                className="h-auto px-0 text-note"
                onClick={() => setRequestsOpen(true)}
              >
                {t("assistant.requestRecords")}
              </Button>
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
