// The app's proxy. web_search reaches the Ofox gateway through it unless
// OFOX_NO_PROXY excludes the gateway; web_fetch checks every DNS answer here
// first, then tunnels by host name and verifies the certificate for that name.
import assert from "node:assert/strict";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { createServer as createTlsServer } from "node:https";
import { connect } from "node:net";
import test from "node:test";
import { bypassesProxy, proxiedFetch } from "../src/proxy.mjs";
import { SearchError, webSearch } from "../src/search.mjs";
import { FetchError, readWebPage, tunneledRequest } from "../src/web.mjs";
import { replyJson, startGateway } from "./helpers/gateway.mjs";

const KEY = "sk-of-proxy-test-0123456789abcdef";
const PROXY_PASSWORD = "proxy-password-never-shown";
const PUBLIC = [{ address: "93.184.215.14", family: 4 }];
const tls = {
  key: await readFile(
    new URL("./fixtures/tls/test-only-example.com-key.txt", import.meta.url),
  ),
  cert: await readFile(
    new URL("./fixtures/tls/test-only-example.com-cert.txt", import.meta.url),
  ),
};

async function listen(t, server, host = "localhost") {
  server.listen(0, host);
  await once(server, "listening");
  t.after(async () => {
    server.closeAllConnections?.();
    await new Promise((resolve) => server.close(resolve));
  });
  return server.address().port;
}

/**
 * An HTTP CONNECT proxy that records every tunnel. `route(target)` returns
 * the local port a target is tunnelled to, or nothing to refuse it (403).
 */
async function connectProxy(t, route) {
  const tunnels = [];
  const sockets = new Set();
  const server = createServer((_request, response) =>
    response.writeHead(405).end(),
  );
  server.on("connect", (request, client, head) => {
    sockets.add(client);
    tunnels.push({
      target: request.url,
      authorization: request.headers["proxy-authorization"],
    });
    const port = route(request.url);
    if (!port) {
      client.end("HTTP/1.1 403 Forbidden\r\n\r\n");
      return;
    }
    const upstream = connect(port, "localhost", () => {
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
  const port = await listen(t, server, "127.0.0.1");
  return { port, url: `http://127.0.0.1:${port}`, tunnels };
}

/** An HTTPS site with a certificate for example.com only. */
async function exampleSite(t, body = "<h1>Through the tunnel</h1>") {
  const requests = [];
  const server = createTlsServer(tls, (request, response) => {
    requests.push({ host: request.headers.host, url: request.url });
    response.writeHead(200, { "content-type": "text/html" });
    response.end(body);
  });
  return { port: await listen(t, server), requests };
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

test("NO_PROXY entries match exact hosts, subdomains and ports like undici", () => {
  const bypass = (url, noProxy) => bypassesProxy(new URL(url), noProxy);
  assert.equal(bypass("https://example.com/", ""), false);
  assert.equal(bypass("https://example.com/", "*"), true);
  assert.equal(bypass("https://example.com/", "example.com"), true);
  assert.equal(bypass("https://Example.COM./a", "example.com"), true);
  assert.equal(bypass("https://www.example.com/", "example.com"), true);
  assert.equal(bypass("https://www.example.com/", ".example.com"), true);
  assert.equal(bypass("https://www.example.com/", "*.example.com"), true);
  assert.equal(bypass("https://badexample.com/", "example.com"), false);
  assert.equal(bypass("https://example.com/", "example.com:443"), true);
  assert.equal(bypass("https://example.com/", "example.com:8443"), false);
  assert.equal(bypass("https://example.com:8443/", "example.com:8443"), true);
  assert.equal(bypass("https://example.com/", "localhost, example.com"), true);
  assert.equal(bypass("https://[2606:4700::1]/", "2606:4700::1"), true);
  assert.equal(bypass("https://example.org/", "example.com,localhost"), false);
});

test("web_search tunnels to the gateway through the proxy with its credentials", async (t) => {
  const gateway = await startGateway((_req, res) =>
    replyJson(res, 200, { output: [] }),
  );
  t.after(gateway.close);
  const gatewayPort = Number(new URL(gateway.baseUrl).port);
  const proxy = await connectProxy(t, (target) =>
    target === `localhost:${gatewayPort}` ? gatewayPort : undefined,
  );
  const result = await webSearch(
    "through the proxy",
    { apiKey: KEY, baseUrl: gateway.baseUrl, model: "openai/gpt-5-nano" },
    {
      fetchImpl: proxiedFetch({
        url: `http://user:${PROXY_PASSWORD}@127.0.0.1:${proxy.port}`,
        noProxy: "",
      }),
    },
  );
  assert.equal(result.usage.searchCalls, 0);
  assert.deepEqual(
    proxy.tunnels.map((tunnel) => tunnel.target),
    [`localhost:${gatewayPort}`],
  );
  assert.equal(
    proxy.tunnels[0].authorization,
    `Basic ${Buffer.from(`user:${PROXY_PASSWORD}`).toString("base64")}`,
  );
  assert.equal(gateway.requests.length, 1);
  assert.equal(gateway.requests[0].headers.authorization, `Bearer ${KEY}`);
});

test("a gateway excluded by NO_PROXY is reached directly", async (t) => {
  const gateway = await startGateway((_req, res) =>
    replyJson(res, 200, { output: [] }),
  );
  t.after(gateway.close);
  const proxy = await connectProxy(t, () => undefined);
  await webSearch(
    "direct",
    { apiKey: KEY, baseUrl: gateway.baseUrl, model: "openai/gpt-5-nano" },
    { fetchImpl: proxiedFetch({ url: proxy.url, noProxy: "localhost" }) },
  );
  assert.equal(proxy.tunnels.length, 0);
  assert.equal(gateway.requests.length, 1);
});

test("a proxy that refuses or cannot be reached means the search was never sent", async (t) => {
  const gateway = await startGateway((_req, res) =>
    replyJson(res, 200, { output: [] }),
  );
  t.after(gateway.close);
  const refusing = await connectProxy(t, () => undefined);
  const closed = createServer();
  const closedPort = await listen(t, closed, "127.0.0.1");
  await new Promise((resolve) => closed.close(resolve));
  for (const url of [refusing.url, `http://127.0.0.1:${closedPort}`]) {
    const error = await rejection(
      webSearch(
        "refused",
        { apiKey: KEY, baseUrl: gateway.baseUrl, model: "openai/gpt-5-nano" },
        { fetchImpl: proxiedFetch({ url, noProxy: "" }) },
      ),
      SearchError,
    );
    assert.equal(error.code, "search_failed", url);
    assert.equal(error.status, undefined, url);
    assert.match(error.message, /no search was made/, url);
  }
  assert.equal(refusing.tunnels.length, 1);
  assert.equal(gateway.requests.length, 0);
});

test("web_fetch checks DNS locally, then tunnels by host name with a verified certificate", async (t) => {
  const site = await exampleSite(t);
  const proxy = await connectProxy(t, (target) =>
    target.endsWith(":443") ? site.port : undefined,
  );
  const lookups = [];
  const page = await readWebPage("https://example.com/news#top", {
    lookup: async (host) => {
      lookups.push(host);
      return PUBLIC;
    },
    proxy: {
      noProxy: "",
      request: tunneledRequest(
        { url: `http://user:${PROXY_PASSWORD}@127.0.0.1:${proxy.port}` },
        { requestTls: { ca: tls.cert } },
      ),
    },
  });
  assert.deepEqual(page, {
    url: "https://example.com/news",
    text: "Through the tunnel",
    truncated: false,
  });
  assert.deepEqual(lookups, ["example.com"]);
  // By name, never by the checked address: the proxy resolves it.
  assert.deepEqual(
    proxy.tunnels.map((tunnel) => tunnel.target),
    ["example.com:443"],
  );
  assert.ok(proxy.tunnels[0].authorization.startsWith("Basic "));
  assert.deepEqual(site.requests, [{ host: "example.com", url: "/news" }]);
});

test("web_fetch through the proxy never opens a tunnel to a private address", async (t) => {
  const proxy = await connectProxy(t, () => undefined);
  const error = await rejection(
    readWebPage("https://example.com/", {
      lookup: async () => [...PUBLIC, { address: "10.0.0.8", family: 4 }],
      proxy: { noProxy: "", request: tunneledRequest({ url: proxy.url }) },
    }),
    FetchError,
  );
  assert.equal(error.code, "fetch_not_allowed");
  assert.equal(proxy.tunnels.length, 0);
});

test("a certificate for another name or an untrusted one fails the tunnelled fetch", async (t) => {
  const site = await exampleSite(t);
  const proxy = await connectProxy(t, () => site.port);
  for (const [url, requestTls] of [
    ["https://www.example.org/", { ca: tls.cert }],
    ["https://example.com/", undefined],
  ]) {
    const error = await rejection(
      readWebPage(url, {
        lookup: async () => PUBLIC,
        proxy: {
          noProxy: "",
          request: tunneledRequest({ url: proxy.url }, { requestTls }),
        },
      }),
      FetchError,
    );
    assert.equal(error.code, "fetch_failed", url);
    assert.match(error.message, /CERT|SELF_SIGNED/, url);
  }
  assert.equal(proxy.tunnels.length, 2);
  assert.equal(site.requests.length, 0);
});

test("a refused tunnel is a plain fetch failure without the proxy URL", async (t) => {
  const proxy = await connectProxy(t, () => undefined);
  const error = await rejection(
    readWebPage("https://example.com/", {
      lookup: async () => PUBLIC,
      proxy: {
        noProxy: "",
        request: tunneledRequest({
          url: `http://user:${PROXY_PASSWORD}@127.0.0.1:${proxy.port}`,
        }),
      },
    }),
    FetchError,
  );
  assert.equal(error.code, "fetch_failed");
  assert.match(error.message, /proxy/i);
  assert.equal(error.message.includes(PROXY_PASSWORD), false);
  assert.equal(error.message.includes(String(proxy.port)), false);
});

test("a host excluded by NO_PROXY keeps the pinned direct connection", async () => {
  const routes = [];
  const page = (route) => async (_url, pinned) => {
    routes.push([route, pinned.address]);
    return {
      status: 200,
      headers: { "content-type": "text/plain" },
      body: Buffer.from(route),
    };
  };
  const options = {
    lookup: async () => PUBLIC,
    request: page("pinned"),
    proxy: { noProxy: ".example.com", request: page("tunnel") },
  };
  assert.equal(
    (await readWebPage("https://docs.example.com/", options)).text,
    "pinned",
  );
  assert.equal(
    (await readWebPage("https://example.org/", options)).text,
    "tunnel",
  );
  assert.deepEqual(routes, [
    ["pinned", "93.184.215.14"],
    ["tunnel", "93.184.215.14"],
  ]);
});

test("a tunnelled page over 2 MiB is refused while it streams", async (t) => {
  const site = await exampleSite(t, Buffer.alloc(2 * 1024 * 1024 + 1, 97));
  const proxy = await connectProxy(t, () => site.port);
  const error = await rejection(
    readWebPage("https://example.com/", {
      lookup: async () => PUBLIC,
      proxy: {
        noProxy: "",
        request: tunneledRequest(
          { url: proxy.url },
          { requestTls: { ca: tls.cert } },
        ),
      },
    }),
    FetchError,
  );
  assert.equal(error.code, "fetch_too_large");
});
