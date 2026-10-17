import { parseTree, type Node, type ParseError } from "jsonc-parser";
import type { FieldEdit, McpDefinition } from "@/lib/api/library";
import {
  createMcpDraft,
  type LibraryMcpDraft,
  type LibraryMcpPair,
} from "./libraryRecommendations";

export type ExactMcpDraft = LibraryMcpDraft & { args?: string[] };
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
export function rejectMcpMasks(value: unknown): void {
  if (
    value === undefined ||
    typeof value === "bigint" ||
    typeof value === "function" ||
    typeof value === "symbol"
  )
    throw new Error("invalidJson");
  if (typeof value === "number" && !Number.isFinite(value))
    throw new Error("invalidJson");
  if (typeof value === "string" && MASK.test(value))
    throw new Error("maskedValue");
  if (Array.isArray(value)) value.forEach(rejectMcpMasks);
  else if (isObject(value)) Object.values(value).forEach(rejectMcpMasks);
}
export function mcpPairEdits(
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
      rejectMcpMasks(pair.value);
      return { path: [kind, name], action: "set", value: pair.value };
    });
}
function validateUrl(value: string) {
  try {
    const url = new URL(value);
    if (
      !["http:", "https:"].includes(url.protocol) ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    )
      throw new Error();
  } catch {
    throw new Error("invalidUrl");
  }
}
function validateEdits(edits: FieldEdit[]) {
  if (edits.length > 64) throw new Error("tooManyEdits");
  for (let index = 0; index < edits.length; index++) {
    const edit = edits[index];
    if (
      !edit.path.length ||
      edit.path.some(
        (part) =>
          !part || ["__proto__", "prototype", "constructor"].includes(part),
      )
    )
      throw new Error("invalidPair");
    if (
      edits
        .slice(index + 1)
        .some((other) =>
          edit.path
            .slice(0, Math.min(edit.path.length, other.path.length))
            .every((part, i) => part === other.path[i]),
        )
    )
      throw new Error("overlappingEdits");
    if (edit.action === "set") rejectMcpMasks(edit.value);
  }
  return edits;
}
/** Create-only conversion. Stored values never pass through this function. */
export function mcpDraftToEdits(draft: ExactMcpDraft): FieldEdit[] {
  if (!draft.name.trim()) throw new Error("missingName");
  const edits: FieldEdit[] = [
    { path: ["type"], action: "set", value: draft.transport },
  ];
  if (draft.transport === "stdio") {
    if (!draft.command.trim()) throw new Error("missingCommand");
    const args = draft.args
      ? [...draft.args]
      : draft.argsText
        ? draft.argsText.split(/\r?\n/)
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
      ...mcpPairEdits(draft.environment, "env"),
    );
  } else if (draft.transport === "http" || draft.transport === "sse") {
    validateUrl(draft.url.trim());
    edits.push(
      { path: ["url"], action: "set", value: draft.url.trim() },
      ...mcpPairEdits(draft.headers, "headers"),
    );
  } else throw new Error("advancedRequired");
  for (const field of draft.advancedFields ?? []) {
    if (field.path.length !== 1 || COMMON_FIELDS.has(field.path[0]))
      throw new Error("advancedRequired");
    edits.push({ path: [...field.path], action: "set", value: field.value });
  }
  return validateEdits(edits);
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
/** Compare decimal spellings exactly without first rounding either coefficient. */
function decimalIdentity(literal: string): string {
  const [mantissa, exponent = "0"] = literal.toLowerCase().split("e");
  const negative = mantissa.startsWith("-");
  const [integer, fraction = ""] = mantissa.replace(/^-/, "").split(".");
  const digits = `${integer}${fraction}`.replace(/^0+/, "");
  if (!digits) return "0";
  const coefficient = digits.replace(/0+$/, "");
  const power =
    BigInt(exponent) -
    BigInt(fraction.length) +
    BigInt(digits.length - coefficient.length);
  return `${negative ? "-" : ""}${coefficient}e${power}`;
}
function verifyJsonTree(node: Node, input: string, depth = 0): void {
  if (depth > 64) throw new Error("invalidJson");
  if (node.type === "number") {
    if (!Number.isFinite(node.value)) throw new Error("invalidJson");
    const literal = input.slice(node.offset, node.offset + node.length);
    if (
      (Number.isInteger(node.value) && !Number.isSafeInteger(node.value)) ||
      decimalIdentity(literal) !== decimalIdentity(String(node.value))
    )
      throw new Error("advancedRequired");
  }
  if (node.type === "object") {
    const keys = new Set<string>();
    for (const property of node.children ?? []) {
      const key = property.children?.[0]?.value as string;
      if (keys.has(key)) throw new Error("duplicateJsonKey");
      keys.add(key);
    }
  }
  (node.children ?? []).forEach((child) =>
    verifyJsonTree(child, input, depth + 1),
  );
}
function parseDefinition(
  config: Record<string, unknown>,
  name: string,
  format: "standard" | "opencode",
): ExactMcpDraft {
  if (config.name !== undefined && typeof config.name !== "string")
    throw new Error("advancedRequired");
  let normalized = { ...config };
  if (format === "opencode") {
    const { type, command, environment, ...rest } = config;
    if (type === "local") {
      if (
        !Array.isArray(command) ||
        !command.length ||
        command.some((arg) => typeof arg !== "string") ||
        "env" in config ||
        "args" in config
      )
        throw new Error("advancedRequired");
      normalized = {
        ...rest,
        type: "stdio",
        command: command[0],
        args: command.slice(1),
        ...(environment !== undefined ? { env: environment } : {}),
      };
    } else if (
      type === "remote" &&
      typeof config.url === "string" &&
      command === undefined &&
      environment === undefined
    )
      normalized = { ...rest, type: "http" };
    else throw new Error("advancedRequired");
  }
  if (normalized.type === undefined && typeof normalized.url === "string")
    throw new Error("missingTransport");
  const transport = normalized.type ?? "stdio";
  if (!["stdio", "http", "sse"].includes(transport as string))
    throw new Error("advancedRequired");
  if (
    normalized.args !== undefined &&
    (!Array.isArray(normalized.args) ||
      normalized.args.some((arg) => typeof arg !== "string"))
  )
    throw new Error("advancedRequired");
  if (
    (transport === "stdio" &&
      ("url" in normalized || "headers" in normalized)) ||
    (transport !== "stdio" &&
      ("command" in normalized || "args" in normalized || "env" in normalized))
  )
    throw new Error("advancedRequired");
  if (
    (normalized.command !== undefined &&
      typeof normalized.command !== "string") ||
    (normalized.url !== undefined && typeof normalized.url !== "string")
  )
    throw new Error("advancedRequired");
  rejectMcpMasks(normalized);
  const args = (normalized.args ?? []) as string[];
  const draft: ExactMcpDraft = {
    ...createMcpDraft({
      name,
      transport: transport as LibraryMcpDraft["transport"],
      command: (normalized.command as string | undefined) ?? "",
      argsText: args.join("\n"),
      url: (normalized.url as string | undefined) ?? "",
      environment: parsePairs(normalized.env),
      headers: parsePairs(normalized.headers),
      advancedFields: Object.entries(normalized)
        .filter(([key]) => !COMMON_FIELDS.has(key))
        .map(([key, value]) => ({ path: [key], value })),
    }),
    args,
  };
  mcpDraftToEdits({ ...draft, name: draft.name || "server" });
  return draft;
}
/** Strict JSON only; detect duplicate keys before JSON.parse can erase evidence. */
export function parseMcpJsonValue(input: string): unknown {
  if (input.length > 1024 * 1024) throw new Error("invalidJson");
  const errors: ParseError[] = [];
  let tree: Node | undefined;
  try {
    tree = parseTree(input, errors, {
      allowTrailingComma: false,
      disallowComments: true,
    });
  } catch {
    throw new Error("invalidJson");
  }
  if (!tree || errors.length) throw new Error("invalidJson");
  verifyJsonTree(tree, input);
  let parsed: unknown;
  try {
    parsed = JSON.parse(input);
  } catch {
    throw new Error("invalidJson");
  }
  rejectMcpMasks(parsed);
  return parsed;
}
export function mcpDraftsFromJson(input: string): ExactMcpDraft[] {
  const parsed = parseMcpJsonValue(input);
  if (!isObject(parsed)) throw new Error("invalidJson");
  const wrappers = ["mcpServers", "servers", "mcp"].filter(
    (key) => key in parsed,
  );
  if (wrappers.length > 1) throw new Error("ambiguousJson");
  if (wrappers.length) {
    const wrapper = wrappers[0];
    if (Object.keys(parsed).some((key) => key !== wrapper && key !== "$schema"))
      throw new Error("advancedRequired");
    const servers = parsed[wrapper];
    if (
      !isObject(servers) ||
      !Object.keys(servers).length ||
      Object.keys(servers).length > 64
    )
      throw new Error("invalidJson");
    return Object.entries(servers).map(([name, config]) => {
      if (!name.trim() || !isObject(config)) throw new Error("invalidJson");
      if (typeof config.name === "string" && config.name !== name)
        throw new Error("ambiguousJson");
      return parseDefinition(
        config,
        name,
        wrapper === "mcp" ? "opencode" : "standard",
      );
    });
  }
  return [
    parseDefinition(
      parsed,
      typeof parsed.name === "string" ? parsed.name : "",
      "standard",
    ),
  ];
}
export function mcpDraftFromJson(input: string): ExactMcpDraft {
  const drafts = mcpDraftsFromJson(input);
  if (drafts.length !== 1) throw new Error("oneServer");
  return drafts[0];
}
export function createMcpIdentifier(
  name: string,
  existingIds: readonly string[] = [],
  suffix = "",
): string {
  const stem =
    name
      .normalize("NFKD")
      .toLowerCase()
      .replace(/[^a-z0-9_-]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 64) || "mcp";
  if (!existingIds.includes(stem)) return stem;
  const safeSuffix = suffix
    .toLowerCase()
    .replace(/[^a-z0-9]/g, "")
    .slice(0, 8);
  if (safeSuffix && !existingIds.includes(`${stem}-${safeSuffix}`))
    return `${stem}-${safeSuffix}`;
  for (let number = 2; ; number++)
    if (!existingIds.includes(`${stem}-${number}`)) return `${stem}-${number}`;
}
export type ParsedMcpCommand = { command: string; args: string[] };
/** Tokenizes arguments only. Never invokes a shell or expands variables. */
export function parseMcpCommand(
  input: string,
  platform: "posix" | "windows",
): ParsedMcpCommand {
  if (!input.trim() || input.length > 16384 || /[\r\n\0]/.test(input))
    throw new Error("invalidCommand");
  if (/\$\(|`|[|<>;&]|%[^%]+%|\$\{[^}]+\}|\$[A-Za-z_]/.test(input))
    throw new Error("shellCommand");
  const argv: string[] = [];
  let token = "",
    started = false,
    quote: "'" | '"' | null = null;
  for (let index = 0; index < input.length; index++) {
    const character = input[index];
    if (platform === "windows" && character === "\\") {
      let end = index;
      while (input[end] === "\\") end++;
      const count = end - index;
      if (input[end] === '"') {
        token += "\\".repeat(Math.floor(count / 2));
        started = true;
        if (count % 2) token += '"';
        else if (quote === '"' && input[end + 1] === '"') {
          token += '"';
          end++;
        } else quote = quote ? null : '"';
        index = end;
      } else {
        token += "\\".repeat(count);
        started = true;
        index = end - 1;
      }
    } else if (platform === "posix" && character === "\\" && quote !== "'") {
      if (index + 1 === input.length) throw new Error("invalidCommand");
      const next = input[++index];
      if (quote === '"' && !["\\", '"', "$", "`"].includes(next)) token += "\\";
      token += next;
      started = true;
    } else if (
      character === '"' ||
      (platform === "posix" && character === "'")
    ) {
      if (
        platform === "windows" &&
        quote === '"' &&
        character === '"' &&
        input[index + 1] === '"'
      ) {
        token += '"';
        index++;
      } else if (quote === character) quote = null;
      else if (quote) token += character;
      else quote = character;
      started = true;
    } else if (/\s/.test(character) && !quote) {
      if (started) {
        argv.push(token);
        token = "";
        started = false;
      }
    } else {
      token += character;
      started = true;
    }
  }
  if (quote) throw new Error("invalidCommand");
  if (started) argv.push(token);
  const [command, ...args] = argv;
  const executable =
    command
      ?.split(/[\\/]/)
      .at(-1)
      ?.toLowerCase()
      .replace(/\.(exe|cmd|bat)$/i, "") ?? "";
  if (
    !command ||
    /[=]/.test(command) ||
    [
      "sh",
      "bash",
      "zsh",
      "fish",
      "cmd",
      "powershell",
      "pwsh",
      "env",
      "sudo",
      "su",
      "wsl",
    ].includes(executable) ||
    /\.bat$|\.cmd$/i.test(command)
  )
    throw new Error("shellCommand");
  return { command, args };
}
/** Changed leaves only; opaque refs prove the original values without exposing them. */
export function mcpEditDelta(
  initial: McpDefinition,
  changes: FieldEdit[],
  transportConfirmation = false,
): FieldEdit[] {
  let edits = changes
    .filter((change) => change.action !== "keep")
    .map((change) => {
      const field = initial.fields.find(
        (item) => JSON.stringify(item.path) === JSON.stringify(change.path),
      );
      return {
        ...change,
        path: [...change.path],
        ...(field ? { fieldRef: field.fieldRef } : {}),
      };
    });
  const transport = edits.find(
    (edit) =>
      edit.path.length === 1 &&
      edit.path[0] === "type" &&
      edit.action === "set",
  )?.value;
  const finalTransport = transport ?? initial.transport;
  if (
    edits.some(
      (edit) =>
        edit.action === "clear" &&
        edit.path.length === 1 &&
        (edit.path[0] === "type" ||
          (finalTransport === "stdio" && edit.path[0] === "command") ||
          (finalTransport !== "stdio" && edit.path[0] === "url")),
    )
  )
    throw new Error(
      finalTransport === "stdio" ? "missingCommand" : "invalidUrl",
    );
  if (transport !== undefined && transport !== initial.transport) {
    if (!transportConfirmation) throw new Error("confirmTransport");
    const incompatible =
      transport === "stdio" ? ["url", "headers"] : ["command", "args", "env"];
    for (const field of initial.fields.filter(
      (field) => field.exists && incompatible.includes(field.path[0]),
    )) {
      if (
        edits.some(
          (edit) => edit.path[0] === field.path[0] && edit.action === "set",
        )
      )
        throw new Error("overlappingEdits");
      if (
        !edits.some(
          (edit) => JSON.stringify(edit.path) === JSON.stringify(field.path),
        )
      )
        edits.push({
          path: [...field.path],
          fieldRef: field.fieldRef,
          action: "clear",
        });
    }
    edits = edits.filter(
      (edit) => !incompatible.includes(edit.path[0]) || edit.action === "clear",
    );
  }
  const headerNames = new Set<string>();
  for (const edit of edits) {
    if (edit.path[0] === "headers" && edit.path.length === 2) {
      const key = edit.path[1].toLowerCase();
      if (headerNames.has(key)) throw new Error("duplicateHeader");
      headerNames.add(key);
    }
    if (
      edit.path.length === 1 &&
      edit.path[0] === "type" &&
      edit.action === "set" &&
      !["stdio", "http", "sse"].includes(edit.value as string)
    )
      throw new Error("advancedRequired");
    if (
      edit.path.length === 1 &&
      edit.path[0] === "command" &&
      edit.action === "set" &&
      (typeof edit.value !== "string" || !edit.value.trim())
    )
      throw new Error("missingCommand");
    if (edit.path[0] === "env" || edit.path[0] === "headers") {
      const emptyStoredContainer =
        edit.path.length === 1 &&
        edit.action === "clear" &&
        initial.fields.some(
          (field) =>
            field.path.length === 1 &&
            field.path[0] === edit.path[0] &&
            field.valueType === "object",
        );
      if (edit.path.length !== 2 && !emptyStoredContainer)
        throw new Error("overlappingEdits");
      if (edit.action === "set")
        mcpPairEdits(
          [
            {
              id: "edit",
              name: edit.path[1],
              value:
                typeof edit.value === "string"
                  ? edit.value
                  : (() => {
                      throw new Error("invalidPair");
                    })(),
            },
          ],
          edit.path[0],
        );
    }
    if (edit.path[0] === "url" && edit.action === "set") {
      if (typeof edit.value !== "string") throw new Error("invalidUrl");
      validateUrl(edit.value);
    }
    if (
      edit.path[0] === "args" &&
      edit.action === "set" &&
      (!Array.isArray(edit.value) ||
        edit.value.some((arg) => typeof arg !== "string"))
    )
      throw new Error("invalidArguments");
  }
  return validateEdits(edits);
}
