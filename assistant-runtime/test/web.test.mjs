import assert from "node:assert/strict";
import test from "node:test";
import { validateWebUrl, isPublicAddress, readWebPage } from "../src/web.mjs";

test("web URLs reject local addresses, credentials, non-HTTPS and alternate ports", () => {
  for (const url of [
    "http://example.com",
    "https://localhost/",
    "https://user:pass@example.com",
    "https://example.com:8443",
    "https://127.0.0.1/",
    "https://api.ofox.ai/admin",
  ]) {
    assert.throws(() => validateWebUrl(url), /URL|host|HTTPS|port|credential/i);
  }
  for (const address of [
    "127.0.0.1",
    "10.0.0.2",
    "169.254.169.254",
    "::1",
    "fc00::1",
    "::ffff:127.0.0.1",
    "2001:2::1",
    "2001:20::1",
    "2001:db8::1",
    "2002:7f00:1::",
    "3fff::1",
  ])
    assert.equal(isPublicAddress(address), false, address);
  assert.equal(isPublicAddress("8.8.8.8"), true);
  assert.equal(isPublicAddress("2606:4700:4700::1111"), true);
});

test("web extraction enforces content type, body and output limits", async () => {
  const options = {
    allowedUrls: ["https://example.com/"],
    lookup: async () => [{ address: "8.8.8.8", family: 4 }],
  };
  const page = await readWebPage("https://example.com/", {
    ...options,
    request: async () => ({
      status: 200,
      headers: { "content-type": "text/html" },
      body: Buffer.from(
        `<script>hidden()</script><h1>Title</h1><p>${"a".repeat(25000)}</p>`,
      ),
    }),
  });
  assert.equal(page.text.length, 20000);
  assert.equal(page.truncated, true);
  assert.equal(page.text.includes("hidden"), false);
  await assert.rejects(
    () =>
      readWebPage("https://example.com/", {
        ...options,
        request: async () => ({
          status: 200,
          headers: { "content-type": "text/html" },
          body: Buffer.alloc(2 * 1024 * 1024 + 1),
        }),
      }),
    /2 MiB/,
  );
  await assert.rejects(
    () =>
      readWebPage("https://example.com/", {
        ...options,
        request: async () => ({
          status: 200,
          headers: { "content-type": "application/pdf" },
          body: Buffer.from("PDF"),
        }),
      }),
    /static HTML/,
  );
  await assert.rejects(
    () => readWebPage("https://other.example/", options),
    /not provided/,
  );
});

test("all DNS answers and every redirect are checked before a pinned connection", async () => {
  let connections = 0;
  await assert.rejects(
    () =>
      readWebPage("https://example.com", {
        allowedUrls: ["https://example.com"],
        lookup: async () => [
          { address: "8.8.8.8", family: 4 },
          { address: "127.0.0.1", family: 4 },
        ],
        request: async () => {
          connections++;
          throw new Error("must not connect");
        },
      }),
    /address|DNS/i,
  );
  assert.equal(connections, 0);
  await assert.rejects(
    () =>
      readWebPage("https://example.com", {
        allowedUrls: ["https://example.com"],
        lookup: async () => [{ address: "8.8.8.8", family: 4 }],
        request: async (url, pinned) => {
          connections++;
          assert.equal(pinned.address, "8.8.8.8");
          return {
            status: 302,
            headers: { location: "https://127.0.0.1/secret" },
            body: new Uint8Array(),
          };
        },
      }),
    /URL|host|address/i,
  );
  assert.equal(connections, 1);
});
