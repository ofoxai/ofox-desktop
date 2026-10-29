import { useRef, useState } from "react";
import { Loader2, RotateCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AssistantModel } from "@/lib/api/assistant";
import { assistantErrorKey } from "@/lib/api/assistant";
import { Button } from "@/components/ui/button";
import { MenuSelect } from "@/components/ui/menu-select";
import { cn } from "@/lib/utils";
import { useModelOptions } from "./useModelOptions";

/** A search whose result couldn't be confirmed: it may be billed twice. */
const SEARCH_OUTCOME_UNKNOWN = "assistant_search_outcome_unknown";

/**
 * Under a failed or stopped reply: why it ended (plain text; the page's
 * banner announced a live failure) and the ways to ask again, with the same
 * model or with another one, which becomes the conversation's. A search that
 * may already be billed says so first, in place of the generic reason.
 *
 * Retrying stays pending until `onRetry` settles, whether or not it sent:
 * the buttons are `aria-disabled` rather than `disabled` for that time, so
 * the focus the press had is still there when a refused send gives it back.
 */
export function RetryCard({
  errorCode,
  models,
  currentModelId,
  disabled,
  onRetry,
}: {
  errorCode?: string | null;
  models: ReadonlyArray<AssistantModel>;
  /** The conversation's model: not "another" one. */
  currentModelId: string;
  disabled: boolean;
  /** Settles when the send is over; false when it did not go out. */
  onRetry: (modelId?: string) => Promise<boolean> | void;
}) {
  const { t } = useTranslation();
  const [pending, setPending] = useState(false);
  // Read at once, where `pending` waits for the next render: two presses
  // that close together send one retry.
  const inFlight = useRef(false);
  const options = useModelOptions(models);
  const blocked = disabled || pending;
  const retry = (modelId?: string) => {
    if (disabled || inFlight.current) return;
    inFlight.current = true;
    setPending(true);
    void Promise.resolve(onRetry(modelId)).finally(() => {
      inFlight.current = false;
      setPending(false);
    });
  };
  const reason = !errorCode
    ? null
    : errorCode === SEARCH_OUTCOME_UNKNOWN
      ? t("assistant.retryBilledNote")
      : t(assistantErrorKey(errorCode));
  return (
    <div
      className={cn(
        "mt-2 space-y-2",
        reason && "rounded-list border border-line bg-card-2 px-3 py-2",
      )}
    >
      {reason && (
        <p
          className={cn(
            "text-ctl",
            errorCode === SEARCH_OUTCOME_UNKNOWN ? "text-ink-2" : "text-danger",
          )}
        >
          {reason}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="secondary"
          aria-disabled={blocked || undefined}
          aria-busy={pending || undefined}
          className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
          onClick={() => retry()}
        >
          {pending ? (
            <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <RotateCw aria-hidden className="h-3.5 w-3.5" />
          )}
          {t("assistant.retry")}
        </Button>
        <MenuSelect
          aria-label={t("assistant.retryWithModel")}
          value={currentModelId}
          onChange={(modelId) => retry(modelId)}
          disabled={blocked}
          searchable
          searchPlaceholder={t("assistant.searchModels")}
          valueLabel={t("assistant.retryWithModel")}
          width={410}
          menuClassName="max-h-[min(480px,70vh)]"
          options={options}
        />
      </div>
    </div>
  );
}
