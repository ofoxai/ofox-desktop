import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  libraryApi,
  type ResourcePackPreview,
  type ResourcePackResult,
} from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { MenuSelect } from "@/components/ui/menu-select";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { ListCard, ListRow } from "@/components/ui/list";
import { libraryError } from "./libraryText";

export function ResourcePackDialog({
  onChanged,
}: {
  onChanged: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [plan, setPlan] = useState<{
    path: string;
    preview: ResourcePackPreview;
  } | null>(null);
  const [result, setResult] = useState<ResourcePackResult | null>(null);
  const [conflicts, setConflicts] = useState<"skip" | "copy">("skip");
  const running = useRef(false);
  const trigger = useRef<HTMLButtonElement>(null);
  async function run(action: () => Promise<void>) {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
    } catch (e) {
      setError(libraryError(e, t));
    } finally {
      running.current = false;
      setBusy(false);
    }
  }
  function close() {
    if (!running.current) setOpen(false);
  }
  return (
    <>
      <Button ref={trigger} variant="secondary" onClick={() => setOpen(true)}>
        {t("library.pack.title")}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (value) setOpen(true);
          else close();
        }}
      >
        <DialogContent
          className="sm:max-w-3xl"
          onEscapeKeyDown={(e) => {
            if (busy) e.preventDefault();
          }}
          onCloseAutoFocus={(e) => {
            e.preventDefault();
            trigger.current?.focus();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("library.pack.title")}</DialogTitle>
            <DialogDescription>
              {t("library.pack.description")}
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 space-y-3 overflow-y-auto px-[18px] py-3">
            <p className="text-ctl text-ink-muted">
              {t("library.pack.contents")}
            </p>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="secondary"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    if (await libraryApi.exportPack())
                      setNotice(t("library.pack.exported"));
                  })
                }
              >
                {t("library.pack.export")}
              </Button>
              <Button
                variant="secondary"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    const path = await libraryApi.pickPack();
                    if (!path) return;
                    setPlan(null);
                    setResult(null);
                    const preview = await libraryApi.previewPack(path);
                    if (
                      !Number.isSafeInteger(preview.epoch) ||
                      preview.epoch < 0 ||
                      !Number.isSafeInteger(preview.revision) ||
                      preview.revision < 0 ||
                      !preview.fingerprint
                    )
                      throw new Error("library.packInvalid");
                    setPlan({ path, preview });
                    setConflicts("skip");
                  })
                }
              >
                {t("library.pack.pick")}
              </Button>
            </div>
            {error && (
              <p role="alert" className="text-ctl text-danger">
                {error}
              </p>
            )}
            {notice && (
              <p role="status" className="text-ctl text-ink-muted">
                {notice}
              </p>
            )}
            {busy && (
              <p role="status" className="text-ctl text-ink-muted">
                {t("library.loading")}
              </p>
            )}
            {plan && !result && (
              <>
                <p className="break-all text-note text-ink-muted">
                  {plan.path}
                </p>
                <ListCard>
                  {plan.preview.items.map((item) => (
                    <ListRow
                      key={`${item.kind}:${item.id}`}
                      className="flex-wrap"
                    >
                      <div className="min-w-0 flex-1 break-words text-ctl">
                        {item.name}
                        <span className="ml-2 text-note text-ink-muted">
                          {t(`library.tabs.${item.kind}`)}
                        </span>
                      </div>
                      <span className="text-note text-ink-muted">
                        {t(
                          item.conflict
                            ? "library.pack.conflict"
                            : "library.pack.newDraft",
                        )}
                      </span>
                      {!!item.omittedFields && (
                        <span className="text-note text-warn">
                          {t("library.pack.omitted", {
                            count: item.omittedFields,
                          })}
                        </span>
                      )}
                    </ListRow>
                  ))}
                </ListCard>
                <MenuSelect
                  aria-label={t("library.pack.conflicts")}
                  value={conflicts}
                  onChange={setConflicts}
                  disabled={busy}
                  options={[
                    { value: "skip", label: t("library.pack.skip") },
                    { value: "copy", label: t("library.pack.copy") },
                  ]}
                />
                <p className="text-note text-ink-muted">
                  {t("library.pack.restoreHint")}
                </p>
                <Button
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      const restored = await libraryApi.restorePack(
                        plan.path,
                        plan.preview,
                        conflicts,
                      );
                      setResult(restored);
                      setPlan(null);
                      await onChanged();
                    })
                  }
                >
                  {t("library.pack.confirm")}
                </Button>
              </>
            )}
            {result && (
              <>
                <p role="status" className="text-ctl">
                  {t("library.pack.result", { count: result.imported })}
                </p>
                <ListCard>
                  {result.items.map((item, index) => (
                    <ListRow key={index} className="flex-wrap">
                      <span className="min-w-0 flex-1 break-words text-ctl">
                        {item.name}
                      </span>
                      <span
                        className={
                          item.status === "failed"
                            ? "text-note text-danger"
                            : "text-note text-ink-muted"
                        }
                      >
                        {t(`library.pack.${item.status}`)}
                      </span>
                    </ListRow>
                  ))}
                </ListCard>
              </>
            )}
          </div>
          <DialogFooter>
            <Button variant="secondary" disabled={busy} onClick={close}>
              {t("library.close")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
