import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, readFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "node:http";
import { StdioTransport } from "@earendil-works/pi-mcp";
import { AssistantRuntime } from "../src/runtime.mjs";
const limits = {
  repeatWarning: 3,
  repeatStop: 5,
  maxDurationSeconds: 900,
  maxCostUsd: 1,
  maxToolCalls: 60,
  completionRetries: 2,
};
const sse = (chunks) =>
  new Response(
    chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`).join("") +
      "data: [DONE]\n\n",
    { headers: { "content-type": "text/event-stream" } },
  );
let id = 0;
const call = (name, args) =>
  sse([
    {
      choices: [
        {
          index: 0,
          delta: {
            role: "assistant",
            tool_calls: [
              {
                index: 0,
                id: `local-${++id}`,
                type: "function",
                function: { name, arguments: JSON.stringify(args) },
              },
            ],
          },
        },
      ],
    },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
      usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
    },
  ]);
const reply = (
  usage = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
) =>
  sse([
    {
      choices: [
        { index: 0, delta: { role: "assistant", content: "Finished." } },
      ],
    },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage,
    },
  ]);
async function fixture(t, turns, host, extra = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), "ofox-mcp-runtime-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const requests = [],
    events = [],
    calls = [];
  const runtime = new AssistantRuntime({
    retrySleep: async () => {},
    fetch: async (url, options) => {
      requests.push(JSON.parse(options.body));
      return turns[requests.length - 1]?.() ?? reply();
    },
    emit: (e) => events.push(e),
    hostCall: async (c) => {
      calls.push(c);
      return host(c.name, c.args);
    },
  });
  t.after(() => runtime.close());
  const params = {
    sessionId: "local-session",
    runId: "local-run",
    operationId: "local-operation",
    sessionFile: path.join(root, "session.jsonl"),
    model: {
      id: "test/local",
      pricingKnown: true,
      cost: { input: 1, output: 1, cacheRead: 0, cacheWrite: 0 },
      supportedEndpoints: ["openai"],
      contextWindow: 32000,
      maxOutputTokens: 1024,
    },
    apiKey: "local-test-key-never-persist",
    apex: "ofox.io",
    prompt: "Fix the tool",
    localConfig: {
      cwd: root,
      platform: process.platform === "win32" ? "windows" : "macos",
      skillPath: fileURLToPath(
        new URL("../skills/local-config/SKILL.md", import.meta.url),
      ),
      definition: { limits },
    },
    ...extra,
  };
  return {
    root,
    runtime,
    requests,
    events,
    calls,
    params,
    run: () => runtime.run(params),
  };
}

test("real Pi discovers deferred MCP and calls it through host admission", async (t) => {
  const f = await fixture(
    t,
    [
      () => call("tool_search", { query: "fixture" }),
      () => call("mcp__fixture__read_fixture", {}),
      reply,
    ],
    () => ({ approved: true }),
    { localConfig: undefined },
  );
  f.params.mcp = {
    servers: {
      fixture: {
        command: process.execPath,
        args: [
          fileURLToPath(new URL("./fixtures/mcp-fixture.mjs", import.meta.url)),
        ],
      },
    },
    uvAvailable: false,
  };
  const result = await f.run();
  assert.equal(result.status, "completed");
  assert.ok(
    (f.requests[0].tools ?? []).some((t) => t.function.name === "tool_search"),
  );
  assert.ok(
    !(f.requests[0].tools ?? []).some((t) =>
      t.function.name.startsWith("mcp__"),
    ),
  );
  assert.ok(
    !(f.requests[0].tools ?? []).some((t) => t.function.name === "codemode"),
  );
  assert.ok(
    (f.requests[1].tools ?? []).some(
      (t) => t.function.name === "mcp__fixture__read_fixture",
    ),
  );
  assert.ok(
    f.calls.some(
      (c) =>
        c.name === "authorize_mcp" &&
        c.args.toolName === "mcp__fixture__read_fixture" &&
        c.args.readOnly === true,
    ),
  );
  assert.match(JSON.stringify(result), /Fixture worked|Finished/);
  const config = JSON.parse(
    await readFile(path.join(f.root, "runtime-settings/mcp.json"), "utf8"),
  );
  assert.equal(config.mcpServers.fixture.exposure, "deferred");
});

test("missing server does not prevent reply and reports unavailable", async (t) => {
  const f = await fixture(
    t,
    [() => call("tool_search", { query: "missing" }), reply],
    () => ({ approved: true }),
    {
      localConfig: undefined,
      mcp: { servers: { missing: { command: "/ofox/no-such-mcp-command" } } },
    },
  );
  assert.equal((await f.run()).status, "completed");
  assert.ok(
    f.events.some(
      (e) => e.type === "mcp_status" && e.data.reason === "unavailable",
    ),
  );
});
test("MCP configuration filters missing runners, replaces stale files, and preserves explicit registry", async (t) => {
  const { writeSessionMcpConfig } = await import("../src/mcp-config.mjs");
  const root = await mkdtemp(path.join(os.tmpdir(), "ofox-mcp-config-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const unavailable = await writeSessionMcpConfig(root, {
    servers: { node: { command: "npx" }, uv: { command: "uvx" } },
  });
  assert.deepEqual(unavailable.unavailable, [
    { name: "node", reason: "needs_node" },
    { name: "uv", reason: "needs_uv" },
  ]);
  const configured = await writeSessionMcpConfig(root, {
    servers: {
      node: {
        command: "npx",
        env: { npm_config_registry: "https://explicit.invalid" },
      },
    },
    npxPath: "/fixture/npx",
    npmRegistry: "https://default.invalid",
  });
  assert.equal(configured.servers.node.command, "/fixture/npx");
  assert.equal(
    configured.servers.node.env.npm_config_registry,
    "https://explicit.invalid",
  );
  assert.equal(configured.servers.node.exposure, "deferred");
  const partial = await writeSessionMcpConfig(root, {
    servers: {
      missing: { unavailable: true, identity: "captured-config" },
      public: { url: "https://example.test/mcp" },
    },
  });
  assert.deepEqual(partial.written, ["public"]);
  assert.deepEqual(partial.unavailable, [
    { name: "missing", reason: "unavailable" },
  ]);
  assert.ok(
    !(await readFile(path.join(root, "mcp.json"), "utf8")).includes(
      "captured-config",
    ),
  );
  const named = await writeSessionMcpConfig(root, {
    servers: JSON.parse('{"__proto__":{"command":"node"}}'),
  });
  assert.deepEqual(named.written, ["__proto__"]);
  await writeSessionMcpConfig(root, null);
  assert.deepEqual(
    JSON.parse(await readFile(path.join(root, "mcp.json"), "utf8")).mcpServers,
    {},
  );
});
test("source boundary scrubs short/long secrets and decoded text blobs, rejects identifiers and opaque binary", async () => {
  const { mcpScrubber, sanitizeMcpMessage } = await import("../src/mcp.mjs");
  const scrub = mcpScrubber(["abc-secret", "abc", "短密"]);
  const safe = sanitizeMcpMessage(
    {
      jsonrpc: "2.0",
      id: 1,
      result: {
        content: [
          { type: "text", text: "abc-secret abc 短密" },
          { type: "image", data: "YWJj", mimeType: "image/png" },
          {
            type: "resource",
            resource: {
              uri: "fixture://text",
              blob: Buffer.from("abc-secret").toString("base64"),
            },
          },
        ],
      },
    },
    scrub,
  );
  assert.equal(safe.result.content[0].text, "*** *** ***");
  assert.match(safe.result.content[1].text, /unsupported_binary/);
  assert.equal(safe.result.content[2].resource.text, "***");
  assert.throws(
    () =>
      sanitizeMcpMessage({ result: { tools: [{ name: "abc_tool" }] } }, scrub),
    /identifier/,
  );
});

test("real MCP clears protocol logs and SDK large-output files before persistence and drains split UTF8 stderr", async (t) => {
  const { stat } = await import("node:fs/promises");
  const f = await fixture(
    t,
    [
      () => call("tool_search", { query: "fixture" }),
      () => call("mcp__fixture__read_fixture", {}),
      reply,
    ],
    () => ({ approved: true }),
    { localConfig: undefined },
  );
  f.params.mcp = {
    servers: {
      fixture: {
        command: process.execPath,
        args: [
          fileURLToPath(new URL("./fixtures/mcp-fixture.mjs", import.meta.url)),
        ],
        env: {
          MCP_SCENARIO: "secrets",
          API_KEY: "sk-test-123",
          SHORT_SECRET: "短密",
        },
      },
    },
  };
  const result = await f.run();
  assert.equal(result.status, "completed");
  const log = await readFile(
    path.join(f.root, "runtime-settings/mcp.log"),
    "utf8",
  );
  assert.match(log, /Readable notification \*\*\* \*\*\*/);
  const transcript = await readFile(f.params.sessionFile, "utf8");
  const paths = [
    ...transcript.matchAll(/Full output: ([^ ]+pi-mcp-[^ ]+\.txt)/g),
  ].map((m) => m[1]);
  assert.ok(paths.length > 0, "SDK actually wrote a large result");
  const large = await readFile(paths[0], "utf8");
  t.after(() =>
    Promise.all([...new Set(paths)].map((file) => rm(file, { force: true }))),
  );
  assert.match(large, /Readable result \*\*\* \*\*\*/);
  assert.match(large, /unsupported_binary/);
  for (const content of [
    log,
    transcript,
    large,
    JSON.stringify(f.events),
    JSON.stringify(f.requests),
  ]) {
    assert.ok(!content.includes("sk-test-123"));
    assert.ok(!content.includes("短密"));
  }
  assert.equal((await stat(paths[0])).mode & 0o777, 0o600);
});

test("normal and cancelled pending MCP processes are reaped", async (t) => {
  const unsafeClosePids = [],
    negativeGroupSignals = [];
  const originalClose = StdioTransport.prototype.close;
  t.mock.method(StdioTransport.prototype, "close", function () {
    if (this.pid !== undefined) unsafeClosePids.push(this.pid);
    assert.equal(
      this.pid,
      undefined,
      "official close must only run after public onClose clears the child, on every platform",
    );
    return originalClose.call(this);
  });
  const kill = process.kill.bind(process);
  t.mock.method(process, "kill", (pid, ...args) => {
    if (pid < 0) negativeGroupSignals.push(pid);
    assert.ok(
      pid >= 0,
      "the parent must never signal a cached process group after launcher exit",
    );
    return kill(pid, ...args);
  });
  for (const scenario of ["normal", "hang", "fail"]) {
    const f = await fixture(
      t,
      [
        () => call("tool_search", { query: "fixture" }),
        () => call("mcp__fixture__read_fixture", {}),
        reply,
      ],
      () => ({ approved: true }),
      { localConfig: undefined },
    );
    const pidFile = path.join(f.root, "pid");
    f.params.mcp = {
      servers: {
        fixture: {
          command: process.execPath,
          args: [
            fileURLToPath(
              new URL("./fixtures/mcp-fixture.mjs", import.meta.url),
            ),
          ],
          env: {
            MCP_PID_FILE: pidFile,
            // Windows descendants belong to the real Host JobObject; that
            // cross-platform integration fixture covers the whole run tree.
            ...(process.platform !== "win32"
              ? { MCP_CHILD_FILE: path.join(f.root, "child-pid") }
              : {}),
            MCP_SCENARIO: scenario,
          },
        },
      },
    };
    const running = f.run();
    let pid;
    for (let i = 0; i < 100; i++) {
      try {
        pid = Number(await readFile(pidFile, "utf8"));
        break;
      } catch {
        await new Promise((r) => setTimeout(r, 10));
      }
    }
    assert.ok(pid, "real subprocess started");
    if (scenario === "hang") await f.runtime.stop(f.params);
    await running;
    assert.deepEqual(
      unsafeClosePids,
      [],
      "no SDK close entered with a captured child PID",
    );
    assert.deepEqual(
      negativeGroupSignals,
      [],
      "no SDK cached group signal was attempted",
    );
    if (process.platform !== "win32") {
      const childPid = Number(
        await readFile(path.join(f.root, "child-pid"), "utf8"),
      );
      assert.throws(
        () => process.kill(childPid, 0),
        (error) => error.code === "ESRCH",
      );
    }
    assert.throws(
      () => process.kill(pid, 0),
      (error) => error.code === "ESRCH",
    );
  }
});

test("deferred search and resource calls share the same admission and exact tool count", async (t) => {
  const f = await fixture(
    t,
    [
      () => call("tool_search", { query: "resources" }),
      () => call("list_mcp_resources", {}),
      () => call("list_mcp_resource_templates", {}),
      reply,
    ],
    () => ({ approved: true }),
    { localConfig: undefined },
  );
  f.params.mcp = {
    limits,
    servers: {
      fixture: {
        command: process.execPath,
        args: [
          fileURLToPath(new URL("./fixtures/mcp-fixture.mjs", import.meta.url)),
        ],
      },
    },
  };
  assert.equal((await f.run()).status, "completed");
  assert.deepEqual(
    f.calls.map((c) => c.args.toolName),
    ["tool_search", "list_mcp_resources", "list_mcp_resource_templates"],
  );
  assert.equal(f.events.filter((e) => e.type === "tool_start").length, 3);
});

test("unknown OFox model cost stops MCP while known model usage permits external tools", async (t) => {
  const f = await fixture(
    t,
    [() => call("tool_search", { query: "fixture" }), reply],
    () => ({ approved: true }),
    { localConfig: undefined },
  );
  f.params.mcp = {
    limits,
    servers: {
      fixture: {
        command: process.execPath,
        args: [
          fileURLToPath(new URL("./fixtures/mcp-fixture.mjs", import.meta.url)),
        ],
      },
    },
  };
  f.params.model.pricingKnown = false;
  f.params.model.cost = {};
  const result = await f.run();
  assert.equal(f.calls.length, 0);
  assert.match(JSON.stringify(result), /assistant_cost_unavailable/);
});

test("MCP repeats and configured tool limits stop before extra host admission", async (t) => {
  const f = await fixture(
    t,
    [
      () => call("tool_search", { query: "fixture" }),
      ...Array.from(
        { length: 6 },
        () => () => call("mcp__fixture__read_fixture", {}),
      ),
      reply,
    ],
    () => ({ approved: true }),
    { localConfig: undefined },
  );
  f.params.mcp = {
    limits: { ...limits, maxToolCalls: 4 },
    servers: {
      fixture: {
        command: process.execPath,
        args: [
          fileURLToPath(new URL("./fixtures/mcp-fixture.mjs", import.meta.url)),
        ],
      },
    },
  };
  const result = await f.run();
  assert.equal(f.calls.length, 4);
  assert.match(JSON.stringify(result), /assistant_tool_limit/);
});

test("HTTP authentication stays on the transport and remote failures remain safe and readable", async (t) => {
  for (const denied of [false, true]) {
    const secret = "Bearer local-http-secret";
    const received = [];
    const server = createServer(async (req, res) => {
      received.push(req.headers.authorization);
      if (denied) {
        res.writeHead(401, {
          "www-authenticate":
            'Bearer resource_metadata="https://invalid.example/oauth"',
        });
        res.end(`Sign-in required ${secret}`);
        return;
      }
      if (req.method !== "POST") {
        res.writeHead(405).end();
        return;
      }
      let body = "";
      for await (const chunk of req) body += chunk;
      const q = JSON.parse(body);
      if (q.id === undefined) {
        res.writeHead(202).end();
        return;
      }
      const result =
        q.method === "initialize"
          ? {
              protocolVersion: "2025-11-25",
              capabilities: { tools: {} },
              serverInfo: { name: "http-fixture", version: "1" },
            }
          : q.method === "tools/list"
            ? {
                tools: [
                  {
                    name: "read",
                    description: "HTTP fixture read",
                    inputSchema: { type: "object", properties: {} },
                    annotations: { readOnlyHint: true },
                  },
                ],
              }
            : {
                content: [
                  {
                    type: "text",
                    text: `Readable HTTP result ${secret} token ${secret.slice(7)}`,
                  },
                ],
              };
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ jsonrpc: "2.0", id: q.id, result }));
    });
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    t.after(
      () =>
        new Promise((resolve) => {
          server.closeAllConnections();
          server.close(resolve);
        }),
    );
    const f = await fixture(
      t,
      [
        () => call("tool_search", { query: "remote" }),
        ...(denied ? [] : [() => call("mcp__remote__read", {})]),
        reply,
      ],
      () => ({ approved: true }),
      { localConfig: undefined },
    );
    f.params.mcp = {
      limits,
      servers: {
        remote: {
          url: `http://127.0.0.1:${server.address().port}/mcp`,
          headers: { Authorization: secret },
        },
      },
    };
    assert.equal((await f.run()).status, "completed");
    assert.ok(received.length > 0);
    assert.ok(received.every((value) => value === secret));
    const outputs = JSON.stringify([
      f.events,
      f.requests,
      await readFile(f.params.sessionFile, "utf8"),
      await readFile(path.join(f.root, "runtime-settings/mcp.log"), "utf8"),
    ]);
    assert.ok(!outputs.includes(secret));
    assert.ok(
      !outputs.includes(secret.slice(7)),
      "bare authentication token must be scrubbed too",
    );
    if (denied) assert.ok(f.events.some((e) => e.type === "mcp_status"));
    else assert.match(outputs, /Readable HTTP result \*\*\*/);
  }
});

test("close waits pending setup, is idempotent, and prevents later MCP spawn", async (t) => {
  const f = await fixture(t, [reply], () => ({ approved: true }), {
    localConfig: undefined,
  });
  const marker = path.join(f.root, "server-pid");
  f.params.mcp = {
    servers: {
      fixture: {
        command: process.execPath,
        args: [
          fileURLToPath(new URL("./fixtures/mcp-fixture.mjs", import.meta.url)),
        ],
        env: { MCP_PID_FILE: marker },
      },
    },
  };
  const running = f.run();
  const context = f.runtime.runs.get(f.params.runId);
  assert.ok(context, "real run is pending before SDK/session setup");
  const first = f.runtime.close();
  assert.equal(f.runtime.close(), first);
  assert.equal((await running).status, "stopped");
  assert.equal(await first, true);
  await context.done;
  assert.equal(f.runtime.runs.size, 0);
  assert.equal(f.requests.length, 0);
  await assert.rejects(readFile(marker), { code: "ENOENT" });
  await assert.rejects(f.run(), /runtime is closed/);
});

test("no-local production MCP limits keep exactly twenty mixed wrapped calls", async (t) => {
  for (const maxToolCalls of [60, 20]) {
    const turns = [() => call("tool_search", { query: "fixture" })];
    for (let index = 2; index <= 21; index++)
      turns.push(() =>
        index % 5 === 0
          ? call("use_skill", { name: "fixture-skill" })
          : call("mcp__fixture__read_fixture", { index }),
      );
    turns.push(reply);
    const f = await fixture(
      t,
      turns,
      () => ({ approved: true, body: "Readable skill", files: [] }),
      {
        localConfig: undefined,
        skills: [
          {
            name: "fixture-skill",
            description: "Fixture",
            dir: "/fixture",
            mode: "auto",
          },
        ],
      },
    );
    f.params.mcp = {
      servers: {
        fixture: {
          command: process.execPath,
          args: [
            fileURLToPath(
              new URL("./fixtures/mcp-fixture.mjs", import.meta.url),
            ),
          ],
        },
      },
      limits: { ...limits, maxToolCalls },
      snapshot: "host-snapshot",
      npxPath: null,
      uvxPath: null,
      uvAvailable: false,
      searchPath: process.env.PATH,
      searchPrice: null,
    };
    await f.run();
    const started = f.events.filter((event) => event.type === "tool_start");
    assert.equal(
      started.length,
      20,
      `no-local maxToolCalls ${maxToolCalls} cannot expand or reserve away the twentieth tool`,
    );
    assert.equal(
      started.at(-1).data.name,
      "use_skill",
      "twentieth call executes",
    );
    assert.equal(f.calls.length, 20);
    assert.equal(
      f.requests[20].tool_choice,
      "none",
      "final answer follows twenty executed tools",
    );
    assert.ok(!started.some((event) => event.data.args?.index === 21));
  }
});

test("numeric structuredContent is scrubbed before Pi serialization and SDK large-result files", async (t) => {
  const f = await fixture(
    t,
    [
      () => call("tool_search", { query: "fixture" }),
      () => call("mcp__fixture__read_fixture", {}),
      reply,
    ],
    () => ({ approved: true }),
    { localConfig: undefined },
  );
  f.params.mcp = {
    servers: {
      fixture: {
        command: process.execPath,
        args: [
          fileURLToPath(new URL("./fixtures/mcp-fixture.mjs", import.meta.url)),
        ],
        env: {
          MCP_SCENARIO: "numeric",
          PIN: "418731",
          BOOLEAN: "false",
          NULL_VALUE: "null",
          TRUE_VALUE: "true",
          SCHEMA_TYPE: "object",
          JSONRPC: "2.0",
        },
      },
    },
  };
  const result = await f.run();
  assert.equal(result.status, "completed");
  const transcript = await readFile(f.params.sessionFile, "utf8");
  const paths = [
    ...transcript.matchAll(/Full output: ([^ ]+pi-mcp-[^ ]+\.txt)/g),
  ].map((match) => match[1]);
  assert.ok(
    paths.length > 0,
    "structuredContent genuinely takes SDK large-output path",
  );
  t.after(() =>
    Promise.all([...new Set(paths)].map((file) => rm(file, { force: true }))),
  );
  const large = await readFile(paths[0], "utf8");
  for (const text of [
    large,
    transcript,
    JSON.stringify(f.requests),
    JSON.stringify(f.events),
    JSON.stringify(result),
  ]) {
    assert.ok(
      !text.includes("418731"),
      "numeric secret must not reach any persisted/model surface",
    );
  }
  for (const text of [
    large,
    transcript,
    JSON.stringify(f.requests),
    JSON.stringify(result),
  ])
    assert.match(text, /Readable fixture result/);
  assert.match(large, /"pin": "\*\*\*"/);
  assert.equal(JSON.parse(large).flag, "***");
  assert.equal(JSON.parse(large).none, "***");
});

test("MCP JSON payload scalars and keys are scrubbed while fixed control syntax keeps its meaning", async () => {
  const { sanitizeMcpMessage, mcpScrubber } = await import("../src/mcp.mjs");
  const cleaner = mcpScrubber([
    "418731",
    "false",
    "null",
    "object",
    "2.0",
    "true",
  ]);
  const message = {
    jsonrpc: "2.0",
    id: 7,
    result: {
      tools: [
        {
          name: "safe_tool",
          inputSchema: {
            type: "object",
            properties: { index: { type: "integer" } },
            additionalProperties: false,
          },
          annotations: { readOnlyHint: true, destructiveHint: false },
        },
      ],
    },
  };
  assert.deepEqual(
    sanitizeMcpMessage(message, cleaner),
    message,
    "fixed schema and permission hints are unchanged",
  );
  const payload = sanitizeMcpMessage(
    {
      jsonrpc: "2.0",
      id: 7,
      result: {
        content: [],
        structuredContent: {
          pin: 418731,
          flag: false,
          none: null,
          safe: "Readable payload",
        },
      },
    },
    cleaner,
  );
  assert.equal(
    sanitizeMcpMessage(
      {
        result: {
          structuredContent: {
            nested: { blob: Buffer.from("Readable 418731").toString("base64") },
          },
        },
      },
      cleaner,
    ).result.structuredContent.nested.text,
    "Readable ***",
  );
  assert.deepEqual(payload.result.structuredContent, {
    pin: "***",
    flag: "***",
    none: "***",
    safe: "Readable payload",
  });
  for (const constraint of [
    { const: 418731 },
    { default: false },
    { enum: [null] },
    { properties: { 418731: { type: "string" } } },
  ])
    assert.throws(
      () =>
        sanitizeMcpMessage(
          {
            result: { tools: [{ name: "safe_tool", inputSchema: constraint }] },
          },
          cleaner,
        ),
      /control value/,
    );
  assert.throws(
    () =>
      sanitizeMcpMessage({ jsonrpc: "2.0", id: 418731, result: {} }, cleaner),
    /control value/,
  );
  assert.throws(
    () =>
      sanitizeMcpMessage(
        {
          result: {
            tools: [{ name: "418731", inputSchema: { type: "object" } }],
          },
        },
        cleaner,
      ),
    /control value/,
  );
  assert.throws(
    () =>
      sanitizeMcpMessage(
        {
          result: {
            structuredContent: { 418731: "secret key", "***": "existing key" },
          },
        },
        cleaner,
      ),
    /keys collide/,
  );
  assert.deepEqual(
    sanitizeMcpMessage(
      { result: { structuredContent: { 418731: "Readable key value" } } },
      cleaner,
    ).result.structuredContent,
    { "***": "Readable key value" },
  );
});

test("unsafe credential-bearing schema is rejected before tool registration and authorization", async (t) => {
  const f = await fixture(
    t,
    [() => call("tool_search", { query: "fixture" }), reply],
    () => ({ approved: true }),
    { localConfig: undefined },
  );
  f.params.mcp = {
    servers: {
      fixture: {
        command: process.execPath,
        args: [
          fileURLToPath(new URL("./fixtures/mcp-fixture.mjs", import.meta.url)),
        ],
        env: { MCP_SCENARIO: "unsafe_schema", PIN: "418731" },
      },
    },
  };
  assert.equal((await f.run()).status, "completed");
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0].args.toolName, "tool_search");
  assert.ok(
    !f.requests[1].tools?.some((tool) =>
      tool.function.name.startsWith("mcp__fixture__"),
    ),
  );
  assert.ok(
    f.events.some(
      (event) =>
        event.type === "mcp_status" && event.data.reason === "unavailable",
    ),
  );
  assert.ok(!JSON.stringify(f.requests).includes("418731"));
});
