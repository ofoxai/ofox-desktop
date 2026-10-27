import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ArchiveRestore } from "lucide-react";
import {
  assistantApi,
  type AssistantRestorePreview,
} from "@/lib/api/assistant";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogClose,
  DialogFooter,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";

export function AssistantBackupDialog({
  onRestored,
  onError,
  onPrepare,
}: {
  onRestored: () => Promise<void>;
  onError: (error: unknown) => void;
  onPrepare?: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<AssistantRestorePreview | null>(null);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const perform = async (action: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
      onError(error);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!busy) {
          setOpen(value);
          setPreview(null);
          setNotice("");
          setError("");
        }
      }}
    >
      <DialogTrigger asChild>
        <Button
          variant="ghost"
          className="h-auto min-h-[27px] w-full justify-start whitespace-normal py-1 text-left text-note"
        >
          <ArchiveRestore aria-hidden className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 break-words">
            {t("assistant.backup.title")}
          </span>
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t("assistant.backup.title")}</DialogTitle>
          <DialogDescription>
            {t("assistant.backup.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4 px-[18px] py-4">
          <p className="text-note text-ink-muted">
            {t("assistant.backup.stopFirst")}
          </p>
          <div className="flex flex-wrap gap-2">
            <Button
              disabled={busy}
              onClick={() =>
                void perform(async () => {
                  setPreview(null);
                  await onPrepare?.();
                  const result = await assistantApi.backup();
                  if (result.path) setNotice(t("assistant.backup.saved"));
                })
              }
            >
              {t("assistant.backup.create")}
            </Button>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() =>
                void perform(async () => {
                  setNotice("");
                  setPreview(null);
                  await onPrepare?.();
                  setPreview(await assistantApi.previewRestore());
                })
              }
            >
              {t("assistant.backup.preview")}
            </Button>
          </div>
          {preview && (
            <div className="space-y-3 rounded-lg border border-line p-3 text-ui">
              <p>
                {t("assistant.backup.summary", {
                  sessions: preview.sessionCount,
                  files: preview.fileCount,
                  count: preview.newRecords,
                })}
              </p>
              <p>
                {t("assistant.backup.skipped", {
                  count: preview.skippedRecords,
                })}
              </p>
              <p className="text-note text-ink-muted">
                {t("assistant.backup.safeMerge")}
              </p>
              {preview.conflicts.length > 0 && (
                <>
                  <p role="status">
                    {t("assistant.backup.conflicts", {
                      count: preview.conflicts.length,
                    })}
                  </p>
                  <ul className="max-h-32 overflow-y-auto break-all text-note">
                    {preview.conflicts.map((key) => (
                      <li key={key}>{key}</li>
                    ))}
                  </ul>
                </>
              )}
              <Button
                disabled={busy}
                onClick={() =>
                  void perform(async () => {
                    const report = await assistantApi.restore(preview.receipt);
                    setPreview(null);
                    setNotice(
                      t("assistant.backup.restored", {
                        count: report.importedRecords,
                      }),
                    );
                    await onRestored();
                  })
                }
              >
                {t("assistant.backup.confirm")}
              </Button>
            </div>
          )}
          {busy && (
            <p role="status" className="text-note text-ink-muted">
              {t("assistant.backup.working")}
            </p>
          )}
          {error && (
            <p role="alert" className="text-ui text-danger">
              {t(error, { defaultValue: t("assistant.backup.failed") })}
            </p>
          )}
          {notice && (
            <p role="status" className="text-ui">
              {notice}
            </p>
          )}
        </div>
        <DialogFooter>
          <DialogClose asChild>
            <Button variant="secondary" disabled={busy}>
              {t("common.close")}
            </Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
