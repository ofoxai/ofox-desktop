const credentialStem =
  /key|token|secret|passw|credential|cookie|auth|signature|private/i;
const credentialWord = /(^|[^a-z0-9])(pin|pwd|pass|pat)([^a-z0-9]|$)/i;
const plainNames = new Set([
  "debug",
  "log_level",
  "node_env",
  "lang",
  "tz",
  "content-type",
  "accept",
  "user-agent",
]);
// Flags and small counts (AUTH_ENABLED=true, TOKEN_TTL=60) are settings.
const setting = (value) =>
  ["", "true", "false", "null", "yes", "no", "on", "off"].includes(
    value.toLowerCase(),
  ) || /^\d{1,3}$/.test(value);

// Configured values are private by default (a webhook URL or a database URL
// can carry a credential under any name). Only plain settings are exempt, so
// DEBUG=1 or LOG_LEVEL=info do not rewrite every matching word or collide
// with request ids. Mirrors credential_value in
// src-tauri/src/services/assistant/mcp.rs.
export function credentialValue(name, value) {
  if (setting(value)) return false;
  if (credentialStem.test(name) || credentialWord.test(name)) return true;
  return !plainNames.has(name.toLowerCase()) && [...value].length >= 8;
}

function querySecrets(server) {
  if (typeof server.url !== "string") return [];
  let url;
  try {
    url = new URL(server.url);
  } catch {
    // The Host validates URLs; an unavailable/malformed server has no query.
    return [];
  }
  const raw = url.search.slice(1).split("&").filter(Boolean);
  const configured = server.url
    .slice(server.url.indexOf("?") + 1)
    .split("#")[0]
    .split("&")
    .filter(Boolean);
  return [...url.searchParams].flatMap(([name, value], index) => {
    if (!credentialValue(name, value)) return [];
    const pair = raw[index];
    const encoded = pair.slice(pair.indexOf("=") + 1);
    const bare = /^(?:Bearer|Basic|Token)\s+(.+)$/i.exec(value)?.[1];
    return [
      encoded,
      configured[index]?.slice(configured[index].indexOf("=") + 1),
      ...[value, ...(bare ? [bare] : [])].flatMap((credential) => [
        credential,
        encodeURIComponent(credential),
        new URLSearchParams([["", credential]]).toString().slice(1),
      ]),
    ].filter(Boolean);
  });
}

export function mcpSecrets(params) {
  const servers = Object.values(params?.servers ?? {});
  return servers
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
    })
    .concat(servers.flatMap(querySecrets));
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
