import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { modelForOfox, redact } from "../src/security.mjs";

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
