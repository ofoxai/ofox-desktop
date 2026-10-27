const REDACTED = "[REDACTED]";

// Same rules as assistant-runtime/src/security.mjs: the known secrets first, then
// anything that looks like a bearer token, an Ofox/OpenAI-style key or a
// `key=value` secret. Everything this server writes to stderr or returns as
// error text passes through here.
export function redact(text, secrets = []) {
  return scrubSecrets(String(text), secrets)
    .replace(/\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+/gi, `$1 ${REDACTED}`)
    .replace(/\bsk-(?:of-|ofi-|ant-|proj-)?[A-Za-z0-9_-]{8,}/g, REDACTED)
    .replace(
      /((?:api[_-]?key|access[_-]?token|refresh[_-]?token|password|secret)\s*[=:]\s*)(["']?)[^\s,"'}]+/gi,
      `$1${REDACTED}`,
    );
}

// The secret as written and percent-encoded (upper- and lower-case hex), the
// ways a key can come back inside page text or a URL.
function secretForms(secret) {
  const encoded = encodeURIComponent(secret);
  const lower = encoded.replace(/%[0-9A-F]{2}/g, (hex) => hex.toLowerCase());
  return [...new Set([secret, encoded, lower])];
}

// Same length as the text it replaces, so offsets into the text (citation
// indices) stay valid after scrubbing.
function mask(length) {
  return `${REDACTED}${"*".repeat(length)}`.slice(0, length);
}

// The secrets (in the forms above) only, in every string of a JSON-like
// value: upstream data (search results, page text) is passed on unchanged
// except for the key.
export function scrubSecrets(value, secrets = []) {
  if (typeof value === "string") {
    let result = value;
    for (const secret of secrets)
      if (typeof secret === "string" && secret.length > 0)
        for (const form of secretForms(secret))
          result = result.split(form).join(mask(form.length));
    return result;
  }
  if (Array.isArray(value))
    return value.map((item) => scrubSecrets(item, secrets));
  if (value && typeof value === "object")
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [
        key,
        scrubSecrets(item, secrets),
      ]),
    );
  return value;
}
