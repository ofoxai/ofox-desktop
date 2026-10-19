import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { SegmentedControl } from "@/components/ui/segmented";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { MenuSelect } from "@/components/ui/menu-select";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { isWindows } from "@/lib/platform";
import type {
  FieldEdit,
  McpDefinition,
  McpField,
  ResourceTarget,
} from "@/lib/api/library";
import {
  type LibraryMcpDraft,
  type LibraryMcpPair,
} from "./libraryRecommendations";
import { toolLabel } from "./libraryText";
import type { LibraryEditorController } from "./useLibraryInline";
import {
  createMcpIdentifier,
  mcpDraftsFromJson,
  mcpDraftToEdits,
  mcpEditDelta,
  mcpPairEdits,
  parseMcpJsonValue,
  parseMcpCommand,
  type ExactMcpDraft,
  type ParsedMcpCommand,
} from "./libraryMcpInput";
export { mcpDraftFromJson, mcpDraftToEdits } from "./libraryMcpInput";

export interface LibraryMcpFormProps {
  value: LibraryMcpDraft;
  onChange: (draft: LibraryMcpDraft) => void;
  busy?: boolean;
  onSave: (
    draft: LibraryMcpDraft,
    intent: "draft" | "apply",
  ) => void | Promise<unknown>;
  onAdvanced?: (draft: LibraryMcpDraft, dirty: boolean) => void;
  initial?: McpDefinition;
  identifier?: string;
  existingIdentifiers?: string[];
  onIdentifierChange?: (identifier: string) => void;
  targets?: ResourceTarget[];
  selectedTools?: string[];
  onToolsChange?: (tools: string[]) => void;
  onSaveEdits?: (
    edits: FieldEdit[],
    intent: "draft" | "apply",
  ) => void | Promise<unknown>;
  onImportMany?: (
    drafts: LibraryMcpDraft[],
    intent: "draft" | "apply",
  ) => void | Promise<unknown>;
  onDiscover?: () => void;
  saveOnly?: boolean;
  /** The apply button writes the chosen tools at once, without a preview step. */
  immediate?: boolean;
  initialFocusPath?: string[];
  onController?: (controller: LibraryEditorController) => void;
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

function SavedField({
  field,
  edit,
  disabled,
  onChange,
}: {
  field: McpField;
  edit?: FieldEdit;
  disabled: boolean;
  onChange: (edit: FieldEdit) => void;
}) {
  const { t } = useTranslation();
  const label = field.path.join(".");
  const action = edit?.action ?? "keep";
  const [input, setInput] = useState("");
  return (
    <div
      data-mcp-field={JSON.stringify(field.path)}
      className="flex min-w-0 flex-wrap items-center gap-2 rounded-ctl border border-line-2 p-2"
    >
      <span className="min-w-0 flex-1 break-all font-mono text-note">
        {label}
      </span>
      <MenuSelect
        aria-label={t("library.mcpComposer.action", { field: label })}
        value={action}
        disabled={disabled}
        onChange={(next) => {
          let replacement: unknown = input;
          if (field.valueType !== "string") {
            try {
              replacement = parseMcpJsonValue(input);
            } catch {
              replacement = undefined;
            }
          }
          onChange({
            path: field.path,
            action: next,
            ...(next === "set" ? { value: replacement } : {}),
          });
        }}
        options={["keep", "set", "clear"].map((value) => ({
          value: value as FieldEdit["action"],
          label: t(
            `library.mcpComposer.${value === "set" ? "replace" : value}`,
          ),
        }))}
      />
      {action === "set" && (
        <Input
          className="w-full font-mono"
          type={field.valueType === "string" ? "password" : "text"}
          autoComplete="new-password"
          aria-label={t("library.mcpComposer.savedValue", { field: label })}
          value={input}
          disabled={disabled}
          placeholder={
            field.valueType === "string"
              ? ""
              : t("library.mcpComposer.jsonValue")
          }
          onChange={(event) => {
            const text = event.target.value;
            setInput(text);
            let value: unknown = text;
            if (field.valueType !== "string") {
              try {
                value = parseMcpJsonValue(text);
              } catch {
                value = undefined;
              }
            }
            onChange({ path: field.path, action: "set", value });
          }}
        />
      )}
    </div>
  );
}

export function LibraryMcpForm({
  value,
  onChange,
  busy = false,
  onSave,
  onAdvanced,
  initial,
  identifier,
  existingIdentifiers = [],
  onIdentifierChange,
  targets,
  selectedTools = [],
  onToolsChange,
  onSaveEdits,
  onImportMany,
  onDiscover,
  saveOnly = false,
  immediate = false,
  initialFocusPath,
  onController,
}: LibraryMcpFormProps) {
  const { t } = useTranslation();
  const [error, setError] = useState("");
  const [working, setWorking] = useState(false);
  const [mode, setMode] = useState<"form" | "paste">("form");
  const [json, setJson] = useState("");
  const [candidates, setCandidates] = useState<ExactMcpDraft[]>([]);
  const [selectedJson, setSelectedJson] = useState<number[]>([]);
  const [commandPaste, setCommandPaste] = useState(false);
  const [commandInput, setCommandInput] = useState("");
  const [commandPlatform, setCommandPlatform] = useState<"posix" | "windows">(
    () => (isWindows() ? "windows" : "posix"),
  );
  const [parsedCommand, setParsedCommand] = useState<ParsedMcpCommand | null>(
    null,
  );
  const [changes, setChanges] = useState<FieldEdit[]>([]);
  const [invalidInputs, setInvalidInputs] = useState<string[]>([]);
  const [transportConfirmed, setTransportConfirmed] = useState(false);
  const [localIdentifier, setLocalIdentifier] = useState(() =>
    createMcpIdentifier(value.name, existingIdentifiers),
  );
  const manualIdentifier = useRef(false);
  const submittedIdentifier = useRef(false);
  const submitting = useRef(false);
  const generation = useRef(0);
  const previousInitial = useRef(initial);
  const fieldsId = useId();
  const form = useRef<HTMLFormElement>(null);
  const baseline = useRef({ value, identifier });
  const focusKey = JSON.stringify(initialFocusPath);
  useLayoutEffect(() => {
    if (!initialFocusPath) return;
    const group = Array.from(
      form.current?.querySelectorAll<HTMLElement>("[data-mcp-field]") ?? [],
    ).find((node) => node.dataset.mcpField === focusKey);
    for (
      let parent = group?.parentElement;
      parent;
      parent = parent.parentElement
    ) {
      if (parent instanceof HTMLDetailsElement) parent.open = true;
      if (parent === form.current) break;
    }
    const target =
      group?.querySelector<HTMLElement>(
        '[role="combobox"], input, textarea, button',
      ) ?? form.current;
    target?.scrollIntoView?.({ block: "center" });
    target?.focus();
  }, [focusKey, initial?.id, initial?.revision]);
  useEffect(() => {
    generation.current++;
    submitting.current = false;
    setWorking(false);
    setChanges([]);
    setInvalidInputs([]);
    setTransportConfirmed(false);
    previousInitial.current = initial;
    return () => {
      generation.current++;
    };
  }, [initial?.id, initial?.revision]);
  const disabled = busy || working;
  const configIdentifier = initial?.id ?? identifier ?? localIdentifier;
  const changedTransport = !!initial && value.transport !== initial.transport;
  const incompatible = changedTransport
    ? initial.fields.filter(
        (field) =>
          field.exists &&
          (value.transport === "stdio"
            ? ["url", "headers"]
            : ["command", "args", "env"]
          ).includes(field.path[0]),
      )
    : [];
  const setField = (edit: FieldEdit) => {
    setChanges((previous) => [
      ...previous.filter(
        (item) => JSON.stringify(item.path) !== JSON.stringify(edit.path),
      ),
      edit,
    ]);
    setError("");
  };
  const update = (patch: Partial<ExactMcpDraft>) => {
    setError("");
    if (
      patch.name !== undefined &&
      !initial &&
      !manualIdentifier.current &&
      !submittedIdentifier.current
    ) {
      const next = createMcpIdentifier(patch.name, existingIdentifiers);
      setLocalIdentifier(next);
      onIdentifierChange?.(next);
    }
    onChange({ ...value, ...patch });
  };
  const errorMessage = (code: string) =>
    t(`library.mcpComposer.errors.${code}`, {
      defaultValue: t(`library.addFlow.errors.${code}`, {
        defaultValue: t("library.mcpComposer.errors.saveFailed"),
      }),
    });
  const run = async (callback: () => void | Promise<unknown>) => {
    if (disabled || submitting.current) return false;
    submitting.current = true;
    setWorking(true);
    setError("");
    const current = generation.current;
    try {
      const saved = await callback();
      return saved !== false;
    } catch {
      if (current === generation.current) setError("saveFailed");
      return false;
    } finally {
      if (current === generation.current) {
        submitting.current = false;
        setWorking(false);
      }
    }
  };
  const submit = async (intent: "draft" | "apply") => {
    if (disabled || submitting.current) return false;
    submittedIdentifier.current = true;
    if (!initial) onIdentifierChange?.(configIdentifier);
    let edits: FieldEdit[] = [];
    try {
      if (!value.name.trim()) throw new Error("missingName");
      if (invalidInputs.length) throw new Error("invalidJson");
      if (initial) {
        if (
          previousInitial.current?.id !== initial.id ||
          previousInitial.current?.revision !== initial.revision
        )
          throw new Error("saveFailed");
        if (!onSaveEdits) throw new Error("advancedRequired");
        if (
          changes.some(
            (edit) => edit.action === "set" && edit.value === undefined,
          )
        )
          throw new Error("invalidJson");
        const pairChanges =
          value.transport === "stdio"
            ? mcpPairEdits(value.environment, "env")
            : mcpPairEdits(value.headers, "headers");
        for (const edit of pairChanges) {
          if (
            initial.fields.some(
              (field) =>
                field.path.length === 2 &&
                field.path[0] === edit.path[0] &&
                (edit.path[0] === "headers"
                  ? field.path[1].toLowerCase() === edit.path[1].toLowerCase()
                  : field.path[1] === edit.path[1]),
            )
          )
            throw new Error(
              edit.path[0] === "headers"
                ? "duplicateHeader"
                : "duplicateEnvironment",
            );
        }
        edits = mcpEditDelta(
          initial,
          [...changes, ...pairChanges],
          transportConfirmed,
        );
        if (
          changedTransport &&
          value.transport === "stdio" &&
          !edits.some(
            (edit) =>
              edit.path[0] === "command" &&
              edit.action === "set" &&
              typeof edit.value === "string" &&
              edit.value.trim(),
          )
        )
          throw new Error("missingCommand");
        if (
          changedTransport &&
          value.transport !== "stdio" &&
          !initial.fields.some(
            (field) => field.path[0] === "url" && field.exists,
          ) &&
          !edits.some((edit) => edit.path[0] === "url" && edit.action === "set")
        )
          throw new Error("invalidUrl");
      } else {
        if (!/^[A-Za-z0-9_-]{1,80}$/.test(configIdentifier))
          throw new Error("invalidIdentifier");
        if (existingIdentifiers.includes(configIdentifier))
          throw new Error("duplicateIdentifier");
        mcpDraftToEdits(value);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "invalidJson");
      return false;
    }
    return run(() =>
      initial && onSaveEdits
        ? onSaveEdits(edits, intent)
        : onSave({ ...value, identifier: configIdentifier }, intent),
    );
  };
  const dirty =
    JSON.stringify(value) !== JSON.stringify(baseline.current.value) ||
    identifier !== baseline.current.identifier ||
    changes.some((edit) => edit.action !== "keep") ||
    !!json.trim() ||
    !!commandInput.trim();
  useEffect(() => {
    onController?.({
      dirty,
      saving: working,
      save: async () => (await submit("draft")) === true,
      discard: () => {
        onChange(baseline.current.value);
        onIdentifierChange?.(baseline.current.identifier ?? "");
        setChanges([]);
        setJson("");
        setCommandInput("");
        setCandidates([]);
        setSelectedJson([]);
        setError("");
      },
    });
  });
  const setScalar = (path: string, text: string) => {
    update({ [path]: text });
    if (initial) setField({ path: [path], action: "set", value: text });
  };
  const stored = (path: string) =>
    initial?.fields.some(
      (field) =>
        field.exists && field.path.length === 1 && field.path[0] === path,
    );
  const scalarAction = (path: string) =>
    changes.find((edit) => edit.path.length === 1 && edit.path[0] === path)
      ?.action ?? "keep";
  const scalarControls = (path: string) =>
    initial && stored(path) ? (
      <MenuSelect
        aria-label={t("library.mcpComposer.action", { field: path })}
        value={scalarAction(path)}
        disabled={disabled}
        onChange={(action) =>
          setField({
            path: [path],
            action,
            ...(action === "set"
              ? {
                  value:
                    path === "args"
                      ? (exactArgs ??
                        (value.argsText ? value.argsText.split(/\r?\n/) : []))
                      : value[path as "command" | "url"],
                }
              : {}),
          })
        }
        options={["keep", "set", "clear"].map((action) => ({
          value: action as FieldEdit["action"],
          label: t(
            `library.mcpComposer.${action === "set" ? "replace" : action}`,
          ),
        }))}
      />
    ) : null;
  const exactArgs = (value as ExactMcpDraft).args;
  const useJsonArgs =
    !!exactArgs &&
    (exactArgs.some((arg) => arg === "" || /[\r\n]/.test(arg)) ||
      exactArgs.join("\n") !== value.argsText);
  const [argsMode, setArgsMode] = useState<"lines" | "json">(
    useJsonArgs ? "json" : "lines",
  );
  const [argsJson, setArgsJson] = useState(() =>
    JSON.stringify(
      exactArgs ?? (value.argsText ? value.argsText.split(/\r?\n/) : []),
      null,
      2,
    ),
  );
  const scalarField = (path: "command" | "url", label: string) => (
    <div data-mcp-field={JSON.stringify([path])} className="space-y-1">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <label htmlFor={`${fieldsId}-${path}`} className="text-ctl">
          {label}
        </label>
        {scalarControls(path)}
      </div>
      <Input
        id={`${fieldsId}-${path}`}
        className="font-mono"
        aria-label={label}
        value={initial && scalarAction(path) !== "set" ? "" : value[path]}
        disabled={disabled || (!!initial && scalarAction(path) === "clear")}
        autoComplete="off"
        spellCheck={false}
        placeholder={stored(path) ? t("library.mcpComposer.saved") : undefined}
        onChange={(event) => setScalar(path, event.target.value)}
      />
    </div>
  );
  const inlineInitialFields =
    initial?.fields.filter(
      (field) =>
        field.path[0] !== "type" &&
        !(
          field.path.length === 1 &&
          ["command", "args", "url"].includes(field.path[0])
        ) &&
        field.path[0] === (value.transport === "stdio" ? "env" : "headers"),
    ) ?? [];
  const advancedInitialFields =
    initial?.fields.filter(
      (field) =>
        !["type", "command", "args", "url", "env", "headers"].includes(
          field.path[0],
        ),
    ) ?? [];
  const [extraPath, setExtraPath] = useState('["timeout"]');
  const [extraValue, setExtraValue] = useState("5000");
  const chosenCandidates = () => {
    const drafts = selectedJson.map((index) => candidates[index]);
    const ids = new Set<string>();
    for (const draft of drafts) {
      if (!draft.identifier || !/^[A-Za-z0-9_-]{1,80}$/.test(draft.identifier))
        throw new Error("invalidIdentifier");
      if (
        ids.has(draft.identifier) ||
        existingIdentifiers.includes(draft.identifier)
      )
        throw new Error("duplicateIdentifier");
      ids.add(draft.identifier);
    }
    return drafts;
  };
  const submitMany = (intent: "draft" | "apply") => {
    try {
      const drafts = chosenCandidates();
      if (!drafts.length) throw new Error("chooseJson");
      if (onImportMany) void run(() => onImportMany(drafts, intent));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "invalidJson");
    }
  };
  return (
    <form
      ref={form}
      tabIndex={-1}
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
          {errorMessage(error)}
        </p>
      )}
      {!initial && (
        <div className="flex flex-wrap items-center gap-2">
          <SegmentedControl
            aria-label={t("library.mcpComposer.entry")}
            value={mode}
            disabled={disabled}
            onChange={(next) => {
              setMode(next);
              setError("");
            }}
            options={[
              { value: "form", label: t("library.mcpComposer.form") },
              { value: "paste", label: t("library.mcpComposer.paste") },
            ]}
          />
          {onDiscover && (
            <Button
              type="button"
              variant="ghost"
              disabled={disabled}
              onClick={onDiscover}
            >
              {t("library.mcpComposer.discover")}
            </Button>
          )}
        </div>
      )}
      {!initial && mode === "paste" && (
        <div className="min-w-0 space-y-3 rounded-ctl border border-line-2 bg-card-2 p-3">
          <label className="block space-y-1 text-ctl">
            <span>{t("library.mcpComposer.jsonPaste")}</span>
            <Textarea
              className="min-h-[130px] font-mono"
              value={json}
              disabled={disabled}
              onChange={(event) => {
                setJson(event.target.value);
                setCandidates([]);
                setSelectedJson([]);
              }}
            />
          </label>
          <Button
            type="button"
            variant="secondary"
            disabled={disabled || !json.trim()}
            onClick={() => {
              try {
                const drafts = mcpDraftsFromJson(json);
                const used = [...existingIdentifiers];
                setCandidates(
                  drafts.map((draft) => {
                    const next = createMcpIdentifier(draft.name, used);
                    used.push(next);
                    return { ...draft, identifier: next };
                  }),
                );
                setSelectedJson([]);
                setError("");
              } catch (cause) {
                setError(
                  cause instanceof Error ? cause.message : "invalidJson",
                );
              }
            }}
          >
            {t("library.mcpComposer.parseJson")}
          </Button>
          {!!candidates.length && (
            <fieldset className="space-y-2">
              <legend className="text-ctl font-medium">
                {t("library.mcpComposer.selectJson")}
              </legend>
              {candidates.map((draft, index) => (
                <div
                  key={index}
                  className="flex min-w-0 flex-wrap items-center gap-2 rounded-ctl border border-line-2 p-2 text-ctl"
                >
                  <label className="flex min-w-[120px] flex-1 items-center gap-2">
                    <Checkbox
                      aria-label={t("library.mcpComposer.selectItem", {
                        name: draft.name || String(index + 1),
                      })}
                      checked={selectedJson.includes(index)}
                      disabled={disabled}
                      onCheckedChange={(checked) =>
                        setSelectedJson((previous) =>
                          checked
                            ? [...previous, index]
                            : previous.filter((item) => item !== index),
                        )
                      }
                    />
                    <span className="min-w-0 truncate">
                      {draft.name || String(index + 1)}
                    </span>
                    <span className="text-note text-ink-muted">
                      {t(`library.addFlow.${draft.transport}`)}
                    </span>
                  </label>
                  <Input
                    className="min-w-[130px] flex-1 font-mono"
                    aria-label={t("library.mcpComposer.identifierFor", {
                      name: draft.name || String(index + 1),
                    })}
                    value={draft.identifier ?? ""}
                    disabled={disabled}
                    onChange={(event) =>
                      setCandidates((previous) =>
                        previous.map((item, i) =>
                          i === index
                            ? { ...item, identifier: event.target.value }
                            : item,
                        ),
                      )
                    }
                  />
                </div>
              ))}
              <p className="text-note text-ink-muted">
                {t("library.mcpComposer.selectedJson", {
                  count: selectedJson.length,
                })}
              </p>
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  disabled={disabled || selectedJson.length !== 1}
                  onClick={() => {
                    let draft: ExactMcpDraft;
                    try {
                      draft = chosenCandidates()[0];
                    } catch (cause) {
                      setError(
                        cause instanceof Error
                          ? cause.message
                          : "invalidIdentifier",
                      );
                      return;
                    }
                    onChange({ ...draft, name: draft.name || value.name });
                    const next =
                      draft.identifier ??
                      createMcpIdentifier(
                        draft.name || value.name,
                        existingIdentifiers,
                      );
                    manualIdentifier.current = true;
                    setLocalIdentifier(next);
                    onIdentifierChange?.(next);
                    setArgsMode(
                      draft.args?.some((arg) => !arg || /[\r\n]/.test(arg))
                        ? "json"
                        : "lines",
                    );
                    setArgsJson(JSON.stringify(draft.args ?? [], null, 2));
                    setMode("form");
                    setJson("");
                    setCandidates([]);
                    setSelectedJson([]);
                  }}
                >
                  {t("library.mcpComposer.fillJson")}
                </Button>
                {onImportMany && (
                  <>
                    <Button
                      type="button"
                      variant="secondary"
                      disabled={disabled || !selectedJson.length}
                      onClick={() => submitMany("draft")}
                    >
                      {t("library.mcpComposer.importSelected")}
                    </Button>
                    <Button
                      type="button"
                      variant="secondary"
                      disabled={disabled || !selectedJson.length}
                      onClick={() => submitMany("apply")}
                    >
                      {t("library.mcpComposer.previewSelected")}
                    </Button>
                  </>
                )}
              </div>
            </fieldset>
          )}
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
      <label className="block space-y-1 text-ctl">
        <span>{t("library.mcpComposer.identifier")}</span>
        <Input
          className="font-mono"
          aria-label={t("library.mcpComposer.identifier")}
          value={configIdentifier}
          readOnly={!!initial}
          disabled={disabled}
          onChange={(event) => {
            manualIdentifier.current = true;
            setLocalIdentifier(event.target.value);
            onIdentifierChange?.(event.target.value);
          }}
        />
        <span className="block text-note text-ink-muted">
          {t(
            initial
              ? "library.mcpComposer.existingIdentifier"
              : "library.mcpComposer.identifierHint",
          )}
        </span>
      </label>
      <SegmentedControl
        aria-label={t("library.addFlow.transport")}
        value={value.transport}
        disabled={disabled}
        onChange={(transport) => {
          update({ transport });
          setTransportConfirmed(false);
          if (initial) {
            const excluded =
              transport === "stdio"
                ? ["url", "headers"]
                : ["command", "args", "env"];
            setChanges((previous) => [
              ...previous.filter(
                (edit) =>
                  edit.path[0] !== "type" && !excluded.includes(edit.path[0]),
              ),
              {
                path: ["type"],
                action: transport === initial.transport ? "keep" : "set",
                ...(transport !== initial.transport
                  ? { value: transport }
                  : {}),
              },
            ]);
            setInvalidInputs([]);
          }
        }}
        options={[
          { value: "stdio", label: t("library.addFlow.stdio") },
          { value: "http", label: t("library.addFlow.http") },
          { value: "sse", label: t("library.addFlow.sse") },
        ]}
      />
      {changedTransport && (
        <div className="space-y-2 rounded-ctl border border-warn/40 bg-warn/5 p-3 text-ctl">
          <p className="font-medium">
            {t("library.mcpComposer.transportChanges")}
          </p>
          <p>
            {incompatible.length
              ? t("library.mcpComposer.transportClear", {
                  fields: incompatible
                    .map((field) => field.path.join("."))
                    .join(", "),
                })
              : t("library.mcpComposer.transportNoClear")}
          </p>
          <label className="flex items-start gap-2">
            <Checkbox
              checked={transportConfirmed}
              disabled={disabled}
              onCheckedChange={(checked) =>
                setTransportConfirmed(checked === true)
              }
            />
            <span>{t("library.mcpComposer.confirmTransport")}</span>
          </label>
        </div>
      )}
      {value.transport === "stdio" ? (
        <>
          {!initial && (
            <div className="space-y-2">
              <Button
                type="button"
                variant="ghost"
                disabled={disabled}
                aria-expanded={commandPaste}
                onClick={() => setCommandPaste(!commandPaste)}
              >
                {t("library.mcpComposer.commandPaste")}
              </Button>
              {commandPaste && (
                <div className="min-w-0 space-y-2 rounded-ctl border border-line-2 bg-card-2 p-3">
                  <SegmentedControl
                    aria-label={t("library.mcpComposer.commandPlatform")}
                    value={commandPlatform}
                    disabled={disabled}
                    onChange={(platform) => {
                      setCommandPlatform(platform);
                      setParsedCommand(null);
                    }}
                    options={[
                      { value: "posix", label: t("library.mcpComposer.posix") },
                      {
                        value: "windows",
                        label: t("library.mcpComposer.windows"),
                      },
                    ]}
                  />
                  <Textarea
                    aria-label={t("library.mcpComposer.commandInput")}
                    className="font-mono"
                    value={commandInput}
                    disabled={disabled}
                    onChange={(event) => {
                      setCommandInput(event.target.value);
                      setParsedCommand(null);
                    }}
                  />
                  <Button
                    type="button"
                    variant="secondary"
                    disabled={disabled || !commandInput.trim()}
                    onClick={() => {
                      try {
                        setParsedCommand(
                          parseMcpCommand(commandInput, commandPlatform),
                        );
                        setError("");
                      } catch (cause) {
                        setError(
                          cause instanceof Error
                            ? cause.message
                            : "invalidCommand",
                        );
                      }
                    }}
                  >
                    {t("library.mcpComposer.parseCommand")}
                  </Button>
                  {parsedCommand && (
                    <div className="space-y-2">
                      <p className="text-ctl font-medium">
                        {t("library.mcpComposer.commandReview")}
                      </p>
                      <dl className="min-w-0 text-note">
                        <dt>{t("library.mcpComposer.program")}</dt>
                        <dd className="break-all font-mono">
                          {parsedCommand.command}
                        </dd>
                        <dt>{t("library.mcpComposer.argv")}</dt>
                        <dd>
                          <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all font-mono">
                            {JSON.stringify(parsedCommand.args, null, 2)}
                          </pre>
                        </dd>
                      </dl>
                      <Button
                        type="button"
                        disabled={disabled}
                        onClick={() => {
                          update({
                            command: parsedCommand.command,
                            args: parsedCommand.args,
                            argsText: parsedCommand.args.join("\n"),
                          });
                          setArgsMode(
                            parsedCommand.args.some((arg) => !arg)
                              ? "json"
                              : "lines",
                          );
                          setArgsJson(
                            JSON.stringify(parsedCommand.args, null, 2),
                          );
                          setCommandInput("");
                          setParsedCommand(null);
                          setCommandPaste(false);
                        }}
                      >
                        {t("library.mcpComposer.confirmCommand")}
                      </Button>
                    </div>
                  )}
                  <p className="text-note text-ink-muted">
                    {t("library.mcpComposer.noExecution")}
                  </p>
                </div>
              )}
            </div>
          )}
          {scalarField("command", t("library.addFlow.command"))}
          <div className="space-y-2">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <label htmlFor={`${fieldsId}-args`} className="text-ctl">
                {t("library.addFlow.arguments")}
              </label>
              {scalarControls("args")}
              <SegmentedControl
                aria-label={t("library.mcpComposer.argsMode")}
                value={argsMode}
                disabled={disabled}
                onChange={(next) => {
                  if (
                    next === "lines" &&
                    exactArgs?.some((arg) => !arg || /[\r\n]/.test(arg))
                  ) {
                    setError("invalidArguments");
                    return;
                  }
                  setArgsMode(next);
                  setArgsJson(
                    JSON.stringify(
                      exactArgs ??
                        (value.argsText ? value.argsText.split(/\r?\n/) : []),
                      null,
                      2,
                    ),
                  );
                }}
                options={[
                  { value: "lines", label: t("library.mcpComposer.lines") },
                  { value: "json", label: t("library.mcpComposer.jsonArgs") },
                ]}
              />
            </div>
            <Textarea
              id={`${fieldsId}-args`}
              className="min-h-[80px] font-mono"
              value={
                initial && scalarAction("args") !== "set"
                  ? ""
                  : argsMode === "json"
                    ? argsJson
                    : value.argsText
              }
              disabled={
                disabled || (!!initial && scalarAction("args") === "clear")
              }
              placeholder={
                stored("args") ? t("library.mcpComposer.saved") : undefined
              }
              onChange={(event) => {
                const text = event.target.value;
                if (argsMode === "json") {
                  setArgsJson(text);
                  try {
                    const args: unknown = parseMcpJsonValue(text);
                    if (
                      !Array.isArray(args) ||
                      args.some((arg) => typeof arg !== "string")
                    )
                      throw new Error();
                    setInvalidInputs((previous) =>
                      previous.filter((key) => key !== "args"),
                    );
                    update({ args, argsText: args.join("\n") });
                    if (initial)
                      setField({ path: ["args"], action: "set", value: args });
                  } catch {
                    setInvalidInputs((previous) => [
                      ...previous.filter((key) => key !== "args"),
                      "args",
                    ]);
                    setError("invalidArguments");
                    if (initial)
                      setField({
                        path: ["args"],
                        action: "set",
                        value: undefined,
                      });
                  }
                } else {
                  setInvalidInputs((previous) =>
                    previous.filter((key) => key !== "args"),
                  );
                  update({ args: undefined, argsText: text });
                  if (initial)
                    setField({
                      path: ["args"],
                      action: "set",
                      value: text ? text.split(/\r?\n/) : [],
                    });
                }
              }}
            />
          </div>
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
          {scalarField("url", t("library.addFlow.url"))}
          <PairFields
            kind="headers"
            pairs={value.headers}
            disabled={disabled}
            onChange={(headers) => update({ headers })}
          />
        </>
      )}
      {!!inlineInitialFields.length && (
        <fieldset className="min-w-0 space-y-2">
          <legend className="mb-2 text-ctl font-medium">
            {t("library.mcpComposer.savedFields")}
          </legend>
          {inlineInitialFields.map((field) => (
            <SavedField
              key={`${initial?.id}:${initial?.revision}:${JSON.stringify(field.path)}`}
              field={field}
              edit={changes.find(
                (edit) =>
                  JSON.stringify(edit.path) === JSON.stringify(field.path),
              )}
              disabled={disabled}
              onChange={setField}
            />
          ))}
        </fieldset>
      )}
      <details className="rounded-ctl border border-line-2 p-3">
        <summary className="cursor-pointer text-ctl font-medium">
          {t("library.mcpComposer.advanced")}
          {!!value.advancedFields?.length &&
            ` · ${value.advancedFields.length}`}
        </summary>
        <div className="mt-3 min-w-0 space-y-2">
          {advancedInitialFields.map((field) => (
            <SavedField
              key={`${initial?.id}:${initial?.revision}:${JSON.stringify(field.path)}`}
              field={field}
              edit={changes.find(
                (edit) =>
                  JSON.stringify(edit.path) === JSON.stringify(field.path),
              )}
              disabled={disabled}
              onChange={setField}
            />
          ))}
          {value.advancedFields?.map((field) => (
            <label
              key={`${initial?.id}:${initial?.revision}:${JSON.stringify(field.path)}`}
              className="block space-y-1 text-note"
            >
              <span className="break-all font-mono">
                {field.path.join(".")}
              </span>
              <Textarea
                className="font-mono"
                aria-label={t("library.mcpComposer.advancedValue", {
                  field: field.path.join("."),
                })}
                defaultValue={JSON.stringify(field.value, null, 2)}
                disabled={disabled}
                onChange={(event) => {
                  try {
                    const next = parseMcpJsonValue(
                      event.target.value,
                    ) as unknown;
                    setInvalidInputs((previous) =>
                      previous.filter(
                        (key) => key !== JSON.stringify(field.path),
                      ),
                    );
                    update({
                      advancedFields: value.advancedFields?.map((item) =>
                        JSON.stringify(item.path) === JSON.stringify(field.path)
                          ? { ...item, value: next }
                          : item,
                      ),
                    });
                  } catch {
                    setInvalidInputs((previous) => [
                      ...previous.filter(
                        (key) => key !== JSON.stringify(field.path),
                      ),
                      JSON.stringify(field.path),
                    ]);
                    setError("invalidJson");
                  }
                }}
              />
            </label>
          ))}
          <label className="block space-y-1 text-note">
            <span>{t("library.mcpComposer.fieldPath")}</span>
            <Input
              className="font-mono"
              value={extraPath}
              disabled={disabled}
              onChange={(event) => setExtraPath(event.target.value)}
            />
          </label>
          <label className="block space-y-1 text-note">
            <span>{t("library.mcpComposer.jsonValue")}</span>
            <Input
              className="font-mono"
              value={extraValue}
              disabled={disabled}
              onChange={(event) => setExtraValue(event.target.value)}
            />
          </label>
          <Button
            type="button"
            variant="secondary"
            disabled={disabled}
            onClick={() => {
              try {
                const path: unknown = parseMcpJsonValue(extraPath);
                const next: unknown = parseMcpJsonValue(extraValue);
                if (
                  !Array.isArray(path) ||
                  !path.length ||
                  path.some((part) => typeof part !== "string" || !part) ||
                  (!initial && path.length !== 1) ||
                  ["type", "command", "args", "url", "env", "headers"].includes(
                    path[0],
                  ) ||
                  initial?.fields.some(
                    (field) =>
                      JSON.stringify(field.path) === JSON.stringify(path),
                  ) ||
                  value.advancedFields?.some(
                    (field) =>
                      JSON.stringify(field.path) === JSON.stringify(path),
                  )
                )
                  throw new Error("invalidFieldPath");
                if (initial) setField({ path, action: "set", value: next });
                else
                  update({
                    advancedFields: [
                      ...(value.advancedFields ?? []),
                      { path, value: next },
                    ],
                  });
                setError("");
              } catch (cause) {
                setError(
                  cause instanceof Error && cause.message === "invalidFieldPath"
                    ? cause.message
                    : "invalidJson",
                );
              }
            }}
          >
            {t("library.mcpComposer.addField")}
          </Button>
          {changes
            .filter(
              (edit) =>
                initial &&
                !initial.fields.some(
                  (field) =>
                    JSON.stringify(field.path) === JSON.stringify(edit.path),
                ) &&
                edit.path.length > 0 &&
                !["type", "command", "args", "url", "env", "headers"].includes(
                  edit.path[0],
                ),
            )
            .map((edit) => (
              <p
                key={JSON.stringify(edit.path)}
                className="break-all font-mono text-note"
              >
                {edit.path.join(".")}{" "}
                <Button
                  type="button"
                  variant="ghost"
                  disabled={disabled}
                  aria-label={t("library.mcpComposer.remove", {
                    field: edit.path.join("."),
                  })}
                  onClick={() =>
                    setChanges((previous) =>
                      previous.filter((item) => item !== edit),
                    )
                  }
                >
                  <X aria-hidden className="h-3 w-3" />
                </Button>
              </p>
            ))}
          {onAdvanced && (
            <Button
              type="button"
              variant="ghost"
              disabled={disabled || !!json.trim() || !!commandInput.trim()}
              title={
                json.trim() || commandInput.trim()
                  ? t("library.interaction.completeMcpInput")
                  : undefined
              }
              onClick={() => onAdvanced(value, dirty)}
            >
              {t("library.addFlow.advanced")}
            </Button>
          )}
        </div>
      </details>
      {targets && onToolsChange && (
        <fieldset className="min-w-0 space-y-2">
          <legend className="mb-2 text-ctl font-medium">
            {t("library.mcpComposer.targets")}
          </legend>
          <TooltipProvider delayDuration={150}>
            <div className="flex flex-wrap gap-2">
              {targets
                .filter(
                  (target, index, list) =>
                    list.findIndex((item) => item.toolId === target.toolId) ===
                    index,
                )
                .map((target) => {
                  const reason = target.disabledReason
                    ? t(target.disabledReason, {
                        defaultValue: t(
                          "library.mcpComposer.targetUnavailable",
                        ),
                      })
                    : !target.available
                      ? t("library.mcpComposer.targetUnavailable")
                      : "";
                  return (
                    <Tooltip key={target.toolId}>
                      <TooltipTrigger asChild>
                        <button
                          type="button"
                          aria-pressed={selectedTools.includes(target.toolId)}
                          aria-label={toolLabel(target.toolId, t)}
                          aria-disabled={disabled || !target.available}
                          aria-describedby={
                            reason
                              ? `${fieldsId}-${target.toolId}-reason`
                              : undefined
                          }
                          title={reason || undefined}
                          className={`inline-flex max-w-full items-center gap-2 rounded-ctl border px-3 py-2 text-ctl outline-none focus-visible:ring-2 focus-visible:ring-brand ${selectedTools.includes(target.toolId) ? "border-brand bg-brand/10" : "border-line bg-card"}`}
                          onClick={() => {
                            if (!disabled && target.available)
                              onToolsChange(
                                selectedTools.includes(target.toolId)
                                  ? selectedTools.filter(
                                      (tool) => tool !== target.toolId,
                                    )
                                  : [...selectedTools, target.toolId],
                              );
                          }}
                        >
                          <ToolBadge
                            toolId={target.toolId}
                            size={18}
                            variant="plain"
                          />
                          <span>{toolLabel(target.toolId, t)}</span>
                        </button>
                      </TooltipTrigger>
                      {reason && (
                        <>
                          <span
                            id={`${fieldsId}-${target.toolId}-reason`}
                            className="sr-only"
                          >
                            {reason}
                          </span>
                          <TooltipContent>{reason}</TooltipContent>
                        </>
                      )}
                    </Tooltip>
                  );
                })}
            </div>
          </TooltipProvider>
          <p className="text-note text-ink-muted">
            {t(
              immediate
                ? "library.mcpTable.targetsHint"
                : "library.mcpComposer.targetsHint",
            )}
          </p>
        </fieldset>
      )}
      <div className="flex flex-wrap items-center gap-2 border-t border-line-2 pt-3">
        {!saveOnly && (
          <Button
            type="button"
            disabled={
              disabled ||
              (!!targets && !!onToolsChange && !selectedTools.length)
            }
            onClick={() => void submit("apply")}
          >
            {t(
              immediate
                ? "library.mcpTable.saveAndEnable"
                : "library.mcpComposer.preview",
            )}
          </Button>
        )}
        <Button type="submit" variant="secondary" disabled={disabled}>
          {t(immediate ? "library.mcpTable.save" : "library.mcpComposer.save")}
        </Button>
      </div>
      <p className="text-note text-ink-muted">
        {t(
          immediate ? "library.mcpTable.saveNote" : "library.mcpComposer.note",
        )}
      </p>
    </form>
  );
}
