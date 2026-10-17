import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { libraryApi, type InventorySourceDetail } from "@/lib/api/library";
import { copyText } from "@/lib/clipboard";
import { Button } from "@/components/ui/button";
import {
  toolLabel,
  libraryError,
  libraryCommandUnavailable,
} from "./libraryText";

export function LibrarySourceDetail({
  ticketId,
  entryId,
  sourceRef,
  epoch,
  onBack,
  onLocate,
}: {
  ticketId: string;
  entryId: string;
  sourceRef: string;
  epoch: number;
  onBack: () => void;
  onLocate: (id: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const [detail, setDetail] = useState<InventorySourceDetail>();
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const request = useRef(0);
  const lifetime = useRef(0);
  const invalidated = useRef(false);
  const actionBusy = useRef(false);
  useEffect(() => {
    const viewLifetime = ++lifetime.current;
    invalidated.current = false;
    actionBusy.current = false;
    setBusy(false);
    let validating = false;
    setDetail(undefined);
    setError("");
    const validate = async () => {
      if (validating || invalidated.current || actionBusy.current) return;
      validating = true;
      const generation = ++request.current;
      try {
        const value = await libraryApi.inventorySource(
          ticketId,
          entryId,
          sourceRef,
          epoch,
        );
        if (generation === request.current && viewLifetime === lifetime.current)
          setDetail(value);
      } catch (reason) {
        if (
          generation === request.current &&
          viewLifetime === lifetime.current
        ) {
          invalidated.current = true;
          setDetail(undefined);
          setError(
            t(
              libraryCommandUnavailable(reason)
                ? "library.workbench2.unsupported"
                : "library.workbench2.sourceUnconfirmed",
            ),
          );
        }
      } finally {
        validating = false;
      }
    };
    void validate();
    const interval = setInterval(() => void validate(), 5000);
    const focus = () => void validate();
    window.addEventListener("focus", focus);
    return () => {
      request.current++;
      lifetime.current++;
      invalidated.current = true;
      clearInterval(interval);
      window.removeEventListener("focus", focus);
    };
  }, [ticketId, entryId, sourceRef, epoch, t]);
  async function action(copy = false) {
    if (actionBusy.current || invalidated.current) return;
    actionBusy.current = true;
    setBusy(true);
    setError("");
    const generation = ++request.current;
    const viewLifetime = lifetime.current;
    try {
      // Revalidate even when copying: never reuse an expired source path.
      const fresh = await libraryApi.inventorySource(
        ticketId,
        entryId,
        sourceRef,
        epoch,
      );
      if (generation !== request.current || viewLifetime !== lifetime.current)
        return;
      if (copy && fresh.logicalPath) {
        await copyText(fresh.logicalPath);
        if (generation === request.current && viewLifetime === lifetime.current)
          setNotice(t("library.workbench2.copied"));
      } else if (!copy)
        await libraryApi.revealInventorySource(
          ticketId,
          entryId,
          sourceRef,
          epoch,
        );
    } catch (reason) {
      if (generation === request.current && viewLifetime === lifetime.current) {
        invalidated.current = true;
        setDetail(undefined);
        setError(libraryError(reason, t));
      }
    } finally {
      if (viewLifetime === lifetime.current) {
        actionBusy.current = false;
        setBusy(false);
      }
    }
  }
  async function locate(id: string) {
    if (actionBusy.current || invalidated.current) return;
    actionBusy.current = true;
    setBusy(true);
    const generation = ++request.current;
    const viewLifetime = lifetime.current;
    try {
      const fresh = await libraryApi.inventorySource(
        ticketId,
        entryId,
        sourceRef,
        epoch,
      );
      if (generation !== request.current || viewLifetime !== lifetime.current)
        return;
      if (!fresh.matchedResourceIds.includes(id))
        throw new Error("library.inventoryChanged");
      onLocate(id);
    } catch {
      if (generation === request.current && viewLifetime === lifetime.current) {
        invalidated.current = true;
        setDetail(undefined);
        setError(t("library.workbench2.sourceUnconfirmed"));
      }
    } finally {
      if (viewLifetime === lifetime.current) {
        actionBusy.current = false;
        setBusy(false);
      }
    }
  }
  return (
    <div className="space-y-3">
      <Button variant="ghost" onClick={onBack}>
        {t("library.workbench2.back")}
      </Button>
      {error && (
        <p role="alert" className="text-ctl text-warn">
          {error}
        </p>
      )}
      {!detail && !error && <p role="status">{t("library.loading")}</p>}
      {detail && (
        <>
          <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-ctl">
            <dt className="text-ink-muted">
              {t("library.workbench.agentFilter")}
            </dt>
            <dd>{toolLabel(detail.toolId, t)}</dd>
            <dt className="text-ink-muted">
              {t("library.workbench2.sourceType")}
            </dt>
            <dd>
              {t(`library.workbench.source.${detail.label}`, {
                defaultValue: t("library.workbench.nativeSource"),
              })}
            </dd>
            <dt className="text-ink-muted">{t("library.workbench2.readAt")}</dt>
            <dd>
              {new Intl.DateTimeFormat(i18n.language, {
                dateStyle: "medium",
                timeStyle: "short",
              }).format(detail.observedAtMs)}
            </dd>
            {detail.logicalPath && (
              <>
                <dt className="text-ink-muted">
                  {t("library.workbench2.location")}
                </dt>
                <dd className="break-all font-mono text-note">
                  {detail.logicalPath}
                  {detail.isSymlink && detail.canonicalPath && (
                    <span className="block text-ink-muted">
                      → {detail.canonicalPath}
                    </span>
                  )}
                </dd>
              </>
            )}
          </dl>
          <p className="text-note text-ink-muted">
            {t("library.workbench2.observedHint")}
          </p>
          <div className="flex flex-wrap gap-2">
            {detail.canReveal ? (
              <>
                <Button
                  variant="secondary"
                  disabled={busy}
                  onClick={() => void action()}
                >
                  {t("library.workbench2.reveal")}
                </Button>
                <Button
                  variant="ghost"
                  disabled={busy}
                  onClick={() => void action(true)}
                >
                  {t("library.workbench2.copyPath")}
                </Button>
              </>
            ) : (
              <p className="text-note text-ink-muted">
                {t("library.workbench2.noFile")}
              </p>
            )}
            {detail.matchedResourceIds.map((id) => (
              <Button
                key={id}
                variant="ghost"
                disabled={busy}
                onClick={() => void locate(id)}
              >
                {t("library.workbench2.locate")}
              </Button>
            ))}
          </div>
          {notice && (
            <p role="status" className="text-note">
              {notice}
            </p>
          )}
          {detail.fields.length > 0 && (
            <details className="rounded-lg border border-line p-3">
              <summary className="cursor-pointer text-ctl">
                {t("library.workbench2.sanitized")}
              </summary>
              <p className="my-2 text-note text-ink-muted">
                {t("library.workbench2.noValues")}
              </p>
              <div className="max-h-64 space-y-1 overflow-auto">
                {detail.fields.map((field) => (
                  <div
                    key={JSON.stringify(field.path)}
                    className="flex justify-between gap-3 text-note"
                  >
                    <code className="break-all">{field.path.join(".")}</code>
                    <span className="shrink-0 text-ink-muted">
                      {field.valueType}
                    </span>
                  </div>
                ))}
              </div>
            </details>
          )}
        </>
      )}
    </div>
  );
}
