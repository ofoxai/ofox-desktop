import { writeFile, chmod, appendFile } from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { createHash, randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";
import {
  createMcpExtension,
  createToolSearchExtension,
} from "@earendil-works/pi-coding-agent";
import {
  StdioTransport,
  StreamableHttpTransport,
} from "@earendil-works/pi-mcp";

import { mcpSecrets, mcpScrubber } from "./mcp-security.mjs";
import { writeSessionMcpConfig } from "./mcp-config.mjs";
export { mcpSecrets, mcpScrubber } from "./mcp-security.mjs";

const resources = new Set([
  "list_mcp_resources",
  "list_mcp_resource_templates",
  "read_mcp_resource",
]);
const unsupported =
  "unsupported_binary: Binary MCP content is not supported; readable text was retained.";

// This runs at the client boundary, before Pi logs notifications or writes large
// results. Routing identifiers are validated, never rewritten into another tool.
export function sanitizeMcpMessage(message, cleaner, requestMethod) {
  const control = (value) => {
    if (cleaner.scalar(value) !== value)
      throw new Error(
        "MCP control value or identifier contains a configured secret",
      );
    return value;
  };
  const schemaTypes = new Set([
    "null",
    "boolean",
    "object",
    "array",
    "number",
    "string",
    "integer",
  ]);
  const booleanHints = new Set([
    "readOnlyHint",
    "destructiveHint",
    "idempotentHint",
    "openWorldHint",
  ]);
  const isObject = (value) =>
    value !== null && typeof value === "object" && !Array.isArray(value);
  const schemaMaps = new Set([
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
  ]);
  const schemaSingles = new Set([
    "additionalProperties",
    "unevaluatedProperties",
    "propertyNames",
    "additionalItems",
    "unevaluatedItems",
    "contains",
    "not",
    "if",
    "then",
    "else",
    "contentSchema",
  ]);
  const schemaLists = new Set(["allOf", "anyOf", "oneOf", "prefixItems"]);
  const schemaFlags = new Set([
    "uniqueItems",
    "readOnly",
    "writeOnly",
    "deprecated",
    "$recursiveAnchor",
  ]);
  const invalidSchema = () => {
    throw new Error(
      "MCP schema must use an object or a legal boolean subschema",
    );
  };
  const visit = (value, mode = "protocol", key = "") => {
    // Schema authority comes from a known grammar position, never from a key
    // encountered later inside arbitrary metadata. MCP tool roots are objects;
    // only explicit subschema positions may contain boolean schemas (not null).
    if (mode === "schemaRoot") {
      if (!isObject(value)) invalidSchema();
      return visit(value, "schema");
    }
    if (mode === "schema") {
      if (typeof value === "boolean") return value;
      if (!isObject(value)) invalidSchema();
      return Object.fromEntries(
        Object.entries(value).map(([field, item]) => {
          control(field);
          if (schemaMaps.has(field)) {
            if (!isObject(item)) invalidSchema();
            return [
              field,
              Object.fromEntries(
                Object.entries(item).map(([name, child]) => {
                  control(name);
                  return [name, visit(child, "schema")];
                }),
              ),
            ];
          }
          if (schemaSingles.has(field)) return [field, visit(item, "schema")];
          if (schemaLists.has(field)) {
            if (!Array.isArray(item)) invalidSchema();
            return [field, item.map((child) => visit(child, "schema"))];
          }
          // Legacy tuple items/dependencies have explicit schema positions too.
          if (field === "items")
            return [
              field,
              Array.isArray(item)
                ? item.map((child) => visit(child, "schema"))
                : visit(item, "schema"),
            ];
          if (field === "dependencies") {
            if (!isObject(item)) invalidSchema();
            return [
              field,
              Object.fromEntries(
                Object.entries(item).map(([name, child]) => {
                  control(name);
                  return [
                    name,
                    Array.isArray(child)
                      ? visit(child, "constraint")
                      : visit(child, "schema"),
                  ];
                }),
              ),
            ];
          }
          if (field === "$vocabulary") {
            if (!isObject(item)) invalidSchema();
            return [
              field,
              Object.fromEntries(
                Object.entries(item).map(([name, required]) => {
                  control(name);
                  if (typeof required !== "boolean") invalidSchema();
                  return [name, required];
                }),
              ),
            ];
          }
          if (schemaFlags.has(field) && typeof item === "boolean")
            return [field, item];
          if (
            field === "type" &&
            (schemaTypes.has(item) ||
              (Array.isArray(item) &&
                item.every((type) => schemaTypes.has(type))))
          )
            return [field, item];
          if (
            ["description", "title", "$comment"].includes(field) &&
            typeof item === "string"
          )
            return [field, cleaner.text(item)];
          // Includes const/default/enum and every unknown extension. This mode
          // validates the complete tree and cannot re-enter schema or annotations.
          return [field, visit(item, "constraint", field)];
        }),
      );
    }
    // Data and constraints are sticky. No nested key can re-enter a protocol,
    // schema or permission mode from either of these untrusted subtrees.
    if (mode === "constraint") {
      if (Array.isArray(value)) return value.map((item) => visit(item, mode));
      if (!isObject(value)) return control(value);
      return Object.fromEntries(
        Object.entries(value).map(([field, item]) => {
          control(field);
          return [field, visit(item, mode)];
        }),
      );
    }
    if (mode === "data") {
      if (Array.isArray(value)) return value.map((item) => visit(item, mode));
      if (!isObject(value)) return cleaner.scalar(value);
      if (value.type === "image" || value.type === "audio")
        return { type: "text", text: unsupported };
      if (typeof value.blob === "string") return decodeBlob(value, "data");
      return entries(value, (field, item) => [
        cleaner.text(field),
        visit(item, "data"),
      ]);
    }
    if (mode === "tool")
      return fields(value, {
        name: control,
        inputSchema: (v) => visit(v, "schemaRoot"),
        outputSchema: (v) => visit(v, "schemaRoot"),
        annotations: (v) => visit(v, "toolAnnotations"),
      });
    if (mode === "toolAnnotations")
      return fields(
        value,
        Object.fromEntries([...booleanHints].map((hint) => [hint, boolean])),
      );
    if (mode === "resource")
      return fields(value, {
        uri: control,
        uriTemplate: control,
        name: control,
        mimeType: control,
      });
    if (mode === "resourceContent") {
      if (typeof value?.blob === "string") return decodeBlob(value, mode);
      return fields(value, { uri: control, mimeType: control });
    }
    if (mode === "content") {
      if (!isObject(value)) throw new Error("Invalid MCP content");
      if (value.type === "image" || value.type === "audio")
        return { type: "text", text: unsupported };
      if (value.type === "text") return fields(value, { type: fixed("text") });
      if (value.type === "resource")
        return fields(value, {
          type: fixed("resource"),
          resource: (v) => visit(v, "resourceContent"),
        });
      if (value.type === "resource_link")
        return fields(value, {
          type: fixed("resource_link"),
          uri: control,
          name: control,
          mimeType: control,
        });
      return visit(value, "constraint");
    }
    // Unknown response/request kinds are checked conservatively, never inferred
    // from a `tools`, `resources`, `inputSchema`, or `isError` property.
    return visit(value, "constraint");
  };
  const entries = (value, fn) => {
    if (!isObject(value)) throw new Error("Invalid MCP object");
    const pairs = Object.entries(value).map(([field, item]) => fn(field, item));
    if (new Set(pairs.map(([field]) => field)).size !== pairs.length)
      throw new Error("MCP data keys collide after redaction");
    return Object.fromEntries(pairs);
  };
  const fields = (value, rules) =>
    entries(value, (field, item) =>
      Object.hasOwn(rules, field)
        ? [field, rules[field](item)]
        : [cleaner.text(field), visit(item, "data")],
    );
  const list = (value, mode) => {
    if (!Array.isArray(value)) throw new Error("Invalid MCP list");
    return value.map((item) => visit(item, mode));
  };
  const boolean = (value) => {
    if (typeof value !== "boolean")
      throw new Error("Invalid MCP boolean control");
    return value;
  };
  const fixed = (expected) => (value) => {
    if (value !== expected) throw new Error("Invalid MCP fixed protocol value");
    return value;
  };
  const decodeBlob = (value, mode) => {
    if (Object.hasOwn(value, "text"))
      throw new Error("MCP blob and text content conflict");
    let decoded;
    try {
      decoded = new TextDecoder("utf-8", { fatal: true }).decode(
        Buffer.from(value.blob, "base64"),
      );
    } catch {
      /* opaque */
    }
    const { blob, ...rest } = value;
    return visit(
      {
        ...rest,
        text:
          decoded !== undefined && !/[\x00-\x08\x0e-\x1f]/.test(decoded)
            ? decoded
            : unsupported,
      },
      mode,
    );
  };
  const result = (value) => {
    if (message.method !== undefined) return visit(value, "constraint");
    switch (requestMethod) {
      case "initialize":
        return fields(value, {
          protocolVersion: control,
          serverInfo: (v) => fields(v, { name: control, version: control }),
          capabilities: (v) =>
            fields(v, {
              tools: (v) => fields(v, { listChanged: boolean }),
              prompts: (v) => fields(v, { listChanged: boolean }),
              resources: (v) =>
                fields(v, { listChanged: boolean, subscribe: boolean }),
            }),
        });
      case "tools/list":
        return fields(value, {
          tools: (v) => list(v, "tool"),
          nextCursor: control,
        });
      case "tools/call":
        return fields(value, {
          content: (v) => list(v, "content"),
          isError: boolean,
        });
      case "resources/list":
        return fields(value, {
          resources: (v) => list(v, "resource"),
          nextCursor: control,
        });
      case "resources/templates/list":
        return fields(value, {
          resourceTemplates: (v) => list(v, "resource"),
          nextCursor: control,
        });
      case "resources/read":
        return fields(value, { contents: (v) => list(v, "resourceContent") });
      default:
        return visit(value, "constraint");
    }
  };
  return fields(message, {
    jsonrpc: fixed("2.0"),
    id: control,
    method: control,
    result,
    error: (v) => fields(v, { code: control }),
    params: (v) =>
      message.method === "notifications/message"
        ? visit(v, "data")
        : visit(v, "constraint"),
  });
}

export async function mcpExtensions(runtime, context, agentDir) {
  const source = path.join(agentDir, "mcp.json");
  const options = context.params.mcp ?? {};
  const cleaner = mcpScrubber(mcpSecrets(options));
  const config = await writeSessionMcpConfig(
    agentDir,
    context.compactOnly || context.params.model?.tools === false
      ? null
      : options,
  );
  const raw = config.servers;
  const entries = Object.entries(raw).map(([name, config]) => ({
    name,
    config: cleaner.scrub(config, false),
    source,
    scope: "extension",
  }));
  context.mcpUnavailable = [
    ...config.unavailable,
    ...(options.unavailable ? [{ name: null, reason: "unavailable" }] : []),
  ];
  for (const unavailable of context.mcpUnavailable)
    runtime.notify(context, "mcp_status", unavailable);
  context.mcpEnabled = entries.length > 0;
  const transports = new Set();
  let closing = runtime.closing || context.abort.signal.aborted;
  let closePromise;
  let cleanupConfirmed = true;
  context.closeMcp = () => {
    closing = true;
    return (closePromise ??= Promise.allSettled(
      [...transports].map((transport) => transport.close()),
    ).then(
      (results) =>
        cleanupConfirmed &&
        results.every(
          (result) => result.status === "fulfilled" && result.value === true,
        ),
    ));
  };
  if (!entries.length)
    return context.mcpUnavailable.length
      ? [
          (pi) => {
            pi.on("before_agent_start", (event) => ({
              systemPrompt: `${event.systemPrompt}\nMCP unavailable: ${JSON.stringify(context.mcpUnavailable)}`,
            }));
          },
        ]
      : [];
  const logPath = path.join(agentDir, "mcp.log");
  await writeFile(logPath, "", { mode: 0o600 });
  await chmod(logPath, 0o600);
  const diagnostic = (name, text) => {
    const safe = cleaner.text(String(text));
    void appendFile(logPath, `${cleaner.text(name)}: ${safe}\n`, {
      mode: 0o600,
    }).catch(() => {});
  };
  const createTransport = (entry, cwd) => {
    if (closing || runtime.closing || context.abort.signal.aborted)
      throw new Error("MCP run closed");
    const config = raw[entry.name];
    if (!config) throw new Error("MCP server snapshot changed");
    if (cleaner.text(entry.name) !== entry.name)
      throw new Error("MCP identifier contains a configured secret");

    // Pi receives no raw stderr accessor. Suppress this diagnostic channel: the
    // SDK callback decodes individual chunks and cannot safely reconstruct UTF-8.
    const command = config.command;
    const env = { ...config.env };
    if (options.searchPath && env.PATH === undefined)
      env.PATH = options.searchPath;
    if (
      options.npmRegistry &&
      env.npm_config_registry === undefined &&
      env.NPM_CONFIG_REGISTRY === undefined
    )
      env.npm_config_registry = options.npmRegistry;
    const controlNonce = randomUUID();
    const control = (action) => ({
      jsonrpc: "2.0",
      method: "ofox/private_control",
      params: { nonce: controlNonce, action },
    });
    const inner = config.url
      ? new StreamableHttpTransport({
          url: config.url,
          headers: config.headers,
        })
      : new StdioTransport({
          command: process.execPath,
          args: [fileURLToPath(new URL("./mcp-launcher.mjs", import.meta.url))],
          cwd: config.cwd ?? cwd,
          env: { ...env, OFOX_MCP_CONTROL: controlNonce },
          stderr: "pipe",
          onStderr: () => {},
        });
    let closed = false;
    let closingTransport;
    let resolveClosed;
    const didClose = new Promise((resolve) => {
      resolveClosed = resolve;
    });
    const unavailable = () => {
      if (!context.mcpUnavailable.some((item) => item.name === entry.name))
        context.mcpUnavailable.push({
          name: entry.name,
          reason: "unavailable",
        });
      runtime.notify(context, "mcp_status", {
        name: entry.name,
        reason: "unavailable",
      });
    };
    // Only outstanding client requests confer response grammar authority.
    const pending = new Map();
    inner.onClose(() => {
      pending.clear();
      closed = true;
      resolveClosed();
      if (!closingTransport && !closing) unavailable();
    });
    const bounded = (promise, ms) =>
      Promise.race([
        promise,
        delay(ms, undefined, { ref: false }).then(() => {
          throw new Error("MCP cleanup timeout");
        }),
      ]);
    const lifetime = {
      close: () =>
        (closingTransport ??= (async () => {
          pending.clear();
          try {
            if (!config.url && !closed && inner.pid) {
              await bounded(
                inner.send(control("stop")).catch(() => {}),
                500,
              );
              await bounded(didClose, 2500);
            }
            // Stdio's public onClose has fired and its child is cleared. Calling
            // close now cannot enter the SDK's cached numeric PID/tree-kill branch.
            if (config.url || closed || !inner.pid) await inner.close();
            return true;
          } catch {
            cleanupConfirmed = false;
            diagnostic(
              entry.name,
              "MCP cleanup timeout; run teardown required",
            );
            return false;
          } finally {
            transports.delete(lifetime);
          }
        })()),
    };
    transports.add(lifetime);
    const observe = (phase, method, started) =>
      runtime.observeMcp?.({
        phase,
        server: entry.name,
        method,
        pid: inner.pid,
        timeMs: performance.now(),
        ...(started === undefined
          ? {}
          : { durationMs: performance.now() - started }),
      });
    const safeError = (error) =>
      new Error(cleaner.text(String(error?.message ?? error)).slice(0, 2000));
    const guard = async (fn) => {
      try {
        return await fn();
      } catch (error) {
        throw safeError(error);
      }
    };
    return {
      start: async () => {
        try {
          if (closing || runtime.closing || context.abort.signal.aborted)
            throw new Error("MCP run closed");
          await guard(() => inner.start());
          if (closing || runtime.closing || context.abort.signal.aborted) {
            await lifetime.close();
            throw new Error("MCP run closed");
          }
          if (!config.url)
            await inner.send({
              ...control("start"),
              params: {
                ...control("start").params,
                command,
                args: config.args ?? [],
                cwd: config.cwd ?? cwd,
              },
            });
        } catch (error) {
          runtime.notify(context, "mcp_status", {
            name: entry.name,
            reason: "unavailable",
          });
          throw error;
        }
      },
      send: (message, sendOptions) => {
        const request =
          message.id !== undefined && typeof message.method === "string";
        if (request) {
          if (pending.has(message.id) || pending.size >= 256)
            return Promise.reject(
              new Error("MCP request context limit or duplicate ID"),
            );
          pending.set(message.id, {
            method: message.method,
            started: performance.now(),
          });
        }
        if (message.method === "notifications/cancelled")
          pending.delete(message.params?.requestId);
        observe("send", message.method);
        return guard(() => inner.send(message, sendOptions)).catch((error) => {
          if (request) pending.delete(message.id);
          throw error;
        });
      },
      setProtocolVersion: (version) => inner.setProtocolVersion?.(version),
      close: lifetime.close,
      onClose: (listener) => inner.onClose(listener),
      onError: (listener) =>
        inner.onError((error) => {
          runtime.notify(context, "mcp_status", {
            name: entry.name,
            reason: "unavailable",
          });
          listener(safeError(error));
        }),
      onMessage: (listener) =>
        inner.onMessage((message) => {
          try {
            const isResponse =
              message.method === undefined &&
              message.id !== undefined &&
              (Object.hasOwn(message, "result") ||
                Object.hasOwn(message, "error"));
            const request = isResponse ? pending.get(message.id) : undefined;
            if (request) {
              pending.delete(message.id);
              observe("result", request.method, request.started);
            }
            const safe = sanitizeMcpMessage(message, cleaner, request?.method);
            if (safe.method === "notifications/message") {
              diagnostic(entry.name, JSON.stringify(safe.params));
              return;
            }
            listener(safe);
          } catch {
            unavailable();
            void lifetime.close();
          }
        }),
    };
  };
  const wrap = (factory) => (pi) =>
    factory(
      new Proxy(pi, {
        get(target, key) {
          if (key !== "registerTool") return Reflect.get(target, key);
          return (definition) => {
            const server = definition.namespace?.aliases?.[0];
            const metadata = {
              server: server ?? null,
              toolName: definition.name,
              readOnly:
                definition.annotations?.readOnlyHint === true ||
                resources.has(definition.name) ||
                definition.name === "tool_search",
            };
            const identity = createHash("sha256")
              .update(
                JSON.stringify([
                  metadata,
                  definition.parameters,
                  definition.annotations,
                ]),
              )
              .digest("hex");
            pi.registerTool({
              ...definition,
              execute: async (id, args, signal, onUpdate, ctx) =>
                runtime
                  .wrapTool(
                    context,
                    definition.name,
                    definition.description,
                    definition.parameters,
                    async (safeArgs, callSignal, callId) => {
                      const authorized = await runtime.hostTool(
                        context,
                        "authorize_mcp",
                        {
                          ...metadata,
                          identity,
                          args: cleaner.scrub(safeArgs),
                          snapshot: options.snapshot,
                        },
                        callSignal,
                        callId,
                      );
                      if (authorized?.approved !== true)
                        throw new Error("MCP operation was not approved");
                      const result = await definition.execute(
                        id,
                        safeArgs,
                        callSignal,
                        onUpdate,
                        ctx,
                      );
                      if (
                        definition.name === "tool_search" &&
                        context.mcpUnavailable.length
                      )
                        result.content.push({
                          type: "text",
                          text: `Unavailable MCP servers: ${JSON.stringify(context.mcpUnavailable)}`,
                        });
                      return result;
                    },
                    {
                      mcp: {
                        server: server ?? null,
                        tool: definition.name,
                        identity,
                      },
                    },
                  )
                  .execute(id, args, signal),
            });
          };
        },
      }),
    );
  const credentials = {
    forServer: () => ({
      load: () => undefined,
      save: () => {
        throw new Error("MCP sign-in unsupported");
      },
      withRefreshLock: (fn) => fn(),
    }),
    tokens: () => undefined,
    remove: () => false,
  };
  return [
    (pi) => {
      pi.on("before_agent_start", (event) =>
        context.mcpUnavailable.length
          ? {
              systemPrompt: `${event.systemPrompt}\nMCP servers unavailable: ${JSON.stringify(context.mcpUnavailable)}`,
            }
          : undefined,
      );
    },
    wrap(
      createMcpExtension({
        loadConfig: () => ({
          servers: entries,
          errors: [],
          autoEnableCodemode: false,
        }),
        createTransport,
        credentials,
        logPath,
        openUrl: () => {
          throw new Error("MCP sign-in unsupported");
        },
        updateConfig: () => {
          throw new Error("MCP settings are managed by Ofox");
        },
      }),
    ),
    wrap(createToolSearchExtension()),
  ];
}
