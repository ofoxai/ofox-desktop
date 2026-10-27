// Read one public web page as text. Ported from the reference runtime's
// web.mjs: HTTPS on 443 only, every DNS answer must be public, the connection
// is pinned to the checked address, every redirect is re-checked, and the
// 15 s / 2 MiB / 20,000-character limits hold. Only static HTML or plain text
// is read; no cookies are sent or kept.
//
// The "only URLs from the user or this conversation's search" allowlist is
// not here: the client wrapper that exposes this tool to the model owns it.
import { lookup as dnsLookup } from "node:dns/promises";
import https from "node:https";
import { isIP } from "node:net";
import { scrubSecrets } from "./redact.mjs";

export const FETCH_TIMEOUT_MS = 15_000;
export const MAX_PAGE_BYTES = 2 * 1024 * 1024;
export const MAX_PAGE_CHARS = 20_000;
const MAX_REDIRECTS = 4;
const BLOCKED_HOSTS = new Set([
  "localhost",
  "api.ofox.ai",
  "api.ofox.io",
  "app.ofox.ai",
  "app.ofox.io",
  "console.ofox.ai",
  "console.ofox.io",
]);

export class FetchError extends Error {
  constructor(code, message, status) {
    super(message);
    this.name = "FetchError";
    this.code = code;
    if (status !== undefined) this.status = status;
  }
}

const notAllowed = (message) => new FetchError("fetch_not_allowed", message);
const tooLarge = () =>
  new FetchError("fetch_too_large", "Web response exceeds 2 MiB");

export function isPublicAddress(address) {
  if (isIP(address) === 4) {
    const [a, b, c] = address.split(".").map(Number);
    return !(
      a === 0 ||
      a === 10 ||
      a === 127 ||
      a >= 224 ||
      (a === 100 && b >= 64 && b <= 127) ||
      (a === 169 && b === 254) ||
      (a === 172 && b >= 16 && b <= 31) ||
      (a === 192 && (b === 168 || b === 0 || (b === 88 && c === 99))) ||
      (a === 198 && (b === 18 || b === 19 || (b === 51 && c === 100))) ||
      (a === 203 && b === 0 && c === 113)
    );
  }
  if (isIP(address) === 6) {
    const [first, second] = address
      .split(":")
      .slice(0, 2)
      .map((part) => Number.parseInt(part || "0", 16));
    // Only global unicast; exclude transition/tunnel and documentation ranges.
    return (
      first >= 0x2000 &&
      first < 0x4000 &&
      !(first === 0x2001 && (second < 0x200 || second === 0xdb8)) &&
      ![0x2002, 0x3ffe, 0x3fff].includes(first)
    );
  }
  return false;
}

export function validateWebUrl(value) {
  let url;
  try {
    url = new URL(value);
  } catch {
    throw notAllowed("Invalid web URL");
  }
  const host = url.hostname
    .toLowerCase()
    .replace(/^\[|\]$/g, "")
    .replace(/\.$/, "");
  if (url.protocol !== "https:") throw notAllowed("Web URL must use HTTPS");
  if (url.username || url.password)
    throw notAllowed("Web URL credentials are forbidden");
  if (url.port && url.port !== "443")
    throw notAllowed("Web URL port must be 443");
  if (
    BLOCKED_HOSTS.has(host) ||
    /(?:^|\.)(?:localhost|local|internal|invalid|test)$/.test(host) ||
    (!host.includes(":") && !host.includes("."))
  )
    throw notAllowed("Web host is not allowed");
  if (isIP(host) && !isPublicAddress(host))
    throw notAllowed("Web URL address is not public");
  url.hash = "";
  return url;
}

function normalizedAddress(value) {
  const address = value.toLowerCase();
  return address.startsWith("::ffff:") && isIP(address.slice(7)) === 4
    ? address.slice(7)
    : address;
}

async function pinnedRequest(url, pinned, signal) {
  return await new Promise((resolve, reject) => {
    const request = https.request(
      url,
      {
        method: "GET",
        signal,
        agent: false,
        rejectUnauthorized: true,
        maxHeaderSize: 16 * 1024,
        headers: {
          Accept: "text/html,text/plain;q=0.9",
          "Accept-Encoding": "identity",
          "User-Agent": "OfoxDesktop-WebRead/1",
        },
        lookup: (_hostname, options, callback) =>
          callback(
            null,
            options.all ? [pinned] : pinned.address,
            ...(options.all ? [] : [pinned.family]),
          ),
      },
      (response) => {
        if (
          !response.socket.remoteAddress ||
          normalizedAddress(response.socket.remoteAddress) !==
            normalizedAddress(pinned.address)
        ) {
          response.destroy();
          reject(
            notAllowed("Connected web address differs from pinned DNS result"),
          );
          return;
        }
        const chunks = [];
        let length = 0;
        response.on("data", (chunk) => {
          length += chunk.length;
          if (length > MAX_PAGE_BYTES) response.destroy(tooLarge());
          else chunks.push(chunk);
        });
        response.on("end", () =>
          resolve({
            status: response.statusCode,
            headers: response.headers,
            body: Buffer.concat(chunks),
          }),
        );
        response.on("error", reject);
      },
    );
    request.on("error", reject);
    request.end();
  });
}

export function htmlText(html) {
  return html
    .replace(
      /<(script|style|noscript|template)\b[^>]*>[\s\S]*?<\/\1\s*>/gi,
      " ",
    )
    .replace(/<!--[^]*?-->/g, " ")
    .replace(/<\/?(?:p|div|h[1-6]|li|tr|br|section|article)\b[^>]*>/gi, "\n")
    .replace(/<[^>]*>/g, " ")
    .replace(/&#(?:x([0-9a-f]+)|(\d+));/gi, (_match, hex, decimal) => {
      const code = Number.parseInt(hex || decimal, hex ? 16 : 10);
      return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : " ";
    })
    .replace(
      /&(?:amp|lt|gt|quot|apos|nbsp);/g,
      (entity) =>
        ({
          "&amp;": "&",
          "&lt;": "<",
          "&gt;": ">",
          "&quot;": '"',
          "&apos;": "'",
          "&nbsp;": " ",
        })[entity],
    )
    .replace(/[ \t]+/g, " ")
    .replace(/\n\s*\n+/g, "\n\n")
    .trim();
}

async function follow(start, { signal, secrets, lookup, request }) {
  let url = start;
  for (let redirects = 0; ; redirects++) {
    signal.throwIfAborted();
    const host = url.hostname.replace(/^\[|\]$/g, "");
    const addresses = isIP(host)
      ? [{ address: host, family: isIP(host) }]
      : await lookup(host, { all: true, verbatim: true });
    if (
      !addresses.length ||
      addresses.some((item) => !isPublicAddress(item.address))
    )
      throw notAllowed("Web DNS contains a non-public address");
    const response = await request(url, addresses[0], signal);
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      if (redirects === MAX_REDIRECTS)
        throw new FetchError("fetch_failed", "Web redirect limit exceeded");
      if (!response.headers.location)
        throw new FetchError("fetch_failed", "Web redirect has no location");
      url = validateWebUrl(new URL(response.headers.location, url).href);
      continue;
    }
    if (response.status < 200 || response.status >= 300)
      throw new FetchError(
        "fetch_failed",
        `Web page returned HTTP ${response.status}`,
        response.status,
      );
    if (response.body.byteLength > MAX_PAGE_BYTES) throw tooLarge();
    const mime = String(response.headers["content-type"] ?? "")
      .split(";")[0]
      .trim()
      .toLowerCase();
    if (!["text/html", "text/plain", "application/xhtml+xml"].includes(mime))
      throw new FetchError(
        "fetch_unsupported_content",
        "Web page must be static HTML or plain text",
      );
    const decoded = new TextDecoder().decode(response.body);
    // Mask the key before the text is cut, so no fragment of it survives.
    const text = scrubSecrets(
      mime === "text/plain" ? decoded.trim() : htmlText(decoded),
      secrets,
    );
    return {
      url: url.href,
      text: text.slice(0, MAX_PAGE_CHARS),
      truncated: text.length > MAX_PAGE_CHARS,
    };
  }
}

export async function readWebPage(
  value,
  {
    signal,
    secrets = [],
    timeoutMs = FETCH_TIMEOUT_MS,
    lookup = dnsLookup,
    request = pinnedRequest,
  } = {},
) {
  const url = validateWebUrl(value);
  const deadline = AbortSignal.timeout(timeoutMs);
  const combined = signal ? AbortSignal.any([signal, deadline]) : deadline;
  let onAbort;
  const aborted = new Promise((_resolve, reject) => {
    onAbort = () =>
      reject(
        new FetchError("fetch_failed", "Web request cancelled or timed out"),
      );
    if (combined.aborted) onAbort();
    else combined.addEventListener("abort", onAbort, { once: true });
  });
  try {
    return await Promise.race([
      aborted,
      follow(url, { signal: combined, secrets, lookup, request }),
    ]);
  } catch (error) {
    if (error instanceof FetchError) throw error;
    const code = typeof error?.code === "string" ? ` (${error.code})` : "";
    throw new FetchError("fetch_failed", `Web request failed${code}`);
  } finally {
    combined.removeEventListener("abort", onAbort);
  }
}
