// Free web search and page reads through Exa's hosted MCP server, without a
// key: one JSON-RPC `tools/call` per request, no session. EXA_URL is a fixed
// third-party infrastructure URL, not an Ofox one: the queries and the URLs
// read go to Exa. Keyless use needs a direct IPv4 connection (Exa answers a
// proxy or IPv6 with 403) and is rate-limited per IP (429). Nothing here is
// billed, so a failure never leaves a charge unknown, and nothing retries.
import { scrubSecrets } from "./redact.mjs";
import { SearchError, queryHasUrl, readBounded } from "./search.mjs";
import { validateWebUrl } from "./web.mjs";

export const EXA_URL = "https://mcp.exa.ai/mcp";
export const EXA_SEARCH_TIMEOUT_MS = 25_000;
// Exa reads a page only after reading it here failed (at most 15 s), within
// the runtime's 30 s for one web_fetch.
export const EXA_READ_TIMEOUT_MS = 10_000;
export const EXA_MODEL = "exa";
const MAX_RESPONSE_BYTES = 1024 * 1024;
const NUM_RESULTS = 8;
const MAX_TITLE_CHARS = 200;
const MAX_EXCERPT_CHARS = 1000;
const MAX_URL_CHARS = 2048;
const MAX_PAGE_CHARS = 20_000;
const PUBLISHED = /\b\d{4}-\d{2}-\d{2}(?!\d)/;
const HEADERS = Object.freeze({
  "Content-Type": "application/json",
  Accept: "application/json, text/event-stream",
  // Attribution only: Exa passes it on as the integration's name.
  "x-exa-source": "ofox-desktop",
});

/** Why Exa did not answer: `status` is its HTTP status, when it sent one. */
class ExaFailure extends Error {
  constructor(message, status) {
    super(message);
    this.name = "ExaFailure";
    if (status !== undefined) this.status = status;
  }
}

// The JSON-RPC answer in a JSON body, or in the last `data:` line of an SSE
// one.
function rpcAnswer(text, contentType) {
  if (contentType.includes("text/event-stream")) {
    const lines = text
      .split(/\r?\n/)
      .filter((line) => line.startsWith("data:"));
    text = lines.at(-1)?.slice(5) ?? "";
  }
  return JSON.parse(text);
}

/** The text of a successful Exa tool call; ExaFailure otherwise. */
async function exaCall(
  name,
  args,
  { fetchImpl = fetch, url = EXA_URL, signal, timeoutMs },
) {
  const deadline = AbortSignal.timeout(timeoutMs);
  const combined = signal ? AbortSignal.any([signal, deadline]) : deadline;
  let response;
  try {
    response = await fetchImpl(url, {
      method: "POST",
      headers: HEADERS,
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: { name, arguments: args },
      }),
      redirect: "manual",
      signal: combined,
    });
  } catch {
    throw new ExaFailure(
      deadline.aborted
        ? "Exa did not answer in time"
        : combined.aborted
          ? "the request was cancelled"
          : "could not reach Exa",
    );
  }
  if (response.status !== 200) {
    await response.body?.cancel().catch(() => {});
    throw new ExaFailure(
      `Exa answered HTTP ${response.status}`,
      response.status,
    );
  }
  let answer;
  try {
    const bytes = await readBounded(response, MAX_RESPONSE_BYTES);
    if (bytes === null) throw new Error("too large");
    answer = rpcAnswer(
      bytes.toString("utf8"),
      response.headers.get("content-type") ?? "",
    );
  } catch {
    throw new ExaFailure(
      combined.aborted
        ? "Exa did not answer in time"
        : "Exa's answer was unreadable",
    );
  }
  const result = answer?.result;
  const text = Array.isArray(result?.content)
    ? result.content.find((part) => typeof part?.text === "string")?.text
    : undefined;
  if (answer?.error || result?.isError || typeof text !== "string")
    throw new ExaFailure("Exa could not complete the request");
  return text;
}

function searchFailure(error) {
  if (error.status === 429)
    return new SearchError(
      "search_rate_limited",
      "Exa's free web search is rate-limited for this network right now " +
        "(HTTP 429); no search was made. Try again later, or use Ofox search.",
      429,
    );
  if (error.status === 403)
    return new SearchError(
      "search_blocked",
      "Exa refused a free web search from this network (HTTP 403): it needs " +
        "a direct IPv4 connection, not a proxy or IPv6. No search was made; " +
        "use Ofox search instead.",
      403,
    );
  return new SearchError(
    "search_failed",
    `The free web search failed: ${error.message}.`,
    error.status,
  );
}

const oneLine = (value) => value.replace(/\s+/g, " ").trim();

function exaUrl(value) {
  if (value.length > MAX_URL_CHARS) return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:" ? value : null;
  } catch {
    return null;
  }
}

/**
 * Exa's results text: blocks separated by a `---` line before the next
 * `Title:` (a page's own Markdown has `---` lines too), each with `Title:`,
 * `URL:`, `Published:` and the page's highlights (or text) after their label.
 */
export function parseExaResults(text) {
  const results = [];
  const seen = new Set();
  for (const block of text.split(/\n[ \t]*---[ \t]*\n\s*(?=Title:)/)) {
    const url = exaUrl(/^URL:[ \t]*(\S+)[ \t]*$/m.exec(block)?.[1] ?? "");
    if (!url || seen.has(url)) continue;
    seen.add(url);
    const title = oneLine(/^Title:[ \t]*(.*)$/m.exec(block)?.[1] ?? "");
    const published = PUBLISHED.exec(
      /^Published:[ \t]*(.*)$/m.exec(block)?.[1] ?? "",
    )?.[0];
    const label = /^(?:Highlights|Text|Summary):[ \t]*$/m.exec(block);
    const excerpt = label
      ? oneLine(block.slice(label.index + label[0].length)).slice(
          0,
          MAX_EXCERPT_CHARS,
        )
      : "";
    results.push({
      title: (title && title !== "N/A" ? title : new URL(url).hostname)
        .replace(/^www\./, "")
        .slice(0, MAX_TITLE_CHARS),
      url,
      ...(excerpt ? { snippet: excerpt } : {}),
      ...(published ? { published } : {}),
    });
  }
  return results;
}

/** A free search: the same result shape as webSearch, at no cost. */
export async function exaSearch(
  query,
  config,
  {
    signal,
    fetchImpl,
    url = config.exaUrl,
    timeoutMs = EXA_SEARCH_TIMEOUT_MS,
  } = {},
) {
  if (queryHasUrl(query))
    throw new SearchError(
      "search_rejected",
      "Search queries must not contain URLs, www. addresses or web " +
        "addresses with a path or query string; no search was made. Search " +
        "with words, a bare domain (example.com) or site:example.com instead.",
    );
  let text;
  try {
    text = await exaCall(
      "web_search_exa",
      { query, numResults: NUM_RESULTS },
      { fetchImpl, url, signal, timeoutMs },
    );
  } catch (error) {
    if (error instanceof ExaFailure) throw searchFailure(error);
    throw error;
  }
  return {
    summary: "",
    results: parseExaResults(scrubSecrets(text, [config.apiKey])),
    usage: {
      searchCalls: 0,
      inputTokens: 0,
      outputTokens: 0,
      model: EXA_MODEL,
    },
  };
}

// Markdown links to HTTPS pages, each once.
function markdownLinks(text) {
  const links = [];
  const seen = new Set();
  for (const match of text.matchAll(/\]\((https:\/\/[^\s)]{1,2048})\)/g))
    if (!seen.has(match[1])) {
      seen.add(match[1]);
      links.push(match[1]);
    }
  return links;
}

/**
 * One page through Exa, as readWebPage returns it, for a page that could not
 * be read here. The URL must pass the same rules as a read here (HTTPS,
 * public host, no credentials) before it is sent. Throws on any failure.
 */
export async function exaReadPage(
  value,
  {
    signal,
    secrets = [],
    fetchImpl,
    url = EXA_URL,
    timeoutMs = EXA_READ_TIMEOUT_MS,
  } = {},
) {
  const page = validateWebUrl(value).href;
  const text = scrubSecrets(
    (
      await exaCall(
        "web_fetch_exa",
        { urls: [page], maxCharacters: MAX_PAGE_CHARS },
        { fetchImpl, url, signal, timeoutMs },
      )
    ).trim(),
    secrets,
  );
  if (!text) throw new ExaFailure("Exa returned no text");
  return {
    url: page,
    text: text.slice(0, MAX_PAGE_CHARS),
    truncated: text.length >= MAX_PAGE_CHARS,
    links: markdownLinks(text),
  };
}
