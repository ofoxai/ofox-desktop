import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer as httpServer } from "node:http";
import { createServer as httpsServer } from "node:https";
import { connect, createServer as tcpServer } from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execute = promisify(execFile);
const certificate = fileURLToPath(
  new URL("./fixtures/transport-localhost-cert.txt", import.meta.url),
);
// The localhost certificate is synthetic and reuses the existing test-only
// key. No real signing material or application credential is read or created.
const cert = await readFile(certificate);
const key = await readFile(
  new URL(
    "../search-mcp/test/fixtures/tls/test-only-example.com-key.txt",
    import.meta.url,
  ),
);

async function listen(t, server) {
  const sockets = new Set();
  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
    socket.on("error", () => {});
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(async () => {
    sockets.forEach((socket) => socket.destroy());
    await new Promise((resolve) => server.close(resolve));
  });
  return server.address().port;
}

async function gateway(t, delayMs = 0) {
  const requests = [];
  const secure = httpsServer({ key, cert }, (request, response) => {
    requests.push(request.headers.authorization);
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify({ protocol: request.httpVersion }));
  });
  const port = await listen(t, secure);
  const timers = new Set();
  const upstreams = new Set();
  // TCP connects immediately, but no ClientHello reaches the TLS server until
  // this delay expires. This exercises the real connector's TLS deadline.
  const delayed = tcpServer((client) => {
    const timer = setTimeout(() => {
      timers.delete(timer);
      if (client.destroyed) return;
      const upstream = connect(port, "127.0.0.1");
      upstreams.add(upstream);
      upstream.once("close", () => upstreams.delete(upstream));
      upstream.on("error", () => client.destroy());
      client.on("error", () => upstream.destroy());
      client.once("close", () => upstream.destroy());
      upstream.pipe(client);
      client.pipe(upstream);
    }, delayMs);
    timers.add(timer);
    client.once("close", () => {
      clearTimeout(timer);
      timers.delete(timer);
    });
  });
  t.after(() => {
    timers.forEach(clearTimeout);
    upstreams.forEach((socket) => socket.destroy());
  });
  const delayedPort = await listen(t, delayed);
  return {
    url: `https://127.0.0.1:${delayedPort}/v1/models`,
    port: delayedPort,
    requests,
  };
}

async function proxy(t, allowedPort) {
  const tunnels = [];
  const upstreams = new Set();
  const server = httpServer((_request, response) =>
    response.writeHead(405).end(),
  );
  server.on("connect", (request, client, head) => {
    tunnels.push(request.headers["proxy-authorization"]);
    if (request.url !== `127.0.0.1:${allowedPort}`) {
      client.end("HTTP/1.1 403 Forbidden\r\n\r\n");
      return;
    }
    const upstream = connect(allowedPort, "127.0.0.1", () => {
      client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
      upstream.write(head);
      upstream.pipe(client);
      client.pipe(upstream);
    });
    upstreams.add(upstream);
    upstream.once("close", () => upstreams.delete(upstream));
    upstream.on("error", () => client.destroy());
    client.on("error", () => upstream.destroy());
    client.once("close", () => upstream.destroy());
  });
  t.after(() => upstreams.forEach((socket) => socket.destroy()));
  return {
    url: `http://test:synthetic-password@127.0.0.1:${await listen(t, server)}`,
    tunnels,
  };
}

async function client(t, options, trusted = true) {
  const home = await mkdtemp(path.join(os.tmpdir(), "ofox-model-transport-"));
  t.after(() => rm(home, { recursive: true, force: true }));
  const env = Object.fromEntries(
    ["PATH", "SystemRoot", "SYSTEMROOT", "WINDIR", "COMSPEC"]
      .filter((name) => process.env[name] !== undefined)
      .map((name) => [name, process.env[name]]),
  );
  Object.assign(env, {
    HOME: home,
    USERPROFILE: home,
    APPDATA: home,
    LOCALAPPDATA: home,
    TEMP: home,
    TMP: home,
    ...(trusted ? { NODE_EXTRA_CA_CERTS: certificate } : {}),
  });
  const { stdout } = await execute(
    process.execPath,
    [
      fileURLToPath(
        new URL("./fixtures/model-transport-client.mjs", import.meta.url),
      ),
      JSON.stringify(options),
    ],
    { env, windowsHide: true, timeout: 35000 },
  );
  return JSON.parse(stdout);
}

test(
  "model connections finish a twelve-second TLS handshake without changing proxy routing",
  { concurrency: true, timeout: 45000 },
  async (t) => {
    await Promise.all(
      ["direct", "proxy", "noProxy"].map((route) =>
        t.test(route, async (t) => {
          const model = await gateway(t, 12000);
          const tunnel = await proxy(t, model.port);
          const result = await client(t, {
            url: model.url,
            ...(route === "direct"
              ? { nativeRequest: true }
              : {
                  proxy: {
                    url: tunnel.url,
                    noProxy: route === "noProxy" ? "127.0.0.1" : "",
                  },
                }),
          });
          t.diagnostic(JSON.stringify(result));
          assert.equal(result.status, 200, JSON.stringify(result));
          assert.ok(result.elapsedMs >= 12000);
          assert.deepEqual(result.body, { protocol: "1.1" });
          assert.deepEqual(model.requests, ["Bearer synthetic-transport-test"]);
          assert.equal(result.closedBlocksRequests, true);
          assert.deepEqual(
            tunnel.tunnels,
            route === "proxy"
              ? [
                  `Basic ${Buffer.from("test:synthetic-password").toString("base64")}`,
                ]
              : [],
          );
        }),
      ),
    );
  },
);

for (const [mode, errorName] of [
  ["abortAfterMs", "TimeoutError"],
  ["cancelAfterMs", "AbortError"],
]) {
  test(`a caller's ${mode} still cancels a pending TLS handshake`, async (t) => {
    const model = await gateway(t, 3000);
    const result = await client(t, { url: model.url, [mode]: 200 });
    assert.equal(result.errorName, errorName);
    assert.equal(result.closedBlocksRequests, true);
    assert.deepEqual(model.requests, []);
  });
}

test("model transport still rejects an untrusted TLS certificate", async (t) => {
  const model = await gateway(t);
  const result = await client(t, { url: model.url }, false);
  assert.equal(result.causeCode, "DEPTH_ZERO_SELF_SIGNED_CERT");
  assert.equal(result.closedBlocksRequests, true);
  assert.deepEqual(model.requests, []);
});
