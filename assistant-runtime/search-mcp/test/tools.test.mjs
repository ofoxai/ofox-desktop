import assert from "node:assert/strict";
import test from "node:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { SearchError } from "../src/search.mjs";
import {
  MAX_TEXT_BYTES,
  capText,
  createSearchServer,
  fetchToolResult,
  searchToolResult,
  toolError,
} from "../src/tools.mjs";
import { FetchError } from "../src/web.mjs";

const KEY = "sk-of-tools-test-0123456789abcdef";
const CONFIG = {
  apiKey: KEY,
  baseUrl: "https://api.ofox.ai",
  model: "openai/gpt-5-nano",
};
const bytes = (text) => Buffer.byteLength(text, "utf8");

async function connect(t, deps) {
  const server = createSearchServer(CONFIG, deps);
  const client = new Client({ name: "test", version: "0.0.0" });
  const [clientSide, serverSide] = InMemoryTransport.createLinkedPair();
  await server.connect(serverSide);
  await client.connect(clientSide);
  t.after(() => client.close());
  return client;
}

test("capText keeps text within 20 KB on a UTF-8 character boundary", () => {
  assert.equal(MAX_TEXT_BYTES, 20 * 1024);
  assert.equal(capText("short"), "short");
  const exact = "a".repeat(MAX_TEXT_BYTES);
  assert.equal(capText(exact), exact);
  for (const unit of ["a", "é", "検", "🔎"]) {
    const capped = capText(unit.repeat(MAX_TEXT_BYTES + 1));
    assert.ok(bytes(capped) <= MAX_TEXT_BYTES, unit);
    assert.ok(bytes(capped) > MAX_TEXT_BYTES - 64, unit);
    assert.equal(capped.includes("�"), false, unit);
    assert.match(capped, /\[Output truncated at 20 KB\]$/);
  }
});

test("search results are readable text plus the same data as structured content", () => {
  const result = {
    summary: "Pi is an agent toolkit.",
    results: [
      { title: "Pi", url: "https://pi.dev/", snippet: "Official site." },
      { title: "github.com", url: "https://github.com/earendil-works/pi" },
    ],
    usage: { searchCalls: 1, inputTokens: 10, outputTokens: 5, model: "m" },
  };
  const tool = searchToolResult(result);
  assert.equal(tool.isError, undefined);
  assert.deepEqual(tool.structuredContent, result);
  assert.equal(tool.content.length, 1);
  assert.equal(tool.content[0].type, "text");
  assert.equal(
    tool.content[0].text,
    [
      "Pi is an agent toolkit.",
      "",
      "Sources:",
      "1. Pi",
      "   https://pi.dev/",
      "   Official site.",
      "2. github.com",
      "   https://github.com/earendil-works/pi",
    ].join("\n"),
  );
});

test("the largest possible search result still fits in 20 KB of text", () => {
  const results = Array.from({ length: 25 }, (_v, index) => ({
    title: "題".repeat(200),
    url: `https://example.com/${index}/${"p".repeat(2000)}`,
    snippet: "抜".repeat(300),
  }));
  const tool = searchToolResult({
    summary: "要".repeat(8000),
    results,
    usage: { searchCalls: 1, inputTokens: 1, outputTokens: 1, model: "m" },
  });
  assert.ok(bytes(tool.content[0].text) <= MAX_TEXT_BYTES);
  assert.equal(tool.structuredContent.results.length, 25);
});

test("a long summary is shortened before the source list is", () => {
  const tool = searchToolResult({
    summary: "要".repeat(8000),
    results: [{ title: "A", url: "https://a.example/" }],
    usage: { searchCalls: 1, inputTokens: 1, outputTokens: 1, model: "m" },
  });
  const text = tool.content[0].text;
  assert.ok(bytes(text) <= MAX_TEXT_BYTES);
  assert.match(text, /\n\nSources:\n1\. A\n {3}https:\/\/a\.example\/$/);
  assert.equal(tool.structuredContent.summary.length, 8000);
});

test("fetched pages fit in 20 KB and report what the model actually saw", () => {
  const short = fetchToolResult({
    url: "https://example.com/",
    text: "Hello",
    truncated: false,
  });
  assert.deepEqual(short.structuredContent, {
    url: "https://example.com/",
    text: "Hello",
    truncated: false,
  });
  assert.equal(short.content[0].text, "Page: https://example.com/\n\nHello");

  const long = fetchToolResult({
    url: "https://example.com/",
    text: "検".repeat(20000),
    truncated: false,
  });
  assert.ok(bytes(long.content[0].text) <= MAX_TEXT_BYTES);
  assert.equal(long.structuredContent.truncated, true);
  assert.ok(long.content[0].text.includes(long.structuredContent.text));
  assert.ok(long.structuredContent.text.length < 20000);
  assert.match(long.content[0].text, /\[Page text truncated\]$/);
});

test("tool errors carry the code, status and a redacted message", () => {
  const error = toolError(
    new SearchError("search_failed", "Ofox rejected it (HTTP 429).", 429),
    "search_outcome_unknown",
    [KEY],
  );
  assert.deepEqual(error, {
    isError: true,
    content: [
      { type: "text", text: "search_failed: Ofox rejected it (HTTP 429)." },
    ],
    structuredContent: {
      error: {
        code: "search_failed",
        message: "Ofox rejected it (HTTP 429).",
        status: 429,
      },
    },
  });
  const unexpected = toolError(
    new Error(`boom Bearer ${KEY} ${KEY}`),
    "search_outcome_unknown",
    [KEY],
  );
  assert.equal(
    unexpected.structuredContent.error.code,
    "search_outcome_unknown",
  );
  assert.equal(JSON.stringify(unexpected).includes(KEY), false);
  assert.equal(
    toolError(new FetchError("fetch_not_allowed", "nope"), "fetch_failed", [])
      .structuredContent.error.code,
    "fetch_not_allowed",
  );
});

test("the MCP server lists exactly web_search and web_fetch with bounded inputs", async (t) => {
  const client = await connect(t);
  const { tools } = await client.listTools();
  assert.deepEqual(tools.map((tool) => tool.name).sort(), [
    "web_fetch",
    "web_search",
  ]);
  const search = tools.find((tool) => tool.name === "web_search");
  assert.deepEqual(search.inputSchema.required, ["query"]);
  assert.equal(search.inputSchema.properties.query.minLength, 1);
  assert.equal(search.inputSchema.properties.query.maxLength, 2000);
  assert.equal(search.inputSchema.additionalProperties, false);
  const fetchTool = tools.find((tool) => tool.name === "web_fetch");
  assert.deepEqual(fetchTool.inputSchema.required, ["url"]);
});

test("tool calls pass the query, config and a cancel signal through", async (t) => {
  const calls = [];
  const client = await connect(t, {
    search: async (query, config, options) => {
      calls.push({ query, config, signal: options.signal });
      return {
        summary: "S",
        results: [],
        usage: { searchCalls: 1, inputTokens: 1, outputTokens: 1, model: "m" },
      };
    },
    fetchPage: async (url, options) => {
      calls.push({ url, signal: options.signal });
      return { url, text: "T", truncated: false };
    },
  });
  const search = await client.callTool({
    name: "web_search",
    arguments: { query: "hello" },
  });
  assert.equal(search.isError, undefined);
  assert.equal(search.structuredContent.summary, "S");
  const page = await client.callTool({
    name: "web_fetch",
    arguments: { url: "https://example.com/" },
  });
  assert.equal(page.structuredContent.text, "T");
  assert.equal(calls[0].query, "hello");
  assert.deepEqual(calls[0].config, CONFIG);
  assert.ok(calls[0].signal instanceof AbortSignal);
  assert.equal(calls[1].url, "https://example.com/");
  assert.ok(calls[1].signal instanceof AbortSignal);
});

test("invalid arguments are rejected before any search is made", async (t) => {
  let searches = 0;
  const client = await connect(t, {
    search: async () => {
      searches++;
      throw new Error("must not search");
    },
  });
  for (const args of [
    {},
    { query: "" },
    { query: "   " },
    { query: "x".repeat(2001) },
  ]) {
    const result = await client.callTool({
      name: "web_search",
      arguments: args,
    });
    assert.equal(result.isError, true, JSON.stringify(args));
  }
  assert.equal(searches, 0);
});

test("unexpected search failures are outcome-unknown and never leak the key", async (t) => {
  const logged = [];
  const client = await connect(t, {
    log: (line) => logged.push(line),
    search: async () => {
      throw new Error(`socket closed for Bearer ${KEY}`);
    },
    fetchPage: async () => {
      throw new Error(`weird ${KEY}`);
    },
  });
  const search = await client.callTool({
    name: "web_search",
    arguments: { query: "q" },
  });
  assert.equal(search.isError, true);
  assert.equal(search.structuredContent.error.code, "search_outcome_unknown");
  assert.equal(JSON.stringify(search).includes(KEY), false);
  const page = await client.callTool({
    name: "web_fetch",
    arguments: { url: "https://example.com/" },
  });
  assert.equal(page.structuredContent.error.code, "fetch_failed");
  assert.equal(JSON.stringify(page).includes(KEY), false);
  assert.equal(logged.length, 2);
  assert.match(logged[0], /unexpected web_search failure: socket closed/);
  assert.equal(logged.join("\n").includes(KEY), false);
});

test("results never echo the key, even when the upstream data contains it", async (t) => {
  const client = await connect(t, {
    search: async () => ({
      summary: `leak ${KEY}`,
      results: [
        { title: `t ${KEY}`, url: `https://a.example/?k=${KEY}`, snippet: KEY },
      ],
      usage: { searchCalls: 1, inputTokens: 1, outputTokens: 1, model: "m" },
    }),
    fetchPage: async (url) => ({ url, text: `page ${KEY}`, truncated: false }),
  });
  const search = await client.callTool({
    name: "web_search",
    arguments: { query: "q" },
  });
  const page = await client.callTool({
    name: "web_fetch",
    arguments: { url: "https://example.com/" },
  });
  for (const result of [search, page]) {
    assert.equal(result.isError, undefined);
    assert.equal(JSON.stringify(result).includes(KEY), false);
    assert.match(result.content[0].text, /\[REDACTED\]/);
  }
});
