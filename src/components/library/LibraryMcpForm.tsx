import { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { SegmentedControl } from "@/components/ui/segmented";
import type { FieldEdit } from "@/lib/api/library";
import {
  createMcpDraft,
  type LibraryMcpDraft,
  type LibraryMcpPair,
} from "./libraryRecommendations";
export interface LibraryMcpFormProps {
  value: LibraryMcpDraft;
  onChange: (draft: LibraryMcpDraft) => void;
  busy?: boolean;
  onSave: (
    draft: LibraryMcpDraft,
    intent: "draft" | "apply",
  ) => void | Promise<unknown>;
  onAdvanced?: (draft: LibraryMcpDraft) => void;
}
const COMMON_FIELDS = new Set([
  "name",
  "type",
  "command",
  "args",
  "url",
  "env",
  "headers",
]);
const MASK = /\[REDACTED\]|«redacted»|<redacted>|^[•●*]{3,}$/i;
const isObject = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);
function rejectMasks(value: unknown) {
  if (typeof value === "number" && !Number.isFinite(value))
    throw new Error("invalidJson");
  if (typeof value === "string" && MASK.test(value))
    throw new Error("maskedValue");
  if (Array.isArray(value)) value.forEach(rejectMasks);
  else if (isObject(value)) Object.values(value).forEach(rejectMasks);
}
function pairEdits(
  pairs: LibraryMcpPair[],
  kind: "env" | "headers",
): FieldEdit[] {
  const names = new Set<string>();
  return pairs
    .filter((pair) => pair.name || pair.value)
    .map((pair) => {
      const name = pair.name.trim();
      const valid =
        kind === "env"
          ? /^[A-Za-z_][A-Za-z0-9_]*$/.test(name)
          : /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name);
      if (!valid || (kind === "headers" && /[\r\n]/.test(pair.value)))
        throw new Error("invalidPair");
      const key = kind === "headers" ? name.toLowerCase() : name;
      if (names.has(key))
        throw new Error(
          kind === "headers" ? "duplicateHeader" : "duplicateEnvironment",
        );
      names.add(key);
      rejectMasks(pair.value);
      return { path: [kind, name], action: "set", value: pair.value };
    });
}

/** Create-only conversion. Existing definitions retain their fieldRef keep/set/clear editor. */
export function mcpDraftToEdits(draft: LibraryMcpDraft): FieldEdit[] {
  if (!draft.name.trim()) throw new Error("missingName");
  const edits: FieldEdit[] = [
    { path: ["type"], action: "set", value: draft.transport },
  ];
  if (draft.transport === "stdio") {
    if (!draft.command.trim()) throw new Error("missingCommand");
    const args = draft.argsText
      ? draft.argsText.split(/\r?\n/).filter((line) => line.length > 0)
      : [];
    if (draft.requiresDirectory) {
      const directory = draft.directory?.trim() ?? "";
      if (!/^(\/|[A-Za-z]:[\\/]|\\\\)/.test(directory))
        throw new Error("missingDirectory");
      args.push(directory);
    }
    edits.push(
      { path: ["command"], action: "set", value: draft.command.trim() },
      { path: ["args"], action: "set", value: args },
      ...pairEdits(draft.environment, "env"),
    );
  } else if (draft.transport === "http" || draft.transport === "sse") {
    try {
      const url = new URL(draft.url.trim());
      if (
        !["http:", "https:"].includes(url.protocol) ||
        url.username ||
        url.password ||
        url.search
      )
        throw new Error();
    } catch {
      throw new Error("invalidUrl");
    }
    edits.push(
      { path: ["url"], action: "set", value: draft.url.trim() },
      ...pairEdits(draft.headers, "headers"),
    );
  } else throw new Error("advancedRequired");
  for (const field of draft.advancedFields ?? []) {
    if (field.path.length !== 1 || COMMON_FIELDS.has(field.path[0]))
      throw new Error("advancedRequired");
    edits.push({ path: [...field.path], action: "set", value: field.value });
  }
  edits.forEach((edit) => rejectMasks(edit.value));
  return edits;
}

function parsePairs(value: unknown): LibraryMcpPair[] {
  if (value === undefined) return [];
  if (
    !isObject(value) ||
    Object.values(value).some((item) => typeof item !== "string")
  )
    throw new Error("advancedRequired");
  return Object.entries(value).map(([name, value], index) => ({
    id: String(index),
    name,
    value: value as string,
  }));
}

/** Reads one canonical definition. No install, check, or IPC is performed. */
export function mcpDraftFromJson(input: string): LibraryMcpDraft {
  let parsed: unknown;
  try {
    if (input.length > 1024 * 1024) throw new Error();
    parsed = JSON.parse(input, (_key, value: unknown) => {
      if (typeof value === "number" && !Number.isFinite(value))
        throw new Error();
      return value;
    });
  } catch {
    throw new Error("invalidJson");
  }
  if (!isObject(parsed)) throw new Error("invalidJson");
  let name = typeof parsed.name === "string" ? parsed.name : "";
  if ("mcpServers" in parsed) {
    if (!isObject(parsed.mcpServers)) throw new Error("invalidJson");
    const servers = Object.entries(parsed.mcpServers);
    if (servers.length !== 1) throw new Error("oneServer");
    name = servers[0][0];
    if (!isObject(servers[0][1])) throw new Error("invalidJson");
    parsed = servers[0][1];
  }
  const config = parsed as Record<string, unknown>;
  const transport =
    config.type ?? (typeof config.url === "string" ? "http" : "stdio");
  if (!["stdio", "http", "sse"].includes(transport as string))
    throw new Error("advancedRequired");
  if (
    config.args !== undefined &&
    (!Array.isArray(config.args) ||
      config.args.some(
        (arg) => typeof arg !== "string" || !arg || /[\r\n]/.test(arg),
      ))
  )
    throw new Error("advancedRequired");
  if (
    (transport === "stdio" && ("url" in config || "headers" in config)) ||
    (transport !== "stdio" &&
      ("command" in config || "args" in config || "env" in config))
  )
    throw new Error("advancedRequired");
  if (
    (config.command !== undefined && typeof config.command !== "string") ||
    (config.url !== undefined && typeof config.url !== "string")
  )
    throw new Error("advancedRequired");
  rejectMasks(config);
  const draft = createMcpDraft({
    name,
    transport: transport as LibraryMcpDraft["transport"],
    command: (config.command as string | undefined) ?? "",
    argsText: ((config.args as string[] | undefined) ?? []).join("\n"),
    url: (config.url as string | undefined) ?? "",
    environment: parsePairs(config.env),
    headers: parsePairs(config.headers),
    advancedFields: Object.entries(config)
      .filter(([key]) => !COMMON_FIELDS.has(key))
      .map(([key, value]) => ({ path: [key], value })),
  });
  // A plain definition may have no name; the caller fills it in after paste.
  mcpDraftToEdits({ ...draft, name: draft.name || "server" });
  return draft;
}

function PairFields({
  kind,
  pairs,
  disabled,
  onChange,
}: {
  kind: "environment" | "headers";
  pairs: LibraryMcpPair[];
  disabled: boolean;
  onChange: (pairs: LibraryMcpPair[]) => void;
}) {
  const { t } = useTranslation();
  const add = useRef<HTMLButtonElement>(null);
  const title = t(`library.addFlow.${kind}`);
  const header = kind === "headers";
  return (
    <fieldset
      aria-label={title}
      className="min-w-0 space-y-2 rounded-ctl border border-line-2 p-3"
    >
      <legend className="px-1 text-ctl font-medium text-ink-2">{title}</legend>
      {pairs.map((pair, index) => (
        <div
          key={pair.id}
          className="flex min-w-0 flex-wrap items-center gap-2"
        >
          <Input
            className="min-w-[130px] flex-1 font-mono"
            aria-label={t("library.addFlow.pairName", { index: index + 1 })}
            value={pair.name}
            disabled={disabled}
            autoComplete="off"
            spellCheck={false}
            onChange={(event) =>
              onChange(
                pairs.map((item) =>
                  item.id === pair.id
                    ? { ...item, name: event.target.value }
                    : item,
                ),
              )
            }
          />
          <Input
            className="min-w-[130px] flex-1 font-mono"
            type="password"
            aria-label={t("library.addFlow.pairValue", { index: index + 1 })}
            value={pair.value}
            disabled={disabled}
            autoComplete="new-password"
            spellCheck={false}
            onChange={(event) =>
              onChange(
                pairs.map((item) =>
                  item.id === pair.id
                    ? { ...item, value: event.target.value }
                    : item,
                ),
              )
            }
          />
          <Button
            type="button"
            variant="icon"
            disabled={disabled}
            aria-label={t(
              header
                ? "library.addFlow.removeHeader"
                : "library.addFlow.removeEnvironment",
              { index: index + 1 },
            )}
            onClick={() => {
              onChange(pairs.filter((item) => item.id !== pair.id));
              add.current?.focus();
            }}
          >
            <X aria-hidden className="h-3.5 w-3.5" />
          </Button>
        </div>
      ))}
      <Button
        ref={add}
        type="button"
        variant="ghost"
        disabled={disabled}
        onClick={() =>
          onChange([...pairs, { id: crypto.randomUUID(), name: "", value: "" }])
        }
      >
        <Plus aria-hidden className="h-3.5 w-3.5" />
        {t(
          header
            ? "library.addFlow.addHeader"
            : "library.addFlow.addEnvironment",
        )}
      </Button>
    </fieldset>
  );
}

export function LibraryMcpForm({
  value,
  onChange,
  busy = false,
  onSave,
  onAdvanced,
}: LibraryMcpFormProps) {
  const { t } = useTranslation();
  const [error, setError] = useState("");
  const [working, setWorking] = useState(false);
  const submitting = useRef(false);
  const [paste, setPaste] = useState(false);
  const [json, setJson] = useState("");
  const pasteId = useId();
  const disabled = busy || working;
  const update = (patch: Partial<LibraryMcpDraft>) => {
    setError("");
    onChange({ ...value, ...patch });
  };
  const submit = async (intent: "draft" | "apply") => {
    if (disabled || submitting.current) return;
    try {
      mcpDraftToEdits(value);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "invalidJson");
      return;
    }
    submitting.current = true;
    setWorking(true);
    setError("");
    try {
      await onSave(value, intent);
    } catch {
      setError("saveFailed");
    } finally {
      submitting.current = false;
      setWorking(false);
    }
  };
  return (
    <form
      className="min-w-0 space-y-3"
      onSubmit={(event) => {
        event.preventDefault();
        void submit("draft");
      }}
      onKeyDown={(event) => {
        if (
          (event.metaKey || event.ctrlKey) &&
          event.key.toLowerCase() === "s"
        ) {
          event.preventDefault();
          void submit("draft");
        }
      }}
    >
      {error && (
        <p role="alert" className="text-ctl text-danger">
          {t(`library.addFlow.errors.${error}`)}
        </p>
      )}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <SegmentedControl
          aria-label={t("library.addFlow.transport")}
          value={value.transport}
          onChange={(transport) => update({ transport })}
          disabled={disabled}
          options={[
            { value: "stdio", label: t("library.addFlow.stdio") },
            { value: "http", label: t("library.addFlow.http") },
            { value: "sse", label: t("library.addFlow.sse") },
          ]}
        />
        <Button
          type="button"
          variant="ghost"
          disabled={disabled}
          aria-expanded={paste}
          aria-controls={pasteId}
          onClick={() => setPaste(!paste)}
        >
          {t("library.addFlow.paste")}
        </Button>
      </div>
      {paste && (
        <div
          id={pasteId}
          className="space-y-2 rounded-ctl border border-line-2 bg-card-2 p-3"
        >
          <Textarea
            className="font-mono"
            aria-label={t("library.addFlow.jsonLabel")}
            value={json}
            disabled={disabled}
            onChange={(event) => setJson(event.target.value)}
          />
          <Button
            type="button"
            variant="secondary"
            disabled={disabled || !json.trim()}
            onClick={() => {
              try {
                const draft = mcpDraftFromJson(json);
                onChange({ ...draft, name: draft.name || value.name });
                setJson("");
                setPaste(false);
                setError("");
              } catch (cause) {
                setError(
                  cause instanceof Error ? cause.message : "invalidJson",
                );
              }
            }}
          >
            {t("library.addFlow.useJson")}
          </Button>
        </div>
      )}
      <label className="block space-y-1 text-ctl">
        <span>{t("library.addFlow.name")}</span>
        <Input
          value={value.name}
          disabled={disabled}
          onChange={(event) => update({ name: event.target.value })}
        />
      </label>
      {value.transport === "stdio" ? (
        <>
          <label className="block space-y-1 text-ctl">
            <span>{t("library.addFlow.command")}</span>
            <Input
              className="font-mono"
              value={value.command}
              disabled={disabled}
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => update({ command: event.target.value })}
            />
          </label>
          <label className="block space-y-1 text-ctl">
            <span>{t("library.addFlow.arguments")}</span>
            <Textarea
              className="min-h-[92px] font-mono"
              value={value.argsText}
              disabled={disabled}
              onChange={(event) => update({ argsText: event.target.value })}
            />
            <span className="block text-note text-ink-muted">
              {t("library.addFlow.argumentsHint")}
            </span>
          </label>
          {value.requiresDirectory && (
            <label className="block space-y-1 text-ctl">
              <span>{t("library.addFlow.directory")}</span>
              <Input
                value={value.directory ?? ""}
                disabled={disabled}
                onChange={(event) => update({ directory: event.target.value })}
              />
              <span className="block text-note text-ink-muted">
                {t("library.addFlow.directoryHint")}
              </span>
            </label>
          )}
          <PairFields
            kind="environment"
            pairs={value.environment}
            disabled={disabled}
            onChange={(environment) => update({ environment })}
          />
        </>
      ) : (
        <>
          <label className="block space-y-1 text-ctl">
            <span>{t("library.addFlow.url")}</span>
            <Input
              className="font-mono"
              value={value.url}
              disabled={disabled}
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => update({ url: event.target.value })}
            />
          </label>
          <PairFields
            kind="headers"
            pairs={value.headers}
            disabled={disabled}
            onChange={(headers) => update({ headers })}
          />
        </>
      )}
      {!!value.advancedFields?.length && (
        <p className="text-note text-ink-muted">
          {t("library.addFlow.extraFields", {
            count: value.advancedFields.length,
          })}
        </p>
      )}
      <p className="text-note text-ink-muted">
        {t("library.addFlow.secretsHint")}
      </p>
      <div className="flex flex-wrap items-center gap-2 border-t border-line-2 pt-3">
        <Button
          type="button"
          disabled={disabled}
          onClick={() => void submit("apply")}
        >
          {t("library.addFlow.preview")}
        </Button>
        <Button type="submit" variant="secondary" disabled={disabled}>
          {t("library.addFlow.save")}
        </Button>
        {onAdvanced && (
          <Button
            type="button"
            variant="ghost"
            disabled={disabled}
            onClick={() => onAdvanced(value)}
          >
            {t("library.addFlow.advanced")}
          </Button>
        )}
      </div>
      <p className="text-note text-ink-muted">
        {t("library.addFlow.draftNote")}
      </p>
    </form>
  );
}
