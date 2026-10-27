import assert from "node:assert/strict";
import test from "node:test";
import {
  ConfigError,
  DEFAULT_SEARCH_MODEL,
  loadConfig,
} from "../src/config.mjs";
import { redact } from "../src/redact.mjs";

const KEY = "sk-of-config-test-0123456789abcdef";

function env(overrides = {}) {
  return {
    OFOX_ASSISTANT_KEY: KEY,
    OFOX_BASE_URL: "https://api.ofox.ai",
    ...overrides,
  };
}

test("both production apexes are accepted with the default search model", () => {
  for (const baseUrl of ["https://api.ofox.ai", "https://api.ofox.io"]) {
    assert.deepEqual(loadConfig(env({ OFOX_BASE_URL: baseUrl })), {
      apiKey: KEY,
      baseUrl,
      model: DEFAULT_SEARCH_MODEL,
    });
  }
  assert.equal(DEFAULT_SEARCH_MODEL, "openai/gpt-5-nano");
});

test("the search model can be overridden but must look like a model id", () => {
  assert.equal(
    loadConfig(env({ OFOX_SEARCH_MODEL: "openai/gpt-5-mini" })).model,
    "openai/gpt-5-mini",
  );
  assert.equal(
    loadConfig(env({ OFOX_SEARCH_MODEL: "" })).model,
    DEFAULT_SEARCH_MODEL,
  );
  for (const model of ["has space", "x".repeat(257), "a\nb", "/leading"])
    assert.throws(
      () => loadConfig(env({ OFOX_SEARCH_MODEL: model })),
      ConfigError,
      model,
    );
});

test("the key is required and must be a single header-safe token", () => {
  for (const key of [undefined, "", "  ", "two words", "line\nbreak", "é"])
    assert.throws(
      () => loadConfig(env({ OFOX_ASSISTANT_KEY: key })),
      (error) =>
        error instanceof ConfigError &&
        /OFOX_ASSISTANT_KEY/.test(error.message),
      String(key),
    );
});

test("only the exact Ofox gateways are accepted", () => {
  for (const baseUrl of [
    undefined,
    "",
    "https://api.ofox.ai/",
    "https://api.ofox.ai/v1",
    "http://api.ofox.ai",
    "https://api.ofox.com",
    "https://api.ofox.ai.attacker.example",
    "https://app.ofox.ai",
    "HTTPS://API.OFOX.AI",
    "http://localhost:8088",
  ])
    assert.throws(
      () => loadConfig(env({ OFOX_BASE_URL: baseUrl })),
      (error) =>
        error instanceof ConfigError && /OFOX_BASE_URL/.test(error.message),
      String(baseUrl),
    );
});

test("a localhost gateway needs the explicit test flag and a bare port", () => {
  assert.equal(
    loadConfig(
      env({
        OFOX_BASE_URL: "http://localhost:49152",
        OFOX_ALLOW_LOCAL_GATEWAY: "1",
      }),
    ).baseUrl,
    "http://localhost:49152",
  );
  for (const [baseUrl, flag] of [
    ["http://localhost:49152", "true"],
    ["http://localhost:49152", "0"],
    ["http://localhost:49152/", "1"],
    ["http://localhost:49152/v1", "1"],
    ["http://localhost", "1"],
    ["http://localhost:0", "1"],
    ["http://localhost:65536", "1"],
    ["http://localhost:8088@attacker.example", "1"],
    ["http://localhost.attacker.example:8088", "1"],
    ["http://127.0.0.1:8088", "1"],
    ["https://localhost:8088", "1"],
  ])
    assert.throws(
      () =>
        loadConfig(
          env({ OFOX_BASE_URL: baseUrl, OFOX_ALLOW_LOCAL_GATEWAY: flag }),
        ),
      ConfigError,
      `${baseUrl} ${flag}`,
    );
});

test("config errors never echo the key, even when it leaks into another variable", () => {
  for (const overrides of [
    { OFOX_BASE_URL: `https://api.ofox.ai/${KEY}` },
    { OFOX_BASE_URL: KEY },
    { OFOX_SEARCH_MODEL: `${KEY} x` },
    { OFOX_ASSISTANT_KEY: `${KEY} x` },
  ]) {
    try {
      loadConfig(env(overrides));
      assert.fail("expected a config error");
    } catch (error) {
      assert.ok(error instanceof ConfigError);
      assert.equal(error.message.includes(KEY), false, error.message);
      assert.equal(error.message.includes("sk-of-"), false, error.message);
    }
  }
});

test("redact removes the key and bearer tokens from any text", () => {
  const text = `failed: Authorization: Bearer ${KEY}; key=${KEY}; other`;
  const result = redact(text, [KEY]);
  assert.equal(result.includes(KEY), false);
  assert.match(result, /\[REDACTED\]/);
  assert.match(result, /other$/);
  assert.equal(redact("plain", [""]), "plain");
});
