import assert from "node:assert/strict";
import test from "node:test";
import { AssistantMessageEventStream } from "@earendil-works/pi-ai";
import { forwardModelStream } from "../src/model-closeout-stream.mjs";

const failed = { role: "assistant", stopReason: "error", content: [] };
const completed = {
  role: "assistant",
  stopReason: "stop",
  content: [{ type: "text", text: "Ready" }],
};
function attempt(events, result) {
  const source = new AssistantMessageEventStream();
  for (const event of events) source.push(event);
  source.end(result);
  return { source };
}
const error = { type: "error", reason: "error", error: failed };
const done = { type: "done", reason: "stop", message: completed };

test("replacement emits one start and one terminal event with no failed message", async () => {
  const pushed = [];
  let recovered = 0;
  const result = await forwardModelStream(
    attempt([{ type: "start" }, error], failed),
    {
      push: (event) => pushed.push(event),
      recover: async () => {
        recovered++;
        return attempt(
          [{ type: "start" }, { type: "text_delta", delta: "Ready" }, done],
          completed,
        );
      },
    },
  );
  assert.equal(result, completed);
  assert.equal(recovered, 1);
  assert.deepEqual(
    pushed.map((event) => event.type),
    ["start", "text_delta", "done"],
  );
});

test("any non-lifecycle event permanently rules out replay", async () => {
  for (const type of [
    "text_start",
    "thinking_start",
    "toolcall_start",
    "future_sdk_event",
  ]) {
    const pushed = [];
    const events = [{ type: "start" }, { type }, error];
    assert.equal(
      await forwardModelStream(attempt(events, failed), {
        push: (event) => pushed.push(event),
        recover: () => assert.fail("activity must not be replayed"),
      }),
      failed,
    );
    assert.deepEqual(pushed, events);
  }
});

test("an ineligible source keeps the original SDK lifecycle", async () => {
  const events = [{ type: "start" }, error],
    pushed = [];
  assert.equal(
    await forwardModelStream(attempt(events, failed), {
      push: (event) => pushed.push(event),
    }),
    failed,
  );
  assert.deepEqual(pushed, events);
});

test("a recovery refusal or budget exception never starts another source", async () => {
  const pushed = [];
  assert.equal(
    await forwardModelStream(attempt([{ type: "start" }, error], failed), {
      push: (event) => pushed.push(event),
      recover: async () => null,
    }),
    failed,
  );
  assert.deepEqual(
    pushed.map((event) => event.type),
    ["start", "error"],
  );
  await assert.rejects(
    forwardModelStream(attempt([{ type: "start" }, error], failed), {
      push: () => {},
      recover: async () => {
        throw new Error("budget exhausted");
      },
    }),
    /budget exhausted/,
  );
});
