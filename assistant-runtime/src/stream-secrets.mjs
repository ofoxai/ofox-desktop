// Longest suffix that could still become a known credential. Compute it in
// linear time even for long, repetitive keys; ordinary text needs no delay.
function prefixTable(secret) {
  const prefix = new Uint32Array(secret.length);
  for (let i = 1, matched = 0; i < secret.length; i++) {
    while (matched && secret[i] !== secret[matched])
      matched = prefix[matched - 1];
    if (secret[i] === secret[matched]) matched++;
    prefix[i] = matched;
  }
  return prefix;
}

function scan(text, secret, tables, spans) {
  // Most ordinary deltas have no candidate start at all. Don't build or scan
  // a long credential's prefix table for them.
  if (!text.includes(secret[0])) return 0;
  let prefix = tables.get(secret);
  if (!prefix) {
    prefix = prefixTable(secret);
    tables.set(secret, prefix);
  }
  let matched = 0;
  let last;
  for (let i = 0; i < text.length; i++) {
    while (matched && text[i] !== secret[matched])
      matched = prefix[matched - 1];
    if (text[i] === secret[matched]) matched++;
    if (matched === secret.length) {
      const at = i + 1 - secret.length;
      if (last && at <= last.end) last.end = i + 1;
      else {
        last = { at, end: i + 1, long: secret.length >= 8 };
        spans.push(last);
      }
      matched = prefix[matched - 1];
    }
  }
  return matched;
}

/** Run-local partitioner; `secrets` is the live Host-updated credential array. */
export function createStreamScrubber(secrets) {
  const tables = new Map();
  return (source, final = false) => {
    // The Host already classified credentials, including short MCP PINs and
    // bare auth values. Never independently reinterpret their configuration.
    const known = [
      ...new Set(
        secrets.filter(
          (secret) => typeof secret === "string" && secret.length > 0,
        ),
      ),
    ];
    let retain = 0;
    const spans = [];
    for (const secret of known)
      retain = Math.max(retain, scan(source, secret, tables, spans));
    let end = final ? source.length : source.length - retain;
    // Provider chunks may split a UTF-16 surrogate pair. Rust's JSON decoder
    // must receive the pair together, including at a credential boundary.
    const last = source.charCodeAt(end - 1);
    if (!final && last >= 0xd800 && last <= 0xdbff) end--;
    // Merge every overlapping raw match before choosing a publish boundary.
    // A short value must not erase a longer prefix or expose its remaining
    // bytes. No regular-expression size limit applies to long credentials.
    spans.sort((a, b) => a.at - b.at);
    const merged = [];
    for (const span of spans) {
      const last = merged.at(-1);
      if (last && span.at < last.end) {
        last.end = Math.max(last.end, span.end);
        last.long ||= span.long;
      } else merged.push({ ...span });
    }
    let ready = "",
      from = 0;
    for (const span of merged) {
      if (span.at >= end) break;
      if (span.end > end) {
        end = span.at;
        break;
      }
      ready += source.slice(from, span.at) + (span.long ? "[REDACTED]" : "***");
      from = span.end;
    }
    // Repetitive overlapping matches can otherwise make the withheld union
    // grow forever. Refuse this pathological stream without publishing it.
    const pendingLimit = known.reduce(
      (limit, value) => Math.max(limit, value.length * 2),
      1024,
    );
    if (source.length - end > pendingLimit)
      throw new Error("assistant_runtime_protocol_limit");
    return {
      ready: ready + source.slice(from, end),
      pending: source.slice(end),
    };
  };
}
