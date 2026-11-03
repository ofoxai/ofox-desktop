import test from "node:test";
import assert from "node:assert/strict";
import { LINE_LIMIT, oversizedReplyId } from "../src/ipc.mjs";

test("an oversized host reply is matched to its pending tool call by id", () => {
  const id = "tool-0b6d3c1e-8f7a-4c51-9d2e-3a4b5c6d7e8f";
  const line = JSON.stringify({ id, result: "中".repeat(LINE_LIMIT) });
  assert.ok(Buffer.byteLength(line) > LINE_LIMIT);
  assert.equal(oversizedReplyId(line), id);
  assert.equal(
    oversizedReplyId(JSON.stringify({ result: "x".repeat(600), id })),
    null,
  );
  assert.equal(oversizedReplyId('{"id":"run-1","result":{}}'), null);
});
