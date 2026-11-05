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
export function sanitizeMcpMessage(message, cleaner) {
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
  const contentTypes = new Set([
    "text",
    "image",
    "audio",
    "resource",
    "resource_link",
  ]);
  const booleanHints = new Set([
    "readOnlyHint",
    "destructiveHint",
    "idempotentHint",
    "openWorldHint",
  ]);
  const visit = (value, mode = "protocol", key = "") => {
    if (Array.isArray(value))
      return value.map((item) => visit(item, mode, key));
    if (!value || typeof value !== "object") {
      if (mode === "data") return cleaner.scalar(value);
      // These are fixed syntax, not credential-derived payload: never change
      // a schema's structural boolean or an annotation that grants permission.
      if (mode === "schema" && (typeof value === "boolean" || value === null))
        return value;
      if (
        mode === "annotation" &&
        booleanHints.has(key) &&
        typeof value === "boolean"
      )
        return value;
      if (mode === "schema" && key === "type" && schemaTypes.has(value))
        return value;
      if (
        mode === "protocol" &&
        ((key === "jsonrpc" && value === "2.0") ||
          (key === "type" && contentTypes.has(value)))
      )
        return value;
      if (
        mode === "protocol" &&
        ["isError", "listChanged", "subscribe"].includes(key) &&
        typeof value === "boolean"
      )
        return value;
      return control(value);
    }
    if (
      (mode === "protocol" || mode === "data") &&
      (value.type === "image" || value.type === "audio")
    )
      return { type: "text", text: unsupported };
    if (
      (mode === "protocol" || mode === "data") &&
      typeof value.blob === "string"
    ) {
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
    }
    if (mode === "data") {
      const entries = Object.entries(value).map(([field, item]) => [
        cleaner.text(field),
        visit(item, "data", field),
      ]);
      if (new Set(entries.map(([field]) => field)).size !== entries.length)
        throw new Error("MCP data keys collide after redaction");
      return Object.fromEntries(entries);
    }
    return Object.fromEntries(
      Object.entries(value).map(([field, item]) => {
        if (mode === "constraint") {
          control(field);
          return [field, visit(item, "constraint", field)];
        }
        if (mode === "names") {
          control(field);
          return [field, visit(item, "schema", field)];
        }
        if (mode === "schema") {
          if (["const", "default", "enum", "examples"].includes(field))
            return [field, visit(item, "constraint", field)];
          if (
            [
              "properties",
              "patternProperties",
              "$defs",
              "definitions",
              "dependentSchemas",
            ].includes(field)
          )
            return [field, visit(item, "names", field)];
          if (["description", "title", "$comment"].includes(field))
            return [field, visit(item, "data", field)];
          control(field);
          return [field, visit(item, "schema", field)];
        }
        if (field === "inputSchema" || field === "outputSchema")
          return [field, visit(item, "schema", field)];
        if (field === "annotations")
          return [field, visit(item, "annotation", field)];
        if (
          [
            "structuredContent",
            "data",
            "text",
            "description",
            "title",
            "message",
          ].includes(field)
        )
          return [field, visit(item, "data", field)];
        control(field);
        return [field, visit(item, mode, field)];
      }),
    );
  };
  return visit(message);
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
    inner.onClose(() => {
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
    const pending = new Map();
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
        if (message.id !== undefined)
          pending.set(message.id, {
            method: message.method,
            started: performance.now(),
          });
        observe("send", message.method);
        return guard(() => inner.send(message, sendOptions));
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
            const request = pending.get(message.id);
            if (request) {
              pending.delete(message.id);
              observe("result", request.method, request.started);
            }
            const safe = sanitizeMcpMessage(message, cleaner);
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
