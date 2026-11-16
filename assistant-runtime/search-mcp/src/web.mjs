// Read one public web page as Markdown (HTML) or text, with the HTTPS pages it
// links to. Ported from the reference runtime's
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

// Where browsers look for an HTML page's <meta> charset.
const META_SCAN_BYTES = 1024;

function charsetLabel(body, contentType, html) {
  if (body[0] === 0xef && body[1] === 0xbb && body[2] === 0xbf) return "utf-8";
  if (body[0] === 0xfe && body[1] === 0xff) return "utf-16be";
  if (body[0] === 0xff && body[1] === 0xfe) return "utf-16le";
  const header = /;\s*charset\s*=\s*["']?([^"';\s]+)/i.exec(contentType);
  if (header) return header[1];
  if (!html) return undefined;
  // `<meta charset=…>` and `<meta http-equiv="Content-Type" content="…;
  // charset=…">` alike. A page that names UTF-16 here was read as ASCII, so
  // it is not UTF-16.
  const meta = /<meta\b[^>]*?\bcharset\s*=\s*["']?\s*([^\s"'/>;]+)/i.exec(
    body.subarray(0, META_SCAN_BYTES).toString("latin1"),
  )?.[1];
  return meta && /^utf-?16/i.test(meta) ? "utf-8" : meta;
}

/**
 * A page's text in its own encoding: a byte order mark, else the
 * Content-Type charset, else (HTML) a meta charset in the first 1024 bytes.
 * No label, or one TextDecoder does not know, is UTF-8.
 */
function decodePage(body, contentType, html) {
  const label = charsetLabel(body, contentType, html) ?? "utf-8";
  let decoder;
  try {
    decoder = new TextDecoder(label);
  } catch {
    decoder = new TextDecoder();
  }
  return decoder.decode(body);
}

// Elements whose content is never page text, and those that stand apart as
// paragraphs.
const HIDDEN_ELEMENTS = new Set([
  ..."script style noscript template svg iframe".split(" "),
]);
const BLOCK_ELEMENTS = new Set([
  ..."p div section article header footer nav main aside form fieldset figure figcaption address details summary dl dt dd blockquote title".split(
    " ",
  ),
]);
const TAG_NAME = /[a-z][a-z0-9-]*/y;
const HREF = /\shref\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))/i;
// The page links a read hands back, at most this many and this long.
export const MAX_PAGE_LINKS = 500;
const MAX_LINK_CHARS = 2048;
const MAX_LIST_INDENT = 6;
const MAX_TABLE_COLUMNS = 64;
// A list item's indentation, which collapsing the page's own spaces must
// keep. Page text never contains NUL: it is dropped as the text is read.
const INDENT = "\0";
// Heading and list marks, and spaces, a link's text starts with: they stay
// in front of the link.
const LINK_LEAD = /^(?:[\s\0]|#{1,6} |- |1\. )+/;

const NAMED_ENTITIES = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: " ",
  mdash: "—",
  ndash: "–",
  hellip: "…",
  copy: "©",
  reg: "®",
  trade: "™",
  lsquo: "‘",
  rsquo: "’",
  ldquo: "“",
  rdquo: "”",
  laquo: "«",
  raquo: "»",
  middot: "·",
  bull: "•",
  times: "×",
};
const ENTITY = new RegExp(
  `&(?:#(?:x([0-9a-f]+)|(\\d+))|(${Object.keys(NAMED_ENTITIES).join("|")}));`,
  "gi",
);

function decodeEntities(text) {
  return text.replace(ENTITY, (match, hex, decimal, name) => {
    if (name)
      return (
        NAMED_ENTITIES[name] ?? NAMED_ENTITIES[name.toLowerCase()] ?? match
      );
    const code = Number.parseInt(hex || decimal, hex ? 16 : 10);
    return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : " ";
  });
}

// A link's page as the Markdown shows it: absolute, HTTP(S), without its
// fragment, and not the page itself. Parentheses are percent-encoded so the
// URL cannot end the Markdown link early.
function linkTarget(tag, base) {
  const match = HREF.exec(tag);
  const raw = match && decodeEntities(match[1] ?? match[2] ?? match[3]).trim();
  if (!raw || raw.startsWith("#") || raw.length > MAX_LINK_CHARS) return null;
  let url;
  try {
    url = new URL(raw, base);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  url.hash = "";
  if (url.href === base) return null;
  const href = url.href.replace(/\(/g, "%28").replace(/\)/g, "%29");
  return href.length > MAX_LINK_CHARS ? null : href;
}

function fence(code) {
  const body = code.replace(/^\n/, "").trimEnd();
  if (!body.trim()) return "";
  const marks = body.includes("```") ? "````" : "```";
  return `${marks}\n${body}\n${marks}`;
}

const normalize = (text) =>
  text
    .replace(/ {2,}/g, " ")
    .replace(/ *\n */g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim()
    .replaceAll(INDENT, "  ");

/**
 * An HTML page as Markdown, in one pass: headings, paragraphs, links
 * (resolved against `baseUrl`), lists, tables, code blocks and inline code;
 * images and hidden elements are dropped. `links` are the HTTPS pages the
 * Markdown links to, in order, each once, at most MAX_PAGE_LINKS.
 *
 * A tag ends at its first ">"; a "<" that starts no tag, comment or
 * declaration is text. As in a browser, an unclosed comment, tag or hidden
 * element hides the rest of the page. Every search for an end resumes past
 * the last one's start, so each costs one scan of the page in all, and no
 * work depends on how deeply elements nest: a page of unclosed or nested
 * tags cannot stall the event loop (and with it the read's deadline).
 */
export function htmlMarkdown(html, baseUrl) {
  let base;
  try {
    const url = new URL(baseUrl);
    url.hash = "";
    base = url.href;
  } catch {
    base = undefined;
  }
  // ASCII only, so every index is the same in both strings.
  const lower = html.replace(/[A-Z]+/g, (letters) => letters.toLowerCase());
  const searches = new Map();
  const next = (mark, from) => {
    const last = searches.get(mark);
    if (last && from >= last.from && (last.at === -1 || last.at >= from))
      return last.at;
    const at = lower.indexOf(mark, from);
    searches.set(mark, { from, at });
    return at;
  };
  // A hidden element's end tag: `</name` then a space, "/" or ">".
  const endTag = (name, from) => {
    let at = next(`</${name}`, from);
    while (at !== -1 && !/[\s/>]/.test(lower[at + name.length + 2] ?? ">"))
      at = next(`</${name}`, at + 1);
    return at;
  };

  const pieces = []; // finished blocks: Markdown text and code blocks
  let chunks = []; // the Markdown text since the last code block
  let code = null; // the open <pre>'s text, verbatim
  let preDepth = 0;
  const lists = [];
  let link = null;
  let table = null;
  let cell = false;
  const links = [];
  const linked = new Set();

  const text = (raw) => {
    const decoded = decodeEntities(raw.replace(/\0/g, ""));
    if (code) code.push(decoded.replace(/\r\n?/g, "\n"));
    else chunks.push(decoded.replace(/\s+/g, " "));
  };
  // A table cell stays on its row.
  const lineBreak = (blank) => {
    if (!code) chunks.push(cell ? " " : blank ? "\n\n" : "\n");
  };
  const closeLink = () => {
    if (!link) return;
    const { href, start } = link;
    link = null;
    const inner = chunks.splice(start).join("");
    const lead = LINK_LEAD.exec(inner)?.[0] ?? "";
    const label = inner
      .slice(lead.length)
      .replace(/[\s\0]+/g, " ")
      .trim();
    chunks.push(lead);
    if (label) chunks.push(`[${label.replace(/[[\]]/g, "\\$&")}](${href})`);
    chunks.push(inner.slice(inner.trimEnd().length));
  };
  const openLink = (tag) => {
    closeLink();
    const href = linkTarget(tag, base);
    if (!href) return;
    link = { href, start: chunks.length };
    if (
      href.startsWith("https:") &&
      links.length < MAX_PAGE_LINKS &&
      !linked.has(href)
    ) {
      linked.add(href);
      links.push(href);
    }
  };
  const flush = () => {
    closeLink();
    const block = normalize(chunks.join(""));
    if (block) pieces.push(block);
    chunks = [];
  };
  const closeCell = () => {
    if (!cell) return;
    cell = false;
    chunks.push(" |");
    table.cells++;
  };
  // The row of a table's first row is its header.
  const closeRow = () => {
    closeCell();
    if (!table?.inRow) return;
    table.inRow = false;
    if (table.rows++ === 0 && table.cells)
      chunks.push(
        `\n|${" --- |".repeat(Math.min(table.cells, MAX_TABLE_COLUMNS))}`,
      );
    table.cells = 0;
  };
  const openRow = () => {
    table ??= { rows: 0, cells: 0, inRow: false };
    closeRow();
    lineBreak(false);
    chunks.push("|");
    table.inRow = true;
  };

  const onTag = (name, closing, tag) => {
    if (name === "pre") {
      if (!closing) {
        if (!code) {
          flush();
          code = [];
        }
        preDepth++;
      } else if (code && --preDepth === 0) {
        const block = fence(code.join(""));
        if (block) pieces.push(block);
        code = null;
      }
      return;
    }
    if (code) {
      if (name === "br") code.push("\n");
      return;
    }
    if (name && /^h[1-6]$/.test(name)) {
      lineBreak(true);
      if (!closing && !cell) chunks.push(`${"#".repeat(Number(name[1]))} `);
      return;
    }
    switch (name) {
      case "a":
        if (closing) closeLink();
        else openLink(tag);
        return;
      case "br":
        lineBreak(false);
        return;
      case "hr":
        lineBreak(true);
        if (!cell) chunks.push("---");
        lineBreak(true);
        return;
      case "code":
        chunks.push("`");
        return;
      case "ul":
      case "ol":
        if (closing) {
          lists.pop();
          if (!lists.length) lineBreak(true);
        } else {
          lists.push(name);
          if (lists.length === 1) lineBreak(true);
        }
        return;
      case "li":
        if (closing) return;
        lineBreak(false);
        if (!cell)
          chunks.push(
            INDENT.repeat(
              Math.min(Math.max(lists.length - 1, 0), MAX_LIST_INDENT),
            ) + (lists.at(-1) === "ol" ? "1. " : "- "),
          );
        return;
      case "table":
        if (table) closeRow();
        table = closing ? null : { rows: 0, cells: 0, inRow: false };
        lineBreak(true);
        return;
      case "tr":
        if (closing) {
          if (table) closeRow();
        } else openRow();
        return;
      case "td":
      case "th":
        if (closing) {
          if (table) closeCell();
          return;
        }
        if (!table?.inRow) openRow();
        closeCell();
        cell = true;
        chunks.push(" ");
        return;
    }
    if (BLOCK_ELEMENTS.has(name)) lineBreak(true);
    else chunks.push(" ");
  };

  let at = 0;
  while (at < html.length) {
    const open = html.indexOf("<", at);
    if (open === -1) {
      text(html.slice(at));
      break;
    }
    text(html.slice(at, open));
    if (lower.startsWith("<!--", open)) {
      const end = next("-->", open + 4);
      if (end === -1) break;
      if (!code) chunks.push(" ");
      at = end + 3;
      continue;
    }
    const after = html[open + 1];
    TAG_NAME.lastIndex = open + (after === "/" ? 2 : 1);
    const name = TAG_NAME.exec(lower)?.[0];
    // `<!DOCTYPE …>`, `<?…>` and `</…>` without a name end at ">" too.
    if (!name && after !== "!" && after !== "?" && after !== "/") {
      text("<");
      at = open + 1;
      continue;
    }
    const end = next(">", open + 1);
    if (end === -1) break;
    at = end + 1;
    if (after !== "/" && HIDDEN_ELEMENTS.has(name)) {
      const close = endTag(name, at);
      const closeEnd = close === -1 ? -1 : next(">", close);
      if (closeEnd === -1) break;
      at = closeEnd + 1;
      if (!code) chunks.push(" ");
      continue;
    }
    onTag(
      name,
      after === "/",
      name === "a" && after !== "/" ? html.slice(open, end + 1) : "",
    );
  }
  if (code) {
    const block = fence(code.join(""));
    if (block) pieces.push(block);
    code = null;
  }
  if (table) closeRow();
  flush();
  return { text: pieces.join("\n\n"), links };
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
    const decoded = decodePage(
      response.body,
      String(response.headers["content-type"] ?? ""),
      mime !== "text/plain",
    );
    const read =
      mime === "text/plain"
        ? { text: decoded.trim(), links: [] }
        : htmlMarkdown(decoded, url.href);
    // Mask the key before the text is cut, so no fragment of it survives.
    const text = scrubSecrets(read.text, secrets);
    return {
      url: url.href,
      text: text.slice(0, MAX_PAGE_CHARS),
      truncated: text.length > MAX_PAGE_CHARS,
      links: read.links,
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
