const SECRET_FIELD =
  /(?:api.?key|access.?token|refresh.?token|authorization|password|secret|credential)/i;
const REDACTED = "[REDACTED]";

function replaceExact(value, secrets) {
  if (typeof value === "string") {
    let result = value;
    for (const secret of secrets)
      if (result.includes(secret)) result = result.split(secret).join(REDACTED);
    return result;
  }
  if (Array.isArray(value)) {
    const items = value.map((item) => replaceExact(item, secrets));
    return items.some((item, index) => item !== value[index]) ? items : value;
  }
  if (value && typeof value === "object") {
    const entries = Object.entries(value);
    const scrubbed = entries.map(([key, item]) => [
      key,
      replaceExact(item, secrets),
    ]);
    return scrubbed.some(([, item], index) => item !== entries[index][1])
      ? Object.fromEntries(scrubbed)
      : value;
  }
  return value;
}

// Conversation content (prompts, replies, native history, events) passes byte
// for byte: code, config samples and "password:" prompts are the user's words.
// Only the exact credentials this process was given are replaced; a value
// without one is returned as it is.
export function scrubSecrets(value, secrets = []) {
  const known = [...secrets].filter(
    (secret) => typeof secret === "string" && secret.length >= 8,
  );
  return known.length ? replaceExact(value, known) : value;
}

// Diagnostics only (stderr, error text): the known credentials and anything
// that looks like one. Never applied to conversation content.
export function redact(value, secrets = []) {
  if (typeof value === "string") {
    let result = value;
    for (const secret of secrets)
      if (typeof secret === "string" && secret.length > 0)
        result = result.split(secret).join(REDACTED);
    return result
      .replace(/\b([a-z][a-z0-9+.-]*:\/\/)[^\s/@]+@/gi, `$1${REDACTED}@`)
      .replace(/\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+/gi, `$1 ${REDACTED}`)
      .replace(/\bsk-(?:of-|ofi-|ant-|proj-)?[A-Za-z0-9_-]{8,}/g, REDACTED)
      .replace(
        /((?:api[_-]?key|access[_-]?token|refresh[_-]?token|password|secret)\s*[=:]\s*)(["']?)[^\s,"'}]+/gi,
        `$1${REDACTED}`,
      );
  }
  if (Array.isArray(value)) return value.map((item) => redact(item, secrets));
  if (value && typeof value === "object")
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [
        key,
        SECRET_FIELD.test(key) ? REDACTED : redact(item, secrets),
      ]),
    );
  return value;
}

export function errorMessage(error, secrets = []) {
  return redact(
    error instanceof Error ? error.message : String(error),
    secrets,
  ).slice(0, 2000);
}

// The host's debug gateway is plain localhost on an explicit port (8088 in the
// app; tests pick a free one). No path, credentials or other host may follow.
function isLocalGateway(gateway) {
  const port = /^http:\/\/localhost:([1-9]\d{0,4})$/.exec(gateway)?.[1];
  return port !== undefined && Number(port) <= 65535;
}

/** Pi's thinking levels, lowest first. */
export const THINKING_LEVELS = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
];

function thinkingLevelOf(value) {
  if (value === undefined || value === null) return undefined;
  if (!THINKING_LEVELS.includes(value))
    throw new Error("Unsupported thinking level");
  return value;
}

// Pi's map from a level to what the provider is sent; null: not offered.
function levelMap(map) {
  if (map === undefined || map === null) return undefined;
  if (
    typeof map !== "object" ||
    Array.isArray(map) ||
    Object.entries(map).some(
      ([level, sent]) =>
        !THINKING_LEVELS.includes(level) ||
        (sent !== null &&
          (typeof sent !== "string" || !/^[A-Za-z0-9_-]{1,64}$/.test(sent))),
    )
  )
    throw new Error("Invalid thinking level map");
  return { ...map };
}

// The Anthropic compat fields Pi's Messages adapter reads for thinking.
function anthropicCompat(value) {
  const compat = {};
  for (const field of ["forceAdaptiveThinking", "supportsTemperature"])
    if (typeof value?.[field] === "boolean") compat[field] = value[field];
  return Object.keys(compat).length ? compat : undefined;
}

export function modelForOfox(
  input,
  apex,
  { baseUrl, allowLocalGateway, thinkingLevel } = {},
) {
  if (apex !== "ofox.ai" && apex !== "ofox.io")
    throw new Error("Unsupported Ofox apex");
  const gateway = baseUrl ?? `https://api.${apex}`;
  if (
    gateway !== `https://api.${apex}` &&
    !(allowLocalGateway === true && isLocalGateway(gateway))
  )
    throw new Error("Host-resolved model gateway is not allowed");
  if (
    !input ||
    typeof input.id !== "string" ||
    !input.id ||
    input.id.length > 256
  )
    throw new Error("Invalid model identifier");
  const endpoints = input.supportedEndpoints ?? input.supported_endpoints ?? [];
  const aliases = new Map([
    ["openai", "openai-completions"],
    ["chatCompletions", "openai-completions"],
    ["openai-completions", "openai-completions"],
    ["chat/completions", "openai-completions"],
    ["responses", "openai-responses"],
    ["openai-responses", "openai-responses"],
    ["anthropic", "anthropic-messages"],
    ["anthropic-messages", "anthropic-messages"],
    ["messages", "anthropic-messages"],
    // How the host lists a model of the /anthropic/v1/models catalog.
    ["anthropic/v1/messages", "anthropic-messages"],
  ]);
  const supported = endpoints
    .map((endpoint) =>
      typeof endpoint === "string"
        ? aliases.get(endpoint.replace(/^\/?(?:v1\/)?/, ""))
        : undefined,
    )
    .filter(Boolean);
  const api =
    input.api && supported.includes(input.api) ? input.api : supported[0];
  if (!api)
    throw new Error(
      "Unsupported model protocol; Google-only OAuth transport is not supported",
    );
  const contextWindow = Number(
    input.contextWindow ?? input.contextLength ?? input.context_length,
  );
  const maxTokens = Number(
    input.maxOutputTokens ?? input.maxTokens ?? input.max_completion_tokens,
  );
  if (
    !Number.isSafeInteger(contextWindow) ||
    contextWindow < 1024 ||
    !Number.isSafeInteger(maxTokens) ||
    maxTokens < 1
  )
    throw new Error("Model context capacity or output limit is unavailable");
  const price = input.cost ?? {};
  // A run thinks only at a level the host chose for a model the catalog says
  // reasons. Otherwise Pi sends no thinking parameter at all and the
  // provider's default applies, exactly as before levels existed.
  const thinks =
    thinkingLevelOf(thinkingLevel) !== undefined && input.reasoning === true;
  const thinkingLevelMap = thinks
    ? levelMap(input.thinkingLevelMap)
    : undefined;
  return {
    id: input.id,
    name: input.name || input.id,
    provider: "ofox-desktop",
    api,
    baseUrl: `${gateway}${api === "anthropic-messages" ? "/anthropic" : "/v1"}`,
    reasoning: thinks,
    ...(thinkingLevelMap ? { thinkingLevelMap } : {}),
    input:
      input.supportsImages ||
      input.inputModalities?.includes("image") ||
      input.input_modalities?.includes("image")
        ? ["text", "image"]
        : ["text"],
    contextWindow,
    maxTokens: Math.min(maxTokens, contextWindow - 512),
    cost: Object.fromEntries(
      ["input", "output", "cacheRead", "cacheWrite"].map((key) => [
        key,
        Number.isFinite(Number(price[key])) && Number(price[key]) >= 0
          ? Number(price[key])
          : 0,
      ]),
    ),
    compat:
      api === "openai-completions"
        ? {
            supportsDeveloperRole: false,
            supportsStore: false,
            supportsUsageInStreaming: true,
            maxTokensField: "max_tokens",
          }
        : api === "anthropic-messages" && thinks
          ? anthropicCompat(input.anthropicCompat)
          : undefined,
  };
}
