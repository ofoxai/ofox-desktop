import { proxyRefused } from "./proxy.mjs";
import { scrubSecrets } from "./redact.mjs";

// One paid web search through the Ofox Responses API.
//
// The gateway bills every `web_search_call` output item (`open_page` included)
// at the model's web_search price. `max_tool_calls: 1` limits a request to one
// billed search, while `include` still returns the 15-20 source URLs that
// search saw. Nothing here retries: a repeated request is a repeated charge.

export const SEARCH_TIMEOUT_MS = 60_000;
export const MAX_RESPONSE_BYTES = 1024 * 1024;
const MAX_ERROR_BODY_BYTES = 64 * 1024;
const MAX_RESULTS = 25;
const MAX_TITLE_CHARS = 200;
const MAX_SNIPPET_CHARS = 300;
const MAX_URL_CHARS = 2048;
const MAX_SUMMARY_CHARS = 8000;

const INSTRUCTION =
  "Search the web once for the query below. Answer with a short factual " +
  "summary (under 150 words) of the most relevant and current findings and " +
  "cite the pages you used. Do not ask follow-up questions.\n\nQuery: ";

// Errors of connect() or the DNS lookup: no request byte can have left the
// machine. The same codes on an open socket (syscall read/write) can follow a
// sent request, so the syscall must match too.
const NOT_SENT_CODES = new Set([
  "ECONNREFUSED",
  "ENOTFOUND",
  "EAI_AGAIN",
  "ENETUNREACH",
  "EHOSTUNREACH",
]);
const NOT_SENT_SYSCALLS = new Set(["connect", "getaddrinfo"]);

export class SearchError extends Error {
  constructor(code, message, status) {
    super(message);
    this.name = "SearchError";
    this.code = code;
    if (status !== undefined) this.status = status;
  }
}

function outcomeUnknown(reason, status) {
  return new SearchError(
    "search_outcome_unknown",
    `The web search may or may not have run and been billed: ${reason}. ` +
      "Do not repeat it automatically.",
    status,
  );
}

export function searchRequestBody(query, model) {
  return {
    model,
    input: `${INSTRUCTION}${query}`,
    tools: [{ type: "web_search", search_context_size: "low" }],
    include: ["web_search_call.action.sources"],
    max_tool_calls: 1,
    reasoning: { effort: "low" },
    max_output_tokens: 1500,
  };
}

// undici wraps the socket error as `cause`; happy eyeballs adds an
// AggregateError with one error per address. A proxy that refused or failed
// before its tunnel opened never passed the request on either.
function neverSent(error) {
  if (proxyRefused(error)) return true;
  const pending = [error];
  for (let seen = 0; pending.length > 0 && seen < 20; seen++) {
    const item = pending.shift();
    if (!item || typeof item !== "object") continue;
    if (
      item.code === "UND_ERR_CONNECT_TIMEOUT" ||
      (NOT_SENT_CODES.has(item.code) && NOT_SENT_SYSCALLS.has(item.syscall))
    )
      return true;
    pending.push(
      item.cause,
      ...(Array.isArray(item.errors) ? item.errors : []),
    );
  }
  return false;
}

async function readBounded(response, limit) {
  const declared = Number(response.headers.get("content-length"));
  if (Number.isFinite(declared) && declared > limit) {
    await response.body?.cancel().catch(() => {});
    return null;
  }
  if (!response.body) return Buffer.alloc(0);
  const reader = response.body.getReader();
  const chunks = [];
  let length = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    length += value.byteLength;
    if (length > limit) {
      await reader.cancel().catch(() => {});
      return null;
    }
    chunks.push(value);
  }
  return Buffer.concat(chunks);
}

// Only Ofox's own out-of-credits error counts: `{ error: { type:
// "insufficient_credits", code: 402 } }`. An upstream provider's quota error
// (OpenAI `insufficient_quota`) says nothing about the user's balance.
// Classification only: the body is never echoed into an error message.
async function isOfoxBalanceError(response) {
  try {
    const bytes = await readBounded(response, MAX_ERROR_BODY_BYTES);
    const error = bytes && JSON.parse(bytes.toString("utf8"))?.error;
    return (
      error?.type === "insufficient_credits" ||
      error?.code === 402 ||
      error?.code === "402"
    );
  } catch {
    return false;
  }
}

async function httpError(response) {
  const { status } = response;
  if (status === 429) {
    await response.body?.cancel().catch(() => {});
    return new SearchError(
      "search_failed",
      "The web search was rate-limited and did not run (HTTP 429). Try " +
        "again later.",
      status,
    );
  }
  if (status >= 400 && status < 500) {
    // Always consume the (bounded) body so the connection is released.
    if ((await isOfoxBalanceError(response)) || status === 402)
      return new SearchError(
        "search_insufficient_balance",
        `The Ofox balance is too low for a web search (HTTP ${status}).`,
        status,
      );
    if (status === 401 || status === 403)
      return new SearchError(
        "search_unauthorized",
        `Ofox rejected the assistant's API key (HTTP ${status}).`,
        status,
      );
    return new SearchError(
      "search_failed",
      `Ofox rejected the web search request (HTTP ${status}).`,
      status,
    );
  }
  await response.body?.cancel().catch(() => {});
  return outcomeUnknown(`the gateway answered HTTP ${status}`, status);
}

function boundedInteger(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function resultUrl(value) {
  if (typeof value !== "string" || value.length > MAX_URL_CHARS) return null;
  let url;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  // Dedupe key only: fragments and utm tracking do not make another page.
  url.hash = "";
  for (const name of [...url.searchParams.keys()])
    if (name.startsWith("utm_")) url.searchParams.delete(name);
  return url.href;
}

function hostTitle(value) {
  return new URL(value).hostname.replace(/^www\./, "");
}

// The citation span is the inline link itself; the words before it on the
// same line are what the source was cited for.
function snippetBefore(text, start) {
  if (!Number.isSafeInteger(start) || start <= 0 || start > text.length)
    return undefined;
  const lineStart = text.lastIndexOf("\n", start - 1) + 1;
  const snippet = text
    .slice(lineStart, start)
    .replace(/^\s*(?:[-*+]|\d+[.)])\s+/, "")
    .replace(/\s+/g, " ")
    .trim();
  return snippet ? snippet.slice(0, MAX_SNIPPET_CHARS) : undefined;
}

// `secrets` are masked in every upstream string before any field is cut, so
// no fragment of the key survives a length limit. The mask keeps the length,
// so citation indices still point into the scrubbed text.
export function parseSearchResponse(body, requestedModel, secrets = []) {
  const clean = (value) => scrubSecrets(value, secrets);
  if (!body || typeof body !== "object" || !Array.isArray(body.output))
    throw outcomeUnknown("the response had no output");
  if (body.status === "failed")
    throw outcomeUnknown("the gateway reported the response as failed");
  let searchCalls = 0;
  const cited = [];
  const sourced = [];
  const texts = [];
  for (const item of body.output) {
    if (item?.type === "web_search_call") {
      searchCalls++;
      const sources = item.action?.sources;
      if (sources === undefined || sources === null) continue;
      if (!Array.isArray(sources))
        throw outcomeUnknown("the response had malformed sources");
      for (const source of sources) {
        if (source?.type !== "url") continue;
        if (typeof source.url !== "string")
          throw outcomeUnknown("the response had a malformed source URL");
        sourced.push({ url: clean(source.url) });
      }
    } else if (item?.type === "message" && Array.isArray(item.content)) {
      for (const part of item.content) {
        if (part?.type !== "output_text" || typeof part.text !== "string")
          continue;
        const text = clean(part.text);
        texts.push(text);
        for (const annotation of part.annotations ?? []) {
          if (annotation?.type !== "url_citation") continue;
          if (typeof annotation.url !== "string")
            throw outcomeUnknown("the response had a malformed citation URL");
          cited.push({
            url: clean(annotation.url),
            title:
              typeof annotation.title === "string"
                ? clean(annotation.title).trim()
                : "",
            snippet: snippetBefore(text, annotation.start_index),
          });
        }
      }
    }
  }
  const results = [];
  const seen = new Set();
  for (const candidate of [...cited, ...sourced]) {
    if (results.length === MAX_RESULTS) break;
    const key = resultUrl(candidate.url);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    const result = {
      title: (candidate.title || hostTitle(candidate.url)).slice(
        0,
        MAX_TITLE_CHARS,
      ),
      url: candidate.url,
    };
    if (candidate.snippet) result.snippet = candidate.snippet;
    results.push(result);
  }
  return {
    summary: texts.join("\n\n").trim().slice(0, MAX_SUMMARY_CHARS),
    results,
    usage: {
      searchCalls,
      inputTokens: boundedInteger(body.usage?.input_tokens),
      outputTokens: boundedInteger(body.usage?.output_tokens),
      model:
        typeof body.model === "string" && body.model && body.model.length <= 256
          ? body.model
          : requestedModel,
    },
  };
}

export async function webSearch(
  query,
  { apiKey, baseUrl, model },
  { signal, timeoutMs = SEARCH_TIMEOUT_MS, fetchImpl = fetch } = {},
) {
  const deadline = AbortSignal.timeout(timeoutMs);
  const combined = signal ? AbortSignal.any([signal, deadline]) : deadline;
  const stopped = () =>
    deadline.aborted
      ? outcomeUnknown("the request timed out")
      : outcomeUnknown("the request was cancelled");
  let response;
  try {
    response = await fetchImpl(`${baseUrl}/v1/responses`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${apiKey}`,
        "Content-Type": "application/json",
        Accept: "application/json",
      },
      body: JSON.stringify(searchRequestBody(query, model)),
      // A redirect must never carry the key to another host.
      redirect: "manual",
      signal: combined,
    });
  } catch (error) {
    // Check the deadline and cancellation first: once either fired, even a
    // connect-looking error (EHOSTUNREACH/ENETUNREACH can also come from a
    // connected socket) leaves the outcome unknown.
    if (combined.aborted) throw stopped();
    if (neverSent(error))
      throw new SearchError(
        "search_failed",
        "Could not connect to the Ofox gateway; no search was made.",
      );
    // TLS failures happen before any byte is sent, but they stay unknown
    // for safety; the wording does not claim the request was sent.
    throw outcomeUnknown(
      "the connection failed and the request may have been sent",
    );
  }
  try {
    if (response.status < 200 || response.status >= 300)
      throw await httpError(response);
    const bytes = await readBounded(response, MAX_RESPONSE_BYTES);
    if (bytes === null)
      throw outcomeUnknown("the response was larger than 1 MiB");
    let body;
    try {
      body = JSON.parse(bytes.toString("utf8"));
    } catch {
      throw outcomeUnknown("the response was not valid JSON");
    }
    return parseSearchResponse(body, model, [apiKey]);
  } catch (error) {
    if (error instanceof SearchError) throw error;
    if (combined.aborted) throw stopped();
    throw outcomeUnknown("the response could not be read");
  }
}
