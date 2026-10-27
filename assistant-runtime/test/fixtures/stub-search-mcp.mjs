// A stand-in for the bundled search server (search-mcp/): the same two tools
// over MCP stdio, answers scripted by the query or URL. It records how it was
// started (argv, environment) and every request in log.jsonl next to itself,
// so each test copies it into a directory of its own.
import { appendFileSync } from "node:fs";
import path from "node:path";
import { createInterface } from "node:readline";

const log = path.join(path.dirname(process.argv[1]), "log.jsonl");
const record = (entry) => appendFileSync(log, `${JSON.stringify(entry)}\n`);
record({ event: "start", argv: process.argv.slice(1), env: process.env });

const send = (message) =>
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
const text = (value) => [{ type: "text", text: value }];
const failure = (code, message, status) => ({
  isError: true,
  content: text(`${code}: ${message}`),
  structuredContent: {
    error: { code, message, ...(status === undefined ? {} : { status }) },
  },
});

function search(query) {
  if (query === "unknown outcome")
    return failure(
      "search_outcome_unknown",
      "The web search may or may not have run and been billed: the request timed out. Do not repeat it automatically.",
    );
  if (query === "rate limited")
    return failure(
      "search_failed",
      "The web search was rate-limited and did not run (HTTP 429). Try again later.",
      429,
    );
  if (query === "rejected arguments")
    return {
      isError: true,
      content: text("MCP error -32602: Input validation error"),
    };
  if (query === "malformed answer") return { content: "not a list" };
  if (query === "hang") return undefined;
  if (query === "crash") process.exit(3);
  const slug = query.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  const results = [
    {
      title: `About ${query}`,
      url: `https://example.com/${slug}#intro`,
      snippet: `What ${query} is.`,
    },
    { title: "example.org", url: `https://example.org/${slug}` },
  ];
  return {
    content: text(
      `Summary of ${query}.\n\nSources:\n1. About ${query}\n   https://example.com/${slug}`,
    ),
    structuredContent: {
      summary: `Summary of ${query}.`,
      results,
      usage: {
        searchCalls: 1,
        inputTokens: 120,
        outputTokens: 30,
        model: "openai/gpt-5-nano",
      },
    },
  };
}

function fetchPage(url) {
  if (url.includes("missing"))
    return failure("fetch_failed", "Web page returned HTTP 404", 404);
  return {
    content: text(`Page: ${url}\n\nText of ${url}`),
    structuredContent: { url, text: `Text of ${url}`, truncated: false },
  };
}

const tools = ["web_search", "web_fetch"].map((name) => ({
  name,
  inputSchema: { type: "object" },
}));

createInterface({ input: process.stdin })
  .on("line", (line) => {
    const message = JSON.parse(line);
    record({
      event: "request",
      method: message.method,
      params: message.params,
    });
    if (message.method === "initialize")
      send({
        id: message.id,
        result: {
          protocolVersion: message.params.protocolVersion,
          capabilities: { tools: {} },
          serverInfo: { name: "stub-search", version: "0.0.0" },
        },
      });
    else if (message.method === "tools/list")
      send({ id: message.id, result: { tools } });
    else if (message.method === "tools/call") {
      const { name, arguments: args } = message.params;
      if (args.query === "protocol error")
        send({
          id: message.id,
          error: { code: -32602, message: "Invalid params" },
        });
      else {
        const result =
          name === "web_search" ? search(args.query) : fetchPage(args.url);
        if (result) send({ id: message.id, result });
      }
    } else if (message.id !== undefined)
      send({
        id: message.id,
        error: { code: -32601, message: "Method not found" },
      });
  })
  .on("close", () => {
    record({ event: "exit" });
    process.exit(0);
  });
