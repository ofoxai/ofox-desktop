const SECRET_FIELD =
  /(?:api.?key|access.?token|refresh.?token|authorization|password|secret|credential)/i;
const REDACTED = "[REDACTED]";

export function redact(value, secrets = []) {
  if (typeof value === "string") {
    let result = value;
    for (const secret of secrets)
      if (typeof secret === "string" && secret.length > 0)
        result = result.split(secret).join(REDACTED);
    return result
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

export function modelForOfox(input, apex, { baseUrl, allowLocalGateway } = {}) {
  if (apex !== "ofox.ai" && apex !== "ofox.io")
    throw new Error("Unsupported Ofox apex");
  const gateway = baseUrl ?? `https://api.${apex}`;
  if (
    gateway !== `https://api.${apex}` &&
    !(allowLocalGateway === true && gateway === "http://localhost:8088")
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
  return {
    id: input.id,
    name: input.name || input.id,
    provider: "ofox-desktop",
    api,
    baseUrl: `${gateway}${api === "anthropic-messages" ? "/anthropic" : "/v1"}`,
    reasoning: input.reasoning === true,
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
        : undefined,
  };
}
