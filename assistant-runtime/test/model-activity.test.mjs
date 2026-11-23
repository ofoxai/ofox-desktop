import assert from "node:assert/strict";
import test from "node:test";
import { modelActivity } from "../src/model-activity.mjs";

test("streamed arguments and withheld text report cumulative UTF-8 progress without content", () => {
  let at = 0;
  const sent = [];
  const activity = modelActivity((bytes) => sent.push(bytes), () => at);
  activity("甲");
  at = 200;
  activity('{"code":');
  at = 1000;
  activity("乙");
  assert.deepEqual(sent, [3, 14]);
});

test("empty and non-text SDK updates do not turn into liveness heartbeats", () => {
  const sent = [];
  let at = 0;
  const activity = modelActivity((bytes) => sent.push(bytes), () => at);
  for (const delta of ["", undefined, null, {}, 0]) {
    at += 60_000;
    activity(delta);
  }
  assert.deepEqual(sent, []);
  activity("x");
  assert.deepEqual(sent, [1]);
});
