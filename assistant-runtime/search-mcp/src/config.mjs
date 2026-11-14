export const DEFAULT_SEARCH_MODEL = "openai/gpt-5-nano";
// Where web_search searches: Ofox (paid, the default) or Exa (free, keyless).
export const SEARCH_PROVIDERS = ["ofox", "exa"];

const OFOX_GATEWAYS = new Set(["https://api.ofox.ai", "https://api.ofox.io"]);
// Printable ASCII without spaces: anything else cannot be sent in a header.
// At least 16 characters, so masking the key in results cannot hit ordinary
// words.
const KEY_PATTERN = /^[\x21-\x7e]{16,512}$/;
const MODEL_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,255}$/;
const PROXY_SCHEMES = new Set(["http:", "https:", "socks5:"]);

export class ConfigError extends Error {
  constructor(message) {
    super(message);
    this.name = "ConfigError";
  }
}

// The host's debug gateway is plain localhost on an explicit port; tests pick
// a free one. No path, credentials or other host may follow (same rule as
// assistant-runtime/src/security.mjs).
function isLocalGateway(value) {
  const port = /^http:\/\/localhost:([1-9]\d{0,4})$/.exec(value)?.[1];
  return port !== undefined && Number(port) <= 65535;
}

// A test's stand-in for Exa: plain localhost on an explicit port, with a path.
function isLocalExa(value) {
  const port = /^http:\/\/localhost:([1-9]\d{0,4})\/[\w/-]*$/.exec(value)?.[1];
  return port !== undefined && Number(port) <= 65535;
}

// The app's proxy, the one the assistant runtime uses for model requests: an
// HTTP(S) or SOCKS5 URL (credentials included) and the NO_PROXY-style hosts
// that bypass it. The URL is never echoed: it may carry a password.
function proxyConfig(env) {
  const value = env.OFOX_PROXY_URL;
  if (value === undefined || value === "") return null;
  let url;
  try {
    url = new URL(value);
  } catch {
    url = null;
  }
  if (url?.protocol === "socks5h:") url.protocol = "socks5:";
  if (!url || !PROXY_SCHEMES.has(url.protocol) || !url.hostname)
    throw new ConfigError(
      "OFOX_PROXY_URL is not a valid http, https or socks5 proxy URL",
    );
  return {
    url: url.href,
    noProxy: typeof env.OFOX_NO_PROXY === "string" ? env.OFOX_NO_PROXY : "",
  };
}

// Messages are fixed strings: they never echo a value, so a key pasted into
// the wrong variable cannot reach stderr either.
export function loadConfig(env = process.env) {
  const apiKey = env.OFOX_ASSISTANT_KEY;
  if (typeof apiKey !== "string" || !KEY_PATTERN.test(apiKey))
    throw new ConfigError(
      "OFOX_ASSISTANT_KEY is missing or is not one printable token of 16-512 characters",
    );
  const baseUrl = env.OFOX_BASE_URL;
  if (
    typeof baseUrl !== "string" ||
    !(
      OFOX_GATEWAYS.has(baseUrl) ||
      (env.OFOX_ALLOW_LOCAL_GATEWAY === "1" && isLocalGateway(baseUrl))
    )
  )
    throw new ConfigError(
      "OFOX_BASE_URL must be exactly https://api.ofox.ai or https://api.ofox.io",
    );
  const model = env.OFOX_SEARCH_MODEL || DEFAULT_SEARCH_MODEL;
  if (!MODEL_PATTERN.test(model))
    throw new ConfigError("OFOX_SEARCH_MODEL is not a valid model id");
  const provider = env.OFOX_SEARCH_PROVIDER || "ofox";
  if (!SEARCH_PROVIDERS.includes(provider))
    throw new ConfigError("OFOX_SEARCH_PROVIDER must be ofox or exa");
  const config = { apiKey, baseUrl, model, provider, proxy: proxyConfig(env) };
  const exaUrl = env.OFOX_EXA_URL;
  if (exaUrl) {
    if (env.OFOX_ALLOW_LOCAL_GATEWAY !== "1" || !isLocalExa(exaUrl))
      throw new ConfigError(
        "OFOX_EXA_URL is only for tests, with a localhost URL and the local gateway flag",
      );
    config.exaUrl = exaUrl;
  }
  return config;
}
