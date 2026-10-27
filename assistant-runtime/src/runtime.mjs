import { mkdir, stat } from "node:fs/promises";
import path from "node:path";
import { errorMessage, modelForOfox, scrubSecrets } from "./security.mjs";

// Side-effect-free tools that may still run after another operation's outcome
// became unknown. The chat-only runtime registers no tools yet.
const READ_TOOLS = new Set();
const MAX_TOOL_CALLS = 20;

function contentText(content) {
  if (typeof content === "string") return content;
  return (content ?? [])
    .filter((part) => part.type === "text")
    .map((part) => part.text)
    .join("\n");
}

function usageCoverage(usage) {
  return usage &&
    ["input", "output", "cacheRead", "cacheWrite", "totalTokens"].every(
      (key) => Number.isFinite(usage[key]) && usage[key] >= 0,
    ) &&
    usage.totalTokens > 0
    ? "complete"
    : "incomplete";
}

// The gateway refused the request (4xx), or it never left this machine:
// nothing was generated, so its cost is a known zero. A request that was sent
// but got no answer (reset, closed, timed out) may have been billed.
function unbilled(message) {
  return (
    message.stopReason === "error" &&
    ((message.errorStatus >= 400 && message.errorStatus < 500) ||
      message.errorKind === "not_sent")
  );
}

// Why one model request failed: its HTTP status, or no answer at all —
// `not_sent` when the connection never opened, else `network`.
function markFailure(message, outcome) {
  if (message.stopReason !== "error") return message;
  if (outcome.status) message.errorStatus = outcome.status;
  else if (outcome.network) {
    message.errorKind = outcome.network.sent ? "network" : "not_sent";
    if (outcome.network.code) message.errorCode = outcome.network.code;
  }
  return message;
}

// The coded errors along a fetch error's causes; an AggregateError holds one
// per address that was tried.
function codedCauses(error, seen = new Set()) {
  if (!error || typeof error !== "object" || seen.has(error)) return [];
  seen.add(error);
  return [
    ...(typeof error.code === "string" ? [error] : []),
    ...[
      error.cause,
      ...(Array.isArray(error.errors) ? error.errors : []),
    ].flatMap((cause) => codedCauses(cause, seen)),
  ];
}

// Failures that happen before a connection exists: DNS, refusal, routing at
// connect, connect timeout, and TLS certificate checks.
function beforeConnecting({ code, syscall }) {
  return (
    [
      "ECONNREFUSED",
      "ENOTFOUND",
      "EAI_AGAIN",
      "UND_ERR_CONNECT_TIMEOUT",
    ].includes(code) ||
    (["ENETUNREACH", "EHOSTUNREACH"].includes(code) &&
      ["connect", "getaddrinfo"].includes(syscall)) ||
    code.includes("CERT") ||
    code === "HOSTNAME_MISMATCH"
  );
}

function networkFailure(error) {
  const causes = codedCauses(error);
  return {
    sent: !(causes.length > 0 && causes.every(beforeConnecting)),
    code: causes[0]?.code,
  };
}

// A model message is conversation content; its failure text is a diagnostic.
function safeMessage(message, secrets) {
  const safe = scrubSecrets(message, secrets);
  return typeof safe?.errorMessage === "string"
    ? { ...safe, errorMessage: errorMessage(safe.errorMessage, secrets) }
    : safe;
}

function knownPrice(value) {
  return (
    value !== undefined &&
    value !== null &&
    Number.isFinite(Number(value)) &&
    Number(value) >= 0
  );
}

function branchBefore(manager, entryId, regenerate) {
  if (!entryId) return;
  const entries = new Map(
    manager.getEntries().map((entry) => [entry.id, entry]),
  );
  let entry = entries.get(entryId);
  if (!entry || entry.type !== "message")
    throw new Error("Native message entry does not exist");
  if (regenerate) {
    if (entry.message.role !== "assistant")
      throw new Error("Regenerate requires an assistant entry");
    while (
      entry &&
      !(entry.type === "message" && entry.message.role === "user")
    )
      entry = entries.get(entry.parentId);
  }
  if (!entry || entry.type !== "message" || entry.message.role !== "user")
    throw new Error("Edit requires a native user message");
  if (entry.parentId) manager.branch(entry.parentId);
  else manager.resetLeaf();
  return entry.message;
}

function projection(manager, secrets = []) {
  const active = new Set(manager.getBranch().map((entry) => entry.id));
  const all = new Map(manager.getEntries().map((entry) => [entry.id, entry]));
  const markerCache = new Map();
  const markerFor = (entry) => {
    if (!entry) return undefined;
    if (markerCache.has(entry.id)) return markerCache.get(entry.id);
    const lineage = [];
    let marker;
    while (entry) {
      if (markerCache.has(entry.id)) {
        marker = markerCache.get(entry.id);
        break;
      }
      lineage.push(entry.id);
      if (
        entry.type === "custom" &&
        entry.customType === "ofox-desktop-run" &&
        typeof entry.data?.runId === "string" &&
        typeof entry.data?.operationId === "string"
      ) {
        marker = entry.data;
        break;
      }
      entry = all.get(entry.parentId);
    }
    for (const id of lineage) markerCache.set(id, marker);
    return marker;
  };
  const lastAssistant = new Map();
  const summaryHasOwnRecords = (entry) => {
    let parent = all.get(entry.parentId);
    while (parent) {
      if (
        parent.type === "custom" &&
        parent.customType === "ofox-desktop-usage"
      )
        return true;
      if (
        ["compaction", "branch_summary"].includes(parent.type) ||
        (parent.type === "custom" && parent.customType === "ofox-desktop-run")
      )
        return false;
      parent = all.get(parent.parentId);
    }
    return false;
  };
  const entries = [...all.values()]
    .filter(
      (entry) =>
        (entry.type === "message" &&
          ["user", "assistant", "toolResult"].includes(entry.message.role)) ||
        (entry.type === "custom" &&
          entry.customType === "ofox-desktop-usage") ||
        entry.type === "usage" ||
        (["compaction", "branch_summary"].includes(entry.type) &&
          !summaryHasOwnRecords(entry)),
    )
    .map((entry) => {
      if (entry.type !== "message") {
        const data =
          entry.type === "custom"
            ? entry.data
            : {
                type: entry.type === "usage" ? entry.kind : entry.type,
                usage: entry.usage ?? null,
                usageCoverage: usageCoverage(entry.usage),
                pricingKnown: false,
              };
        return scrubSecrets(
          {
            id: entry.id,
            parentId: entry.parentId,
            role: "usage",
            content: "",
            active: active.has(entry.id),
            timestamp: Date.parse(entry.timestamp),
            runId: data.runId,
            operationId: data.operationId,
            type: data.type,
            usage: data.usage,
            usageCoverage: data.usageCoverage,
            pricingKnown: data.pricingKnown === true,
            priceSnapshot: data.priceSnapshot,
          },
          secrets,
        );
      }
      const message = entry.message;
      const marker = markerFor(entry);
      const projected = scrubSecrets(
        {
          id: entry.id,
          parentId: entry.parentId,
          role: message.role === "toolResult" ? "tool" : message.role,
          content: contentText(message.content),
          thinking: Array.isArray(message.content)
            ? message.content
                .filter((part) => part.type === "thinking" && !part.redacted)
                .map((part) => part.thinking)
                .join("\n")
            : "",
          usage: message.usage,
          usageCoverage:
            message.role === "assistant"
              ? unbilled(message)
                ? "complete"
                : usageCoverage(message.usage)
              : undefined,
          active: active.has(entry.id),
          timestamp: message.timestamp ?? Date.parse(entry.timestamp),
          stopReason: message.stopReason,
          errorMessage:
            message.errorMessage === undefined
              ? undefined
              : errorMessage(message.errorMessage, secrets),
          errorStatus: message.errorStatus,
          errorKind: message.errorKind,
          errorCode: message.errorCode,
          runId: marker?.runId,
          operationId: marker?.operationId,
          pricingKnown:
            marker?.pricingKnown === true &&
            (!message.usage?.cacheRead || marker.cacheReadKnown === true) &&
            (!message.usage?.cacheWrite || marker.cacheWriteKnown === true),
        },
        secrets,
      );
      if (marker && message.role === "assistant")
        lastAssistant.set(marker.runId, projected);
      return projected;
    });
  for (const entry of entries)
    if (entry.role === "assistant")
      entry.final = lastAssistant.get(entry.runId) === entry;
  return {
    nativeSessionId: manager.getSessionId(),
    sessionFile: manager.getSessionFile(),
    entries,
    usageCoverage: entries.some((entry) => entry.usageCoverage === "incomplete")
      ? "incomplete"
      : "complete",
  };
}

// Describe only what this run can do. Chat registers no tools; a later tool
// enables its capability by carrying its own Pi-style promptGuidelines.
function systemPrompt(tools) {
  const capabilities = tools.length
    ? [
        "Use only the provided tools.",
        ...tools.flatMap((tool) => tool.promptGuidelines ?? []),
      ]
    : [];
  const role = [
    "You are Ofox Assistant, a general conversation assistant for questions, writing, translation and analysis. Respond in the user's language.",
    ...capabilities,
    "Do not claim to have read images or files you could not access. You have no local configuration or terminal access.",
  ].join(" ");
  return `${role}\nBe clear, useful and concise. Preserve the user's requirements through long conversations.`;
}

export class AssistantRuntime {
  constructor({ hostCall, emit, fetch: providerFetch } = {}) {
    this.hostCall =
      hostCall ??
      (async () => {
        throw new Error("Host tool bridge is unavailable");
      });
    this.emit = emit ?? (() => {});
    this.providerFetch = providerFetch;
    this.runs = new Map();
    this.secrets = new Set();
  }

  notify(context, type, data = {}) {
    this.emit({
      sessionId: context.params.sessionId,
      runId: context.params.runId,
      operationId: context.params.operationId,
      type,
      data: scrubSecrets(data, [...this.secrets]),
    });
  }

  async load(params) {
    const { SessionManager } = await import("@earendil-works/pi-coding-agent");
    await stat(params.sessionFile);
    return projection(
      SessionManager.open(
        params.sessionFile,
        path.dirname(params.sessionFile),
        path.dirname(params.sessionFile),
      ),
      [...this.secrets],
    );
  }

  async stop(params) {
    const context = this.runs.get(params.runId);
    if (
      !context ||
      context.params.sessionId !== params.sessionId ||
      context.params.operationId !== params.operationId
    )
      return { stopped: false };
    context.abort.abort();
    await context.session?.abort();
    return { stopped: true };
  }

  async close() {
    await Promise.all(
      [...this.runs.values()].map(async (context) => {
        context.abort.abort();
        await context.session?.abort();
      }),
    );
    this.secrets.clear();
  }

  // Model tools for this run. Chat has none yet; later tools are built with
  // wrapTool so they share the call budget and the unknown-outcome gate.
  tools(_context, _model) {
    return [];
  }

  hostTool(context, name, args, signal, toolCallId) {
    const { params } = context;
    return this.hostCall(
      {
        sessionId: params.sessionId,
        runId: params.runId,
        operationId: params.operationId,
        toolCallId,
        name,
        args,
      },
      signal,
    );
  }

  wrapTool(context, name, description, parameters, execute) {
    return {
      name,
      label: name,
      description,
      parameters,
      executionMode: "sequential",
      execute: async (toolCallId, args, signal) => {
        signal?.throwIfAborted();
        const fingerprint = JSON.stringify([name, args]);
        const previous = context.calls.get(toolCallId);
        if (previous) {
          if (previous.fingerprint !== fingerprint)
            throw new Error(
              "Tool-call identifier was reused with different arguments",
            );
          return await previous.promise;
        }
        const promise = (async () => {
          if (++context.callCount > MAX_TOOL_CALLS)
            throw new Error("Maximum tool-call count reached");
          if (context.unknownOutcome && !READ_TOOLS.has(name))
            return {
              content: [
                {
                  type: "text",
                  text: "An earlier operation has an unknown outcome. Ask the user to check or retry it; no further side effects were executed.",
                },
              ],
              details: { code: "unknown_outcome" },
              terminate: true,
            };
          this.notify(context, "tool_start", { name, toolCallId, args });
          try {
            const result = await execute(
              scrubSecrets(args, [...this.secrets]),
              signal,
              toolCallId,
            );
            const safe = scrubSecrets(result, [...this.secrets]);
            this.notify(context, "tool_end", {
              name,
              toolCallId,
              success: true,
            });
            return safe?.content
              ? safe
              : {
                  content: [
                    {
                      type: "text",
                      text:
                        typeof safe === "string" ? safe : JSON.stringify(safe),
                    },
                  ],
                  details: safe,
                };
          } catch (error) {
            if (
              error.code === "unknown_outcome" ||
              error.outcomeUnknown === true ||
              /\bassistant_[a-z_]*outcome_unknown\b/.test(String(error.message))
            )
              context.unknownOutcome = true;
            const message = errorMessage(error, [...this.secrets]);
            if (context.unknownOutcome) context.unknownMessage = message;
            this.notify(context, "tool_end", {
              name,
              toolCallId,
              success: false,
              message,
            });
            if (context.unknownOutcome)
              return {
                content: [{ type: "text", text: message }],
                details: { code: "unknown_outcome" },
                terminate: true,
              };
            throw new Error(message);
          }
        })();
        context.calls.set(toolCallId, { fingerprint, promise });
        return await promise;
      },
    };
  }

  async run(params, compactOnly = false) {
    for (const key of [
      "sessionId",
      "runId",
      "operationId",
      "sessionFile",
      "apiKey",
      "apex",
    ])
      if (typeof params[key] !== "string" || !params[key])
        throw new Error(`Missing trusted run parameter: ${key}`);
    if (
      !path.isAbsolute(params.sessionFile) ||
      path.extname(params.sessionFile) !== ".jsonl"
    )
      throw new Error("Native session path must be absolute JSONL");
    if (this.runs.size)
      throw new Error("This sidecar already owns an active run");
    const model = modelForOfox(params.model, params.apex, {
      baseUrl: params.baseUrl,
      allowLocalGateway: params.allowLocalGateway,
    });
    const apiKey = params.apiKey;
    // Every credential of this run: its key, and a key the gateway refused
    // earlier in the same run. Conversation content loses exactly these.
    const secrets = [
      apiKey,
      ...(Array.isArray(params.knownSecrets) ? params.knownSecrets : []),
    ].filter((secret) => typeof secret === "string" && secret.length > 0);
    for (const secret of secrets) this.secrets.add(secret);
    const context = {
      params,
      abort: new AbortController(),
      callCount: 0,
      calls: new Map(),
      unknownOutcome: false,
    };
    this.runs.set(params.runId, context);
    let unsubscribe;
    try {
      const {
        createAgentSession,
        DefaultResourceLoader,
        ModelRuntime,
        SessionManager,
        SettingsManager,
      } = await import("@earendil-works/pi-coding-agent");
      const {
        InMemoryCredentialStore,
        InMemoryModelsStore,
        AssistantMessageEventStream,
      } = await import("@earendil-works/pi-ai");
      const directory = path.dirname(params.sessionFile);
      const workspace = path.join(directory, "workspace");
      const agentDir = path.join(directory, "runtime-settings");
      await mkdir(workspace, { recursive: true, mode: 0o700 });
      await mkdir(agentDir, { recursive: true, mode: 0o700 });
      const manager = SessionManager.open(
        params.sessionFile,
        directory,
        workspace,
      );
      if (params.editEntryId && params.regenerateEntryId)
        throw new Error("Choose edit or regenerate, not both");
      const branchUser = branchBefore(
        manager,
        params.editEntryId ?? params.regenerateEntryId,
        Boolean(params.regenerateEntryId),
      );
      if (!compactOnly) {
        manager.appendCustomEntry("ofox-desktop-run", {
          runId: params.runId,
          operationId: params.operationId,
          pricingKnown: params.model.pricingKnown === true,
          cacheReadKnown: knownPrice(params.model.cost?.cacheRead),
          cacheWriteKnown: knownPrice(params.model.cost?.cacheWrite),
        });
      }
      context.rejectedInput = {
        manager,
        original: scrubSecrets(
          params.regenerateEntryId
            ? contentText(branchUser.content)
            : (params.prompt ?? ""),
          secrets,
        ),
      };
      const settings = SettingsManager.inMemory({
        retry: {
          enabled: false,
          maxRetries: 0,
          provider: { maxRetries: 0, timeoutMs: 900000 },
        },
        compaction: {
          enabled: true,
          reserveTokens: Math.min(
            model.maxTokens,
            Math.floor(model.contextWindow / 4),
          ),
          keepRecentTokens: Math.min(
            20000,
            Math.floor(model.contextWindow / 4),
          ),
        },
        cacheWarming: "off",
        enableInstallTelemetry: false,
        enableAnalytics: false,
      });
      const modelRuntime = await ModelRuntime.create({
        credentials: new InMemoryCredentialStore(),
        modelsStore: new InMemoryModelsStore(),
        modelsPath: null,
        refreshOnCreate: false,
        allowModelNetwork: false,
      });
      modelRuntime.registerProvider(model.provider, {
        name: "Ofox Desktop",
        api: model.api,
        baseUrl: model.baseUrl,
        authHeader: true,
        models: [model],
      });
      await modelRuntime.setRuntimeApiKey(model.provider, apiKey);
      const originalStream = modelRuntime.streamSimple.bind(modelRuntime);
      const providerFetch = this.providerFetch ?? globalThis.fetch;
      const recordSummaryUsage = (target, usage) => {
        const coverage = usageCoverage(usage);
        manager.appendCustomEntry(
          "ofox-desktop-usage",
          scrubSecrets(
            {
              runId: params.runId,
              operationId: params.operationId,
              type: "compaction",
              usage: usage ?? null,
              usageCoverage: coverage,
              pricingKnown:
                coverage === "complete" &&
                params.model.pricingKnown === true &&
                (!usage?.cacheRead ||
                  knownPrice(params.model.cost?.cacheRead)) &&
                (!usage?.cacheWrite ||
                  knownPrice(params.model.cost?.cacheWrite)),
              priceSnapshot: {
                modelId: target.id,
                api: target.api,
                cost: params.model.cost ?? null,
                pricingKnown: params.model.pricingKnown === true,
              },
            },
            secrets,
          ),
        );
      };
      const safeFetch = async (input, options, outcome = {}) => {
        context.abort.signal.throwIfAborted();
        const url = new URL(
          typeof input === "string" || input instanceof URL ? input : input.url,
        );
        if (
          url.origin !== new URL(model.baseUrl).origin ||
          url.href.includes(apiKey) ||
          [...url.searchParams.keys()].some((key) =>
            /key|token|auth/i.test(key),
          )
        )
          throw new Error(
            "Model request URL violates the Ofox credential boundary",
          );
        const headers = new Headers(
          options?.headers ??
            (input instanceof Request ? input.headers : undefined),
        );
        for (const name of ["x-api-key", "api-key", "x-goog-api-key", "cookie"])
          headers.delete(name);
        headers.set("Authorization", `Bearer ${apiKey}`);
        let response;
        try {
          response = await providerFetch(input, {
            ...options,
            headers,
            redirect: "error",
          });
        } catch (error) {
          if (!context.abort.signal.aborted)
            outcome.network = networkFailure(error);
          throw error;
        }
        if (response.status >= 400) outcome.status = response.status;
        return response;
      };
      modelRuntime.streamSimple = (target, transcript, options) => {
        const summaryRequest = options?.sessionId !== manager.getSessionId();
        const outcome = {};
        const source = originalStream(
          target,
          scrubSecrets(transcript, secrets),
          {
            ...options,
            maxRetries: 0,
            timeoutMs: 900000,
            fetch: (input, init) => safeFetch(input, init, outcome),
            env: {},
            headers: { ...options?.headers, Authorization: `Bearer ${apiKey}` },
            identity: { attribution: { name: "Ofox Desktop" } },
          },
        );
        const filtered = new AssistantMessageEventStream();
        (async () => {
          try {
            for await (const event of source) {
              let safe = scrubSecrets(event, secrets);
              // Pi keeps the failed message of the error event.
              if (safe.type === "error" && safe.error)
                safe = {
                  ...safe,
                  error: markFailure(safeMessage(safe.error, secrets), outcome),
                };
              filtered.push(safe);
            }
            const result = markFailure(
              safeMessage(await source.result(), secrets),
              outcome,
            );
            if (summaryRequest) recordSummaryUsage(target, result.usage);
            filtered.end(result);
          } catch (error) {
            if (summaryRequest) recordSummaryUsage(target, null);
            const message = {
              role: "assistant",
              api: target.api,
              provider: target.provider,
              model: target.id,
              content: [],
              timestamp: Date.now(),
              stopReason: "error",
              errorMessage: errorMessage(error, secrets),
              usage: {
                input: 0,
                output: 0,
                cacheRead: 0,
                cacheWrite: 0,
                totalTokens: 0,
                cost: {
                  input: 0,
                  output: 0,
                  cacheRead: 0,
                  cacheWrite: 0,
                  total: 0,
                },
              },
            };
            markFailure(message, outcome);
            filtered.push({ type: "error", reason: "error", error: message });
            filtered.end(message);
          }
        })();
        return filtered;
      };
      const originalComplete = modelRuntime.completeSimple.bind(modelRuntime);
      modelRuntime.completeSimple = async (target, transcript, options) =>
        safeMessage(
          await originalComplete(target, scrubSecrets(transcript, secrets), {
            ...options,
            maxRetries: 0,
            fetch: safeFetch,
          }),
          secrets,
        );
      const customTools = this.tools(context, model);
      const roleText = systemPrompt(customTools);
      const loader = new DefaultResourceLoader({
        cwd: workspace,
        agentDir,
        settingsManager: settings,
        noExtensions: true,
        noSkills: true,
        noPromptTemplates: true,
        noThemes: true,
        noContextFiles: true,
        disabledBuiltinExtensions: ["mcp", "codemode", "tool_search"],
        systemPromptOverride: () => roleText,
        appendSystemPromptOverride: () => [],
        extensionFactories: [
          (pi) => {
            pi.on("message_end", (event) => ({
              message: safeMessage(event.message, secrets),
            }));
            pi.on("context_with_system", (event) => ({
              messages: scrubSecrets(event.messages, secrets),
            }));
            pi.on("tool_result", (event) => ({
              content: scrubSecrets(event.content, secrets),
              details: scrubSecrets(event.details, secrets),
            }));
            pi.on("tool_call", () =>
              context.unknownOutcome || context.abort.signal.aborted
                ? {
                    block: true,
                    terminate: true,
                    reason:
                      "Run stopped or an earlier operation has an unknown outcome",
                  }
                : undefined,
            );
          },
        ],
      });
      await loader.reload();
      context.abort.signal.throwIfAborted();
      const { session } = await createAgentSession({
        cwd: workspace,
        agentDir,
        model,
        modelRuntime,
        sessionManager: manager,
        settingsManager: settings,
        resourceLoader: loader,
        noTools: "builtin",
        tools: customTools.map((tool) => tool.name),
        customTools,
        thinkingLevel: params.thinkingLevel ?? "off",
      });
      context.session = session;
      session.agent.toolExecution = "sequential";
      await session.bindExtensions({
        onError: (event) => {
          throw new Error(errorMessage(event.error, secrets));
        },
      });
      let pendingText = "";
      let pendingThinking = "";
      // Hold back a tail long enough to hold any credential split across
      // deltas until the rest of it arrives and it can be replaced whole.
      const retain = Math.max(
        64,
        ...secrets.map((secret) => secret.length - 1),
      );
      const flush = (type, final = false) => {
        const source = type === "text_delta" ? pendingText : pendingThinking;
        const safe = scrubSecrets(source, secrets);
        const length = final ? safe.length : Math.max(0, safe.length - retain);
        if (length)
          this.notify(context, type, { delta: safe.slice(0, length) });
        if (type === "text_delta") pendingText = safe.slice(length);
        else pendingThinking = safe.slice(length);
      };
      unsubscribe = session.subscribe((event) => {
        if (event.type === "message_update") {
          const update = event.assistantMessageEvent;
          if (update.type === "text_delta") {
            pendingText += update.delta;
            flush("text_delta");
          } else if (update.type === "thinking_delta") {
            pendingThinking += update.delta;
            flush("thinking_delta");
          } else if (update.type === "text_end") flush("text_delta", true);
          else if (update.type === "thinking_end")
            flush("thinking_delta", true);
        } else if (
          event.type === "message_end" &&
          event.message.role === "assistant"
        ) {
          flush("text_delta", true);
          flush("thinking_delta", true);
          this.notify(context, "message_end", {
            role: "assistant",
            content: contentText(event.message.content),
            reasoning: event.message.content
              .filter((part) => part.type === "thinking" && !part.redacted)
              .map((part) => part.thinking)
              .join("\n"),
            usage: event.message.usage,
            stopReason: event.message.stopReason,
          });
          if (event.message.usage)
            this.notify(context, "usage", { usage: event.message.usage });
        } else if (event.type === "auto_compaction_start")
          this.notify(context, "compaction_start");
        else if (event.type === "auto_compaction_end")
          this.notify(context, "compaction_end", {
            success: !event.errorMessage,
          });
      });
      this.notify(context, "run_start");
      context.abort.signal.throwIfAborted();
      if (compactOnly)
        await session.compact(
          scrubSecrets(
            params.instructions ??
              "Preserve the user's requirements, decisions and unfinished work.",
            secrets,
          ),
        );
      else {
        // Regenerate resends the native user turn, including its images.
        const prompt = scrubSecrets(
          params.regenerateEntryId
            ? contentText(branchUser.content)
            : (params.prompt ?? ""),
          secrets,
        );
        const images =
          params.regenerateEntryId && Array.isArray(branchUser.content)
            ? branchUser.content.filter((part) => part.type === "image")
            : [];
        if (images.length && !model.input.includes("image"))
          throw new Error(
            "assistant_vision_model_required: This model cannot read images; choose a vision model",
          );
        if (images.length > 10)
          throw new Error("A message supports at most 10 images");
        context.abort.signal.throwIfAborted();
        context.turnStarted = true;
        await session.prompt(prompt, { images, expandPromptTemplates: false });
      }
      await session.waitForIdle();
      const result = projection(manager, secrets);
      const last = result.entries
        .filter((entry) => entry.active && entry.role === "assistant")
        .at(-1);
      result.status =
        context.abort.signal.aborted || last?.stopReason === "aborted"
          ? "stopped"
          : context.unknownOutcome || last?.stopReason === "error"
            ? "error"
            : "completed";
      if (result.status === "error") {
        result.errorMessage =
          context.unknownMessage ??
          last?.errorMessage ??
          "Assistant operation failed";
        result.errorStatus = last?.errorStatus;
        result.errorKind = last?.errorKind;
        result.errorCode = last?.errorCode;
      }
      this.notify(context, "run_end", { status: result.status });
      return result;
    } catch (error) {
      const message = errorMessage(error, secrets);
      if (!compactOnly && context.rejectedInput && !context.turnStarted) {
        // Preserve failed input in native history before any paid request.
        const { manager, original } = context.rejectedInput;
        manager.appendMessage({
          role: "user",
          content: [{ type: "text", text: original }],
          timestamp: Date.now(),
        });
        manager.appendMessage({
          role: "assistant",
          content: [],
          api: model.api,
          provider: model.provider,
          model: model.id,
          usage: {
            input: 0,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            totalTokens: 0,
            cost: {
              input: 0,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              total: 0,
            },
          },
          stopReason: context.abort.signal.aborted ? "aborted" : "error",
          errorMessage: message,
          errorKind: "not_sent",
          timestamp: Date.now(),
        });
        const rejected = {
          ...projection(manager, secrets),
          status: context.abort.signal.aborted ? "stopped" : "error",
          errorMessage: message,
          errorKind: "not_sent",
        };
        this.notify(context, "error", {
          message,
          code:
            rejected.status === "stopped"
              ? "stopped"
              : "input_preparation_failed",
        });
        this.notify(context, "run_end", { status: rejected.status });
        return rejected;
      }

      this.notify(context, "error", {
        message,
        code: context.abort.signal.aborted ? "stopped" : "runtime_error",
      });
      throw new Error(message);
    } finally {
      unsubscribe?.();
      context.session?.dispose();
      this.runs.delete(params.runId);
      // The process is one run long in production. Retain key fingerprints only until close
      // so a late diagnostic can never echo a previously supplied API key.
    }
  }
}
