const REDACTED = "[REDACTED]";

// Same rules as assistant-runtime/src/security.mjs: the known secrets first, then
// anything that looks like a bearer token, an Ofox/OpenAI-style key or a
// `key=value` secret. Everything this server writes to stderr or returns as
// error text passes through here.
export function redact(text, secrets = []) {
  let result = String(text);
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
