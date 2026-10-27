// The app's proxy. web_search reaches the Ofox gateway through it unless
// OFOX_NO_PROXY excludes the gateway; web_fetch checks every DNS answer here
// first, then tunnels by host name and verifies the certificate for that name.
// Also the pinned TLS connection web_fetch makes without a proxy.
import assert from "node:assert/strict";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { createServer as createTlsServer } from "node:https";
import { connect, isIP } from "node:net";
import test from "node:test";
import { bypassesProxy, proxiedFetch } from "../src/proxy.mjs";
import { SearchError, webSearch } from "../src/search.mjs";
import {
  FetchError,
  pinnedRequest,
  readWebPage,
  tunneledRequest,
} from "../src/web.mjs";
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
  const servernames = [];
  const server = createTlsServer(tls, (request, response) => {
    requests.push({ host: request.headers.host, url: request.url });
    response.writeHead(200, { "content-type": "text/html" });
    response.end(body);
  });
  server.on("secureConnection", (socket) =>
    servernames.push(socket.servername),
  );
  const port = await listen(t, server);
  return { port, address: server.address().address, requests, servernames };
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

test("a fake-IP host is checked with DNS-over-HTTPS through the proxy, then tunnelled to the checked address", async (t) => {
  const site = await exampleSite(t);
  // What the proxy's own resolver would pick for the name: never used.
  const elsewhere = await exampleSite(t, "<h1>Elsewhere</h1>");
  const queries = [];
  const doh = createServer((request, response) => {
    const url = new URL(request.url, "http://localhost");
    queries.push(
      `${url.searchParams.get("type")} ${url.searchParams.get("name")}`,
    );
    const type = url.searchParams.get("type") === "A" ? 1 : 28;
    response.writeHead(200, { "content-type": "application/json" });
    response.end(
      JSON.stringify({
        Status: 0,
        Answer:
          type === 1
            ? [{ name: "example.com.", type, data: PUBLIC[0].address }]
            : [],
      }),
    );
  });
  const dohPort = await listen(t, doh);
  const proxy = await connectProxy(
    t,
    (target) =>
      ({
        [`localhost:${dohPort}`]: dohPort,
        [`${PUBLIC[0].address}:443`]: site.port,
        "example.com:443": elsewhere.port,
      })[target],
  );
  const proxied = proxiedFetch({ url: proxy.url, noProxy: "" });
  const page = await readWebPage("https://example.com/news", {
    lookup: async () => [{ address: "198.18.0.90", family: 4 }],
    doh: { fetchImpl: proxied, url: `http://localhost:${dohPort}/resolve` },
    proxy: {
      noProxy: "",
      request: tunneledRequest(
        { url: proxy.url },
        { requestTls: { ca: tls.cert } },
      ),
    },
  });
  assert.equal(page.text, "Through the tunnel");
  assert.deepEqual(queries.toSorted(), ["A example.com", "AAAA example.com"]);
  const targets = proxy.tunnels.map((tunnel) => tunnel.target);
  assert.ok(targets.includes(`localhost:${dohPort}`), String(targets));
  // The tunnel goes to the address public DNS gave, not to the name.
  assert.equal(targets.at(-1), `${PUBLIC[0].address}:443`);
  assert.equal(targets.includes("example.com:443"), false);
  // TLS and Host still carry the name; the certificate is verified for it.
  assert.deepEqual(site.servernames, ["example.com"]);
  assert.deepEqual(site.requests, [{ host: "example.com", url: "/news" }]);
  assert.deepEqual(elsewhere.requests, []);
});

test("a tunnel to a checked address still verifies the certificate for the name", async (t) => {
  const site = await exampleSite(t);
  const proxy = await connectProxy(t, () => site.port);
  const error = await rejection(
    readWebPage("https://www.example.org/", {
      lookup: async () => [{ address: "198.18.0.90", family: 4 }],
      doh: {
        fetchImpl: async (url) =>
          Response.json({
            Status: 0,
            Answer:
              new URL(url).searchParams.get("type") === "A"
                ? [{ type: 1, data: PUBLIC[0].address }]
                : [],
          }),
      },
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
  assert.equal(error.code, "fetch_failed");
  assert.match(error.message, /CERT/);
  assert.deepEqual(
    proxy.tunnels.map((tunnel) => tunnel.target),
    [`${PUBLIC[0].address}:443`],
  );
  assert.equal(site.requests.length, 0);
});

test("a tunnelled read leaves no Host behind for the next request", async (t) => {
  const site = await exampleSite(t);
  const proxy = await connectProxy(t, () => site.port);
  await readWebPage("https://example.com/first", {
    lookup: async () => PUBLIC,
    proxy: {
      noProxy: "",
      request: tunneledRequest(
        { url: proxy.url },
        { requestTls: { ca: tls.cert } },
      ),
    },
  });
  // undici adds the Host of a tunnelled request to the headers object it is
  // given; a shared one would carry it to every later request.
  const response = await pinnedRequest({ requestTls: { ca: tls.cert } })(
    new URL(`https://example.com:${site.port}/second`),
    { address: site.address, family: isIP(site.address) },
    AbortSignal.timeout(5000),
  );
  assert.equal(response.status, 200);
  assert.deepEqual(site.requests, [
    { host: "example.com", url: "/first" },
    { host: `example.com:${site.port}`, url: "/second" },
  ]);
});

test("a pinned connection goes to the given address with SNI and Host set to the name", async (t) => {
  const site = await exampleSite(t, "<p>pinned</p>");
  const pinned = { address: site.address, family: isIP(site.address) };
  const request = pinnedRequest({ requestTls: { ca: tls.cert } });
  // The name does not resolve to this address; only the pin leads there.
  const response = await request(
    new URL(`https://example.com:${site.port}/doc`),
    pinned,
    AbortSignal.timeout(5000),
  );
  assert.equal(response.status, 200);
  assert.equal(response.body.toString(), "<p>pinned</p>");
  assert.deepEqual(site.servernames, ["example.com"]);
  assert.deepEqual(site.requests, [
    { host: `example.com:${site.port}`, url: "/doc" },
  ]);

  await assert.rejects(
    request(
      new URL(`https://www.example.org:${site.port}/`),
      pinned,
      AbortSignal.timeout(5000),
    ),
    (error) => error.code === "ERR_TLS_CERT_ALTNAME_INVALID",
  );
  assert.equal(site.requests.length, 1);
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
