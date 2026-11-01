import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AssistantRuntime } from "../src/runtime.mjs";

test("media is persisted without provider setup and survives reload as context", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "ofox-media-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  t.mock.method(globalThis, "fetch", () => {
    throw new Error("Media history must be offline");
  });
  const runtime = new AssistantRuntime({ emit() {} });
  const params = {
    sessionFile: path.join(directory, "session.jsonl"),
    runId: "media-run",
    operationId: "media-operation",
    text: "User generated 1 image. Prompt: blue square. File: image.png (attachment id image-1)",
    details: { kind: "image", model: "test/image", outputs: ["image-1"] },
  };
  const result = await runtime.appendMedia(params);
  const entry = result.entries.find((e) => e.role === "media");
  assert.equal(entry.content, params.text);
  assert.equal(entry.runId, params.runId);
  assert.equal(entry.active, true);
  const loaded = await new AssistantRuntime({ emit() {} }).load(params);
  assert.deepEqual(loaded.entries, result.entries);
  const { SessionManager } = await import("@earendil-works/pi-coding-agent");
  const manager = SessionManager.open(params.sessionFile, directory, directory);
  assert.ok(
    manager
      .buildSessionContext()
      .messages.some(
        (m) =>
          m.customType === "ofox-desktop-media" && m.content === params.text,
      ),
  );
  const lines = (await readFile(params.sessionFile, "utf8"))
    .trim()
    .split("\n")
    .map(JSON.parse);
  assert.equal(
    lines.filter((e) => e.customType === "ofox-desktop-run").length,
    1,
  );
  assert.equal(
    lines.filter((e) => e.customType === "ofox-desktop-media").length,
    1,
  );
  // A crash after native persistence must not duplicate the record on repair.
  await runtime.appendMedia(params);
  assert.equal(
    (await runtime.load(params)).entries.filter((e) => e.role === "media")
      .length,
    1,
  );
});

test("fork preserves custom media and compaction context without network", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "ofox-fork-media-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const runtime = new AssistantRuntime({
    fetch: async () => {
      throw new Error("offline only");
    },
  });
  const sessionFile = path.join(directory, "source", "session.jsonl");
  await runtime.appendMedia({
    sessionFile,
    runId: "media",
    operationId: "media",
    text: "Generated a blue image",
    details: { kind: "image", outputs: ["image"] },
  });
  const { SessionManager } = await import("@earendil-works/pi-coding-agent");
  const manager = SessionManager.open(
    sessionFile,
    path.dirname(sessionFile),
    path.dirname(sessionFile),
  );
  manager.appendCompaction(
    "Earlier context summary",
    manager.getEntries().find((e) => e.type === "message").id,
    500,
  );
  manager.appendCustomEntry("ofox-desktop-run", {
    runId: "after",
    operationId: "after",
    pricingKnown: true,
  });
  const entryId = manager.appendMessage({
    role: "user",
    content: "Continue after compaction",
    timestamp: Date.now(),
  });
  const before = await readFile(sessionFile, "utf8");
  const targetFile = path.join(directory, "fork", "session.jsonl");
  await runtime.fork({ sessionFile, entryId, targetFile });
  const copied = SessionManager.open(
    targetFile,
    path.dirname(targetFile),
    path.dirname(targetFile),
  );
  const context = JSON.stringify(copied.buildSessionContext());
  assert.match(context, /Earlier context summary/);
  assert.match(context, /Generated a blue image/);
  assert.match(context, /Continue after compaction/);
  assert.equal(await readFile(sessionFile, "utf8"), before);
  await runtime.close();
});
