import assert from "node:assert/strict";
import test from "node:test";
import { mediaTools } from "../src/media-tools.mjs";

const model = "volcengine/doubao-seedream-5.0-pro";
const tuple = {
  presentationAssetId: "cover_gen_1",
  planId: "plan-1",
  revision: 1,
};
const reply = () => ({
  status: "completed",
  model,
  files: [{ id: "chat-image-id", name: "generated.jpg" }],
  ...tuple,
});
function fixture(result = reply, planning = true) {
  const calls = [];
  const runtime = {
    wrapTool: (_context, name, _description, _parameters, execute) => ({
      name,
      execute,
    }),
    hostTool: async (_context, name, args) => {
      calls.push({ name, args: structuredClone(args) });
      return result(args);
    },
  };
  const tools = mediaTools(runtime, {
    params: {
      ...(planning ? { documents: { presentationPlanning: "v1" } } : {}),
      media: {
        image: {
          model,
          models: [model],
          maxImages: 1,
          sizes: [],
          qualities: [],
        },
      },
    },
  });
  return { calls, tool: tools.find((item) => item.name === "generate_image") };
}

test("planned image result tells the model the frozen PPT asset ID rather than only the chat attachment ID", async () => {
  const f = fixture();
  const result = await f.tool.execute({ prompt: "Concept art", ...tuple });
  assert.equal(f.calls.length, 1);
  const text = result.content.map((item) => item.text ?? "").join(" ");
  assert.match(text, /image\(slide, "cover_gen_1"/);
  assert.match(text, /chat-image-id/);
  assert.match(text, /attachment[^.]*not|not[^.]*attachment/i);
  assert.doesNotMatch(text, /dataBase64|base64|data:|\/Users\/|\/private\//);
  assert.equal(result.details.presentationAssetId, "cover_gen_1");
});

test("ordinary generation stays generic with and without planning capability", async () => {
  for (const planning of [false, true]) {
    const f = fixture(
      () => ({
        status: "completed",
        model,
        files: [{ id: "chat-image-id", name: "generated.jpg" }],
      }),
      planning,
    );
    const result = await f.tool.execute({ prompt: "A cat" });
    const text = result.content[0].text;
    assert.match(text, /Use the attachment ids to change an image/);
    assert.doesNotMatch(text, /image\(slide|frozen presentation|PPT asset/i);
    assert.equal(f.calls.length, 1);
  }
});

test("planned successful Host responses fail closed on absent, malformed or mismatched frozen tuples", async () => {
  const bad = [
    null,
    { ...reply(), presentationAssetId: undefined },
    { ...reply(), presentationAssetId: "wrong" },
    { ...reply(), presentationAssetId: {} },
    { ...reply(), planId: "other-plan" },
    { ...reply(), planId: "/private/synthetic-only" },
    { ...reply(), revision: 2 },
    { ...reply(), revision: "1" },
    { ...reply(), status: "queued" },
  ];
  for (const value of bad) {
    const f = fixture(() => value);
    await assert.rejects(
      () => f.tool.execute({ prompt: "Concept art", ...tuple }),
      (error) => {
        assert.match(error.message, /assistant_presentation_asset_invalid/);
        assert.doesNotMatch(error.message, /synthetic-only|other-plan/);
        return true;
      },
    );
    assert.equal(f.calls.length, 1);
  }
});

test("pending callers cannot alter the frozen tuple used for image-result guidance", async () => {
  let complete;
  const f = fixture(
    () =>
      new Promise((resolve) => {
        complete = resolve;
      }),
  );
  const args = { prompt: "Concept art", ...tuple };
  const pending = f.tool.execute(args);
  args.presentationAssetId = "mutated";
  args.planId = "mutated-plan";
  args.revision = 2;
  complete(reply());
  const result = await pending;
  assert.match(result.content[0].text, /image\(slide, "cover_gen_1"/);
  assert.doesNotMatch(result.content[0].text, /mutated/);
  assert.deepEqual(f.calls[0].args, { prompt: "Concept art", ...tuple });
});
