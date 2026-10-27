import assert from "node:assert/strict";
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
