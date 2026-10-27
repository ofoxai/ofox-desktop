import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { writeFile } from "node:fs/promises";
import path from "node:path";
import { PassThrough } from "node:stream";
import { finished } from "node:stream/promises";
import test from "node:test";
import {
  attachPrivateStdio,
  createStdioAcceptance,
  inheritedContextMarker,
  writeModelStream,
  writeModelToolCall,
} from "./fixtures/stdio-acceptance.mjs";

test("private stdio waits for pipe closure and keeps output delivered after process exit", async () => {
  const child = new EventEmitter();
  child.stdin = new PassThrough();
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.kill = () => assert.fail("A closed fixture must not need termination");
  const wire = attachPrivateStdio(child);
  const response = wire
    .waitFor((message) => message.id === 19)
    .then(
      (value) => ({ value }),
      (error) => ({ error }),
    );
  let closed = false;
  const closing = wire.close().then(() => {
    closed = true;
  });
  child.emit("exit", 0, null);
  try {
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(closed, false, "exit does not establish that stdio drained");
  } finally {
    child.stdout.end('{"id":19,"result":{"tail":"last response"}}\n');
    child.stderr.end("last diagnostic\n");
    child.stdout.resume();
    child.stderr.resume();
    await Promise.all([finished(child.stdout), finished(child.stderr)]);
    child.emit("close", 0, null);
    await closing;
  }
  assert.deepEqual((await response).value, {
    id: 19,
    result: { tail: "last response" },
  });
  assert.match(wire.stdout, /last response/);
  assert.equal(wire.stderr, "last diagnostic\n");
  assert.equal(wire.forcedTermination, false);
});

test("private stdio credential audit rejects unexpected files outside native JSONL", async (t) => {
  const fixture = await createStdioAcceptance(t);
  const token = "synthetic-unexpected-credential-file";
  await writeFile(
    path.join(fixture.root, "unexpected-auth.json"),
    JSON.stringify({ accessToken: token }),
  );
  await assert.rejects(fixture.assertClean([token]), /Credential persisted/);
});

test("sidecar accepts private JSON-RPC without logging credentials or loading user config", async (t) => {
  const fixture = await createStdioAcceptance(t);
  const wire = fixture.spawnSidecar();
  assert.deepEqual(await wire.start("health"), {
    protocolVersion: 1,
    sdkVersion: "1.4.0",
  });
  const malformed = wire.waitFor(
    (message) => message.id === null && message.error,
  );
  wire.writeRaw("opaque-test-credential-not-json");
  assert.equal((await malformed).error.message, "Invalid JSON-RPC JSON");
  await wire.close();
  assert.equal((await wire.exit).code, 0);
  assert.equal(wire.forcedTermination, false);
  assert.equal(wire.stderr, "");
  await fixture.assertClean(["opaque-test-credential-not-json"]);
});

test(
  "private stdio streams, reloads and continues native history without PATH or ambient context",
  { timeout: 30000 },
  async (t) => {
    for (const pathMode of ["empty", "restricted"]) {
      await t.test(pathMode, async (t) => {
        const token = "synthetic-stdio-stream-credential-" + pathMode;
        const fixture = await createStdioAcceptance(
          t,
          (_request, response, number) => {
            writeModelStream(
              response,
              (number === 1 ? "First streamed answer " : "Continued answer ") +
                token,
            );
          },
        );
        const params = fixture.params("stream-" + pathMode, token);
        const first = fixture.spawnSidecar(pathMode);
        const result = await first.start("run", params);
        assert.equal(result.status, "completed");
        assert.match(result.entries.at(-1).content, /First streamed answer/);
        assert.match(result.entries.at(-1).content, /\[REDACTED\]/);
        const events = first.messages
          .filter((message) => message.method === "event")
          .map((message) => message.params);
        assert.ok(events.some((event) => event.type === "text_delta"));
        assert.ok(events.some((event) => event.type === "run_end"));
        for (const event of events) {
          assert.equal(event.sessionId, params.sessionId);
          assert.equal(event.runId, params.runId);
          assert.equal(event.operationId, params.operationId);
        }
        await first.close();
        assert.equal(first.forcedTermination, false);
        const resumed = fixture.spawnSidecar(pathMode);
        assert.deepEqual(
          (await resumed.start("load", { sessionFile: params.sessionFile }))
            .entries,
          result.entries,
        );
        const next = await resumed.start("run", {
          ...params,
          runId: "resumed-run",
          operationId: "resumed-operation",
          prompt: "Continue after restarting the private process",
        });
        assert.equal(next.status, "completed");
        assert.equal(fixture.requests.length, 2);
        assert.match(
          JSON.stringify(fixture.requests[1].body.messages),
          /First streamed answer/,
        );
        for (const request of fixture.requests) {
          assert.equal(request.authorization, "Bearer " + token);
          const body = JSON.stringify(request.body);
          assert.equal(body.includes(token), false);
          assert.equal(body.includes(inheritedContextMarker), false);
          assert.ok(
            request.body.tools.every((tool) =>
              ["read_attachment"].includes(tool.function.name),
            ),
          );
        }
        await resumed.close();
        await fixture.assertClean([token]);
      });
    }
  },
);

test(
  "private stdio stop checks the operation and preserves a reloadable partial turn",
  { timeout: 20000 },
  async (t) => {
    const token = "synthetic-stdio-stop-credential";
    const fixture = await createStdioAcceptance(t, (_request, response) =>
      writeModelStream(
        response,
        "Partial private stream ".repeat(20) + token,
        false,
      ),
    );
    const wire = fixture.spawnSidecar();
    const params = fixture.params("stop", token);
    const running = wire.start("run", params);
    await wire.waitFor(
      (message) =>
        message.method === "event" && message.params.type === "text_delta",
    );
    assert.deepEqual(
      await wire.start("stop", { ...params, operationId: "wrong-operation" }),
      { stopped: false },
    );
    assert.deepEqual(
      await wire.start("stop", { ...params, sessionId: "wrong-session" }),
      { stopped: false },
    );
    assert.deepEqual(
      await wire.start("stop", { ...params, runId: "wrong-run" }),
      { stopped: false },
    );
    assert.deepEqual(await wire.start("stop", params), { stopped: true });
    const stopped = await running;
    assert.equal(stopped.status, "stopped");
    assert.match(stopped.entries.at(-1).content, /Partial private stream/);
    assert.equal(fixture.requests.length, 1);
    assert.deepEqual(
      (await wire.start("load", { sessionFile: params.sessionFile })).entries,
      stopped.entries,
    );
    await wire.close();
    await fixture.assertClean([token]);
  },
);

test(
  "private stdio stop invalidates a pending host reply before a later run",
  { timeout: 20000 },
  async (t) => {
    const token = "synthetic-stdio-pending-host-credential";
    const fixture = await createStdioAcceptance(
      t,
      (_request, response, number) => {
        if (number === 1)
          writeModelToolCall(response, "bind_tool", { tool: "codex" });
        else writeModelStream(response, "A later operation can continue");
      },
    );
    const wire = fixture.spawnSidecar();
    const params = fixture.params("pending-tool", token, {
      kind: "configuration",
      permissionMode: "ask",
    });
    const running = wire.start("run", params);
    const call = await wire.waitFor((message) => message.method === "tool");
    assert.deepEqual(await wire.start("stop", params), { stopped: true });
    assert.equal((await running).status, "stopped");
    wire.write({
      id: call.id,
      result: { output: "Late reply must be ignored " + token },
    });
    const late = await wire.waitFor(
      (message) => message.id === call.id && message.error,
    );
    assert.equal(late.error.message, "Unknown assistant runtime method");
    assert.equal(fixture.requests.length, 1);
    const next = await wire.start("run", {
      ...params,
      runId: "after-stop-run",
      operationId: "after-stop-operation",
      prompt: "Continue with a fresh operation",
    });
    assert.equal(next.status, "completed");
    assert.equal(fixture.requests.length, 2);
    assert.equal(
      JSON.stringify(fixture.requests[1].body.messages).includes(
        "Late reply must be ignored",
      ),
      false,
    );
    await wire.close();
    await fixture.assertClean([token]);
  },
);

test(
  "private host-tool replies retain operation identity and stop uncertain writes",
  { timeout: 30000 },
  async (t) => {
    for (const unknown of [false, true]) {
      await t.test(unknown ? "unknown" : "success", async (t) => {
        const token = "synthetic-stdio-host-credential-" + String(unknown);
        const fixture = await createStdioAcceptance(
          t,
          (_request, response, number) => {
            if (number === 1)
              writeModelToolCall(response, "bind_tool", { tool: "codex" });
            else writeModelStream(response, "Host operation reply received");
          },
        );
        const wire = fixture.spawnSidecar("restricted");
        const params = fixture.params("tool", token, {
          kind: "configuration",
          permissionMode: "ask",
        });
        const running = wire.start("run", params);
        const call = await wire.waitFor((message) => message.method === "tool");
        assert.match(call.id, /^tool-/);
        assert.equal(call.params.name, "bind_tool");
        assert.deepEqual(call.params.args, { tool: "codex" });
        for (const name of ["sessionId", "runId", "operationId"])
          assert.equal(call.params[name], params[name]);
        assert.ok(call.params.toolCallId);
        wire.write({
          id: "unrelated-tool-reply",
          result: { output: "stale unrelated reply" },
        });
        const ignored = await wire.waitFor(
          (message) => message.id === "unrelated-tool-reply",
        );
        assert.equal(ignored.error.message, "Unknown assistant runtime method");
        if (unknown) {
          wire.write({
            id: call.id,
            error: {
              code: -32000,
              message: "assistant_operation_outcome_unknown " + token,
              data: { outcomeUnknown: true },
            },
          });
        } else {
          wire.write({
            id: call.id,
            result: { output: "Verified host reply " + token, api_key: token },
          });
        }
        const result = await running;
        assert.equal(result.status, unknown ? "error" : "completed");
        assert.equal(fixture.requests.length, unknown ? 1 : 2);
        const ended = wire.messages.find(
          (message) =>
            message.method === "event" && message.params.type === "tool_end",
        );
        assert.equal(ended.params.data.success, !unknown);
        if (unknown)
          assert.match(
            result.errorMessage,
            /assistant_operation_outcome_unknown/,
          );
        else {
          const body = JSON.stringify(fixture.requests[1].body.messages);
          assert.match(body, /Verified host reply/);
          assert.match(body, /\[REDACTED\]/);
          assert.equal(body.includes("stale unrelated reply"), false);
        }
        await wire.close();
        await fixture.assertClean([token]);
      });
    }
  },
);
