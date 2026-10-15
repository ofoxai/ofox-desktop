import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import { toast } from "sonner";
import { sessionsApi, type ManagedSession } from "@/lib/api/sessions";
import { copyText } from "@/lib/clipboard";
import { fmtCost, fmtTokens } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { SessionDetails } from "./SessionDetails";
import { canSelect, sessionSubline } from "./model";
export function SessionRow({
  session: s,
  expanded,
  toggle,
  selected,
  select,
  busy,
  trash,
  focusRequest,
  onLocated,
}: {
  session: ManagedSession;
  expanded: boolean;
  toggle: () => void;
  selected: boolean;
  select: (v: boolean) => void;
  busy: boolean;
  trash: () => void;
  focusRequest?: number | null;
  onLocated?: (request: number) => void;
}) {
  const { t, i18n } = useTranslation();
  const titleButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (focusRequest == null) return;
    titleButton.current?.scrollIntoView({ block: "center" });
    titleButton.current?.focus({ preventScroll: true });
    onLocated?.(focusRequest);
  }, [focusRequest, onLocated]);
  const [launching, setLaunching] = useState(false);
  const running = useRef(false);
  const title = s.title || t("sessionCenter.untitled");
  const u = s.usage;
  const cost =
    u && !(u.models.length && u.models.every((m) => m.costUsd == null))
      ? u.costUsd
      : null;
  const launch = async () => {
    if (running.current) return;
    running.current = true;
    setLaunching(true);
    try {
      let result;
      try {
        result = await sessionsApi.resume(s.sessionKey, true);
      } catch {
        result = await sessionsApi.resume(s.sessionKey, false);
      }
      if (!result.launched) {
        await copyText(result.command);
        toast.success(t("sessionCenter.commandCopied"));
      }
    } catch (e) {
      toast.error(String(e));
    } finally {
      running.current = false;
      setLaunching(false);
    }
  };
  return (
    <div>
      <div
        className="flex min-h-[44px] items-center gap-2 px-3 py-1.5 hover:bg-pill"
        onClick={(e) => {
          if (e.target === e.currentTarget) toggle();
        }}
      >
        <Checkbox
          checked={selected}
          disabled={busy || !canSelect(s)}
          onCheckedChange={(v) => select(v === true)}
          aria-label={t("sessionCenter.selectSession", { title })}
        />
        <button
          ref={titleButton}
          className="min-w-0 flex-1 text-left"
          aria-expanded={expanded}
          onClick={toggle}
        >
          <span className="block truncate text-ui font-medium text-ink">
            {title}
          </span>
          <span className="block truncate text-note text-ink-faint">
            {sessionSubline(s, i18n.language, t)}
          </span>
        </button>
        <div className="min-w-[100px] text-right tabular-nums">
          <b className="text-ui text-ink">
            {u ? fmtTokens(u.totalTokens) : "—"}
          </b>
          {u && (
            <div className="text-note text-ink-faint">
              {t("sessionCenter.tokenShort", {
                input: fmtTokens(u.inputTokens),
                output: fmtTokens(u.outputTokens),
                cache: fmtTokens(u.cacheReadTokens),
              })}
            </div>
          )}
        </div>
        <span
          className="min-w-[58px] text-right text-sub text-ok tabular-nums"
          title={t("sessionCenter.estimated")}
        >
          {fmtCost(cost)}
        </span>
        {s.capabilities.canResume && (
          <Button
            variant="secondary"
            disabled={launching || busy}
            onClick={() => void launch()}
          >
            {t("sessionCenter.resume")}
          </Button>
        )}
        <Button
          variant="dangerGhost"
          size="icon"
          disabled={busy || !canSelect(s)}
          onClick={trash}
          aria-label={t("sessionCenter.moveToTrash")}
        >
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      </div>
      {expanded && <SessionDetails session={s} />}
    </div>
  );
}
