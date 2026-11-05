export function mcpSecrets(params) {
  return Object.values(params?.servers ?? {})
    .flatMap((server) =>
      [server.env, server.headers].flatMap((values) =>
        Object.values(values ?? {}),
      ),
    )
    .filter((value) => typeof value === "string" && value.length > 0)
    .flatMap((value) => {
      // Servers often echo only the credential, without its HTTP auth scheme.
      const credential = /^(?:Bearer|Basic|Token)\s+(.+)$/i.exec(value)?.[1];
      return credential ? [value, credential] : [value];
    });
}

// All explicit values are private, even short passwords. Replace in one pass:
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
