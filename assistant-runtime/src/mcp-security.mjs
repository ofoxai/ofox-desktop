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
  const scrub = (value) => {
    if (typeof value === "string") return text(value);
    if (Array.isArray(value)) return value.map(scrub);
    if (value && typeof value === "object")
      return Object.fromEntries(
        Object.entries(value).map(([key, item]) => [text(key), scrub(item)]),
      );
    return value;
  };
  return { text, scrub, secrets };
}
