import assert from "node:assert/strict";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import { createServer } from "node:https";
import { createRequire } from "node:module";
import test from "node:test";
import * as runtimeTransport from "undici";

const searchTransport = createRequire(
  new URL("../search-mcp/package.json", import.meta.url),
)("undici");
const fixture = (name) =>
  readFile(
    new URL(`../search-mcp/test/fixtures/tls/${name}`, import.meta.url),
    "utf8",
  );
const key = await fixture("test-only-example.com-key.txt");
const cert = await fixture("test-only-example.com-cert.txt");

for (const [name, { Agent, fetch }] of [
  ["assistant", runtimeTransport],
  ["search", searchTransport],
]) {
  test(`${name} transport preserves a TLS certificate rejection when creating its per-origin client`, async (t) => {
    let requests = 0;
    let checks = 0;
    const server = createServer({ key, cert }, (_request, response) => {
      requests++;
      response.end("must never reach HTTP");
    });
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    const agent = new Agent({
      tls: {
        ca: cert,
        checkServerIdentity: () => {
          checks++;
          return new Error("test-only certificate pin rejected");
        },
      },
      connect: {
        lookup: (_hostname, options, callback) =>
          callback(
            null,
            options.all ? [{ address: "127.0.0.1", family: 4 }] : "127.0.0.1",
            ...(options.all ? [] : [4]),
          ),
      },
    });
    t.after(async () => {
      await agent.destroy();
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    });
    await assert.rejects(
      fetch(`https://example.com:${server.address().port}/`, {
        dispatcher: agent,
        signal: AbortSignal.timeout(5000),
      }),
      (error) => error.cause?.message === "test-only certificate pin rejected",
    );
    assert.equal(checks, 1);
    assert.equal(requests, 0);
  });
}
