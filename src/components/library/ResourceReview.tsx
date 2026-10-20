import { useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  libraryApi,
  type LibraryKind,
  type LibraryOperation,
  type ResourceTarget,
} from "@/lib/api/library";
import { useDraftState, type DraftStore } from "./useDraftState";
import type { Resource } from "./LibrarySection";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ListCard, ListRow } from "@/components/ui/list";
import { libraryError, toolLabel } from "./libraryText";

export function ResourceReview({
  store,
  externalBusy,
  readOnly,
  expectedEpoch,
  kind,
  resource,
  targets = [],
  operation,
  onChanged,
  onBusyChange,
}: {
  store: DraftStore;
  externalBusy: boolean;
  readOnly: boolean;
  expectedEpoch: number;
  kind: LibraryKind;
  resource?: Resource;
  targets?: ResourceTarget[];
  operation?: LibraryOperation;
  onChanged: () => Promise<unknown>;
  onBusyChange: (value: boolean) => void;
}) {
  const { t } = useTranslation();
  const [selected, setSelected] = useDraftState(store, "selected", () =>
    targets.filter((target) => target.desired).map((target) => target.toolId),
  );
  const [adoptTools, setAdoptTools] = useDraftState<string[]>(
    store,
    "adoptTools",
    [],
  );
  const [review, setReview] = useDraftState(store, "review", operation);
  const [internalBusy, setBusy] = useDraftState(store, "busy", false);
  const busy = internalBusy || externalBusy;
  const running = useRef(false);
  const [error, setError] = useDraftState(store, "error", "");
  async function run(fn: () => Promise<LibraryOperation>) {
    if (running.current || busy || externalBusy || readOnly) return;
    running.current = true;
    setBusy(true);
    onBusyChange(true);
    setError("");
    try {
      setReview(await fn());
      await onChanged();
    } catch (e) {
      setError(libraryError(e, t));
    } finally {
      running.current = false;
      setBusy(false);
      onBusyChange(false);
    }
  }
  return (
    <div className="space-y-3 text-ctl">
      {kind === "skills" && (
        <p className="text-note text-ink-muted">
          {t("library.sharedReaderNotice")}
        </p>
      )}
      {error && (
        <p role="alert" className="text-danger">
          {error}
        </p>
      )}
      {!review && (
        <>
          <ListCard>
            {targets.map((target) => (
              <ListRow key={target.toolId} className="items-start">
                <Checkbox
                  aria-label={target.toolId}
                  checked={selected.includes(target.toolId)}
                  disabled={
                    busy ||
                    readOnly ||
                    (!target.available &&
                      !target.desired &&
                      !selected.includes(target.toolId)) ||
                    (!!target.disabledReason &&
                      !target.desired &&
                      !selected.includes(target.toolId))
                  }
                  onCheckedChange={(checked) =>
                    setSelected((previous) =>
                      checked
                        ? [...previous, target.toolId]
                        : previous.filter((tool) => tool !== target.toolId),
                    )
                  }
                />
                <div className="min-w-0 flex-1">
                  <p>
                    {toolLabel(target.toolId, t)} ·{" "}
                    {t(`library.status.${target.status}`, {
                      defaultValue: t("library.unknown"),
                    })}
                  </p>
                  <p className="break-all text-note text-ink-muted">
                    {target.path}
                  </p>
                  {target.status === "shared" && (
                    <p className="text-note text-ink-muted">
                      {t("library.sharedReadable")}
                    </p>
                  )}
                  {target.readers && target.readers.length > 1 && (
                    <p className="text-note text-ink-muted">
                      {t("library.readers", {
                        tools: target.readers
                          .map((tool) => toolLabel(tool, t))
                          .join(", "),
                      })}
                    </p>
                  )}
                  {target.readable === null && (
                    <p className="text-note text-warn">
                      {t("library.scopeUnknown")}
                    </p>
                  )}
                  {!target.available && (
                    <p className="text-note text-ink-muted">
                      {t("library.nativeCliRequired")}
                    </p>
                  )}
                  {target.disabledReason && (
                    <p className="text-note text-warn">
                      {libraryError(target.disabledReason, t)}
                    </p>
                  )}
                </div>
              </ListRow>
            ))}
          </ListCard>
          <Button
            disabled={busy || readOnly || !resource}
            onClick={() =>
              void run(() =>
                libraryApi.preview(
                  kind,
                  resource!.id,
                  selected,
                  resource!.revision,
                  expectedEpoch,
                ),
              )
            }
          >
            {t("library.preview")}
          </Button>
        </>
      )}
      {review && (
        <>
          <p role="status">
            {t(`library.status.${review.status}`, {
              defaultValue: t("library.unknown"),
            })}
          </p>
          <ListCard>
            {review.targets.map((target) => (
              <ListRow key={target.targetGroupId} className="block space-y-1">
                <div className="flex flex-wrap justify-between gap-2">
                  <span>
                    {target.tools.map((tool) => toolLabel(tool, t)).join(", ")}
                  </span>
                  <span>
                    {t(`library.status.${target.status}`, {
                      defaultValue: t("library.unknown"),
                    })}
                  </span>
                </div>
                <p className="break-all text-note text-ink-muted">
                  {target.path}
                </p>
                {!!target.readers.length && (
                  <p className="text-note text-ink-muted">
                    {t("library.readers", {
                      tools: target.readers
                        .map((tool) => toolLabel(tool, t))
                        .join(", "),
                    })}
                  </p>
                )}
                {target.reason && (
                  <p
                    className={
                      target.reason === "library.adoptionPreview"
                        ? "text-warn"
                        : "text-danger"
                    }
                  >
                    {libraryError(target.reason, t)}
                  </p>
                )}
                {kind === "instructions" && (
                  <div className="grid gap-2 sm:grid-cols-2">
                    <div>
                      <p className="text-note text-ink-muted">
                        {t("library.beforeManaged")}
                      </p>
                      <pre className="max-h-44 overflow-auto whitespace-pre-wrap break-words rounded bg-pill p-2 text-note">
                        {target.beforeManaged ?? "—"}
                      </pre>
                    </div>
                    <div>
                      <p className="text-note text-ink-muted">
                        {t("library.afterManaged")}
                      </p>
                      <pre className="max-h-44 overflow-auto whitespace-pre-wrap break-words rounded bg-pill p-2 text-note">
                        {target.afterManaged ?? "—"}
                      </pre>
                    </div>
                  </div>
                )}
                {kind === "mcp" && (
                  <p className="break-all text-note text-ink-muted">
                    {t("library.changedFields", {
                      fields: target.changedFields.join(", ") || "—",
                    })}
                  </p>
                )}
                {kind === "mcp" &&
                  resource?.revision !== undefined &&
                  target.status === "conflict" &&
                  target.reason?.includes("library.mcpNameConflict") &&
                  target.tools.some((tool) => selected.includes(tool)) && (
                    <div className="space-y-2">
                      <p className="text-note text-ink-muted">
                        {t("library.adoptDescription")}
                      </p>
                      <Button
                        variant="secondary"
                        disabled={busy || readOnly}
                        onClick={() => {
                          const next = Array.from(
                            new Set([
                              ...adoptTools,
                              ...target.tools.filter((tool) =>
                                selected.includes(tool),
                              ),
                            ]),
                          );
                          setAdoptTools(next);
                          void run(() =>
                            libraryApi.previewMcpAdoption(
                              resource!.id,
                              selected,
                              next,
                              resource!.revision!,
                              expectedEpoch,
                            ),
                          );
                        }}
                      >
                        {t("library.adoptPreview")}
                      </Button>
                    </div>
                  )}
                {kind === "skills" && (
                  <>
                    <p className="text-note">
                      {t("library.transferMethod", {
                        method: target.method
                          ? t(`library.method.${target.method}`)
                          : t("library.unknown"),
                      })}
                    </p>
                    {target.fallbackUsed && (
                      <p className="text-note text-warn">
                        {t("library.fallbackCopy")}
                      </p>
                    )}
                    <p className="break-all text-note text-ink-muted">
                      {target.artifactHash ?? "—"}
                    </p>
                  </>
                )}
              </ListRow>
            ))}
          </ListCard>
          <div className="flex flex-wrap gap-2">
            {resource &&
              ["previewed", "partial", "needsReview"].includes(
                review.status,
              ) && (
                <Button
                  variant="secondary"
                  disabled={busy || readOnly}
                  onClick={() => {
                    setReview(undefined);
                    setAdoptTools([]);
                  }}
                >
                  {t("library.changeTargets")}
                </Button>
              )}
            {["previewed", "partial"].includes(review.status) && (
              <Button
                disabled={
                  busy ||
                  readOnly ||
                  (!(
                    review.status === "previewed" && review.targets.length === 0
                  ) &&
                    !review.targets.some((target) =>
                      ["pending", "failed"].includes(target.status),
                    ))
                }
                onClick={() =>
                  void run(() => libraryApi.apply(review.operationId))
                }
              >
                {t(
                  review.status === "partial"
                    ? "library.retryFailed"
                    : "library.apply",
                )}
              </Button>
            )}
            {["prepared", "needsReview", "partial"].includes(review.status) && (
              <Button
                variant="secondary"
                disabled={busy || readOnly}
                onClick={() =>
                  void run(() => libraryApi.recover(review.operationId))
                }
              >
                {t("library.verifyRecovery")}
              </Button>
            )}
            {["previewed", "partial", "needsReview"].includes(
              review.status,
            ) && (
              <Button
                variant="secondary"
                disabled={busy || readOnly}
                onClick={() =>
                  void run(() => libraryApi.cancel(review.operationId))
                }
              >
                {t("library.cancelJournal")}
              </Button>
            )}
          </div>
          {["previewed", "partial", "needsReview"].includes(review.status) && (
            <p className="text-note text-ink-muted">
              {t("library.cancelJournalDescription")}
            </p>
          )}
        </>
      )}
      {busy && (
        <p role="status" className="text-ink-muted">
          {t("library.working")}
        </p>
      )}
    </div>
  );
}
