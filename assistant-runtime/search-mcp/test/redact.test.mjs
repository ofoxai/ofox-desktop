import assert from "node:assert/strict";
import test from "node:test";
import { redact, scrubSecrets } from "../src/redact.mjs";

const KEY = "sk-of-redact-test-0123456789abcdef";
// Characters that percent-encoding changes.
const SPECIAL_KEY = "sk-of+redact/key=0123456789abcdef";

test("redact removes the key and bearer tokens from any text", () => {
  const text = `failed: Authorization: Bearer ${KEY}; key=${KEY}; other`;
  const result = redact(text, [KEY]);
  assert.equal(result.includes(KEY), false);
  assert.match(result, /\[REDACTED\]/);
  assert.match(result, /other$/);
  assert.equal(redact("plain", [""]), "plain");
});

test("redact drops the credentials of any URL, such as the proxy's", () => {
  assert.equal(
    redact("connect failed: http://user:proxy-pass@10.0.0.1:3128 refused"),
    "connect failed: http://[REDACTED]@10.0.0.1:3128 refused",
  );
  assert.equal(
    redact("socks5://only-user@proxy.example:1080"),
    "socks5://[REDACTED]@proxy.example:1080",
  );
});

test("scrubbing masks the key and its percent-encoded forms in every string", () => {
  const encoded = encodeURIComponent(SPECIAL_KEY);
  const lower = encoded.replace(/%[0-9A-F]{2}/g, (hex) => hex.toLowerCase());
  const value = {
    plain: `a ${SPECIAL_KEY} b`,
    nested: [{ url: `https://x.example/?k=${encoded}&l=${lower}` }],
    count: 3,
    none: null,
  };
  const result = scrubSecrets(value, [SPECIAL_KEY]);
  const text = JSON.stringify(result);
  for (const form of [SPECIAL_KEY, encoded, lower])
    assert.equal(text.includes(form), false, form);
  assert.equal(result.count, 3);
  assert.equal(result.none, null);
  assert.match(result.plain, /^a \[REDACTED\]\*+ b$/);
});

test("the mask keeps the original length so text offsets stay valid", () => {
  const text = `before ${KEY} after`;
  const result = scrubSecrets(text, [KEY]);
  assert.equal(result.length, text.length);
  assert.equal(result.indexOf("after"), text.indexOf("after"));
  assert.equal(scrubSecrets("unchanged", [KEY]), "unchanged");
});
