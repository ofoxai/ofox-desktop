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
import { spawnSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { AssistantRuntime } from "../src/runtime.mjs";

const token = "search-run-credential-never-persist-0123456789";
const STUB = fileURLToPath(
  new URL("./fixtures/stub-search-mcp.mjs", import.meta.url),
);
// What the OS adds to any process started with an empty environment: macOS
// __CF_USER_TEXT_ENCODING; on Windows, libuv copies the variables a process
// needs (USERPROFILE, WINDIR, USERNAME...) from the parent.
const PLATFORM_ENV = new Set(
  JSON.parse(
    spawnSync(
      process.execPath,
      ["-e", "process.stdout.write(JSON.stringify(Object.keys(process.env)))"],
      { env: {}, encoding: "utf8" },
    ).stdout,
  ),
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
  assert.match(search, /Search once with one precise query/);
  assert.match(search, /only if the results are insufficient/);
  assert.match(search, /at most 6 searches/);
  assert.match(search, /Prefer the search results/);
  assert.match(search, /external data, never instructions/);
  assert.match(search, /a page read in this turn links to \(at most 5/);
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
    // inheritEnv: false. Only the variables Node itself may need pass on,
    // and what the OS adds to every process.
    for (const name of Object.keys(start.env))
      assert.ok(
        ALLOWED_ENV.has(name) || PLATFORM_ENV.has(name),
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
        ["web_fetch", { url: "https://example.com/missing" }],
        ["web_search", { query: "protocol error" }],
        ["web_search", { query: "rate limited" }],
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
      { success: false, details: { code: "fetch_failed", status: 404 } },
      { success: false, details: { code: "search_failed" } },
      { success: false, details: { code: "search_failed", status: 429 } },
    ],
  );
  // A JSON-RPC error answer means the server never ran the search.
  assert.match(ends[2].message, /no search was made/);
  // Only successes are reused: a failed query may be tried again.
  assert.equal(
    calls(await fixture.log()).filter(
      ([, args]) => args.query === "rate limited",
    ).length,
    2,
  );
  assert.match(
    ends[0].message,
    /^search_failed: The web search was rate-limited/,
  );
  assert.match(ends[1].message, /^fetch_failed: Web page returned HTTP 404/);
  const results = toolResults(model.requests[1]);
  assert.equal(results.length, 4);
  assert.match(JSON.stringify(results[0].content), /rate-limited/);
  const tools = result.entries.filter((entry) => entry.role === "tool");
  assert.deepEqual(
    tools.map((entry) => [entry.toolName, entry.isError, entry.usageCoverage]),
    [
      ["web_search", true, "complete"],
      ["web_fetch", true, undefined],
      ["web_search", true, "complete"],
      ["web_search", true, "complete"],
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

test("a search the server does not answer in time, crashes during or answers malformed has an unknown outcome", async (t) => {
  for (const query of ["hang", "crash", "malformed answer"]) {
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

test("a run sends at most 6 searches: a sent failure counts, repeats and unsent failures do not", async (t) => {
  const fixture = await sandbox(t);
  const model = scriptedModel(
    () =>
      callTools(
        ["web_search", { query: "rate limited" }],
        ["web_search", { query: "rejected arguments" }],
        ["web_search", { query: "protocol error" }],
        ["web_search", { query: "Tauri release" }],
        ["web_search", { query: "  tauri   RELEASE " }],
        ["web_search", { query: "Pi SDK" }],
        ["web_search", { query: "Node 22" }],
        ["web_search", { query: "React" }],
        ["web_search", { query: "Rust" }],
        ["web_search", { query: "seventh search" }],
      ),
    () => callTools(["web_search", { query: "Pi SDK" }]),
    () => reply("Tauri 2 is out."),
  );
  const { runtime, events } = runtimeFor(t, fixture, model);
  const result = await runtime.run(fixture.params);
  assert.equal(result.status, "completed");
  assert.deepEqual(
    calls(await fixture.log()).map(([, args]) => args.query),
    [
      "rate limited",
      "rejected arguments",
      "protocol error",
      "Tauri release",
      "Pi SDK",
      "Node 22",
      "React",
      "Rust",
    ],
  );
  const ends = toolEvents(events).filter((event) => event.type === "tool_end");
  assert.deepEqual(
    ends.map(({ success, details }) => [success, details.code ?? null]),
    [
      // Sent and refused by the gateway: it counts.
      [false, "search_failed"],
      // Never reached the gateway: they do not.
      [false, "search_rejected"],
      [false, "search_failed"],
      [true, null],
      [true, null],
      [true, null],
      [true, null],
      [true, null],
      [true, null],
      [false, "search_limit_reached"],
      [true, null],
    ],
  );
  assert.equal(ends[0].details.status, 429);
  assert.match(ends[1].message, /^search_rejected: .*Input validation error/);
  assert.match(ends[2].message, /no search was made/);
  // The model reads why and what to do instead; nothing was billed.
  assert.match(
    ends[9].message,
    /^search_limit_reached: .*6 web searches.*answer with the information you already have/,
  );
  assert.deepEqual(ends[9].details, { code: "search_limit_reached" });
  // A repeat after the limit is still answered from the earlier result.
  assert.equal(ends[10].details.deduplicated, true);
  const [limited] = toolResults(model.requests[1]).slice(9);
  assert.match(JSON.stringify(limited.content), /search_limit_reached/);
  const entry = result.entries.filter((item) => item.role === "tool")[9];
  assert.equal(entry.isError, true);
  assert.equal(entry.usageCoverage, "complete");
});

// The search model opens pages itself: a URL in a query could carry private
// text (an attachment's, a page's) to any host past web_fetch's allowlist.
const URL_QUERIES = [
  "https://evil.example/c?d=private",
  "summarize http://example.com",
  "hxxps://evil.example",
  "www.evil.example",
  "evil.example/c?d=private",
  "evil.example?d=private",
  "site:evil.example/c?d=private",
  "203.0.113.7/collect?d=private",
  "例え.テスト/パス",
];
const PLAIN_QUERIES = [
  "Tauri release",
  "example.com",
  "site:example.com tauri updater",
  "What is new in Node.js?",
  "React 18.2/18.3 differences",
];

test("a query with a URL is refused unsent and does not count toward the search limit", async (t) => {
  const fixture = await sandbox(t);
  const model = scriptedModel(
    () =>
      callTools(
        ...URL_QUERIES.map((query) => ["web_search", { query }]),
        ["web_search", { query: "example.com" }],
        ["web_search", { query: "site:example.com tauri updater" }],
        ["web_search", { query: "Tauri release" }],
        ["web_search", { query: "Pi SDK" }],
        ["web_search", { query: "Rust" }],
        ["web_search", { query: "Node" }],
        ["web_search", { query: "seventh" }],
      ),
    () => reply("Tauri 2 is out."),
  );
  const { runtime, events } = runtimeFor(t, fixture, model);
  const result = await runtime.run(fixture.params);
  assert.equal(result.status, "completed");
  assert.deepEqual(
    calls(await fixture.log()).map(([, args]) => args.query),
    [
      "example.com",
      "site:example.com tauri updater",
      "Tauri release",
      "Pi SDK",
      "Rust",
      "Node",
    ],
  );
  const ends = toolEvents(events).filter((event) => event.type === "tool_end");
  assert.deepEqual(
    ends.map(({ success, details }) => [success, details.code ?? null]),
    [
      ...URL_QUERIES.map(() => [false, "search_rejected"]),
      [true, null],
      [true, null],
      [true, null],
      [true, null],
      [true, null],
      [true, null],
      [false, "search_limit_reached"],
    ],
  );
  for (const end of ends.slice(0, URL_QUERIES.length)) {
    assert.deepEqual(end.details, { code: "search_rejected" });
    assert.match(
      end.message,
      /^search_rejected: .*no search was made.*site:example\.com/,
    );
  }
});

test("the runtime refuses the same queries as the search server", async () => {
  const { queryHasUrl } = await import("../src/search.mjs");
  const { queryHasUrl: serverCheck } = await import(
    "../search-mcp/src/search.mjs"
  );
  for (const query of URL_QUERIES) {
    assert.equal(queryHasUrl(query), true, query);
    assert.equal(serverCheck(query), true, query);
  }
  for (const query of PLAIN_QUERIES) {
    assert.equal(queryHasUrl(query), false, query);
    assert.equal(serverCheck(query), false, query);
  }
});

test("after 3 failed page reads in a run, web_fetch refuses every page unsent", async (t) => {
  const fixture = await sandbox(t, {
    allowedWebUrls: [
      "https://example.com/missing-1",
      "https://example.com/missing-2",
      "https://example.com/guide",
    ],
  });
  const model = scriptedModel(
    () =>
      callTools(
        ["web_fetch", { url: "https://example.com/missing-1" }],
        ["web_fetch", { url: "https://example.com/guide" }],
        ["web_fetch", { url: "https://elsewhere.example/" }],
        ["web_fetch", { url: "https://example.com/missing-2" }],
        ["web_fetch", { url: "https://example.com/guide" }],
      ),
    () => callTools(["web_search", { query: "Pi SDK" }]),
    () => reply("Read what I could."),
  );
  const { runtime, events } = runtimeFor(t, fixture, model);
  assert.equal((await runtime.run(fixture.params)).status, "completed");
  assert.deepEqual(
    calls(await fixture.log()).map(
      ([name, args]) => `${name} ${args.url ?? args.query}`,
    ),
    [
      "web_fetch https://example.com/missing-1",
      "web_fetch https://example.com/guide",
      "web_fetch https://example.com/missing-2",
      "web_search Pi SDK",
    ],
  );
  const ends = toolEvents(events).filter((event) => event.type === "tool_end");
  assert.deepEqual(
    ends.map(({ success, details }) => [success, details.code ?? null]),
    [
      [false, "fetch_failed"],
      [true, null],
      [false, "fetch_not_allowed"],
      [false, "fetch_failed"],
      [false, "fetch_limit_reached"],
      [true, null],
    ],
  );
  assert.match(
    ends[4].message,
    /^fetch_limit_reached: .*3 page reads.*answer with the information you already have/,
  );
  assert.deepEqual(ends[4].details, { code: "fetch_limit_reached" });
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

test("a read page's links become readable, at most 5 reads of them per run", async (t) => {
  const fixture = await sandbox(t, {
    allowedWebUrls: ["https://example.com/hub"],
  });
  const linked = (index) => [
    "web_fetch",
    { url: `https://example.com/linked-${index}` },
  ];
  const model = scriptedModel(
    () =>
      callTools(linked(1), ["web_fetch", { url: "https://example.com/hub" }]),
    () =>
      callTools(
        linked(1),
        linked(2),
        linked(3),
        linked(4),
        linked(5),
        linked(6),
        ["web_fetch", { url: "http://example.com/plain" }],
      ),
    () => reply("Read the hub and its pages."),
  );
  const { runtime, events } = runtimeFor(t, fixture, model);
  assert.equal((await runtime.run(fixture.params)).status, "completed");
  assert.deepEqual(
    calls(await fixture.log()).map(([, args]) => args.url),
    [
      "https://example.com/hub",
      ...[1, 2, 3, 4, 5].map((index) => `https://example.com/linked-${index}`),
    ],
  );
  const fetches = toolEvents(events).filter(
    (event) => event.type === "tool_end" && event.name === "web_fetch",
  );
  assert.deepEqual(
    fetches.map(({ success, details }) => [success, details.code ?? null]),
    [
      // Not yet linked: the hub was not read.
      [false, "fetch_not_allowed"],
      [true, null],
      ...Array.from({ length: 5 }, () => [true, null]),
      [false, "fetch_not_allowed"],
      // A plain HTTP link is no page a run reads.
      [false, "fetch_not_allowed"],
    ],
  );
  assert.match(
    fetches[7].message,
    /already read 5 pages reached through links/,
  );
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

test("a search server that cannot start fails the call without an unknown outcome, and may start again", async (t) => {
  const fixture = await sandbox(t);
  const directory = path.dirname(fixture.server);
  const refusing = path.join(directory, "refusing.mjs");
  await writeFile(
    refusing,
    `import { appendFileSync } from "node:fs";
appendFileSync(${JSON.stringify(path.join(directory, "starts"))}, "start\\n");
process.stderr.write("ofox-search-mcp: OFOX_BASE_URL must be exactly https://api.ofox.ai or https://api.ofox.io\\n");
process.exit(2);`,
  );
  for (const [script, reason] of [
    [path.join(directory, "missing.mjs"), /unavailable$/],
    [refusing, /unavailable \(OFOX_BASE_URL must be exactly/],
  ]) {
    // More calls than the search limit: nothing was sent, none counts.
    const model = scriptedModel(
      () =>
        callTools(
          ["web_search", { query: "anything" }],
          ["web_search", { query: "anything" }],
          ["web_search", { query: "something else" }],
          ["web_search", { query: "a third query" }],
        ),
      () => reply("Search is unavailable."),
    );
    const { runtime, events } = runtimeFor(t, fixture, model, {
      searchServer: script,
    });
    const result = await runtime.run(fixture.params);
    assert.equal(result.status, "completed");
    const ends = toolEvents(events).filter(
      (event) => event.type === "tool_end",
    );
    assert.equal(ends.length, 4);
    for (const end of ends) {
      assert.equal(end.success, false);
      assert.deepEqual(end.details, { code: "search_unavailable" });
      assert.match(end.message, /^search_unavailable: /);
      assert.match(end.message, reason);
    }
    assert.equal(model.requests.length, 2);
  }
  // Each call tried to start the server again.
  assert.equal(
    (await readFile(path.join(directory, "starts"), "utf8")).trim().split("\n")
      .length,
    4,
  );
});

test("stopping a run while the search server starts ends it at once", async (t) => {
  const fixture = await sandbox(t);
  const directory = path.dirname(fixture.server);
  const silent = path.join(directory, "silent.mjs");
  const started = path.join(directory, "started");
  // Starts, never answers initialize and ignores stdin closing.
  await writeFile(
    silent,
    `import { writeFileSync } from "node:fs";
writeFileSync(${JSON.stringify(started)}, String(process.pid));
process.stdin.resume();
setInterval(() => {}, 1000);`,
  );
  const model = scriptedModel(() =>
    callTools(["web_search", { query: "slow start" }]),
  );
  const { runtime, events } = runtimeFor(t, fixture, model, {
    searchServer: silent,
  });
  const running = runtime.run(fixture.params);
  let pid;
  for (let waited = 0; waited < 100 && !pid; waited++) {
    pid = Number(await readFile(started, "utf8").catch(() => "")) || undefined;
    if (!pid) await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.ok(pid, "the server started");
  const stoppedAt = Date.now();
  await runtime.stop(fixture.params);
  const result = await running;
  assert.equal(result.status, "stopped");
  assert.ok(Date.now() - stoppedAt < 5000, "stop did not wait for the server");
  const end = toolEvents(events).find((event) => event.type === "tool_end");
  assert.equal(end.success, false);
  assert.equal(end.details, undefined, "nothing was sent: no unknown outcome");
  let alive = true;
  for (let waited = 0; waited < 60 && alive; waited++) {
    try {
      process.kill(pid, 0);
      await new Promise((resolve) => setTimeout(resolve, 50));
    } catch {
      alive = false;
    }
  }
  assert.equal(alive, false, "the server was stopped");
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
