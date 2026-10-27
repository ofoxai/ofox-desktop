import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { connect } from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AssistantRuntime } from "../src/runtime.mjs";

const token = "synthetic-proxy-run-credential-0123456789";
const proxyPassword = "proxy-password-never-shown";

function streamed(response, text) {
  response.writeHead(200, { "content-type": "text/event-stream" });
  for (const chunk of [
    { choices: [{ index: 0, delta: { role: "assistant", content: text } }] },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage: { prompt_tokens: 3, completion_tokens: 2, total_tokens: 5 },
    },
  ])
    response.write(`data: ${JSON.stringify({ id: "proxied", ...chunk })}\n\n`);
  response.end("data: [DONE]\n\n");
}

async function listen(t, server, host = "localhost") {
  server.listen(0, host);
  await once(server, "listening");
  t.after(async () => {
    server.closeAllConnections?.();
    await new Promise((resolve) => server.close(resolve));
  });
  return server.address().port;
}

/** A local model gateway that records each request. */
async function gateway(t) {
  const requests = [];
  const server = createServer(async (request, response) => {
    let body = "";
    for await (const chunk of request) body += chunk;
    requests.push({
      authorization: request.headers.authorization,
      body: JSON.parse(body),
    });
    streamed(response, "Through the proxy");
  });
  return { port: await listen(t, server), requests };
}

/**
 * An HTTP CONNECT proxy: it records each tunnel and its credentials, then
 * tunnels to the local gateway; any other target is refused.
 */
async function proxy(t, allowedPort) {
  const tunnels = [];
  const server = createServer((_request, response) =>
    response.writeHead(405).end(),
  );
  const sockets = new Set();
  server.on("connect", (request, client, head) => {
    sockets.add(client);
    tunnels.push({
      target: request.url,
      authorization: request.headers["proxy-authorization"],
    });
    const [host, port] = request.url.split(":");
    if (host !== "localhost" || Number(port) !== allowedPort) {
      client.end("HTTP/1.1 403 Forbidden\r\n\r\n");
      return;
    }
    const upstream = connect(Number(port), host, () => {
      client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
      upstream.write(head);
      upstream.pipe(client);
      client.pipe(upstream);
    });
    sockets.add(upstream);
    upstream.on("error", () => client.destroy());
    client.on("error", () => upstream.destroy());
  });
  t.after(() => sockets.forEach((socket) => socket.destroy()));
  return { port: await listen(t, server, "127.0.0.1"), tunnels };
}

async function sandbox(t, extra) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "ofox-proxy-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return {
    sessionId: "proxy-session",
    runId: "proxy-run",
    operationId: "proxy-operation",
    sessionFile: path.join(directory, "session.jsonl"),
    model: {
      id: "test/proxied",
      supportedEndpoints: ["openai"],
      contextWindow: 32000,
      maxOutputTokens: 1024,
    },
    apiKey: token,
    apex: "ofox.ai",
    prompt: "Hello through the proxy",
    ...extra,
  };
}

test("model requests go through the application's proxy with its credentials", async (t) => {
  const model = await gateway(t);
  const tunnel = await proxy(t, model.port);
  const events = [];
  const runtime = new AssistantRuntime({ emit: (event) => events.push(event) });
  t.after(() => runtime.close());
  const result = await runtime.run(
    await sandbox(t, {
      baseUrl: `http://localhost:${model.port}`,
      allowLocalGateway: true,
      proxy: {
        url: `http://user:${proxyPassword}@127.0.0.1:${tunnel.port}`,
        noProxy: "",
      },
    }),
  );
  assert.equal(result.status, "completed");
  assert.deepEqual(
    tunnel.tunnels.map((item) => item.target),
    [`localhost:${model.port}`],
  );
  assert.equal(
    tunnel.tunnels[0].authorization,
    `Basic ${Buffer.from(`user:${proxyPassword}`).toString("base64")}`,
  );
  assert.equal(model.requests.length, 1);
  assert.equal(model.requests[0].authorization, `Bearer ${token}`);
  assert.equal(result.entries.at(-1).content, "Through the proxy");
  const shown = JSON.stringify([events, result]);
  assert.equal(shown.includes(proxyPassword), false);
});

test("hosts the proxy settings exclude connect directly", async (t) => {
  const model = await gateway(t);
  const tunnel = await proxy(t, model.port);
  const runtime = new AssistantRuntime();
  t.after(() => runtime.close());
  const result = await runtime.run(
    await sandbox(t, {
      baseUrl: `http://localhost:${model.port}`,
      allowLocalGateway: true,
      proxy: {
        url: `http://127.0.0.1:${tunnel.port}`,
        noProxy: "example.invalid,localhost",
      },
    }),
  );
  assert.equal(result.status, "completed");
  assert.equal(tunnel.tunnels.length, 0);
  assert.equal(model.requests.length, 1);
});

test("the Ofox gateway is reached through the proxy, never around it", async (t) => {
  const tunnel = await proxy(t, 0);
  const runtime = new AssistantRuntime();
  t.after(() => runtime.close());
  const result = await runtime.run(
    await sandbox(t, {
      proxy: { url: `http://127.0.0.1:${tunnel.port}`, noProxy: "" },
    }),
  );
  // The fixture refuses every tunnel but its own gateway's.
  assert.equal(result.status, "error");
  assert.deepEqual(
    tunnel.tunnels.map((item) => item.target),
    ["api.ofox.ai:443"],
  );
});

test("an unusable proxy setting stops the run before any request", async (t) => {
  for (const url of ["ftp://127.0.0.1:21", "not a url"]) {
    const runtime = new AssistantRuntime();
    t.after(() => runtime.close());
    const result = await runtime.run(await sandbox(t, { proxy: { url } }));
    assert.equal(result.status, "error", url);
    assert.equal(result.errorKind, "not_sent", url);
    assert.doesNotMatch(result.errorMessage, /127\.0\.0\.1|not a url/, url);
  }
});
