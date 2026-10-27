export const DEFAULT_SEARCH_MODEL = "openai/gpt-5-nano";

const OFOX_GATEWAYS = new Set(["https://api.ofox.ai", "https://api.ofox.io"]);
// Printable ASCII without spaces: anything else cannot be sent in a header.
const KEY_PATTERN = /^[\x21-\x7e]{1,512}$/;
const MODEL_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,255}$/;

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

// Messages are fixed strings: they never echo a value, so a key pasted into
// the wrong variable cannot reach stderr either.
export function loadConfig(env = process.env) {
  const apiKey = env.OFOX_ASSISTANT_KEY;
  if (typeof apiKey !== "string" || !KEY_PATTERN.test(apiKey))
    throw new ConfigError(
      "OFOX_ASSISTANT_KEY is missing or is not a single printable token",
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
  return { apiKey, baseUrl, model };
}
