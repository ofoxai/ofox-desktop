const credentialStem =
  /key|token|secret|passw|credential|cookie|auth|signature|private/i;
const credentialWord = /(^|[^a-z0-9])(pin|pwd|pass|pat)([^a-z0-9]|$)/i;
// Flags and small counts (AUTH_ENABLED=true, TOKEN_TTL=60) are settings.
const setting = (value) =>
  ["", "true", "false", "null", "yes", "no", "on", "off"].includes(
    value.toLowerCase(),
  ) || /^\d{1,3}$/.test(value);

const tokenShaped = (value) =>
  [...value].length >= 20 &&
  !/[\s/\\]/.test(value) &&
  /[a-z]/i.test(value) &&
  /\d/.test(value);

// Only a value named like a credential, or shaped like a generated token, is
// private. Plain settings (DEBUG=1, LOG_LEVEL=info) would otherwise rewrite
// every matching word and collide with request ids. Mirrors credential_value
// in src-tauri/src/services/assistant/mcp.rs.
export function credentialValue(name, value) {
  const named = credentialStem.test(name) || credentialWord.test(name);
  return (named && !setting(value)) || tokenShaped(value);
}

export function mcpSecrets(params) {
  return Object.values(params?.servers ?? {})
    .flatMap((server) =>
      [server.env, server.headers].flatMap((values) =>
        Object.entries(values ?? {}),
      ),
    )
    .filter(
      ([name, value]) =>
        typeof value === "string" && credentialValue(name, value),
    )
    .flatMap(([, value]) => {
      // Servers often echo only the credential, without its HTTP auth scheme.
      const credential = /^(?:Bearer|Basic|Token)\s+(.+)$/i.exec(value)?.[1];
      return credential ? [value, credential] : [value];
    });
}

// Every selected value is private, even a short password. Replace in one pass:
// a later short match must not corrupt a replacement made for a longer secret.
export function mcpScrubber(values) {
  const secrets = [...new Set(values)]
    .filter(Boolean)
    .sort((a, b) => b.length - a.length);
  const pattern = secrets.length
    ? new RegExp(
        secrets.map((s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|"),
        "g",
      )
    : null;
  const text = (value) => (pattern ? value.replace(pattern, "***") : value);
  const scalar = (value) => {
    if (typeof value === "string") return text(value);
    if (
      value === null ||
      typeof value === "number" ||
      typeof value === "boolean"
    ) {
      const encoded = JSON.stringify(value);
      if (text(encoded) !== encoded) return "***";
    }
    return value;
  };
  const scrub = (value, redactScalars = true) => {
    if (Array.isArray(value))
      return value.map((item) => scrub(item, redactScalars));
    if (value && typeof value === "object") {
      const entries = Object.entries(value).map(([key, item]) => [
        text(key),
        scrub(item, redactScalars),
      ]);
      if (new Set(entries.map(([key]) => key)).size !== entries.length)
        throw new Error("MCP data keys collide after redaction");
      return Object.fromEntries(entries);
    }
    return redactScalars
      ? scalar(value)
      : typeof value === "string"
        ? text(value)
        : value;
  };
  return { text, scalar, scrub, secrets };
}
