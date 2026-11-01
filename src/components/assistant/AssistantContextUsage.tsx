import { useTranslation } from "react-i18next";
import type { AssistantContextUsage as ContextReceipt } from "@/lib/api/assistant";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export function AssistantContextUsage({
  usage,
  modelId,
}: {
  usage?: ContextReceipt | null;
  modelId: string;
}) {
  const { t, i18n } = useTranslation();
  const known =
    usage?.modelId === modelId &&
    Number.isSafeInteger(usage.tokens) &&
    usage.tokens !== null &&
    usage.tokens >= 0 &&
    Number.isSafeInteger(usage.contextWindow) &&
    usage.contextWindow > 0;
  const fraction = known ? Math.min(1, usage.tokens! / usage.contextWindow) : 0;
  const nearFull = known && fraction >= 0.8;
  const format = (n: number) =>
    new Intl.NumberFormat(i18n.resolvedLanguage || i18n.language).format(n);
  const description = known
    ? t("assistant.contextTooltip", {
        used: format(usage.tokens!),
        window: format(usage.contextWindow),
      })
    : t("assistant.contextUnknown");
  const circumference = 2 * Math.PI * 8;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          tabIndex={0}
          role={known ? "meter" : "img"}
          aria-label={t(
            known ? "assistant.contextLabel" : "assistant.contextUnavailable",
          )}
          aria-valuemin={known ? 0 : undefined}
          aria-valuemax={known ? 100 : undefined}
          aria-valuenow={known ? Math.round(fraction * 100) : undefined}
          aria-valuetext={known ? description : undefined}
          className={cn(
            "inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            nearFull ? "text-warn" : "text-ink-muted",
          )}
        >
          <svg
            aria-hidden
            viewBox="0 0 24 24"
            width="24"
            height="24"
            fill="none"
            className="-rotate-90"
          >
            <circle
              cx="12"
              cy="12"
              r="8"
              strokeWidth="2"
              className="stroke-line"
              strokeDasharray={known ? undefined : "3 3"}
            />
            {known && (
              <circle
                cx="12"
                cy="12"
                r="8"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeDasharray={circumference}
                strokeDashoffset={circumference * (1 - fraction)}
              />
            )}
          </svg>
        </span>
      </TooltipTrigger>
      <TooltipContent side="top">
        <p>{description}</p>
        {nearFull && <p className="mt-1">{t("assistant.contextNearFull")}</p>}
      </TooltipContent>
    </Tooltip>
  );
}
