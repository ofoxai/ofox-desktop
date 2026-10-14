import { useTranslation } from "react-i18next";
import { ListCard, ListRow, ListHead } from "@/components/ui/list";
import { Button } from "@/components/ui/button";
import type { SessionTrashEntry } from "@/lib/api/sessions";
import { fmtAgo, fmtBytes } from "@/lib/format";
export function SessionsTrash({
  items,
  busy,
  restore,
  purge,
  clear,
}: {
  items: SessionTrashEntry[];
  busy: boolean;
  restore: (s: SessionTrashEntry) => void;
  purge: (s: SessionTrashEntry) => void;
  clear: () => void;
}) {
  const { t, i18n } = useTranslation();
  return (
    <>
      <p className="mb-2 text-sub text-ink-muted">
        {t("sessionCenter.trashHint")}
      </p>
      <ListCard>
        <ListHead>
          <span>
            {t("sessionCenter.deletedCount", { count: items.length })}
          </span>
          <span className="flex-1" />
          <Button
            variant="dangerGhost"
            disabled={busy || !items.length}
            onClick={clear}
          >
            {t("sessionCenter.clearTrash")}
          </Button>
        </ListHead>
        {!items.length && (
          <p className="p-6 text-center text-sub text-ink-muted">
            {t("sessionCenter.emptyTrash")}
          </p>
        )}
        {items.map((s) => (
          <ListRow key={s.key}>
            <div className="min-w-0 flex-1">
              <p className="truncate text-ui font-medium">
                {s.title || s.sessionId}
              </p>
              <p className="truncate text-note text-ink-faint">
                {s.providerId} · {s.projectDir} ·{" "}
                {s.sizeBytes != null && `${fmtBytes(s.sizeBytes)} · `}
                {t("sessionCenter.deletedAt", {
                  ago: fmtAgo(s.deletedAt, i18n.language),
                })}
              </p>
              {(s.error || s.state !== "trashed") && (
                <p role="alert" className="text-sub text-danger">
                  {s.error || t("sessionCenter.recoveryRequired")}
                </p>
              )}
            </div>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => restore(s)}
            >
              {t("sessionCenter.restore")}
            </Button>
            <Button
              variant="dangerGhost"
              disabled={busy}
              onClick={() => purge(s)}
            >
              {t("sessionCenter.purge")}
            </Button>
          </ListRow>
        ))}
      </ListCard>
    </>
  );
}
