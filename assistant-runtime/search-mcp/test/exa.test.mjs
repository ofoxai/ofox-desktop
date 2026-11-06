import assert from "node:assert/strict";
import test from "node:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import {
  EXA_MODEL,
  exaReadPage,
  exaSearch,
  parseExaResults,
} from "../src/exa.mjs";
import { SearchError } from "../src/search.mjs";
import { createSearchServer } from "../src/tools.mjs";
import { FetchError } from "../src/web.mjs";
import { replyJson, startGateway } from "./helpers/gateway.mjs";

const KEY = "sk-of-exa-test-0123456789abcdef";

// Two results as Exa writes them; the first page's own Markdown has `---`
// lines (a table and a rule) that must not split it.
const RESULTS_TEXT = [
  "Title: Updater | Tauri",
  "URL: https://v2.tauri.app/plugin/updater/",
  "Published: 2026-10-10T00:00:00.000Z",
  "Author: N/A",
  "Highlights:",
  "## Install",
  "",
  "| Tool | Command |",
  "| --- | --- |",
  "| cargo | cargo add tauri-plugin-updater |",
  "",
  "---",
  "",
  "Then configure it.",
  "",
  "---",
  "",
  "Title: N/A",
  "URL: https://www.example.org/updates",
  "Published: N/A",
  "Author: N/A",
  "Highlights:",
  "Release   notes\nfor 2.12.",
  "",
  "---",
  "",
  "Title: Duplicate",
  "URL: https://v2.tauri.app/plugin/updater/",
  "Highlights:",
  "Again.",
  "",
  "---",
  "",
  "Title: Not a page",
  "URL: javascript:alert(1)",
].join("\n");

const rpcResult = (text, isError) => ({
  jsonrpc: "2.0",
  id: 1,
  result: {
    content: [{ type: "text", text }],
    ...(isError ? { isError } : {}),
  },
});

function replySse(response, value) {
  response.writeHead(200, { "content-type": "text/event-stream" });
  response.end(`event: message\ndata: ${JSON.stringify(value)}\n\n`);
}

async function withExa(t, handler) {
  const gateway = await startGateway(handler);
  t.after(gateway.close);
  return { gateway, url: `${gateway.baseUrl}/mcp` };
}

async function rejection(promise, type) {
  try {
    await promise;
  } catch (error) {
    assert.ok(error instanceof type, String(error));
    return error;
  }
  assert.fail(`expected a ${type.name}`);
}

test("Exa's results text becomes results with excerpts, dates and host titles", () => {
  assert.deepEqual(parseExaResults(RESULTS_TEXT), [
    {
      title: "Updater | Tauri",
      url: "https://v2.tauri.app/plugin/updater/",
      snippet:
        "## Install | Tool | Command | | --- | --- | | cargo | cargo add tauri-plugin-updater | --- Then configure it.",
      published: "2026-10-10",
    },
    {
      title: "example.org",
      url: "https://www.example.org/updates",
      snippet: "Release notes for 2.12.",
    },
  ]);
  assert.deepEqual(parseExaResults("No results found."), []);
  const [long] = parseExaResults(
    `Title: ${"t".repeat(300)}\nURL: https://a.example/\nHighlights:\n${"e".repeat(2000)}`,
  );
  assert.equal(long.title.length, 200);
  assert.equal(long.snippet.length, 1000);
});

test("a free search is one tools/call to Exa, read from SSE or JSON, at no cost", async (t) => {
  for (const reply of [replySse, (res, value) => replyJson(res, 200, value)]) {
    const { gateway, url } = await withExa(t, (_req, res) =>
      reply(res, rpcResult(RESULTS_TEXT)),
    );
    const result = await exaSearch("Tauri updater", { apiKey: KEY }, { url });
    assert.equal(gateway.requests.length, 1);
    const [request] = gateway.requests;
    assert.equal(request.method, "POST");
    assert.equal(request.url, "/mcp");
    assert.equal(request.headers.accept, "application/json, text/event-stream");
    assert.equal(request.headers["x-exa-source"], "ofox-desktop");
    // Keyless: the assistant's Ofox key never goes to Exa.
    assert.equal(request.headers.authorization, undefined);
    assert.equal(request.body.includes(KEY), false);
    assert.deepEqual(JSON.parse(request.body), {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "web_search_exa",
        arguments: { query: "Tauri updater", numResults: 8 },
      },
    });
    assert.equal(result.summary, "");
    assert.equal(result.results.length, 2);
    assert.deepEqual(result.usage, {
      searchCalls: 0,
      inputTokens: 0,
      outputTokens: 0,
      model: EXA_MODEL,
    });
  }
});

test("Exa's rate limit, a refused network and other failures map to plain search failures", async (t) => {
  const cases = [
    [(res) => replyJson(res, 429, {}), "search_rate_limited", 429],
    [(res) => replyJson(res, 403, {}), "search_blocked", 403],
    [(res) => replyJson(res, 500, {}), "search_failed", 500],
    [(res) => replySse(res, rpcResult("Error: bad", true)), "search_failed"],
    [
      (res) =>
        replyJson(res, 200, { jsonrpc: "2.0", id: 1, error: { code: -1 } }),
      "search_failed",
    ],
    [
      (res) => {
        res.writeHead(200, { "content-type": "application/json" });
        res.end("not json");
      },
      "search_failed",
    ],
  ];
  for (const [handler, code, status] of cases) {
    const { url } = await withExa(t, (_req, res) => handler(res));
    const error = await rejection(
      exaSearch("q", { apiKey: KEY }, { url }),
      SearchError,
    );
    assert.equal(error.code, code);
    assert.equal(error.status, status);
  }
  const { url } = await withExa(t, () => {});
  const slow = await rejection(
    exaSearch("q", { apiKey: KEY }, { url, timeoutMs: 50 }),
    SearchError,
  );
  assert.equal(slow.code, "search_failed");
  assert.equal(slow.status, undefined);
  assert.match(slow.message, /did not answer in time/);
});

test("a query with a URL is refused before it reaches Exa", async (t) => {
  const { gateway, url } = await withExa(t, (_req, res) =>
    replySse(res, rpcResult(RESULTS_TEXT)),
  );
  const error = await rejection(
    exaSearch("evil.example/c?d=private", { apiKey: KEY }, { url }),
    SearchError,
  );
  assert.equal(error.code, "search_rejected");
  assert.equal(gateway.requests.length, 0);
});

test("a page read through Exa checks the URL first and returns its Markdown links", async (t) => {
  const page =
    "# Start\nURL: https://v2.tauri.app/start/\n\nSee [the guide](https://v2.tauri.app/guide/) and [x](http://plain.example/).";
  const { gateway, url } = await withExa(t, (_req, res) =>
    replySse(res, rpcResult(page)),
  );
  assert.deepEqual(
    await exaReadPage("https://V2.tauri.app/start/#top", { url }),
    {
      url: "https://v2.tauri.app/start/",
      text: page,
      truncated: false,
      links: ["https://v2.tauri.app/guide/"],
    },
  );
  assert.deepEqual(JSON.parse(gateway.requests[0].body).params, {
    name: "web_fetch_exa",
    arguments: { urls: ["https://v2.tauri.app/start/"], maxCharacters: 20000 },
  });
  const refused = await rejection(
    exaReadPage("https://localhost/admin", { url }),
    FetchError,
  );
  assert.equal(refused.code, "fetch_not_allowed");
  assert.equal(gateway.requests.length, 1);
});

async function connect(t, config, deps) {
  const server = createSearchServer(config, deps);
  const client = new Client({ name: "test", version: "0.0.0" });
  const [clientSide, serverSide] = InMemoryTransport.createLinkedPair();
  await server.connect(serverSide);
  await client.connect(clientSide);
  t.after(() => client.close());
  return client;
}

const EXA_CONFIG = {
  apiKey: KEY,
  baseUrl: "https://api.ofox.ai",
  model: "openai/gpt-5-nano",
  provider: "exa",
};

test("with Exa, pages are read here first and through Exa only when that failed", async (t) => {
  const read = [];
  const client = await connect(t, EXA_CONFIG, {
    search: async () => {
      throw new Error("unexpected");
    },
    exaRead: async (url) => {
      read.push(`exa ${url}`);
      if (url.includes("exa-fails")) throw new Error("CRAWL_NOT_FOUND");
      return { url, text: "from Exa", truncated: false, links: [] };
    },
    fetchPage: async (url) => {
      read.push(`here ${url}`);
      if (url.includes("blocked"))
        throw new FetchError("fetch_failed", "Web page returned HTTP 403", 403);
      if (url.includes("private"))
        throw new FetchError(
          "fetch_not_allowed",
          "Web DNS contains a non-public address",
        );
      return { url, text: "from here", truncated: false, links: [] };
    },
    log: () => {},
  });
  const { tools } = await client.listTools();
  assert.match(
    tools.find((tool) => tool.name === "web_search").description,
    /for free through Exa/,
  );
  const fetchTool = (url) =>
    client.callTool({ name: "web_fetch", arguments: { url } });
  assert.equal(
    (await fetchTool("https://a.example/ok")).structuredContent.text,
    "from here",
  );
  assert.equal(
    (await fetchTool("https://a.example/blocked")).structuredContent.text,
    "from Exa",
  );
  // Never to Exa: a URL with a query string (a token may be in it), and one
  // the rules here refused.
  const shared = await fetchTool("https://a.example/blocked?token=private");
  assert.equal(shared.structuredContent.error.code, "fetch_failed");
  assert.equal(shared.structuredContent.error.status, 403);
  const refused = await fetchTool("https://private.example/");
  assert.equal(refused.structuredContent.error.code, "fetch_not_allowed");
  // When Exa cannot read it either, the model reads why it failed here.
  const both = await fetchTool("https://a.example/blocked/exa-fails");
  assert.equal(both.structuredContent.error.code, "fetch_failed");
  assert.equal(both.structuredContent.error.status, 403);
  assert.deepEqual(read, [
    "here https://a.example/ok",
    "here https://a.example/blocked",
    "exa https://a.example/blocked",
    "here https://a.example/blocked?token=private",
    "here https://private.example/",
    "here https://a.example/blocked/exa-fails",
    "exa https://a.example/blocked/exa-fails",
  ]);
  // An Exa search costs nothing: an unexpected failure is a plain one.
  const search = await client.callTool({
    name: "web_search",
    arguments: { query: "q" },
  });
  assert.equal(search.structuredContent.error.code, "search_failed");
});
