// End-to-end over stdio against the BUILT bundle (`npm test` builds it first).
// The bundle is copied into an empty temp directory, so it must run without
// any node_modules.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { copyFile, mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { createServer } from "node:http";
import { connect as connectSocket } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { replyJson, startGateway } from "./helpers/gateway.mjs";

const KEY = "sk-of-bundle-test-0123456789abcdef";
const BUNDLE = fileURLToPath(
  new URL("../dist/ofox-search-mcp.mjs", import.meta.url),
);

async function isolatedBundle(t) {
  await stat(BUNDLE).catch(() => {
    throw new Error("dist/ofox-search-mcp.mjs is missing: run `npm run build`");
  });
  const dir = await mkdtemp(path.join(tmpdir(), "ofox-search-mcp-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const copy = path.join(dir, "ofox-search-mcp.mjs");
  await copyFile(BUNDLE, copy);
  return { dir, copy };
}

async function connect(t, env) {
  const { dir, copy } = await isolatedBundle(t);
  const transport = new StdioClientTransport({
    command: process.execPath,
    args: [copy],
    cwd: dir,
    env,
    stderr: "pipe",
  });
  let stderr = "";
  transport.stderr.on("data", (chunk) => (stderr += chunk));
  const client = new Client({ name: "bundle-test", version: "0.0.0" });
  await client.connect(transport);
  t.after(() => client.close());
  return { client, stderr: () => stderr };
}

function run(copy, env) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [copy], {
      env,
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.on("data", (chunk) => (stderr += chunk));
    child.on("close", (code) => resolve({ code, stdout, stderr }));
  });
}

test("the bundle has no HTTP server transports in it", async () => {
  const source = await readFile(BUNDLE, "utf8");
  assert.doesNotMatch(source, /express|hono/i);
});

test("the bundle serves web_search and web_fetch over stdio", async (t) => {
  const fixture = JSON.parse(
    await readFile(
      new URL("./fixtures/responses-max-tool-calls-1.json", import.meta.url),
      "utf8",
    ),
  );
  const gateway = await startGateway((_req, res, record) => {
    const { input } = JSON.parse(record.body);
    if (input.endsWith("unauthorized please"))
      replyJson(res, 401, { leaked: KEY });
    else replyJson(res, 200, fixture);
  });
  t.after(gateway.close);
  const { client, stderr } = await connect(t, {
    OFOX_ASSISTANT_KEY: KEY,
    OFOX_BASE_URL: gateway.baseUrl,
    OFOX_ALLOW_LOCAL_GATEWAY: "1",
  });

  const { tools } = await client.listTools();
  assert.deepEqual(tools.map((tool) => tool.name).sort(), [
    "web_fetch",
    "web_search",
  ]);

  const search = await client.callTool({
    name: "web_search",
    arguments: { query: "Pi coding agent SDK" },
  });
  assert.equal(search.isError, undefined);
  assert.equal(search.structuredContent.results.length, 17);
  assert.deepEqual(search.structuredContent.usage, {
    searchCalls: 1,
    inputTokens: 8497,
    outputTokens: 820,
    model: "openai/gpt-5-nano",
  });
  const text = search.content[0].text;
  assert.ok(Buffer.byteLength(text) <= 20 * 1024);
  assert.match(text, /^Here are the most relevant/);
  assert.match(text, /\nSources:\n1\. GitHub - earendil-works\/pi/);
  assert.equal(gateway.requests.length, 1);
  const [request] = gateway.requests;
  assert.equal(request.url, "/v1/responses");
  assert.equal(request.headers.authorization, `Bearer ${KEY}`);
  const body = JSON.parse(request.body);
  assert.equal(body.max_tool_calls, 1);
  assert.equal(body.model, "openai/gpt-5-nano");
  assert.deepEqual(body.tools, [
    { type: "web_search", search_context_size: "low" },
  ]);

  const denied = await client.callTool({
    name: "web_search",
    arguments: { query: "unauthorized please" },
  });
  assert.equal(denied.isError, true);
  assert.deepEqual(denied.structuredContent, {
    error: {
      code: "search_unauthorized",
      message: "Ofox rejected the assistant's API key (HTTP 401).",
      status: 401,
    },
  });
  assert.equal(gateway.requests.length, 2);

  const empty = await client.callTool({
    name: "web_search",
    arguments: { query: "" },
  });
  assert.equal(empty.isError, true);
  assert.equal(gateway.requests.length, 2);

  const page = await client.callTool({
    name: "web_fetch",
    arguments: { url: "http://example.com/" },
  });
  assert.equal(page.isError, true);
  assert.equal(page.structuredContent.error.code, "fetch_not_allowed");

  for (const result of [search, denied, empty, page])
    assert.equal(JSON.stringify(result).includes(KEY), false);
  await client.close();
  // Expected failures (401, bad input, blocked URL) log nothing at all.
  assert.equal(stderr(), "");
});

test("the search model can be overridden through the environment", async (t) => {
  const gateway = await startGateway((_req, res) =>
    replyJson(res, 200, { output: [] }),
  );
  t.after(gateway.close);
  const { client } = await connect(t, {
    OFOX_ASSISTANT_KEY: KEY,
    OFOX_BASE_URL: gateway.baseUrl,
    OFOX_ALLOW_LOCAL_GATEWAY: "1",
    OFOX_SEARCH_MODEL: "openai/gpt-5-mini",
  });
  const result = await client.callTool({
    name: "web_search",
    arguments: { query: "q" },
  });
  assert.equal(result.structuredContent.usage.model, "openai/gpt-5-mini");
  assert.equal(JSON.parse(gateway.requests[0].body).model, "openai/gpt-5-mini");
});

test("invalid configuration exits non-zero at startup without echoing the key", async (t) => {
  const { copy } = await isolatedBundle(t);
  for (const env of [
    { OFOX_BASE_URL: "https://api.ofox.ai" },
    { OFOX_ASSISTANT_KEY: KEY },
    { OFOX_ASSISTANT_KEY: KEY, OFOX_BASE_URL: `https://api.ofox.ai/${KEY}` },
    { OFOX_ASSISTANT_KEY: KEY, OFOX_BASE_URL: "http://localhost:8088" },
    {
      OFOX_ASSISTANT_KEY: KEY,
      OFOX_BASE_URL: "https://api.ofox.ai",
      OFOX_SEARCH_MODEL: KEY + " x",
    },
    {
      OFOX_ASSISTANT_KEY: KEY,
      OFOX_BASE_URL: "https://api.ofox.ai",
      OFOX_PROXY_URL: `ftp://user:${KEY}@127.0.0.1:21`,
    },
  ]) {
    const { code, stdout, stderr } = await run(copy, env);
    assert.equal(code, 2, JSON.stringify(Object.keys(env)));
    assert.equal(stdout, "");
    assert.match(stderr, /^ofox-search-mcp: OFOX_[A-Z_]+ /);
    assert.equal(stderr.includes(KEY), false, stderr);
  }
});

test("the bundle sends web_search through the proxy named in its environment", async (t) => {
  const gateway = await startGateway((_req, res) =>
    replyJson(res, 200, { output: [] }),
  );
  t.after(gateway.close);
  const gatewayPort = Number(new URL(gateway.baseUrl).port);
  const tunnels = [];
  const sockets = new Set();
  const proxy = createServer();
  proxy.on("connect", (request, client, head) => {
    tunnels.push(request.url);
    sockets.add(client);
    const upstream = connectSocket(gatewayPort, "localhost", () => {
      client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
      upstream.write(head);
      upstream.pipe(client);
      client.pipe(upstream);
    });
    sockets.add(upstream);
  });
  proxy.listen(0, "127.0.0.1");
  await once(proxy, "listening");
  t.after(async () => {
    sockets.forEach((socket) => socket.destroy());
    await new Promise((resolve) => proxy.close(resolve));
  });
  const { client, stderr } = await connect(t, {
    OFOX_ASSISTANT_KEY: KEY,
    OFOX_BASE_URL: gateway.baseUrl,
    OFOX_ALLOW_LOCAL_GATEWAY: "1",
    OFOX_PROXY_URL: `http://user:proxy-secret@127.0.0.1:${proxy.address().port}`,
    OFOX_NO_PROXY: "example.invalid",
  });
  const result = await client.callTool({
    name: "web_search",
    arguments: { query: "proxied" },
  });
  assert.equal(result.isError, undefined);
  assert.deepEqual(tunnels, [`localhost:${gatewayPort}`]);
  assert.equal(gateway.requests.length, 1);
  await client.close();
  assert.equal(stderr(), "");
});
