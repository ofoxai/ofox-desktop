import * as React from "react";
import { Check, Copy } from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { copyText } from "@/lib/clipboard";
import { cn } from "@/lib/utils";

/** How long the check stays after a copy (magpie's). */
export const COPIED_MS = 1400;

export interface CopyButtonProps
  extends Omit<
    React.ButtonHTMLAttributes<HTMLButtonElement>,
    "onClick" | "children"
  > {
  text: string;
  /** Names what is copied, e.g. "复制会话 ID". Defaults to 复制. */
  "aria-label"?: string;
}

/**
 * magpie's `.copy`: a 22px icon that copies `text` and turns into a green
 * check for a moment. The name stays the same; a status line inside says
 * it was copied.
 */
export function CopyButton({
  text,
  className,
  "aria-label": ariaLabel,
  title,
  ...props
}: CopyButtonProps) {
  const { t } = useTranslation();
  const [copied, setCopied] = React.useState(false);
  const timer = React.useRef<number>();

  React.useEffect(() => () => window.clearTimeout(timer.current), []);

  const copy = async () => {
    try {
      await copyText(text);
    } catch (error) {
      console.error("[CopyButton] copy failed", error);
      toast.error(t("common.copyFailed"));
      return;
    }
    setCopied(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), COPIED_MS);
  };

  const label = ariaLabel ?? t("common.copy");
  const Icon = copied ? Check : Copy;
  return (
    <Button
      type="button"
      variant="copy"
      aria-label={label}
      title={title ?? label}
      data-copied={copied || undefined}
      onClick={() => void copy()}
      className={cn(
        "flex-none",
        copied &&
          "animate-copy-pop bg-ok-soft text-ok hover:bg-ok-soft hover:text-ok",
        className,
      )}
      {...props}
    >
      <Icon aria-hidden className="h-3.5 w-3.5" strokeWidth={2} />
      <span role="status" className="sr-only">
        {copied ? t("common.copied") : ""}
      </span>
    </Button>
  );
}
