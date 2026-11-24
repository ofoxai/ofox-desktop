import assert from "node:assert/strict";
import test from "node:test";
import { createStreamScrubber } from "../src/stream-secrets.mjs";
import { scrubSecrets } from "../src/security.mjs";
import { mcpSecrets } from "../src/mcp-security.mjs";

const streamText = (source, secrets, final = false) =>
  createStreamScrubber(secrets)(source, final);

test("ordinary short text streams immediately, including reasoning and CJK", () => {
  for (const text of ["你好。", "Thinking…", "了解しました。", "👋 Ready"])
    assert.deepEqual(streamText(text, ["synthetic-private-key"]), {
      ready: text,
      pending: "",
    });
});

test("only a possible credential prefix is held and a mismatch releases it", () => {
  const keys = ["abc-private-credential"];
  const first = streamText("Answer abc-pr", keys);
  assert.deepEqual(first, { ready: "Answer ", pending: "abc-pr" });
  assert.deepEqual(streamText(first.pending + "oblem solved", keys), {
    ready: "abc-problem solved",
    pending: "",
  });
  assert.deepEqual(streamText("abc-pr", keys, true), {
    ready: "abc-pr",
    pending: "",
  });
});

test("every split of complete keys is scrubbed before either stream emits it", () => {
  const keys = [
    "synthetic-private-key",
    "日本語🔑-credential",
    "abcabcabc-credential",
  ];
  const source = `Before ${keys.join(" and ")} after 👋.`;
  const expected = scrubSecrets(source, keys);
  function collect(chunks) {
    const scrub = createStreamScrubber(keys);
    let shown = "",
      pending = "";
    for (const chunk of chunks) {
      const result = scrub(pending + chunk);
      shown += result.ready;
      pending = result.pending;
      for (const key of keys) assert.equal(shown.includes(key), false);
      assert.doesNotMatch(result.ready, /[\uD800-\uDBFF]$/u);
    }
    return shown + scrub(pending, true).ready;
  }
  for (let cut = 0; cut <= source.length; cut++)
    assert.equal(collect([source.slice(0, cut), source.slice(cut)]), expected);
  assert.equal(collect(source.split("")), expected);
});

test("current Host secrets are read on each flush and short non-credentials stay intact", () => {
  const keys = ["original-synthetic-key"];
  keys.push("replacement-synthetic-key-with-a-longer-tail");
  const first = streamText("Now replacement-synthetic", keys);
  assert.equal(first.ready, "Now ");
  assert.deepEqual(
    streamText(first.pending + "-key-with-a-longer-tail!", keys),
    {
      ready: "[REDACTED]!",
      pending: "",
    },
  );
  assert.deepEqual(streamText("short", ["", null]), {
    ready: "short",
    pending: "",
  });
});

test("long repetitive credential prefixes retain the exact undecided suffix", () => {
  const key = "a".repeat(32_000) + "b";
  const first = streamText("Answer " + "a".repeat(32_000), [key]);
  assert.equal(first.ready, "Answer ");
  assert.equal(first.pending.length, 32_000);
  assert.deepEqual(streamText(first.pending + "b done", [key]), {
    ready: "[REDACTED] done",
    pending: "",
  });
});

test("short MCP credentials remain private across chunks without rewriting plain short settings", () => {
  const pin = "418731";
  const first = streamText("PIN: 418", [pin]);
  assert.deepEqual(first, { ready: "PIN: ", pending: "418" });
  const second = streamText(first.pending + "731 done", [pin]);
  assert.deepEqual(second, { ready: "*** done", pending: "" });
  const mcpValues = mcpSecrets({
    servers: {
      fixture: { env: { PIN: pin, AUTH_ENABLED: "true", DEBUG: "1" } },
    },
  });
  const scrub = createStreamScrubber(mcpValues);
  let pending = "",
    shown = "";
  for (const chunk of "PIN: 418731 DEBUG=1 AUTH_ENABLED=true".split("")) {
    const next = scrub(pending + chunk);
    shown += next.ready;
    pending = next.pending;
    assert.equal(shown.includes(pin), false);
  }
  assert.equal(
    shown + scrub(pending, true).ready,
    "PIN: *** DEBUG=1 AUTH_ENABLED=true",
  );
});

test("a run reuses matching state for long credentials and accepts later Host keys", () => {
  const keys = ["a".repeat(65_535) + "b"];
  const scrub = createStreamScrubber(keys);
  const started = performance.now();
  for (let n = 0; n < 1000; n++)
    assert.deepEqual(scrub("normal"), { ready: "normal", pending: "" });
  // Generous guard for a deterministic microfixture, not a Windows latency SLA.
  assert.ok(
    performance.now() - started < 1000,
    "ordinary deltas must not rebuild a 64KiB key table each time",
  );
  keys.push("later-host-key");
  assert.deepEqual(scrub("later-host"), { ready: "", pending: "later-host" });
  assert.deepEqual(scrub("later-host-key"), {
    ready: "[REDACTED]",
    pending: "",
  });
  keys.push("912345");
  assert.deepEqual(scrub("PIN: 912"), { ready: "PIN: ", pending: "912" });
  assert.deepEqual(scrub("912345"), { ready: "***", pending: "" });
});

test("Host-recognized short credentials are authoritative even when they contain newlines", () => {
  const scrub = createStreamScrubber(["Bearer ab\ncd", "ab\ncd"]);
  const first = scrub("Result ab\n");
  assert.deepEqual(first, { ready: "Result ", pending: "ab\n" });
  assert.deepEqual(scrub(first.pending + "cd!"), {
    ready: "***!",
    pending: "",
  });
});

test("an empty auth placeholder never turns ordinary spaces into credentials", () => {
  assert.deepEqual(createStreamScrubber(["Bearer  "])("Hello world"), {
    ready: "Hello world",
    pending: "",
  });
});

test("short values cannot erase an unfinished longer credential's matching evidence", () => {
  const pin = "418731";
  for (const long of [
    pin + "-synthetic-long-private-material",
    "prefix-" + pin + "-synthetic-long-private-material",
  ]) {
    const scrub = createStreamScrubber([pin, long]);
    const cut = long.indexOf(pin) + pin.length;
    const first = scrub("Result " + long.slice(0, cut));
    assert.equal(first.ready, "Result ");
    assert.deepEqual(scrub(first.pending + long.slice(cut) + " done"), {
      ready: "[REDACTED] done",
      pending: "",
    });
  }
});

test("a pending suffix cannot cut a complete credential across emitted chunks", () => {
  const scrub = createStreamScrubber(["abcABCDE", "ABCDEzzz"]);
  const first = scrub("Result abcABCDE");
  assert.equal(first.ready, "Result ");
  const second = scrub(first.pending + " next");
  assert.deepEqual(second, { ready: "[REDACTED] next", pending: "" });
});

test("overlapping complete credentials hide their full combined span", () => {
  const scrub = createStreamScrubber(["abcX", "Xlongsecret"]);
  const first = scrub("Result abcX");
  assert.equal(first.ready, "Result ");
  assert.deepEqual(scrub(first.pending + "longsecret done"), {
    ready: "[REDACTED] done",
    pending: "",
  });
});

test("pathological overlapping repeats stay bounded and never enter a delta", () => {
  const scrub = createStreamScrubber(["aaaaaaaa"]);
  assert.throws(
    () => scrub("a".repeat(2048)),
    /assistant_runtime_protocol_limit/,
  );
  assert.equal(createStreamScrubber([])("a".repeat(2048)).ready.length, 2048);
});
