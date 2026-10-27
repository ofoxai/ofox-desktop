// Web search through the bundled search MCP server, here a scripted stub with
// the same tools (fixtures/stub-search-mcp.mjs), started by the real pi-mcp
// client exactly as the bundle would be.
import assert from "node:assert/strict";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { AssistantRuntime } from "../src/runtime.mjs";

const token = "search-run-credential-never-persist-0123456789";
const STUB = fileURLToPath(
  new URL("./fixtures/stub-search-mcp.mjs", import.meta.url),
);
const ALLOWED_ENV = new Set([
  "OFOX_ASSISTANT_KEY",
  "OFOX_BASE_URL",
  "OFOX_ALLOW_LOCAL_GATEWAY",
  "OFOX_PROXY_URL",
  "OFOX_NO_PROXY",
  "SystemRoot",
  "TEMP",
  "TMP",
  "PATH",
]);

function sse(chunks) {
  return new Response(
    chunks.map((chunk) => `data: ${JSON.stringify(chunk)}\n\n`).join("") +
      "data: [DONE]\n\n",
    { headers: { "content-type": "text/event-stream" } },
  );
}

const reply = (text) =>
  sse([
    { choices: [{ index: 0, delta: { role: "assistant", content: text } }] },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
    },
  ]);

// Tool-call IDs are unique within a run, as a provider's are.
let lastCallId = 0;

/** One model turn that calls the given tools, as [name, args] pairs. */
const callTools = (...calls) =>
  sse([
    {
      choices: [
        {
          index: 0,
          delta: {
            role: "assistant",
            tool_calls: calls.map(([name, args], index) => ({
              index,
              id: `call-${++lastCallId}`,
              type: "function",
              function: { name, arguments: JSON.stringify(args) },
            })),
          },
        },
      ],
    },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
      usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
    },
  ]);

/** A model that answers with `turns` in order and records every request. */
function scriptedModel(...turns) {
  const requests = [];
  return {
    requests,
    fetch: async (_url, options) => {
      requests.push(JSON.parse(options.body));
      return turns[requests.length - 1]?.() ?? reply("done");
    },
  };
}

async function sandbox(t, extra = {}) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "ofox-search-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const server = path.join(directory, "server", "ofox-search-mcp.mjs");
  await mkdir(path.dirname(server));
  await copyFile(STUB, server);
  return {
    server,
    log: async () => {
      try {
        return (
          await readFile(path.join(path.dirname(server), "log.jsonl"), "utf8")
        )
          .trim()
          .split("\n")
          .map((line) => JSON.parse(line));
      } catch (error) {
        if (error.code === "ENOENT") return [];
        throw error;
      }
    },
    params: {
      sessionId: "search-session",
      runId: "search-run",
      operationId: "search-operation",
      sessionFile: path.join(directory, "session.jsonl"),
      model: {
        id: "test/search",
        supportedEndpoints: ["openai"],
        contextWindow: 32000,
        maxOutputTokens: 1024,
      },
      apiKey: token,
      apex: "ofox.io",
      prompt: "What is new?",
      webSearch: true,
      ...extra,
    },
  };
}

function runtimeFor(t, fixture, model, options = {}) {
  const events = [];
  const runtime = new AssistantRuntime({
    fetch: model.fetch,
    emit: (event) => events.push(event),
    searchServer: fixture.server,
    ...options,
  });
  t.after(() => runtime.close());
  return { runtime, events };
}

const toolEvents = (events) =>
  events
    .filter((event) => ["tool_start", "tool_end"].includes(event.type))
    .map((event) => ({ type: event.type, ...event.data }));
const calls = (log) =>
  log
    .filter((entry) => entry.method === "tools/call")
    .map((entry) => [entry.params.name, entry.params.arguments]);
const toolResults = (body) =>
  body.messages.filter((message) => message.role === "tool");

test("web tools exist only when the run enables web search, and start nothing until used", async (t) => {
  const prompts = [];
  for (const webSearch of [undefined, false, true]) {
    const fixture = await sandbox(t, { webSearch });
    const model = scriptedModel(() => reply("no search needed"));
    const { runtime } = runtimeFor(t, fixture, model);
    assert.equal((await runtime.run(fixture.params)).status, "completed");
    const [body] = model.requests;
    prompts.push(body.messages[0].content.split("\n<cwd>")[0]);
    assert.deepEqual(
      (body.tools ?? []).map((tool) => tool.function.name),
      webSearch ? ["web_search", "web_fetch"] : [],
    );
    assert.deepEqual(await fixture.log(), [], "no server without a call");
  }
  const [, chat, search] = prompts;
  assert.doesNotMatch(chat, /tool|search|web|cite|URL/i);
  assert.match(search, /Use only the provided tools\./);
  assert.match(search, /web_search searches the web/);
  assert.match(search, /web_fetch reads/);
  assert.match(search, /cite the sources/i);
});

test("a search reaches the model, its sources and usage reach the host, and the server stops with the run", async (t) => {
  const fixture = await sandbox(t);
  const model = scriptedModel(
    () => callTools(["web_search", { query: "Pi SDK" }]),
    () =>
      reply(
        "Pi is an agent toolkit [About Pi SDK](https://example.com/pi-sdk).",
      ),
  );
  const { runtime, events } = runtimeFor(t, fixture, model);
  const result = await runtime.run(fixture.params);
  assert.equal(result.status, "completed");

  const sources = [
    {
      title: "About Pi SDK",
      url: "https://example.com/pi-sdk#intro",
      snippet: "What Pi SDK is.",
    },
    { title: "example.org", url: "https://example.org/pi-sdk" },
  ];
  const usage = {
    searchCalls: 1,
    inputTokens: 120,
    outputTokens: 30,
    model: "openai/gpt-5-nano",
  };
  const toolCallId = events.find((event) => event.type === "tool_start").data
    .toolCallId;
  assert.deepEqual(toolEvents(events), [
    {
      type: "tool_start",
      name: "web_search",
      toolCallId,
      args: { query: "Pi SDK" },
    },
    {
      type: "tool_end",
      name: "web_search",
      toolCallId,
      success: true,
      details: { usage, sources },
    },
  ]);
  const [toolResult] = toolResults(model.requests[1]);
  assert.match(JSON.stringify(toolResult.content), /Summary of Pi SDK\./);

  const entry = result.entries.find((item) => item.role === "tool");
  assert.equal(entry.toolName, "web_search");
  assert.equal(entry.toolCallId, toolCallId);
  assert.equal(entry.isError, false);
  assert.deepEqual(entry.details, { usage, sources });
  assert.equal(entry.usageCoverage, "complete");
  assert.equal(result.usageCoverage, "complete");
  assert.deepEqual(
    (await runtime.load(fixture.params)).entries.find(
      (item) => item.role === "tool",
    ),
    entry,
  );

  const log = await fixture.log();
  assert.deepEqual(calls(log), [["web_search", { query: "Pi SDK" }]]);
  assert.equal(log.at(-1).event, "exit", "closed when the run finished");
  assert.equal(
    (await readFile(fixture.params.sessionFile, "utf8")).includes(token),
    false,
  );
});

test("the server gets the key and gateway only through its own environment", async (t) => {
  const proxy = {
    url: "http://user:proxy-pass@127.0.0.1:3128",
    noProxy: "localhost",
  };
  for (const extra of [
    {},
    {
      baseUrl: "http://localhost:8088",
      allowLocalGateway: true,
      apex: "ofox.ai",
      proxy,
    },
  ]) {
    const fixture = await sandbox(t, extra);
    const model = scriptedModel(() =>
      callTools(["web_search", { query: "environment" }]),
    );
    const { runtime } = runtimeFor(t, fixture, model);
    await runtime.run(fixture.params);
    const [start] = await fixture.log();
    assert.equal(start.event, "start");
    assert.deepEqual(start.argv, [fixture.server]);
    assert.equal(JSON.stringify(start.argv).includes(token), false);
    // inheritEnv: false. Only the variables Node itself may need pass on
    // (macOS adds __CF_USER_TEXT_ENCODING to every process it starts).
    for (const name of Object.keys(start.env))
      assert.ok(
        ALLOWED_ENV.has(name) || name === "__CF_USER_TEXT_ENCODING",
        `unexpected ${name}`,
      );
    if (process.env.PATH) assert.equal(start.env.PATH, process.env.PATH);
    assert.equal(start.env.OFOX_ASSISTANT_KEY, token);
    if (extra.baseUrl) {
      assert.equal(start.env.OFOX_BASE_URL, "http://localhost:8088");
      assert.equal(start.env.OFOX_ALLOW_LOCAL_GATEWAY, "1");
      assert.equal(start.env.OFOX_PROXY_URL, proxy.url);
      assert.equal(start.env.OFOX_NO_PROXY, "localhost");
    } else {
      assert.equal(start.env.OFOX_BASE_URL, "https://api.ofox.io");
      for (const name of [
        "OFOX_ALLOW_LOCAL_GATEWAY",
        "OFOX_PROXY_URL",
        "OFOX_NO_PROXY",
      ])
        assert.equal(start.env[name], undefined, name);
    }
  }
});

test("search and fetch failures reach the model as errors and the run goes on", async (t) => {
  const fixture = await sandbox(t, {
    allowedWebUrls: ["https://example.com/missing"],
  });
  const model = scriptedModel(
    () =>
      callTools(
        ["web_search", { query: "rate limited" }],
        ["web_search", { query: "rejected arguments" }],
        ["web_fetch", { url: "https://example.com/missing" }],
      ),
    () => reply("I could not search right now."),
  );
  const { runtime, events } = runtimeFor(t, fixture, model);
  const result = await runtime.run(fixture.params);
  assert.equal(result.status, "completed");
  const ends = toolEvents(events).filter((event) => event.type === "tool_end");
  assert.deepEqual(
    ends.map(({ success, details }) => ({ success, details })),
    [
      { success: false, details: { code: "search_failed", status: 429 } },
      { success: false, details: { code: "search_rejected" } },
      { success: false, details: { code: "fetch_failed", status: 404 } },
    ],
  );
  assert.match(
    ends[0].message,
    /^search_failed: The web search was rate-limited/,
  );
  assert.match(ends[1].message, /^search_rejected: .*Input validation error/);
  assert.match(ends[2].message, /^fetch_failed: Web page returned HTTP 404/);
  const results = toolResults(model.requests[1]);
  assert.equal(results.length, 3);
  assert.match(JSON.stringify(results[0].content), /rate-limited/);
  const tools = result.entries.filter((entry) => entry.role === "tool");
  assert.deepEqual(
    tools.map((entry) => [entry.toolName, entry.isError, entry.usageCoverage]),
    [
      ["web_search", true, "complete"],
      ["web_search", true, "complete"],
      ["web_fetch", true, undefined],
    ],
  );
  assert.equal(result.usageCoverage, "complete");
});

test("a search with an unknown outcome is never repeated and ends the run", async (t) => {
  const fixture = await sandbox(t);
  const model = scriptedModel(() =>
    callTools(
      ["web_search", { query: "unknown outcome" }],
      ["web_search", { query: "second search" }],
    ),
  );
  const { runtime, events } = runtimeFor(t, fixture, model);
  const result = await runtime.run(fixture.params);
  assert.equal(result.status, "error");
  assert.match(result.errorMessage, /^search_outcome_unknown: /);
  assert.equal(model.requests.length, 1, "no further model request");
  assert.deepEqual(calls(await fixture.log()), [
    ["web_search", { query: "unknown outcome" }],
  ]);
  const [end] = toolEvents(events).filter((event) => event.type === "tool_end");
  assert.equal(end.success, false);
  assert.deepEqual(end.details, { code: "search_outcome_unknown" });
  const [entry] = result.entries.filter((item) => item.role === "tool");
  assert.equal(entry.toolName, "web_search");
  assert.deepEqual(entry.details.error, { code: "search_outcome_unknown" });
  assert.equal(entry.usageCoverage, "incomplete");
  assert.equal(result.usageCoverage, "incomplete");
});

test("a search the server does not answer in time, or crashes during, has an unknown outcome", async (t) => {
  for (const query of ["hang", "crash"]) {
    const fixture = await sandbox(t);
    const model = scriptedModel(() => callTools(["web_search", { query }]));
    const { runtime, events } = runtimeFor(t, fixture, model, {
      searchTimeoutMs: 300,
    });
    const result = await runtime.run(fixture.params);
    assert.equal(result.status, "error", query);
    assert.match(result.errorMessage, /^search_outcome_unknown: /, query);
    const end = toolEvents(events).find((event) => event.type === "tool_end");
    assert.deepEqual(end.details, { code: "search_outcome_unknown" }, query);
    assert.equal(model.requests.length, 1, query);
  }
});

test("a repeated query in one run is answered once, without a second charge", async (t) => {
  const fixture = await sandbox(t);
  const model = scriptedModel(
    () =>
      callTools(
        ["web_search", { query: "Tauri release" }],
        ["web_search", { query: "  tauri   RELEASE " }],
      ),
    () => reply("Tauri 2 is out."),
  );
  const { runtime, events } = runtimeFor(t, fixture, model);
  assert.equal((await runtime.run(fixture.params)).status, "completed");
  assert.equal(calls(await fixture.log()).length, 1);
  const ends = toolEvents(events).filter((event) => event.type === "tool_end");
  assert.equal(ends[0].details.usage.searchCalls, 1);
  assert.equal(ends[1].details.deduplicated, true);
  assert.deepEqual(ends[1].details.usage, {
    searchCalls: 0,
    inputTokens: 0,
    outputTokens: 0,
    model: "openai/gpt-5-nano",
  });
  assert.deepEqual(ends[1].details.sources, ends[0].details.sources);
  const results = toolResults(model.requests[1]);
  assert.match(JSON.stringify(results[1].content), /already searched/);
});

test("web_fetch reads only pages the host allows or this run's searches returned", async (t) => {
  const fixture = await sandbox(t, {
    allowedWebUrls: ["https://Docs.Example.net/guide#install", "not a url"],
  });
  const model = scriptedModel(
    () =>
      callTools(
        ["web_fetch", { url: "https://example.com/pi-sdk" }],
        ["web_fetch", { url: "https://docs.example.net/guide" }],
      ),
    () => callTools(["web_search", { query: "Pi SDK" }]),
    () =>
      callTools(
        ["web_fetch", { url: "https://example.com/pi-sdk" }],
        ["web_fetch", { url: "https://example.com:443/pi-sdk#top" }],
        ["web_fetch", { url: "http://example.org/pi-sdk" }],
      ),
    () => reply("Read."),
  );
  const { runtime, events } = runtimeFor(t, fixture, model);
  assert.equal((await runtime.run(fixture.params)).status, "completed");
  assert.deepEqual(
    calls(await fixture.log()).map(
      ([name, args]) => `${name} ${args.url ?? args.query}`,
    ),
    [
      "web_fetch https://docs.example.net/guide",
      "web_search Pi SDK",
      "web_fetch https://example.com/pi-sdk",
      "web_fetch https://example.com:443/pi-sdk#top",
    ],
  );
  const fetches = toolEvents(events).filter(
    (event) => event.type === "tool_end" && event.name === "web_fetch",
  );
  assert.deepEqual(
    fetches.map((event) => event.success),
    [false, true, true, true, false],
  );
  assert.deepEqual(fetches[0].details, { code: "fetch_not_allowed" });
  assert.match(fetches[0].message, /^fetch_not_allowed: /);
  assert.deepEqual(fetches[1].details, {
    url: "https://docs.example.net/guide",
    truncated: false,
  });
});

test("stopping a run during a search ends it as stopped and stops the server", async (t) => {
  const fixture = await sandbox(t);
  const model = scriptedModel(() =>
    callTools(["web_search", { query: "hang" }]),
  );
  const { runtime, events } = runtimeFor(t, fixture, model);
  const running = runtime.run(fixture.params);
  for (let waited = 0; waited < 100; waited++) {
    if (calls(await fixture.log()).length) break;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.deepEqual(await runtime.stop(fixture.params), { stopped: true });
  const result = await running;
  assert.equal(result.status, "stopped");
  const end = toolEvents(events).find((event) => event.type === "tool_end");
  assert.equal(end.success, false);
  assert.deepEqual(end.details, { code: "search_outcome_unknown" });
  assert.equal((await fixture.log()).at(-1).event, "exit");
});

test("a search server that cannot start fails the call without an unknown outcome", async (t) => {
  const fixture = await sandbox(t);
  const refusing = path.join(path.dirname(fixture.server), "refusing.mjs");
  await writeFile(
    refusing,
    'process.stderr.write("ofox-search-mcp: OFOX_BASE_URL must be exactly https://api.ofox.ai or https://api.ofox.io\\n"); process.exit(2);',
  );
  for (const [script, reason] of [
    [path.join(path.dirname(fixture.server), "missing.mjs"), /unavailable$/],
    [refusing, /unavailable \(OFOX_BASE_URL must be exactly/],
  ]) {
    const model = scriptedModel(
      () => callTools(["web_search", { query: "anything" }]),
      () => reply("Search is unavailable."),
    );
    const { runtime, events } = runtimeFor(t, fixture, model, {
      searchServer: script,
    });
    const result = await runtime.run(fixture.params);
    assert.equal(result.status, "completed");
    const end = toolEvents(events).find((event) => event.type === "tool_end");
    assert.equal(end.success, false);
    assert.deepEqual(end.details, { code: "search_unavailable" });
    assert.match(end.message, /^search_unavailable: /);
    assert.match(end.message, reason);
    assert.equal(model.requests.length, 2);
  }
});

test("a malformed web URL allowlist rejects the run before any request", async (t) => {
  for (const allowedWebUrls of [
    "https://example.com/",
    [42],
    ["https://example.com/" + "a".repeat(2048)],
    Array.from({ length: 501 }, (_, index) => `https://example.com/${index}`),
  ]) {
    const fixture = await sandbox(t, { allowedWebUrls });
    const model = scriptedModel();
    const { runtime } = runtimeFor(t, fixture, model);
    await assert.rejects(runtime.run(fixture.params), /web URL allowlist/);
    assert.equal(model.requests.length, 0);
  }
});
