import assert from "node:assert/strict";
import test from "node:test";
import { AssistantRuntime } from "../src/runtime.mjs";

test("private Host scrub updates bind the live operation and mutate its existing scrub array only", () => {
  const runtime = new AssistantRuntime();
  const authority = {
    sessionId: "session",
    runId: "run",
    operationId: "operation",
  };
  const secrets = ["synthetic-original-key"];
  const context = {
    params: { ...authority, apiKey: "synthetic-original-key" },
    secrets,
    abort: new AbortController(),
  };
  runtime.runs.set(authority.runId, context);
  for (const change of [
    { sessionId: "other" },
    { runId: "other" },
    { operationId: "other" },
  ])
    assert.equal(
      runtime.addHostSecrets({ ...authority, ...change }, [
        "synthetic-rejected-key",
      ]),
      false,
    );
  for (const values of [null, {}, ["valid-but-atomic", null], [""]])
    assert.equal(runtime.addHostSecrets(authority, values), false);
  assert.deepEqual(
    secrets,
    ["synthetic-original-key"],
    "invalid updates must be atomic",
  );
  assert.equal(runtime.secrets.has("synthetic-rejected-key"), false);
  assert.equal(
    runtime.addHostSecrets(authority, [
      "synthetic-rotated-key",
      "synthetic-rotated-key",
    ]),
    true,
  );
  assert.equal(
    context.secrets,
    secrets,
    "native/model/event callbacks retain this exact array",
  );
  assert.deepEqual(secrets, [
    "synthetic-original-key",
    "synthetic-rotated-key",
  ]);
  assert.equal(context.params.apiKey, "synthetic-original-key");
  assert.equal(runtime.secrets.has("synthetic-rotated-key"), true);
  context.abort.abort();
  assert.equal(
    runtime.addHostSecrets(authority, ["synthetic-cancelled-key"]),
    false,
  );
  runtime.runs.delete(authority.runId);
  assert.equal(
    runtime.addHostSecrets(authority, ["synthetic-stale-key"]),
    false,
  );
  assert.equal(runtime.secrets.has("synthetic-cancelled-key"), false);
  assert.equal(runtime.secrets.has("synthetic-stale-key"), false);
});
