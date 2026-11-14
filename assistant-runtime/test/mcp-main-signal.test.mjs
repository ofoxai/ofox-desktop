import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { createInterface } from "node:readline";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
const main = fileURLToPath(
  new URL("../resources/runtime/src/main.mjs", import.meta.url),
);
const fixture = fileURLToPath(
  new URL("./fixtures/mcp-fixture.mjs", import.meta.url),
);
const observer = new URL("./fixtures/mcp-exit-observer.mjs", import.meta.url)
  .href;
const pause = () => new Promise((resolve) => setTimeout(resolve, 10));
async function until(fn, ms = 10000) {
  const start = Date.now();
  while (Date.now() - start < ms) {
    if (await fn()) return;
    await pause();
  }
  throw new Error("Timed out waiting for real fixture readiness");
}
async function exists(file) {
  try {
    await readFile(file);
    return true;
  } catch {
    return false;
  }
}
function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error.code === "ESRCH") return false;
    throw error;
  }
}
test("real main signals and EOF confirm MCP cleanup before ordinary exit; failed cleanup uses only self termination", async (t) => {
  const unix = process.platform !== "win32";
  const sentinel = spawn(process.execPath, ["-e", "setInterval(()=>{},1000)"], {
    stdio: "ignore",
  });
  t.after(() => sentinel.kill());
  const cases = unix
    ? [
        ["SIGTERM", "normal", false],
        ["SIGINT", "normal", false],
        ["SIGTERM", "hang", false],
        ["EOF", "hang", false],
        ["SIGTERM", "normal", true],
        ["EOF", "completed", true],
        ["EOF", "normal", true, true],
      ]
    : [
        ["EOF", "normal", false],
        ["EOF", "hang", false],
        ["EOF", "normal", true],
        ["EOF", "completed", true],
        ["EOF", "normal", true, true],
      ];
  for (const [signal, scenario, dropStop, rejectSelf = false] of cases) {
    const root = await mkdtemp(path.join(tmpdir(), "ofox-mcp-signal-"));
    t.after(() => rm(root, { recursive: true, force: true }));
    const trace = path.join(root, "trace"),
      childFile = path.join(root, "child"),
      ready = path.join(root, "ready");
    let requests = 0;
    const gateway = createServer(async (req, res) => {
      for await (const _ of req) {
      }
      requests++;
      res.writeHead(200, { "content-type": "text/event-stream" });
      if (requests === 1) {
        for (const chunk of [
          {
            choices: [
              {
                index: 0,
                delta: {
                  role: "assistant",
                  tool_calls: [
                    {
                      index: 0,
                      id: "discover",
                      type: "function",
                      function: {
                        name: "tool_search",
                        arguments: '{"query":"fixture"}',
                      },
                    },
                  ],
                },
              },
            ],
          },
          {
            choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
            usage: {
              prompt_tokens: 10,
              completion_tokens: 5,
              total_tokens: 15,
            },
          },
        ])
          res.write(`data: ${JSON.stringify(chunk)}\n\n`);
        res.end("data: [DONE]\n\n");
      } else if (scenario === "completed") {
        res.end(
          "data: " +
            JSON.stringify({
              choices: [
                {
                  index: 0,
                  delta: {
                    role: "assistant",
                    content: "Completed reply preserved.",
                  },
                },
              ],
            }) +
            "\n\ndata: " +
            JSON.stringify({
              choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
              usage: {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
              },
            }) +
            "\n\ndata: [DONE]\n\n",
        );
      } else res.write(": waiting for signal\n\n");
    });
    await new Promise((resolve) => gateway.listen(0, "127.0.0.1", resolve));
    t.after(
      () =>
        new Promise((resolve) => {
          gateway.closeAllConnections();
          gateway.close(resolve);
        }),
    );
    const env = Object.fromEntries(
      [
        "HOME",
        "PATH",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "COMSPEC",
        "PATHEXT",
      ]
        .filter((key) => process.env[key] !== undefined)
        .map((key) => [key, process.env[key]]),
    );
    Object.assign(env, {
      MCP_EXIT_TRACE: trace,
      MCP_EXIT_CHILD_FILE: childFile,
      MCP_MAIN_ENTRY: main,
      MCP_DROP_STOP: String(dropStop),
      MCP_REJECT_SELF: String(rejectSelf),
    });
    const child = spawn(
      process.execPath,
      ["--experimental-import-meta-resolve", "--import", observer, main],
      { env, stdio: ["pipe", "pipe", "pipe"] },
    );
    t.after(() => {
      if (child.exitCode === null && child.signalCode === null)
        child.kill("SIGKILL");
    });
    let stderr = "";
    let frames = [];
    child.stderr.on("data", (chunk) => (stderr += chunk));
    const exited = new Promise((resolve) =>
      child.once("close", (code, signal) => resolve({ code, signal })),
    );
    const lines = createInterface({ input: child.stdout });
    lines.on("line", (line) => {
      const frame = JSON.parse(line);
      frames.push(frame);
      if (frame.method === "tool")
        child.stdin.write(
          JSON.stringify({ id: frame.id, result: { approved: true } }) + "\n",
        );
    });
    child.stdin.write(
      JSON.stringify({
        id: "run",
        method: "run",
        params: {
          sessionId: "signal-session",
          runId: "signal-run",
          operationId: "signal-operation",
          sessionFile: path.join(root, "session.jsonl"),
          apiKey: "fake-signal-key",
          apex: "ofox.io",
          baseUrl: `http://localhost:${gateway.address().port}`,
          allowLocalGateway: true,
          prompt: "Discover fixture",
          model: {
            id: "test/local",
            pricingKnown: true,
            cost: { input: 1, output: 1, cacheRead: 0, cacheWrite: 0 },
            supportedEndpoints: ["openai"],
            contextWindow: 32000,
            maxOutputTokens: 1024,
          },
          mcp: {
            servers: {
              fixture: {
                command: process.execPath,
                args: [fixture],
                env: {
                  MCP_PID_FILE: unix ? path.join(root, "server") : childFile,
                  ...(unix ? { MCP_CHILD_FILE: childFile } : {}),
                  MCP_READY_FILE: ready,
                  MCP_SCENARIO: scenario,
                },
              },
            },
          },
        },
      }) + "\n",
    );
    await until(() => exists(childFile)).catch((error) => {
      throw new Error(
        `${error.message}; stderr=${stderr}; frames=${JSON.stringify(frames)}`,
      );
    });
    const descendant = Number(await readFile(childFile, "utf8"));
    assert.ok(alive(descendant));
    if (scenario === "normal")
      await until(async () => requests > 1 && (await exists(ready)));
    if (scenario === "completed") {
      await until(() =>
        frames.some((frame) => frame.id === "run" && frame.result),
      );
      assert.equal(
        frames.find((frame) => frame.id === "run").result.status,
        "completed",
      );
    }
    if (signal === "EOF") child.stdin.end();
    else child.kill(signal);
    if (rejectSelf) {
      await until(() =>
        stderr.includes("Runtime awaiting owned Host termination"),
      );
      assert.equal(child.exitCode, null);
      assert.ok(alive(descendant), "service remains until owned Host teardown");
      // Simulates the production Host's owned Child/Job teardown, never a
      // cached numeric descendant kill. The Unix launcher then receives EOF.
      child.kill("SIGKILL");
    }
    const outcome = await Promise.race([
      exited,
      new Promise((_, reject) =>
        setTimeout(
          () => reject(new Error(`main failed to exit: ${stderr}`)),
          10000,
        ).unref(),
      ),
    ]);
    await until(() => !alive(descendant));
    const records = (await readFile(trace, "utf8"))
      .trim()
      .split("\n")
      .filter(Boolean)
      .map(JSON.parse);
    assert.ok(alive(sentinel.pid), "unrelated sentinel survives");
    t.diagnostic(
      JSON.stringify({
        signal,
        scenario,
        dropStop,
        rejectSelf,
        outcome,
        records,
        descendantGone: !alive(descendant),
        sentinelAlive: alive(sentinel.pid),
      }),
    );
    assert.ok(
      !records.some((row) => row.kind === "cached_group_signal"),
      `${signal}/${scenario} must not enter SDK cached-PGID exit hook: ${JSON.stringify(records)}`,
    );
    if (dropStop) {
      if (unix) assert.equal(outcome.signal, "SIGKILL");
      else
        assert.notEqual(
          outcome.code,
          0,
          "Windows force termination is nonzero",
        );
      assert.ok(
        records.some(
          (row) => row.kind === "self_kill" && row.pid === child.pid,
        ),
      );
      assert.ok(!records.some((row) => row.kind === "ordinary_exit"));
      assert.match(stderr, /cleanup incomplete/i);
    } else {
      assert.equal(outcome.code, 0);
      assert.ok(
        records.some((row) => row.kind === "ordinary_exit" && !row.childAlive),
        `cleanup must precede exit: ${JSON.stringify(records)}`,
      );
    }
  }
});
