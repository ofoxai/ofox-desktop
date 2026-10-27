import assert from "node:assert/strict";
import { once } from "node:events";
import { createServer } from "node:http";
import test from "node:test";
import {
  FetchError,
  htmlText,
  isPublicAddress,
  readWebPage,
  validateWebUrl,
} from "../src/web.mjs";

const publicLookup = async () => [{ address: "8.8.8.8", family: 4 }];

function page(body, contentType = "text/html") {
  return async () => ({
    status: 200,
    headers: { "content-type": contentType },
    body: Buffer.from(body),
  });
}

async function fetchError(promise) {
  try {
    await promise;
  } catch (error) {
    assert.ok(error instanceof FetchError, String(error));
    return error;
  }
  assert.fail("expected a FetchError");
}

test("web URLs reject local addresses, credentials, non-HTTPS and alternate ports", () => {
  for (const url of [
    "not a url",
    "http://example.com",
    "https://localhost/",
    "https://user:pass@example.com",
    "https://example.com:8443",
    "https://127.0.0.1/",
    "https://[::1]/",
    "https://api.ofox.ai/admin",
    "https://api.ofox.io/v1/responses",
    "https://router.local/",
    "https://intranet/",
  ]) {
    assert.throws(
      () => validateWebUrl(url),
      (error) =>
        error instanceof FetchError && error.code === "fetch_not_allowed",
      url,
    );
  }
  assert.equal(
    validateWebUrl("https://Example.com:443/a#frag").href,
    "https://example.com/a",
  );
  for (const address of [
    "127.0.0.1",
    "10.0.0.2",
    "169.254.169.254",
    "100.64.0.1",
    "192.168.1.1",
    "172.16.0.1",
    "::1",
    "fc00::1",
    "fe80::1",
    "::ffff:127.0.0.1",
    "2001:2::1",
    "2001:20::1",
    "2001:db8::1",
    "2002:7f00:1::",
    "3fff::1",
    "not-an-ip",
  ])
    assert.equal(isPublicAddress(address), false, address);
  assert.equal(isPublicAddress("8.8.8.8"), true);
  assert.equal(isPublicAddress("2606:4700:4700::1111"), true);
});

test("web extraction enforces content type, body and output limits", async () => {
  const result = await readWebPage("https://example.com/", {
    lookup: publicLookup,
    request: page(
      `<script>hidden()</script><style>.x{}</style><h1>Title</h1><p>${"a".repeat(25000)}</p>`,
    ),
  });
  assert.equal(result.url, "https://example.com/");
  assert.equal(result.text.length, 20000);
  assert.equal(result.truncated, true);
  assert.equal(result.text.includes("hidden"), false);
  assert.match(result.text, /^Title\n/);

  const tooLarge = await fetchError(
    readWebPage("https://example.com/", {
      lookup: publicLookup,
      request: page(Buffer.alloc(2 * 1024 * 1024 + 1)),
    }),
  );
  assert.equal(tooLarge.code, "fetch_too_large");
  assert.match(tooLarge.message, /2 MiB/);

  const pdf = await fetchError(
    readWebPage("https://example.com/", {
      lookup: publicLookup,
      request: page("PDF", "application/pdf"),
    }),
  );
  assert.equal(pdf.code, "fetch_unsupported_content");
  assert.match(pdf.message, /static HTML/);
});

test("plain text pages are returned trimmed and untruncated when short", async () => {
  const result = await readWebPage("https://example.com/notes.txt", {
    lookup: publicLookup,
    request: page("  hello <b>world</b>  \n", "text/plain; charset=utf-8"),
  });
  assert.deepEqual(result, {
    url: "https://example.com/notes.txt",
    text: "hello <b>world</b>",
    truncated: false,
  });
});

test("non-2xx pages fail with their status", async () => {
  const error = await fetchError(
    readWebPage("https://example.com/", {
      lookup: publicLookup,
      request: async () => ({
        status: 404,
        headers: {},
        body: Buffer.alloc(0),
      }),
    }),
  );
  assert.equal(error.code, "fetch_failed");
  assert.equal(error.status, 404);
});

test("all DNS answers and every redirect are checked before a pinned connection", async () => {
  let connections = 0;
  const privateAnswer = await fetchError(
    readWebPage("https://example.com", {
      lookup: async () => [
        { address: "8.8.8.8", family: 4 },
        { address: "127.0.0.1", family: 4 },
      ],
      request: async () => {
        connections++;
        throw new Error("must not connect");
      },
    }),
  );
  assert.equal(privateAnswer.code, "fetch_not_allowed");
  assert.equal(connections, 0);

  const redirect = await fetchError(
    readWebPage("https://example.com", {
      lookup: publicLookup,
      request: async (_url, pinned) => {
        connections++;
        assert.equal(pinned.address, "8.8.8.8");
        return {
          status: 302,
          headers: { location: "https://127.0.0.1/secret" },
          body: Buffer.alloc(0),
        };
      },
    }),
  );
  assert.equal(redirect.code, "fetch_not_allowed");
  assert.equal(connections, 1);
});

test("redirects are followed to public pages at most four times", async () => {
  const seen = [];
  const result = await readWebPage("https://example.com/a", {
    lookup: publicLookup,
    request: async (url) => {
      seen.push(url.href);
      return url.pathname === "/a"
        ? { status: 301, headers: { location: "/b" }, body: Buffer.alloc(0) }
        : {
            status: 200,
            headers: { "content-type": "text/html" },
            body: Buffer.from("<p>B</p>"),
          };
    },
  });
  assert.deepEqual(seen, ["https://example.com/a", "https://example.com/b"]);
  assert.equal(result.url, "https://example.com/b");

  let hops = 0;
  const loop = await fetchError(
    readWebPage("https://example.com/0", {
      lookup: publicLookup,
      request: async () => ({
        status: 302,
        headers: { location: `/${++hops}` },
        body: Buffer.alloc(0),
      }),
    }),
  );
  assert.equal(loop.code, "fetch_failed");
  assert.match(loop.message, /redirect/);
  assert.equal(hops, 5);
});

test("DNS failures, network errors and timeouts are plain fetch failures", async () => {
  const dns = await fetchError(
    readWebPage("https://example.com/", {
      lookup: async () => {
        throw Object.assign(new Error("getaddrinfo ENOTFOUND"), {
          code: "ENOTFOUND",
        });
      },
      request: page("x"),
    }),
  );
  assert.equal(dns.code, "fetch_failed");

  const reset = await fetchError(
    readWebPage("https://example.com/", {
      lookup: publicLookup,
      request: async () => {
        throw Object.assign(new Error("socket hang up"), {
          code: "ECONNRESET",
        });
      },
    }),
  );
  assert.equal(reset.code, "fetch_failed");

  // AbortSignal.timeout() does not keep the process alive; a socket would.
  const keepAlive = setTimeout(() => {}, 5000);
  const slow = await fetchError(
    readWebPage("https://example.com/", {
      timeoutMs: 50,
      lookup: publicLookup,
      request: (_url, _pinned, signal) =>
        new Promise((_resolve, reject) =>
          signal.addEventListener("abort", () => reject(signal.reason), {
            once: true,
          }),
        ),
    }),
  );
  clearTimeout(keepAlive);
  assert.equal(slow.code, "fetch_failed");
  assert.match(slow.message, /timed out|cancelled/);
});

test("HTML text extraction drops markup and decodes entities", () => {
  assert.equal(
    htmlText(
      "<!-- c --><div>a &amp; b&nbsp;&#x41;&#66;</div><template>t</template><br>c",
    ),
    "a & b AB\n\nc",
  );
});

test("the key is scrubbed from page text before it is cut to 20,000 characters", async () => {
  const key = "sk-of+web/key=0123456789abcdef";
  const encoded = encodeURIComponent(key);
  const result = await readWebPage("https://example.com/", {
    secrets: [key],
    lookup: publicLookup,
    request: page(`${"a".repeat(19995)}${key} tail ${encoded}`, "text/plain"),
  });
  assert.equal(result.text.length, 20000);
  assert.equal(result.truncated, true);
  assert.equal(result.text.endsWith("a[REDA"), true);
  assert.equal(result.text.includes(key.slice(0, 5)), false);

  const echoed = await readWebPage("https://example.com/", {
    secrets: [key],
    lookup: publicLookup,
    request: page(`<p>${encoded}</p>`),
  });
  assert.equal(echoed.text.includes(encoded), false);
  assert.match(echoed.text, /^\[REDACTED\]/);
});

// Fake-IP DNS: a Clash/Surge-style TUN answers every name with an address in
// 198.18.0.0/15 and resolves the real host itself. Such a host is checked
// with DNS-over-HTTPS (here a stub of the Alibaba JSON API) instead.
const FAKE = [{ address: "198.18.0.90", family: 4 }];

/** A stub DNS-over-HTTPS JSON API; `answer(name, type)` gives each reply. */
async function dohServer(t, answer) {
  const queries = [];
  const sockets = new Set();
  const server = createServer((request, response) => {
    const url = new URL(request.url, "http://localhost");
    const name = url.searchParams.get("name");
    const type = url.searchParams.get("type");
    queries.push(`${type} ${name}`);
    const reply = answer(name, type);
    if (reply === undefined) return; // never answers
    const body =
      typeof reply.body === "string" || Buffer.isBuffer(reply.body)
        ? reply.body
        : JSON.stringify(reply.body);
    response.writeHead(reply.status ?? 200, {
      "content-type": "application/json",
    });
    response.end(body);
  });
  server.on("connection", (socket) => sockets.add(socket));
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(async () => {
    sockets.forEach((socket) => socket.destroy());
    await new Promise((resolve) => server.close(resolve));
  });
  return {
    url: `http://127.0.0.1:${server.address().port}/resolve`,
    queries,
  };
}

// The Alibaba answer shape: `Answer` may hold CNAMEs (type 5) before the
// addresses (A 1, AAAA 28); an empty AAAA answer has only `Authority`.
function dohAnswer(type, ...addresses) {
  const code = type === "A" ? 1 : 28;
  return {
    body: {
      Status: 0,
      TC: false,
      Question: { name: "example.com.", type: code },
      ...(addresses.length
        ? {
            Answer: [
              {
                name: "example.com.",
                TTL: 60,
                type: 5,
                data: "edge.example.net.",
              },
              ...addresses.map((data) => ({
                name: "edge.example.net.",
                TTL: 60,
                type: code,
                data,
              })),
            ],
          }
        : { Authority: [{ name: "example.com.", type: 6, data: "soa" }] }),
    },
  };
}

const publicDoh = (_name, type) =>
  type === "A" ? dohAnswer("A", "20.27.177.113") : dohAnswer("AAAA");

// Each host its own public answers, so a test can tell which one was used.
const PUBLIC_DOH = {
  "example.com": ["20.27.177.113", "20.27.177.114"],
  "docs.example.org": ["151.101.1.1"],
};
const byNameDoh = (name, type) =>
  type === "A" ? dohAnswer("A", ...PUBLIC_DOH[name]) : dohAnswer("AAAA");

test("fake-IP answers are checked with DNS-over-HTTPS, then read from a pinned public answer", async (t) => {
  const doh = await dohServer(t, byNameDoh);
  const connections = [];
  const result = await readWebPage("https://example.com/a", {
    lookup: async () => [...FAKE, { address: "198.19.255.254", family: 4 }],
    doh: { url: doh.url },
    request: async (url, pinned) => {
      connections.push([url.href, pinned.address]);
      return url.pathname === "/a"
        ? {
            status: 302,
            headers: { location: "https://docs.example.org/b" },
            body: Buffer.alloc(0),
          }
        : {
            status: 200,
            headers: { "content-type": "text/plain" },
            body: Buffer.from("from the checked address"),
          };
    },
  });
  assert.equal(result.text, "from the checked address");
  // By host name (SNI and Host stay the name), at an address public DNS gave
  // for that host, never at the fake-IP the TUN would resolve again itself.
  assert.deepEqual(connections, [
    ["https://example.com/a", "20.27.177.113"],
    ["https://docs.example.org/b", "151.101.1.1"],
  ]);
  // Both record types, for every hop.
  assert.deepEqual(doh.queries.toSorted(), [
    "A docs.example.org",
    "A example.com",
    "AAAA docs.example.org",
    "AAAA example.com",
  ]);
});

test("a pinned public answer that cannot be reached fails without falling back", async (t) => {
  const doh = await dohServer(t, byNameDoh);
  const attempts = [];
  const error = await fetchError(
    readWebPage("https://example.com/", {
      lookup: async () => FAKE,
      doh: { url: doh.url },
      request: async (_url, pinned) => {
        attempts.push(pinned.address);
        throw Object.assign(new Error("connect ECONNREFUSED"), {
          code: "ECONNREFUSED",
        });
      },
    }),
  );
  assert.equal(error.code, "fetch_failed");
  // One attempt: neither the fake-IP nor another address is tried.
  assert.deepEqual(attempts, ["20.27.177.113"]);
});

test("fake-IP answers are refused when public DNS has a non-public address or none", async (t) => {
  for (const [label, answer] of [
    [
      "private A",
      (_name, type) =>
        type === "A"
          ? dohAnswer("A", "20.27.177.113", "10.0.0.8")
          : dohAnswer("AAAA"),
    ],
    [
      "loopback AAAA",
      (_name, type) =>
        type === "A"
          ? dohAnswer("A", "20.27.177.113")
          : dohAnswer("AAAA", "::1"),
    ],
    [
      "fake-IP A",
      (_name, type) =>
        type === "A" ? dohAnswer("A", "198.18.0.7") : dohAnswer("AAAA"),
    ],
    ["no address", (_name, type) => dohAnswer(type)],
    [
      "NXDOMAIN",
      () => ({ body: { Status: 3, Question: { name: "example.com." } } }),
    ],
  ]) {
    const doh = await dohServer(t, answer);
    let connections = 0;
    const error = await fetchError(
      readWebPage("https://example.com/", {
        lookup: async () => FAKE,
        doh: { url: doh.url },
        request: async () => {
          connections++;
          throw new Error("must not connect");
        },
      }),
    );
    assert.equal(error.code, "fetch_not_allowed", label);
    assert.match(error.message, /fake-IP/, label);
    assert.match(error.message, /DNS-over-HTTPS/, label);
    assert.equal(connections, 0, label);
  }
});

test("a DNS-over-HTTPS check that fails, is malformed, too large or too slow refuses the page", async (t) => {
  const closed = createServer();
  closed.listen(0, "127.0.0.1");
  await once(closed, "listening");
  const closedUrl = `http://127.0.0.1:${closed.address().port}/resolve`;
  await new Promise((resolve) => closed.close(resolve));
  const cases = [
    ["HTTP 500", () => ({ status: 500, body: { Status: 0 } })],
    ["not JSON", () => ({ body: "<html>" })],
    ["SERVFAIL", () => ({ body: { Status: 2 } })],
    ["no Status", () => ({ body: { Answer: [] } })],
    [
      "malformed address",
      (_name, type) => ({
        body: { Status: 0, Answer: [{ type: type === "A" ? 1 : 28, data: 7 }] },
      }),
    ],
    [
      "wrong family",
      (_name, type) => ({
        body: {
          Status: 0,
          Answer: [{ type: type === "A" ? 1 : 28, data: "2606:4700::1" }],
        },
      }),
    ],
    [
      "over 64 KiB",
      (name, type) => {
        const { body } = publicDoh(name, type);
        return { body: { ...body, padding: "x".repeat(64 * 1024) } };
      },
    ],
    ["no answer", () => undefined],
  ];
  // AbortSignal.timeout() does not keep the process alive; a socket would.
  const keepAlive = setTimeout(() => {}, 5000);
  for (const [label, answer] of cases) {
    const doh = await dohServer(t, answer);
    const error = await fetchError(
      readWebPage("https://example.com/", {
        lookup: async () => FAKE,
        doh: {
          url: doh.url,
          ...(label === "no answer" ? { timeoutMs: 100 } : {}),
        },
        request: async () => assert.fail("must not connect"),
      }),
    );
    assert.equal(error.code, "fetch_not_allowed", label);
    assert.match(error.message, /DNS-over-HTTPS check failed/, label);
  }
  const unreachable = await fetchError(
    readWebPage("https://example.com/", {
      lookup: async () => FAKE,
      doh: { url: closedUrl },
      request: async () => assert.fail("must not connect"),
    }),
  );
  clearTimeout(keepAlive);
  assert.equal(unreachable.code, "fetch_not_allowed");
  assert.match(unreachable.message, /DNS-over-HTTPS check failed/);
});

test("mixed and other reserved answers follow the normal rules without DNS-over-HTTPS", async (t) => {
  const doh = await dohServer(t, publicDoh);
  for (const answers of [
    [...FAKE, { address: "8.8.8.8", family: 4 }],
    [...FAKE, { address: "2606:4700::1111", family: 6 }],
    [{ address: "100.64.0.1", family: 4 }],
    [{ address: "198.51.100.7", family: 4 }],
    [{ address: "198.20.0.1", family: 4 }, ...FAKE],
    [{ address: "::ffff:198.18.0.90", family: 6 }],
  ]) {
    const error = await fetchError(
      readWebPage("https://example.com/", {
        lookup: async () => answers,
        doh: { url: doh.url },
        request: async () => assert.fail("must not connect"),
      }),
    );
    assert.equal(error.code, "fetch_not_allowed", JSON.stringify(answers));
    assert.equal(error.message, "Web DNS contains a non-public address");
  }
  assert.deepEqual(doh.queries, []);
  // A fake-IP address is never accepted as a URL literal either.
  assert.throws(
    () => validateWebUrl("https://198.18.0.90/"),
    (error) => error.code === "fetch_not_allowed",
  );
});
