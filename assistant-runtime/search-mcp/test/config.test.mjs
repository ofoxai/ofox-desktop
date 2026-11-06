import assert from "node:assert/strict";
import test from "node:test";
import {
  ConfigError,
  DEFAULT_SEARCH_MODEL,
  loadConfig,
} from "../src/config.mjs";

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
      provider: "ofox",
      proxy: null,
    });
  }
  assert.equal(DEFAULT_SEARCH_MODEL, "openai/gpt-5-nano");
});

test("the search provider is Ofox unless Exa is named; Exa's URL moves only in tests", () => {
  assert.equal(
    loadConfig(env({ OFOX_SEARCH_PROVIDER: "exa" })).provider,
    "exa",
  );
  assert.equal(loadConfig(env({ OFOX_SEARCH_PROVIDER: "" })).provider, "ofox");
  assert.equal(loadConfig(env()).exaUrl, undefined);
  assert.throws(
    () => loadConfig(env({ OFOX_SEARCH_PROVIDER: "bing" })),
    (error) =>
      error instanceof ConfigError && /must be ofox or exa/.test(error.message),
  );
  const local = { OFOX_ALLOW_LOCAL_GATEWAY: "1" };
  assert.equal(
    loadConfig(env({ ...local, OFOX_EXA_URL: "http://localhost:4321/mcp" }))
      .exaUrl,
    "http://localhost:4321/mcp",
  );
  for (const overrides of [
    { OFOX_EXA_URL: "http://localhost:4321/mcp" },
    { ...local, OFOX_EXA_URL: "https://evil.example/mcp" },
    { ...local, OFOX_EXA_URL: "http://localhost:4321" },
    { ...local, OFOX_EXA_URL: "http://user@localhost:4321/mcp" },
  ])
    assert.throws(
      () => loadConfig(env(overrides)),
      (error) =>
        error instanceof ConfigError && /only for tests/.test(error.message),
      JSON.stringify(overrides),
    );
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
  assert.equal(
    loadConfig(env({ OFOX_ASSISTANT_KEY: "k".repeat(16) })).apiKey,
    "k".repeat(16),
  );
  // A short key would also mask common text when results are scrubbed.
  for (const key of [
    undefined,
    "",
    "  ",
    "two words",
    "line\nbreak",
    "é",
    "k".repeat(15),
    "k".repeat(513),
  ])
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

test("the app's proxy comes from the environment, socks5h meaning socks5", () => {
  assert.equal(loadConfig(env({ OFOX_PROXY_URL: "" })).proxy, null);
  assert.deepEqual(
    loadConfig(
      env({
        OFOX_PROXY_URL: "http://user:secret@127.0.0.1:8080",
        OFOX_NO_PROXY: "localhost,.corp.example",
      }),
    ).proxy,
    {
      url: "http://user:secret@127.0.0.1:8080/",
      noProxy: "localhost,.corp.example",
    },
  );
  assert.deepEqual(
    loadConfig(env({ OFOX_PROXY_URL: "socks5h://127.0.0.1:1080" })).proxy,
    { url: "socks5://127.0.0.1:1080", noProxy: "" },
  );
  assert.equal(
    loadConfig(env({ OFOX_PROXY_URL: "https://proxy.example:8443" })).proxy.url,
    "https://proxy.example:8443/",
  );
});

test("an unusable proxy setting is a config error that never echoes it", () => {
  for (const proxy of [
    "not a url",
    "ftp://user:secret@127.0.0.1:21",
    "socks4://user:secret@127.0.0.1:1080",
    "file:///etc/passwd",
  ]) {
    try {
      loadConfig(env({ OFOX_PROXY_URL: proxy }));
      assert.fail(`expected a config error for ${proxy}`);
    } catch (error) {
      assert.ok(error instanceof ConfigError, proxy);
      assert.match(error.message, /^OFOX_PROXY_URL /);
      assert.equal(error.message.includes("secret"), false, error.message);
      assert.equal(error.message.includes("127.0.0.1"), false, error.message);
    }
  }
});
