import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
// A namespace import lets esbuild drop zod's ~250 KiB of unused locales.
import * as z from "zod";
import { proxiedFetch } from "./proxy.mjs";
import { redact, scrubSecrets } from "./redact.mjs";
import { SearchError, webSearch } from "./search.mjs";
import { FetchError, readWebPage, tunneledRequest } from "./web.mjs";

export const SERVER_NAME = "ofox-search";
export const SERVER_VERSION = "0.1.0";
// Pi truncates MCP text above 20 KB (UTF-8) and moves the rest to a temp file
// the assistant has no tool to read, so every text result stays within it.
export const MAX_TEXT_BYTES = 20 * 1024;
const TRUNCATED_MARK = "\n[Output truncated at 20 KB]";
const PAGE_TRUNCATED_MARK = "\n\n[Page text truncated]";

const FALLBACK_MESSAGES = {
  search_outcome_unknown:
    "The web search may or may not have run and been billed: it failed " +
    "unexpectedly. Do not repeat it automatically.",
  fetch_failed: "Web request failed",
};

const byteLength = (text) => Buffer.byteLength(text, "utf8");

// Longest prefix of `text` within `maxBytes`, cut on a character boundary.
function prefixWithin(text, maxBytes) {
  const buffer = Buffer.from(text, "utf8");
  if (buffer.length <= maxBytes) return text;
  let end = Math.max(0, maxBytes);
  while (end > 0 && (buffer[end] & 0xc0) === 0x80) end--;
  return buffer.subarray(0, end).toString("utf8");
}

export function capText(text, maxBytes = MAX_TEXT_BYTES) {
  if (byteLength(text) <= maxBytes) return text;
  return (
    prefixWithin(text, maxBytes - byteLength(TRUNCATED_MARK)) + TRUNCATED_MARK
  );
}

function sourceLines(results) {
  if (results.length === 0) return "No source links were returned.";
  const lines = ["Sources:"];
  results.forEach((result, index) => {
    lines.push(`${index + 1}. ${result.title}`, `   ${result.url}`);
    if (result.snippet) lines.push(`   ${result.snippet}`);
  });
  return lines.join("\n");
}

// The summary gives way first, so the source links survive a long answer.
export function searchToolResult(result) {
  const sources = sourceLines(result.results);
  let summary = result.summary;
  if (summary) {
    const budget = MAX_TEXT_BYTES - byteLength(`\n\n${sources}`);
    if (byteLength(summary) > budget)
      summary = budget > 3 ? `${prefixWithin(summary, budget - 3)}…` : "";
  }
  return {
    content: [
      {
        type: "text",
        text: capText(summary ? `${summary}\n\n${sources}` : sources),
      },
    ],
    structuredContent: result,
  };
}

export function fetchToolResult(page) {
  const header = `Page: ${page.url}\n\n`;
  let { text, truncated } = page;
  const mark = () => (truncated ? PAGE_TRUNCATED_MARK : "");
  if (byteLength(header + text + mark()) > MAX_TEXT_BYTES) {
    truncated = true;
    text = prefixWithin(
      text,
      MAX_TEXT_BYTES - byteLength(header) - byteLength(PAGE_TRUNCATED_MARK),
    );
  }
  return {
    content: [{ type: "text", text: capText(header + text + mark()) }],
    structuredContent: { url: page.url, text, truncated },
  };
}

export function toolError(error, fallbackCode, secrets) {
  const known = error instanceof SearchError || error instanceof FetchError;
  const code = known ? error.code : fallbackCode;
  const message = redact(
    known ? error.message : FALLBACK_MESSAGES[fallbackCode],
    secrets,
  );
  const detail = { code, message };
  if (known && error.status !== undefined) detail.status = error.status;
  return {
    isError: true,
    content: [{ type: "text", text: `${code}: ${message}` }],
    structuredContent: { error: detail },
  };
}

function stderrLog(line) {
  process.stderr.write(`ofox-search-mcp: ${line}\n`);
}

export function createSearchServer(
  config,
  { search = webSearch, fetchPage = readWebPage, log = stderrLog } = {},
) {
  const secrets = [config.apiKey];
  const unexpected = (tool, error) =>
    log(
      `unexpected ${tool} failure: ${redact(
        error instanceof Error ? error.message : String(error),
        secrets,
      ).slice(0, 300)}`,
    );
  // One set of proxy connections per server, when the app has a proxy. A
  // fake-IP host's DNS-over-HTTPS check takes the same route as a search.
  const fetchImpl = config.proxy ? proxiedFetch(config.proxy) : undefined;
  const fetchProxy = config.proxy
    ? { noProxy: config.proxy.noProxy, request: tunneledRequest(config.proxy) }
    : null;
  const doh = { fetchImpl };
  const server = new McpServer({ name: SERVER_NAME, version: SERVER_VERSION });

  server.registerTool(
    "web_search",
    {
      title: "Web search",
      description:
        "Search the web through Ofox and get a short summary with source " +
        "links. Each call is one paid web search: search only when current " +
        "or outside information is needed, and never repeat a search that " +
        "returned search_outcome_unknown.",
      inputSchema: z.strictObject({
        query: z
          .string()
          .min(1)
          .max(2000)
          .regex(/\S/)
          .describe("What to search for (1-2000 characters)"),
      }),
      annotations: { readOnlyHint: true, openWorldHint: true },
    },
    async ({ query }, extra) => {
      try {
        const result = await search(query, config, {
          signal: extra.signal,
          fetchImpl,
        });
        return searchToolResult(scrubSecrets(result, secrets));
      } catch (error) {
        if (!(error instanceof SearchError)) unexpected("web_search", error);
        return toolError(error, "search_outcome_unknown", secrets);
      }
    },
  );

  server.registerTool(
    "web_fetch",
    {
      title: "Read web page",
      description:
        "Read the text of a public HTTPS web page (static HTML or plain " +
        "text, at most 20,000 characters). Use it for pages the user gave " +
        "or that a web search returned.",
      inputSchema: z.strictObject({
        url: z.string().min(1).max(2048).describe("The https:// page URL"),
      }),
      annotations: { readOnlyHint: true, openWorldHint: true },
    },
    async ({ url }, extra) => {
      try {
        const page = await fetchPage(url, {
          signal: extra.signal,
          secrets,
          proxy: fetchProxy,
          doh,
        });
        return fetchToolResult(scrubSecrets(page, secrets));
      } catch (error) {
        if (!(error instanceof FetchError)) unexpected("web_fetch", error);
        return toolError(error, "fetch_failed", secrets);
      }
    },
  );

  return server;
}
