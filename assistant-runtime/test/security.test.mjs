import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import {
  errorMessage,
  modelForOfox,
  redact,
  scrubSecrets,
} from "../src/security.mjs";

test("models only use the selected Ofox apex and an advertised protocol", () => {
  const model = modelForOfox(
    {
      id: "test/chat",
      supportedEndpoints: ["openai", "anthropic"],
      contextWindow: 32000,
      maxOutputTokens: 4096,
    },
    "ofox.io",
  );
  assert.equal(model.baseUrl, "https://api.ofox.io/v1");
  assert.equal(model.api, "openai-completions");
  for (const [endpoint, api] of [
    ["/v1/chat/completions", "openai-completions"],
    ["/v1/responses", "openai-responses"],
    ["/v1/messages", "anthropic-messages"],
  ]) {
    assert.equal(
      modelForOfox(
        {
          id: "test/catalog",
          supportedEndpoints: [endpoint],
          contextWindow: 32000,
          maxOutputTokens: 4096,
        },
        "ofox.ai",
      ).api,
      api,
    );
  }
  assert.throws(
    () =>
      modelForOfox(
        { id: "test/chat", supportedEndpoints: ["unknown"] },
        "ofox.ai",
      ),
    /protocol/i,
  );
  assert.throws(
    () =>
      modelForOfox(
        { id: "test/chat", supportedEndpoints: ["openai"] },
        "attacker.invalid",
      ),
    /apex/i,
  );
});

test("the shared catalog projection fixture retains its actual protocol, limits and costs", async () => {
  const fixture = JSON.parse(
    await readFile(
      new URL("./fixtures/catalog-model.json", import.meta.url),
      "utf8",
    ),
  );
  const model = modelForOfox(fixture.runtime, "ofox.ai");
  assert.equal(model.api, "openai-responses");
  assert.equal(model.contextWindow, fixture.catalog.data[0].context_length);
  assert.equal(model.maxTokens, 16000);
  assert.deepEqual(model.cost, fixture.runtime.cost);
});

test("catalog models keep the protocol the host's catalog names for them", async () => {
  const fixture = JSON.parse(
    await readFile(
      new URL("./fixtures/catalog-protocols.json", import.meta.url),
      "utf8",
    ),
  );
  for (const { id, supportedEndpoints, api } of fixture.models) {
    const model = modelForOfox(
      { id, supportedEndpoints, contextWindow: 128000, maxOutputTokens: 4096 },
      "ofox.ai",
    );
    assert.equal(model.api, api, id);
    assert.equal(
      model.baseUrl,
      api === "anthropic-messages"
        ? "https://api.ofox.ai/anthropic"
        : "https://api.ofox.ai/v1",
      id,
    );
  }
});

test("a Claude model of the merged catalog runs with Anthropic Messages", async () => {
  const fixture = JSON.parse(
    await readFile(
      new URL("./fixtures/catalog-claude-model.json", import.meta.url),
      "utf8",
    ),
  );
  const model = modelForOfox(fixture.runtime, "ofox.io");
  assert.deepEqual(model, {
    id: "anthropic/claude-haiku-5.5",
    name: "Anthropic: Claude Haiku 5.5",
    provider: "ofox-desktop",
    api: "anthropic-messages",
    baseUrl: "https://api.ofox.io/anthropic",
    // Thinking stays as it is today until a run asks for a level.
    reasoning: false,
    input: ["text", "image"],
    contextWindow: 1000000,
    maxTokens: 128000,
    cost: { input: 0.1, output: 0.5, cacheRead: 0.01, cacheWrite: 0.125 },
    compat: undefined,
  });
  assert.equal(
    modelForOfox(fixture.runtime, "ofox.ai", {
      baseUrl: "http://localhost:8088",
      allowLocalGateway: true,
    }).baseUrl,
    "http://localhost:8088/anthropic",
  );
});

test("the Pi model reasons only for a run at a level, with the map and Claude's compat", async () => {
  const fixture = JSON.parse(
    await readFile(
      new URL("./fixtures/catalog-claude-model.json", import.meta.url),
      "utf8",
    ),
  );
  // What the host adds for a run at a level (catalog.rs for_runtime).
  const thinkingLevelMap = {
    high: "high",
    low: "low",
    max: "max",
    medium: "medium",
    minimal: null,
    xhigh: "xhigh",
  };
  const claude = {
    ...fixture.runtime,
    reasoning: true,
    thinkingLevelMap,
    anthropicCompat: {
      forceAdaptiveThinking: true,
      supportsTemperature: false,
    },
  };
  const plain = modelForOfox(fixture.runtime, "ofox.io");
  // Without a level the catalog's fields change nothing.
  assert.deepEqual(modelForOfox(claude, "ofox.io"), plain);
  const thinking = modelForOfox(claude, "ofox.io", { thinkingLevel: "high" });
  assert.deepEqual(thinking, {
    ...plain,
    reasoning: true,
    thinkingLevelMap,
    compat: { forceAdaptiveThinking: true, supportsTemperature: false },
  });
  // A model the catalog says cannot reason never thinks.
  const { reasoning: _, ...unlisted } = claude;
  assert.deepEqual(
    modelForOfox(unlisted, "ofox.io", { thinkingLevel: "high" }),
    plain,
  );
  // Chat Completions keeps its own compat; Claude's is for Messages only.
  const completions = modelForOfox(
    {
      ...claude,
      id: "openai/gpt-6-luna",
      supportedEndpoints: ["/v1/chat/completions"],
    },
    "ofox.io",
    { thinkingLevel: "low" },
  );
  assert.equal(completions.reasoning, true);
  assert.deepEqual(completions.thinkingLevelMap, thinkingLevelMap);
  assert.equal(completions.compat.maxTokensField, "max_tokens");
  assert.equal(completions.compat.forceAdaptiveThinking, undefined);
  // A level without Pi data runs unmapped.
  const { thinkingLevelMap: __, ...unmapped } = claude;
  assert.equal(
    "thinkingLevelMap" in
      modelForOfox(unmapped, "ofox.io", { thinkingLevel: "low" }),
    false,
  );
});

test("an unknown thinking level or a malformed level map is refused", () => {
  const input = {
    id: "test/chat",
    supportedEndpoints: ["/v1/chat/completions"],
    contextWindow: 32000,
    maxOutputTokens: 4096,
    reasoning: true,
  };
  for (const thinkingLevel of ["extreme", "", 3, "default"])
    assert.throws(
      () => modelForOfox(input, "ofox.ai", { thinkingLevel }),
      /thinking level/i,
    );
  for (const thinkingLevelMap of [
    { turbo: "turbo" },
    { high: 3 },
    { high: "x".repeat(65) },
    ["high"],
  ])
    assert.throws(
      () =>
        modelForOfox({ ...input, thinkingLevelMap }, "ofox.ai", {
          thinkingLevel: "high",
        }),
      /thinking level map/i,
    );
});

test("the host names the protocol only when it is not the first advertised", () => {
  const input = {
    id: "anthropic/claude-test",
    supportedEndpoints: ["/v1/chat/completions", "/v1/messages"],
    contextWindow: 32000,
    maxOutputTokens: 4096,
  };
  assert.equal(modelForOfox(input, "ofox.ai").api, "openai-completions");
  assert.equal(
    modelForOfox({ ...input, api: "anthropic-messages" }, "ofox.ai").api,
    "anthropic-messages",
  );
  // A protocol the endpoints do not advertise is never taken.
  assert.equal(
    modelForOfox(
      {
        ...input,
        supportedEndpoints: ["/v1/chat/completions"],
        api: "anthropic-messages",
      },
      "ofox.ai",
    ).api,
    "openai-completions",
  );
});

test("credential values are redacted before any protocol error or tool output", () => {
  const secret = "private-test-token-that-must-not-escape";
  const value = redact(
    {
      error: `Bearer ${secret}`,
      nested: { token: secret, message: `failure ${secret}` },
    },
    [secret],
  );
  assert.equal(JSON.stringify(value).includes(secret), false);
  assert.match(value.error, /REDACTED/);
});

test("conversation content loses only the exact known secrets", () => {
  const secret = "private-test-token-that-must-not-escape";
  const text = [
    'OpenAI(api_key=os.environ["OPENAI_API_KEY"])',
    'password = input("Password: ")',
    "Authorization: Bearer YOUR_TOKEN",
    "sk-proj-EXAMPLEexample0123456789",
  ].join("\n");
  const value = { content: `${text} ${secret}`, nested: [{ password: text }] };
  assert.deepEqual(scrubSecrets(value, [secret, "short"]), {
    content: `${text} [REDACTED]`,
    nested: [{ password: text }],
  });
  const untouched = { content: text };
  assert.equal(scrubSecrets(untouched, [secret]), untouched);
  // Diagnostics keep the pattern scrub.
  assert.doesNotMatch(errorMessage(new Error(text), [secret]), /YOUR_TOKEN/);
});

test("diagnostics never show the credentials of a proxy URL", () => {
  const message = errorMessage(
    new Error("tunnel via http://ofox-user:proxy-pass-1@127.0.0.1:7890 failed"),
  );
  assert.doesNotMatch(message, /ofox-user|proxy-pass-1/);
  assert.match(message, /127\.0\.0\.1:7890 failed/);
});

test("only a trusted explicit debug flag can route a host-resolved gateway locally", () => {
  const input = {
    id: "test/local",
    supportedEndpoints: ["/v1/messages"],
    contextWindow: 32000,
    maxOutputTokens: 4096,
  };
  assert.equal(
    modelForOfox(input, "ofox.io", {
      baseUrl: "http://localhost:8088",
      allowLocalGateway: true,
    }).baseUrl,
    "http://localhost:8088/anthropic",
  );
  assert.throws(
    () => modelForOfox(input, "ofox.io", { baseUrl: "http://localhost:8088" }),
    /gateway/i,
  );
  assert.throws(
    () =>
      modelForOfox(input, "ofox.io", {
        baseUrl: "https://attacker.invalid",
        allowLocalGateway: true,
      }),
    /gateway/i,
  );
  assert.throws(
    () => modelForOfox(input, "ofox.io", { baseUrl: "https://api.ofox.ai" }),
    /gateway/i,
  );
});

test("the debug flag accepts any local gateway port but nothing else on localhost", () => {
  const input = {
    id: "test/local",
    supportedEndpoints: ["openai"],
    contextWindow: 32000,
    maxOutputTokens: 4096,
  };
  assert.equal(
    modelForOfox(input, "ofox.ai", {
      baseUrl: "http://localhost:49152",
      allowLocalGateway: true,
    }).baseUrl,
    "http://localhost:49152/v1",
  );
  for (const baseUrl of [
    "http://localhost:8088/",
    "http://localhost:8088/v1",
    "http://localhost",
    "http://localhost:0",
    "http://localhost:65536",
    "http://localhost:8088@attacker.invalid",
    "http://localhost.attacker.invalid:8088",
    "https://localhost:8088",
    "http://127.0.0.1:8088",
  ])
    assert.throws(
      () =>
        modelForOfox(input, "ofox.ai", { baseUrl, allowLocalGateway: true }),
      /gateway/i,
      baseUrl,
    );
});
