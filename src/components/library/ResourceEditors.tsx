import { useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import MarkdownEditor from "@/components/MarkdownEditor";
import { useDarkMode } from "@/hooks/useDarkMode";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { MenuSelect } from "@/components/ui/menu-select";
import { ListCard, ListRow } from "@/components/ui/list";
import {
  libraryApi,
  type FieldEdit,
  type InstructionSet,
  type McpDefinition,
  type SkillDefinition,
} from "@/lib/api/library";
import { promptsApi } from "@/lib/api/prompts";
import {
  skillsApi,
  type DiscoverableSkill,
  type SkillBackupEntry,
  type UnmanagedSkill,
} from "@/lib/api/skills";
import type { AppId } from "@/lib/api/types";
import { useDraftState, type DraftStore } from "./useDraftState";
import { libraryError } from "./libraryText";

export function InstructionEditor({
  store,
  initial,
  importMode,
  busy,
  onSave,
}: {
  store: DraftStore;
  initial?: InstructionSet;
  importMode?: boolean;
  busy: boolean;
  onSave: (set: InstructionSet, imported: boolean) => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useDraftState<InstructionSet>(
    store,
    "instructionDraft",
    () =>
      initial ?? {
        id: crypto.randomUUID(),
        name: "",
        content: "",
        revision: 0,
      },
  );
  const [error, setError] = useDraftState(store, "instructionError", "");
  const [source, setSource] = useDraftState(
    store,
    "instructionSource",
    "markdown",
  );
  const [legacy, setLegacy] = useDraftState<InstructionSet[]>(
    store,
    "legacyPrompts",
    [],
  );
  const [loading, setLoading] = useDraftState(
    store,
    "instructionLoading",
    false,
  );
  const file = useRef<HTMLInputElement>(null);
  const [editGeneration] = useDraftState(
    store,
    "instructionEditGeneration",
    () => ({ current: 0 }),
  );
  const isDarkMode = useDarkMode();
  return (
    <form
      className="space-y-3"
      onSubmit={(event) => {
        event.preventDefault();
        void onSave(draft, !!importMode);
      }}
    >
      {importMode && (
        <>
          <MenuSelect
            aria-label={t("library.importSource")}
            value={source}
            disabled={busy || loading}
            onChange={(value) => {
              editGeneration.current++;
              setSource(value);
              setLegacy([]);
            }}
            options={[
              { value: "markdown", label: t("library.markdownFile") },
              ...["claude", "codex", "gemini", "opencode", "hermes"].map(
                (tool) => ({
                  value: tool,
                  label: t("library.legacyPrompts", { tool }),
                }),
              ),
            ]}
          />
          {source === "markdown" ? (
            <>
              <input
                ref={file}
                className="hidden"
                type="file"
                accept=".md,.markdown,text/markdown,text/plain"
                aria-label={t("library.markdownFile")}
                onChange={async (event) => {
                  const selected = event.target.files?.[0];
                  if (!selected) return;
                  if (selected.size > 1024 * 1024) {
                    setError(t("library.skillFileTooLarge"));
                    return;
                  }
                  try {
                    const generation = ++editGeneration.current;
                    const content = await selected.text();
                    if (generation !== editGeneration.current) return;
                    setDraft((previous) => ({
                      ...previous,
                      name:
                        previous.name ||
                        selected.name.replace(/\.(md|markdown)$/i, ""),
                      content,
                    }));
                    setError("");
                  } catch {
                    setError(t("library.failed"));
                  }
                }}
              />
              <Button
                type="button"
                variant="secondary"
                disabled={busy}
                onClick={() => file.current?.click()}
              >
                {t("library.chooseFile")}
              </Button>
            </>
          ) : (
            <>
              <Button
                type="button"
                variant="secondary"
                disabled={busy || loading}
                onClick={async () => {
                  setLoading(true);
                  setError("");
                  try {
                    const rows = await promptsApi.getPrompts(source as AppId);
                    setLegacy(
                      Object.values(rows).map((row) => ({
                        id: row.id,
                        name: row.name,
                        content: row.content,
                        revision: 0,
                      })),
                    );
                  } catch (e) {
                    setError(libraryError(e, t));
                  } finally {
                    setLoading(false);
                  }
                }}
              >
                {t("library.loadLegacy")}
              </Button>
              <ListCard>
                {legacy.map((row) => (
                  <ListRow key={row.id}>
                    <span className="min-w-0 flex-1 break-words text-ctl">
                      {row.name}
                    </span>
                    <Button
                      type="button"
                      variant="secondary"
                      disabled={busy || loading}
                      onClick={() => {
                        editGeneration.current++;
                        setDraft((previous) => ({
                          ...previous,
                          name: row.name,
                          content: row.content,
                        }));
                      }}
                    >
                      {t("library.useContent")}
                    </Button>
                  </ListRow>
                ))}
              </ListCard>
            </>
          )}
          <p className="text-note text-ink-muted">
            {t("library.legacyDescription")}
          </p>
        </>
      )}
      {error && (
        <p role="alert" className="text-ctl text-danger">
          {error}
        </p>
      )}
      <label className="block space-y-1 text-ctl">
        <span>{t("library.name")}</span>
        <Input
          required
          value={draft.name}
          disabled={busy}
          onChange={(event) => {
            editGeneration.current++;
            setDraft((previous) => ({ ...previous, name: event.target.value }));
          }}
        />
      </label>
      <MarkdownEditor
        value={draft.content}
        onChange={(content) => {
          editGeneration.current++;
          setDraft((previous) => ({ ...previous, content }));
        }}
        darkMode={isDarkMode}
        readOnly={busy}
        minHeight="240px"
        maxHeight="45vh"
      />
      <Button type="submit" disabled={busy || !draft.name.trim()}>
        {t("library.saveDraft")}
      </Button>
    </form>
  );
}

type EditableField = FieldEdit & { input: string; key: string };
export function McpEditor({
  store,
  initial,
  busy,
  onSave,
}: {
  store: DraftStore;
  initial?: McpDefinition;
  busy: boolean;
  onSave: (
    id: string,
    name: string,
    revision: number | undefined,
    edits: FieldEdit[],
  ) => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [id] = useDraftState(
    store,
    "mcpId",
    () => initial?.id ?? crypto.randomUUID(),
  );
  const [name, setName] = useDraftState(store, "mcpName", initial?.name ?? "");
  const [fields, setFields] = useDraftState<EditableField[]>(
    store,
    "mcpFields",
    () =>
      initial
        ? initial.fields.map((field, index) => ({
            path: field.path,
            fieldRef: field.fieldRef,
            action: "keep",
            input: "",
            key: String(index),
          }))
        : [
            { path: ["type"], action: "set", input: '"stdio"', key: "type" },
            { path: ["command"], action: "set", input: '""', key: "command" },
          ],
  );
  const [path, setPath] = useDraftState(
    store,
    "mcpPath",
    '["env", "VARIABLE_NAME"]',
  );
  const [error, setError] = useDraftState(store, "mcpError", "");
  const update = (key: string, edit: Partial<EditableField>) =>
    setFields((previous) =>
      previous.map((field) =>
        field.key === key ? { ...field, ...edit } : field,
      ),
    );
  return (
    <form
      className="space-y-3"
      onSubmit={(event) => {
        event.preventDefault();
        setError("");
        try {
          const edits: FieldEdit[] = fields.map((field) => ({
            path: field.path,
            ...(field.fieldRef ? { fieldRef: field.fieldRef } : {}),
            action: field.action,
            ...(field.action === "set"
              ? { value: JSON.parse(field.input) }
              : {}),
          }));
          void onSave(id, name, initial?.revision, edits);
        } catch {
          setError(t("library.invalidFieldValue"));
        }
      }}
    >
      <p className="text-note text-ink-muted">
        {t("library.privateFieldsDescription")}
      </p>
      {error && (
        <p role="alert" className="text-ctl text-danger">
          {error}
        </p>
      )}
      <label className="block space-y-1 text-ctl">
        <span>{t("library.name")}</span>
        <Input
          required
          value={name}
          disabled={busy}
          onChange={(event) => setName(event.target.value)}
        />
      </label>
      <ListCard>
        {fields.map((field) => (
          <ListRow key={field.key} className="flex-wrap">
            <code className="min-w-0 flex-1 break-all text-note">
              {JSON.stringify(field.path)}
            </code>
            <MenuSelect
              aria-label={t("library.fieldAction", {
                path: field.path.join("."),
              })}
              value={field.action}
              disabled={busy}
              onChange={(action) => update(field.key, { action })}
              options={(field.fieldRef
                ? ["keep", "set", "clear"]
                : ["set", "clear"]
              ).map((action) => ({
                value: action as FieldEdit["action"],
                label: t(`library.editAction.${action}`),
              }))}
            />
            {field.action === "set" && (
              <Input
                className="w-full font-mono text-note"
                aria-label={t("library.fieldValue", {
                  path: field.path.join("."),
                })}
                value={field.input}
                disabled={busy}
                placeholder={t("library.jsonValue")}
                onChange={(event) =>
                  update(field.key, { input: event.target.value })
                }
              />
            )}
          </ListRow>
        ))}
      </ListCard>
      <div className="flex flex-wrap gap-2">
        <Input
          className="min-w-0 flex-1 font-mono text-note"
          aria-label={t("library.fieldPath")}
          value={path}
          disabled={busy}
          onChange={(event) => setPath(event.target.value)}
        />
        <Button
          type="button"
          variant="secondary"
          disabled={busy}
          onClick={() => {
            try {
              const parsed: unknown = JSON.parse(path);
              if (
                !Array.isArray(parsed) ||
                !parsed.length ||
                parsed.some((part) => typeof part !== "string" || !part) ||
                fields.some(
                  (field) =>
                    JSON.stringify(field.path) === JSON.stringify(parsed),
                )
              )
                throw new Error();
              setFields((previous) => [
                ...previous,
                {
                  path: parsed,
                  action: "set",
                  input: '""',
                  key: crypto.randomUUID(),
                },
              ]);
              setError("");
            } catch {
              setError(t("library.invalidFieldPath"));
            }
          }}
        >
          {t("library.addField")}
        </Button>
      </div>
      <Button type="submit" disabled={busy || !name.trim()}>
        {t("library.saveDraft")}
      </Button>
    </form>
  );
}

export function SkillDetails({
  store,
  skill,
}: {
  store: DraftStore;
  skill: SkillDefinition;
}) {
  const { t } = useTranslation();
  const [path, setPath] = useDraftState(store, "skillPath", "SKILL.md");
  const query = useQuery({
    queryKey: ["library", "skillDetail", skill.id, skill.revision, path],
    queryFn: () => libraryApi.skillDetail(skill.id, path),
    retry: false,
  });
  const [files, setFiles] = useDraftState<
    NonNullable<typeof query.data>["files"]
  >(store, "verifiedSkillFiles", []);
  useEffect(() => {
    if (query.data) setFiles(query.data.files);
  }, [query.data, setFiles]);
  return (
    <div className="space-y-3 text-ctl">
      <p className="break-all text-note text-ink-muted">
        {skill.artifactHash ?? "—"}
      </p>
      <p className="text-note text-ink-muted">
        {t(`library.sourceState.${skill.sourceState}`, {
          defaultValue: t("library.unknown"),
        })}
      </p>
      {query.isPending && <p role="status">{t("library.loading")}</p>}
      {query.isError && (
        <p role="alert" className="text-danger">
          {libraryError(query.error, t)}
        </p>
      )}
      {!!files.length && (
        <MenuSelect
          aria-label={t("library.skillFiles")}
          searchable
          width={380}
          value={path}
          onChange={setPath}
          options={files
            .filter((file) => !file.isDirectory)
            .map((file) => ({
              value: file.path,
              label: file.path,
              note: `${file.sizeBytes ?? 0} B${file.executable ? ` · ${t("library.executable")}` : ""}`,
            }))}
        />
      )}
      {query.data && (
        <pre className="max-h-[45vh] overflow-auto whitespace-pre-wrap break-words rounded-card bg-pill p-3 text-note">
          {query.data.content}
        </pre>
      )}
    </div>
  );
}

type ImportMode = "directory" | "zip" | "repository" | "existing" | "backup";
export function SkillImporter({
  store,
  busy,
  onImport,
}: {
  store: DraftStore;
  busy: boolean;
  onImport: (action: () => Promise<unknown>) => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [mode, setMode] = useDraftState<ImportMode>(
    store,
    "skillImportMode",
    "directory",
  );
  const [source, setSource] = useDraftState(store, "skillImportSource", "");
  const [name, setName] = useDraftState(store, "skillImportName", "");
  const [directory, setDirectory] = useDraftState(
    store,
    "skillImportDirectory",
    "",
  );
  const [repo, setRepo] = useDraftState(store, "skillRepository", "");
  const [branch, setBranch] = useDraftState(store, "skillRef", "main");
  const [available, setAvailable] = useDraftState<DiscoverableSkill[]>(
    store,
    "discoveredSkills",
    [],
  );
  const [unmanaged, setUnmanaged] = useDraftState<UnmanagedSkill[]>(
    store,
    "unmanagedSkills",
    [],
  );
  const [backups, setBackups] = useDraftState<SkillBackupEntry[]>(
    store,
    "skillBackups",
    [],
  );
  const [loading, setLoading] = useDraftState(
    store,
    "skillImportLoading",
    false,
  );
  const [error, setError] = useDraftState(store, "skillImportError", "");
  const running = useRef(false);
  async function load(action: () => Promise<unknown>) {
    if (running.current || busy || loading) return;
    running.current = true;
    setLoading(true);
    setError("");
    try {
      await action();
    } catch (e) {
      setError(libraryError(e, t));
    } finally {
      running.current = false;
      setLoading(false);
    }
  }
  return (
    <div className="space-y-3 text-ctl">
      <MenuSelect
        aria-label={t("library.importSource")}
        value={mode}
        disabled={busy || loading}
        onChange={(value) => {
          setMode(value);
          setSource("");
          setError("");
        }}
        options={(
          ["directory", "zip", "repository", "existing", "backup"] as const
        ).map((value) => ({ value, label: t(`library.skillImport.${value}`) }))}
      />
      <p className="text-note text-ink-muted">
        {t("library.skillImportDescription")}
      </p>
      {error && (
        <p role="alert" className="text-danger">
          {error}
        </p>
      )}
      {mode === "directory" && (
        <>
          <Button
            variant="secondary"
            disabled={busy || loading}
            onClick={() =>
              void load(async () => {
                const path = await libraryApi.pickDirectory();
                if (path) setSource(path);
              })
            }
          >
            {t("library.chooseDirectory")}
          </Button>
          <p className="break-all text-note text-ink-muted">{source || "—"}</p>
          <label className="block space-y-1">
            <span>{t("library.name")}</span>
            <Input
              value={name}
              disabled={busy || loading}
              onChange={(event) => setName(event.target.value)}
            />
          </label>
          <label className="block space-y-1">
            <span>{t("library.directoryName")}</span>
            <Input
              value={directory}
              disabled={busy || loading}
              onChange={(event) => setDirectory(event.target.value)}
            />
          </label>
          <Button
            disabled={
              busy || loading || !source || !name.trim() || !directory.trim()
            }
            onClick={() =>
              void onImport(() =>
                libraryApi.importSkillDirectory(source, directory, name),
              )
            }
          >
            {t("library.importDraft")}
          </Button>
        </>
      )}
      {mode === "zip" && (
        <>
          <Button
            variant="secondary"
            disabled={busy || loading}
            onClick={() =>
              void load(async () => {
                const path = await skillsApi.openZipFileDialog();
                if (path) setSource(path);
              })
            }
          >
            {t("library.chooseFile")}
          </Button>
          <p className="break-all text-note text-ink-muted">{source || "—"}</p>
          <Button
            disabled={busy || loading || !source}
            onClick={() =>
              void onImport(() => skillsApi.installFromZip(source, "claude"))
            }
          >
            {t("library.importDraft")}
          </Button>
        </>
      )}
      {mode === "repository" && (
        <>
          <label className="block space-y-1">
            <span>{t("library.githubRepository")}</span>
            <Input
              value={repo}
              disabled={busy || loading}
              placeholder="owner/repository"
              onChange={(event) => setRepo(event.target.value)}
            />
          </label>
          <label className="block space-y-1">
            <span>{t("library.gitRef")}</span>
            <Input
              value={branch}
              disabled={busy || loading}
              onChange={(event) => setBranch(event.target.value)}
            />
          </label>
          <Button
            variant="secondary"
            disabled={busy || loading || !repo || !branch}
            onClick={() =>
              void load(async () => {
                const parts = repo.trim().split("/");
                if (
                  parts.length !== 2 ||
                  parts.some((part) => !part || !/^[A-Za-z0-9_.-]+$/.test(part))
                )
                  throw new Error();
                await skillsApi.addRepo({
                  owner: parts[0],
                  name: parts[1],
                  branch,
                  enabled: true,
                });
                setAvailable(
                  (await skillsApi.discoverAvailable()).filter(
                    (skill) =>
                      skill.repoOwner === parts[0] &&
                      skill.repoName === parts[1] &&
                      skill.repoBranch === branch,
                  ),
                );
              })
            }
          >
            {t("library.discover")}
          </Button>
          <ListCard>
            {available.map((skill) => (
              <ListRow key={skill.key}>
                <span className="min-w-0 flex-1 break-words">{skill.name}</span>
                <Button
                  disabled={busy || loading}
                  variant="secondary"
                  onClick={() =>
                    void onImport(() =>
                      skillsApi.installUnified(skill, "claude"),
                    )
                  }
                >
                  {t("library.importDraft")}
                </Button>
              </ListRow>
            ))}
          </ListCard>
        </>
      )}
      {mode === "existing" && (
        <>
          <Button
            variant="secondary"
            disabled={busy || loading}
            onClick={() =>
              void load(async () =>
                setUnmanaged(await skillsApi.scanUnmanaged()),
              )
            }
          >
            {t("library.scanExisting")}
          </Button>
          <ListCard>
            {unmanaged.map((skill) => (
              <ListRow key={skill.path}>
                <span className="min-w-0 flex-1 break-all">
                  {skill.name}
                  <span className="block text-note text-ink-muted">
                    {skill.path}
                  </span>
                </span>
                <Button
                  disabled={busy || loading}
                  variant="secondary"
                  onClick={() =>
                    void onImport(() =>
                      libraryApi.importSkillDirectory(
                        skill.path,
                        skill.directory,
                        skill.name,
                      ),
                    )
                  }
                >
                  {t("library.importDraft")}
                </Button>
              </ListRow>
            ))}
          </ListCard>
        </>
      )}
      {mode === "backup" && (
        <>
          <Button
            variant="secondary"
            disabled={busy || loading}
            onClick={() =>
              void load(async () => setBackups(await skillsApi.getBackups()))
            }
          >
            {t("library.loadBackups")}
          </Button>
          <ListCard>
            {backups.map((backup) => (
              <ListRow key={backup.backupId}>
                <span className="min-w-0 flex-1 break-words">
                  {backup.skill.name}
                </span>
                <Button
                  disabled={busy || loading}
                  variant="secondary"
                  onClick={() =>
                    void onImport(() =>
                      skillsApi.restoreBackup(backup.backupId, "claude"),
                    )
                  }
                >
                  {t("library.restoreDraft")}
                </Button>
              </ListRow>
            ))}
          </ListCard>
        </>
      )}
      {loading && (
        <p role="status" className="text-ink-muted">
          {t("library.loading")}
        </p>
      )}
    </div>
  );
}
