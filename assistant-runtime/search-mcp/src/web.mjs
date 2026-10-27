// Read one public web page as text. Ported from the reference runtime's
// web.mjs: HTTPS on 443 only, every DNS answer must be public, the connection
// is pinned to the checked address, every redirect is re-checked, and the
// 15 s / 2 MiB / 20,000-character limits hold. Only static HTML or plain text
// is read; no cookies are sent or kept. Through the app's proxy the DNS
// answers are still checked here, but the tunnel opens by host name, so the
// connection cannot be pinned; the certificate is verified for that name.
//
// Fake-IP DNS: a Clash/Surge-style TUN answers every name with an address in
// 198.18.0.0/15 and resolves the real host itself. When every local answer is
// such an address, the host is checked with public DNS over HTTPS instead
// (every answer must be public), and the page is read from the first checked
// public address, pinned, with SNI and Host the name and the certificate
// verified for it; through the app's proxy the tunnel goes to that address,
// not to the name. Never to the fake-IP: the TUN would resolve the name again
// itself, and a host can answer its resolver with a private address.
//
// The "only URLs from the user or this conversation's search" allowlist is
// not here: the client wrapper that exposes this tool to the model owns it.
import { lookup as dnsLookup } from "node:dns/promises";
import https from "node:https";
import { isIP } from "node:net";
import { ProxyAgent, request as undiciRequest } from "undici";
import { bypassesProxy, proxyRefused } from "./proxy.mjs";
import { scrubSecrets } from "./redact.mjs";
import { readBounded } from "./search.mjs";

export const FETCH_TIMEOUT_MS = 15_000;
export const MAX_PAGE_BYTES = 2 * 1024 * 1024;
export const MAX_PAGE_CHARS = 20_000;
const MAX_REDIRECTS = 4;
// A fixed public resolver, not an Ofox URL: Alibaba public DNS, JSON API.
export const DOH_URL = "https://dns.alidns.com/resolve";
export const DOH_TIMEOUT_MS = 5_000;
const MAX_DOH_BYTES = 64 * 1024;
const DOH_RECORDS = [
  { name: "A", type: 1, family: 4 },
  { name: "AAAA", type: 28, family: 6 },
];
const DOH_NXDOMAIN = 3;
const FAKE_IP_DNS = "Web DNS gave only fake-IP addresses (198.18.0.0/15) and";
// Frozen, and copied for every request: undici's ProxyAgent adds the Host of
// a tunnelled request to the headers object it is given.
const REQUEST_HEADERS = Object.freeze({
  Accept: "text/html,text/plain;q=0.9",
  "Accept-Encoding": "identity",
  "User-Agent": "OfoxDesktop-WebRead/1",
});
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

/** An address of the 198.18.0.0/15 range that fake-IP DNS hands out. */
export function isFakeIpAddress(address) {
  if (isIP(address) !== 4) return false;
  const [a, b] = address.split(".").map(Number);
  return a === 198 && (b === 18 || b === 19);
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

async function directRequest(url, pinned, signal, requestTls) {
  return await new Promise((resolve, reject) => {
    const request = https.request(
      url,
      {
        ...requestTls,
        method: "GET",
        signal,
        agent: false,
        rejectUnauthorized: true,
        maxHeaderSize: 16 * 1024,
        headers: { ...REQUEST_HEADERS },
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

/**
 * A direct request to `pinned`, the checked address, with SNI and Host the
 * URL's name and the certificate verified for it. `requestTls` adds TLS
 * options (tests trust a local certificate with it).
 */
export function pinnedRequest({ requestTls } = {}) {
  return (url, pinned, signal) =>
    directRequest(url, pinned, signal, requestTls);
}

async function tunneled(target, { dispatcher, signal, headers }) {
  let response;
  try {
    response = await undiciRequest(target, {
      method: "GET",
      dispatcher,
      signal,
      headers,
    });
  } catch (error) {
    if (proxyRefused(error))
      throw new FetchError(
        "fetch_failed",
        "The proxy refused or failed the connection to the web page",
      );
    throw error;
  }
  const chunks = [];
  let length = 0;
  for await (const chunk of response.body) {
    length += chunk.length;
    if (length > MAX_PAGE_BYTES) {
      response.body.destroy();
      throw tooLarge();
    }
    chunks.push(chunk);
  }
  return {
    status: response.statusCode,
    headers: response.headers,
    body: Buffer.concat(chunks),
  };
}

/**
 * A request through the app's proxy: a CONNECT (or SOCKS5) tunnel to the
 * host name, TLS verified for that name. The caller checked its DNS answers.
 * With `toAddress` (fake-IP DNS) the tunnel goes to `pinned`, the checked
 * address, instead, so the proxy cannot resolve the name to another one; SNI
 * and Host stay the name. `requestTls` adds TLS options for the page
 * connection (tests trust a local certificate with it).
 */
export function tunneledRequest(proxy, { requestTls } = {}) {
  const options = {
    uri: proxy.url,
    proxyTunnel: true,
    maxHeaderSize: 16 * 1024,
  };
  const dispatcher = new ProxyAgent({
    ...options,
    ...(requestTls ? { requestTls } : {}),
  });
  return async (url, pinned, signal, { toAddress = false } = {}) => {
    if (!toAddress)
      return await tunneled(url, {
        dispatcher,
        signal,
        headers: { ...REQUEST_HEADERS },
      });
    const target = new URL(url);
    target.hostname =
      pinned.family === 6 ? `[${pinned.address}]` : pinned.address;
    // The TLS name is per agent: one for this request only.
    const byAddress = new ProxyAgent({
      ...options,
      requestTls: { ...requestTls, servername: url.hostname },
    });
    try {
      return await tunneled(target, {
        dispatcher: byAddress,
        signal,
        headers: { ...REQUEST_HEADERS, host: url.host },
      });
    } finally {
      byAddress.destroy().catch(() => {});
    }
  };
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

// One record type's addresses from a DNS JSON answer; null when the query
// failed or the answer is malformed. NXDOMAIN is a name without addresses.
async function dohQuery(host, record, { fetchImpl, url, signal }) {
  const target = new URL(url);
  target.searchParams.set("name", host);
  target.searchParams.set("type", record.name);
  const response = await fetchImpl(target, {
    headers: { Accept: "application/dns-json, application/json" },
    redirect: "manual",
    signal,
  });
  if (response.status !== 200) {
    await response.body?.cancel().catch(() => {});
    return null;
  }
  const bytes = await readBounded(response, MAX_DOH_BYTES);
  if (bytes === null) return null;
  let body;
  try {
    body = JSON.parse(bytes.toString("utf8"));
  } catch {
    return null;
  }
  if (body?.Status === DOH_NXDOMAIN) return [];
  if (body?.Status !== 0) return null;
  const answers = body.Answer ?? [];
  if (!Array.isArray(answers)) return null;
  const addresses = [];
  // CNAMEs and other records are skipped; every address record must be one.
  for (const answer of answers) {
    if (answer?.type !== record.type) continue;
    if (typeof answer.data !== "string" || isIP(answer.data) !== record.family)
      return null;
    addresses.push({ address: answer.data, family: record.family });
  }
  return addresses;
}

/**
 * The host's A and AAAA addresses from public DNS over HTTPS, at most
 * `timeoutMs` and 64 KiB per answer. `fetchImpl` is the route web_search
 * takes (the app's proxy, or a direct connection). Any failure refuses.
 */
export async function dohLookup(
  host,
  { fetchImpl = fetch, url = DOH_URL, timeoutMs = DOH_TIMEOUT_MS, signal } = {},
) {
  const done = new AbortController();
  const signals = [done.signal, AbortSignal.timeout(timeoutMs)];
  if (signal) signals.push(signal);
  const options = { fetchImpl, url, signal: AbortSignal.any(signals) };
  let answers;
  try {
    answers = await Promise.all(
      DOH_RECORDS.map((record) => dohQuery(host, record, options)),
    );
  } catch {
    answers = [null];
  } finally {
    // The other query, when one failed.
    done.abort();
  }
  if (answers.includes(null))
    throw notAllowed(`${FAKE_IP_DNS} the DNS-over-HTTPS check failed`);
  return answers.flat();
}

// The checked addresses a hop may connect to: the local answers when all are
// public; when all are fake-IP addresses, public DNS's answers instead.
async function checkedAddresses(host, local, { doh, signal }) {
  if (
    !isIP(host) &&
    local.length &&
    local.every((item) => isFakeIpAddress(item.address))
  ) {
    const resolved = await dohLookup(host, { ...doh, signal });
    if (!resolved.length)
      throw notAllowed(
        `${FAKE_IP_DNS} DNS-over-HTTPS has no address for the host`,
      );
    if (resolved.some((item) => !isPublicAddress(item.address)))
      throw notAllowed(
        `${FAKE_IP_DNS} DNS-over-HTTPS contains a non-public address`,
      );
    return { addresses: resolved, fakeIp: true };
  }
  if (!local.length || local.some((item) => !isPublicAddress(item.address)))
    throw notAllowed("Web DNS contains a non-public address");
  return { addresses: local, fakeIp: false };
}

async function follow(start, { signal, secrets, lookup, request, proxy, doh }) {
  let url = start;
  for (let redirects = 0; ; redirects++) {
    signal.throwIfAborted();
    const host = url.hostname.replace(/^\[|\]$/g, "");
    const local = isIP(host)
      ? [{ address: host, family: isIP(host) }]
      : await lookup(host, { all: true, verbatim: true });
    const { addresses, fakeIp } = await checkedAddresses(host, local, {
      doh,
      signal,
    });
    // One attempt at the first checked address; a failure is final.
    const response =
      proxy && !bypassesProxy(url, proxy.noProxy)
        ? await proxy.request(url, addresses[0], signal, { toAddress: fakeIp })
        : await request(url, addresses[0], signal);
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
    request = pinnedRequest(),
    proxy = null,
    doh = {},
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
      follow(url, { signal: combined, secrets, lookup, request, proxy, doh }),
    ]);
  } catch (error) {
    if (error instanceof FetchError) throw error;
    const code = typeof error?.code === "string" ? ` (${error.code})` : "";
    throw new FetchError("fetch_failed", `Web request failed${code}`);
  } finally {
    combined.removeEventListener("abort", onAbort);
  }
}
