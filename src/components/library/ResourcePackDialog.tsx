import { useEffect, useRef, useState } from "react";
import { Download, MoreHorizontal, Upload } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  libraryApi,
  type ResourcePackPreview,
  type ResourcePackResult,
} from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { MenuSelect } from "@/components/ui/menu-select";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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
  const [mode, setMode] = useState<"import" | "export">("import");
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
  const pendingAction = useRef<"import" | "export" | null>(null);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  async function run(action: () => Promise<void>) {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
    } catch (e) {
      if (mounted.current) setError(libraryError(e, t));
    } finally {
      running.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  function close() {
    if (!running.current) setOpen(false);
  }
  async function choosePack() {
    const path = await libraryApi.pickPack();
    if (!mounted.current) return;
    if (!path) {
      setNotice(t("library.pack.cancelledImport"));
      return;
    }
    setPlan(null);
    setResult(null);
    const preview = await libraryApi.previewPack(path);
    if (!mounted.current) return;
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
  }
  async function exportPack() {
    const saved = await libraryApi.exportPack();
    if (mounted.current)
      setNotice(
        t(saved ? "library.pack.exported" : "library.pack.cancelledExport"),
      );
  }
  function start(action: "import" | "export") {
    if (running.current || !mounted.current) return;
    setMode(action);
    setPlan(null);
    setResult(null);
    setError("");
    setNotice("");
    setOpen(true);
    void run(action === "import" ? choosePack : exportPack);
  }
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            ref={trigger}
            variant="ghost"
            size="icon"
            disabled={busy || open}
            aria-label={t("library.pack.menu")}
          >
            <MoreHorizontal aria-hidden className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const action = pendingAction.current;
            pendingAction.current = null;
            if (action) queueMicrotask(() => start(action));
            else trigger.current?.focus();
          }}
        >
          <DropdownMenuItem
            onSelect={() => {
              pendingAction.current = "import";
            }}
          >
            <Upload aria-hidden className="h-4 w-4" />
            {t("library.pack.importAction")}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => {
              pendingAction.current = "export";
            }}
          >
            <Download aria-hidden className="h-4 w-4" />
            {t("library.pack.exportAction")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
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
            <DialogTitle>
              {t(
                mode === "import"
                  ? "library.pack.importTitle"
                  : "library.pack.exportTitle",
              )}
            </DialogTitle>
            <DialogDescription>
              {t(
                mode === "import"
                  ? "library.pack.importDescription"
                  : "library.pack.exportDescription",
              )}
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 space-y-3 overflow-y-auto px-[18px] py-3">
            <p className="text-ctl text-ink-muted">
              {t("library.pack.contents")}
            </p>
            <div className="flex flex-wrap gap-2">
              {mode === "export" && !!error && (
                <Button
                  variant="secondary"
                  disabled={busy}
                  onClick={() => void run(exportPack)}
                >
                  {t("library.pack.exportAction")}
                </Button>
              )}
              {mode === "import" && (
                <Button
                  variant="secondary"
                  disabled={busy}
                  onClick={() => void run(choosePack)}
                >
                  {t("library.pack.pick")}
                </Button>
              )}
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
                      if (!mounted.current) return;
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
