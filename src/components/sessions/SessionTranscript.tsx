import { useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";
import { sessionsApi } from "@/lib/api/sessions";
import { Button } from "@/components/ui/button";
import { TranscriptPart } from "./TranscriptPart";
export function SessionTranscript({
  sessionKey,
  revision,
}: {
  sessionKey: string;
  revision?: string | number | null;
}) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  const query = useQuery({
    queryKey: ["session-messages", sessionKey, revision],
    queryFn: () => sessionsApi.getMessagesByKey(sessionKey),
    retry: false,
  });
  const rows = query.data ?? [];
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => ref.current,
    estimateSize: () => 160,
    overscan: 4,
  });
  return (
    <div
      ref={ref}
      className="max-h-[min(560px,70vh)] overflow-auto rounded-[9px] border border-line-2 bg-canvas"
    >
      {query.isPending && (
        <p role="status" className="p-3">
          {t("common.loading")}
        </p>
      )}
      {query.isError && (
        <div role="alert" className="p-3 text-danger">
          {String(query.error)}{" "}
          <Button variant="ghost" onClick={() => void query.refetch()}>
            {t("common.retry")}
          </Button>
        </div>
      )}
      {!query.isPending && !query.isError && !rows.length && (
        <p className="p-3 text-ink-muted">{t("sessionCenter.emptyContent")}</p>
      )}
      <div style={{ height: virtual.getTotalSize(), position: "relative" }}>
        {virtual.getVirtualItems().map((item) => (
          <div
            key={item.key}
            data-index={item.index}
            ref={virtual.measureElement}
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              width: "100%",
              transform: `translateY(${item.start}px)`,
            }}
          >
            <TranscriptPart message={rows[item.index]} />
          </div>
        ))}
      </div>
    </div>
  );
}
