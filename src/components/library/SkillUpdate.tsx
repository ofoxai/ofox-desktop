import { useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  libraryApi,
  type SkillDefinition,
  type SkillUpdateCheck,
  type SkillUpdateComparison,
  type SkillDraftChange,
} from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { MenuSelect } from "@/components/ui/menu-select";
import { useDraftState, type DraftStore } from "./useDraftState";
import { libraryError } from "./libraryText";
export function SkillUpdate({
  store,
  skill,
  epoch,
  readOnly,
  externalBusy,
  onBusyChange,
  onChanged,
}: {
  store: DraftStore;
  skill: SkillDefinition;
  epoch: number;
  readOnly: boolean;
  externalBusy: boolean;
  onBusyChange: (busy: boolean) => void;
  onChanged: () => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [checked, setChecked] = useDraftState<SkillUpdateCheck | undefined>(
    store,
    "skillUpdateCheck",
    undefined,
  );
  const [comparison, setComparison] = useDraftState<
    SkillUpdateComparison | undefined
  >(store, "skillUpdateComparison", undefined);
  const [comparePath, setComparePath] = useDraftState(
    store,
    "skillComparePath",
    "SKILL.md",
  );
  const [compareFiles, setCompareFiles] = useDraftState<
    SkillUpdateComparison["files"]
  >(store, "skillCompareFiles", []);
  const [result, setResult] = useDraftState<SkillDraftChange | undefined>(
    store,
    "skillUpdateResult",
    undefined,
  );
  const [error, setError] = useDraftState(store, "skillUpdateError", "");
  const [working, setWorking] = useDraftState(store, "skillUpdateBusy", false);
  const [name, setName] = useDraftState(store, "skillCopyName", () =>
    t("library.copyName", { name: skill.name }),
  );
  const [directory, setDirectory] = useDraftState(
    store,
    "skillCopyDirectory",
    () => `${skill.directory}-copy`,
  );
  const running = useRef(false);
  const busy = working || externalBusy;
  async function run(fn: () => Promise<void>) {
    if (running.current || busy || readOnly) return;
    running.current = true;
    setWorking(true);
    onBusyChange(true);
    setError("");
    try {
      await fn();
    } catch (e) {
      setError(libraryError(e, t));
      if (
        /library\.(skillUpdateExpired|epochChanged|revisionChanged|skillArtifactChanged)\b/.test(
          String(e),
        )
      ) {
        setChecked(undefined);
        setComparison(undefined);
      }
    } finally {
      running.current = false;
      setWorking(false);
      onBusyChange(false);
    }
  }
  async function confirm(mode: "update" | "copy") {
    if (!checked?.candidate) return;
    const changed = await libraryApi.confirmSkillUpdate(
      checked.candidate.ticketId,
      checked.revision,
      checked.epoch,
      mode,
      mode === "copy" ? name : undefined,
      mode === "copy" ? directory : undefined,
    );
    setResult(changed);
    setChecked(undefined);
    setComparison(undefined);
    await onChanged();
  }
  async function loadComparison(ticketId: string, path: string) {
    setComparePath(path);
    setComparison(undefined);
    const next = await libraryApi.compareSkillUpdate(ticketId, path);
    setComparison(next);
    setCompareFiles(next.files);
  }
  const candidate = checked?.candidate;
  return (
    <div className="space-y-3 text-ctl">
      <p className="text-ink-muted">{t("library.skillUpdate.description")}</p>
      {error && (
        <p role="alert" className="text-danger">
          {error}
        </p>
      )}
      <Button
        variant="secondary"
        disabled={busy || readOnly || skill.revision === undefined}
        onClick={() =>
          void run(async () => {
            setChecked(undefined);
            setComparison(undefined);
            setResult(undefined);
            setCompareFiles([]);
            setComparePath("SKILL.md");
            const check = await libraryApi.checkSkillUpdate(
              skill.id,
              skill.revision!,
              epoch,
            );
            setChecked(check);
            if (check.candidate)
              await loadComparison(check.candidate.ticketId, "SKILL.md");
          })
        }
      >
        {t("library.skillUpdate.check")}
      </Button>
      {busy && (
        <p role="status" className="text-ink-muted">
          {t("library.loading")}
        </p>
      )}
      {checked && (
        <p role="status" className="text-ink-muted">
          {t(`library.skillUpdate.${checked.state}`)}
        </p>
      )}
      {candidate && (
        <>
          <p className="break-all text-note text-ink-muted">
            {t("library.skillUpdate.frozen", { commit: candidate.commitSha })}
          </p>
          <p className="break-all text-note text-ink-muted">
            {candidate.artifactHash}
          </p>
          {(checked.sourceVerified === false ||
            comparison?.beforeVerified === false) && (
            <p className="text-warn">
              {t("library.skillUpdate.sourceUnverified")}
            </p>
          )}
          {!!checked.targetConflicts?.length && (
            <div className="space-y-1 text-warn">
              <p>{t("library.skillUpdate.targetModified")}</p>
              {checked.targetConflicts.map((path) => (
                <p key={path} className="break-all text-note">
                  {path}
                </p>
              ))}
            </div>
          )}
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() =>
              void run(() => loadComparison(candidate.ticketId, comparePath))
            }
          >
            {t("library.skillUpdate.retryCompare")}
          </Button>
          {(candidate.canCopy === false || comparison?.canCopy === false) && (
            <p className="text-warn">
              {libraryError(
                candidate.copyBlockedReason ??
                  "library.skillArtifactHashOccupied",
                t,
              )}
            </p>
          )}
          {!!compareFiles.length && (
            <MenuSelect
              aria-label={t("library.skillUpdate.compareFile")}
              value={comparePath}
              disabled={busy}
              onChange={(path) =>
                void run(() => loadComparison(candidate.ticketId, path))
              }
              searchable
              options={compareFiles.map((file) => ({
                value: file.path,
                label: file.path,
                note: `${t(`library.skillUpdate.file.${file.status}`)}${file.beforeExecutable !== null && file.beforeExecutable !== file.afterExecutable ? ` · ${t("library.skillUpdate.executionChanged")}` : ""}`,
              }))}
            />
          )}
          {comparison && (
            <>
              <div className="grid gap-2 sm:grid-cols-2">
                {(["before", "after"] as const).map((side) => (
                  <div key={side}>
                    <p className="text-note text-ink-muted">
                      {t(`library.skillUpdate.${side}`)}
                    </p>
                    <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded bg-pill p-2 text-note">
                      {comparison[side] ?? t("library.skillUpdate.noText")}
                    </pre>
                  </div>
                ))}
              </div>
            </>
          )}
          <Button
            disabled={
              busy ||
              readOnly ||
              !comparison ||
              comparison.path !== comparePath ||
              comparison.beforeVerified === false ||
              checked.sourceVerified === false ||
              !!checked.targetConflicts?.length
            }
            onClick={() => void run(() => confirm("update"))}
          >
            {t("library.skillUpdate.update")}
          </Button>
          <div className="space-y-2 border-t border-line pt-3">
            <p className="text-note text-ink-muted">
              {t("library.skillUpdate.copyHint")}
            </p>
            <label className="block space-y-1">
              <span>{t("library.name")}</span>
              <Input
                disabled={busy}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            <label className="block space-y-1">
              <span>{t("library.skillUpdate.copyDirectory")}</span>
              <Input
                disabled={busy}
                value={directory}
                onChange={(e) => setDirectory(e.target.value)}
              />
            </label>
            <Button
              variant="secondary"
              disabled={
                busy ||
                readOnly ||
                !comparison ||
                comparison.path !== comparePath ||
                candidate.canCopy === false ||
                comparison?.canCopy === false ||
                !name.trim() ||
                !directory.trim()
              }
              onClick={() => void run(() => confirm("copy"))}
            >
              {t("library.skillUpdate.copy")}
            </Button>
          </div>
        </>
      )}
      {result && (
        <p role="status" className="text-ink-muted">
          {t(`library.skillUpdate.${result.state}`)}
        </p>
      )}
    </div>
  );
}
