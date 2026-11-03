import test from "node:test";
import assert from "node:assert/strict";
import { mediaTools } from "../src/media-tools.mjs";

const MEDIA = {
  image: {
    model: "openai/gpt-image-2",
    models: ["openai/gpt-image-2", "google/gemini-3-pro-image"],
    sizes: ["1024x1024", "1536x1024"],
    qualities: ["low", "high"],
    edits: true,
    several: true,
    maxImages: 4,
  },
  video: {
    model: "bytedance/seedance-2.5",
    models: ["bytedance/seedance-2.5"],
    modes: ["t2v", "i2v", "v2v"],
    resolutions: ["480p", "720p"],
    aspectRatios: ["16:9", "9:16"],
    minDuration: 4,
    maxDuration: 30,
    audio: true,
  },
  speech: {
    model: "elevenlabs/eleven_v4",
    models: ["elevenlabs/eleven_v4"],
    voices: ["21m00Tcm4TlvDq8ikWAM"],
    style: false,
  },
  hints: [],
};

function fixture(media = MEDIA, reply = () => ({ status: "completed" })) {
  const calls = [];
  const runtime = {
    hostTool: async (context, name, args, signal, toolCallId) => {
      calls.push({ name, args, toolCallId });
      return reply(name, args);
    },
    wrapTool: (context, name, description, parameters, execute) => ({
      name,
      description,
      parameters,
      execute: (id, args, signal) => execute(args, signal, id),
    }),
  };
  return { calls, tools: mediaTools(runtime, { params: { media } }) };
}

const tool = (tools, name) => tools.find((t) => t.name === name);

test("each generation tool exists only when the run has a model for it", () => {
  assert.deepEqual(
    fixture().tools.map((t) => t.name),
    ["generate_image", "generate_video", "generate_speech"],
  );
  const { tools } = fixture({ image: MEDIA.image, hints: [] });
  assert.deepEqual(
    tools.map((t) => t.name),
    ["generate_image"],
  );
  assert.deepEqual(fixture({ hints: [] }).tools, []);
});

test("a generation call goes to the host with the model's own arguments", async () => {
  const { calls, tools } = fixture(MEDIA, () => ({
    status: "completed",
    model: "openai/gpt-image-2",
    files: [{ id: "file-1", name: "generated-1.png", mimeType: "image/png" }],
    estimatedCostUsd: "0.04",
  }));
  const result = await tool(tools, "generate_image").execute("call-1", {
    prompt: "a red fox",
    size: "1024x1024",
  });
  assert.deepEqual(calls, [
    {
      name: "generate_image",
      args: { prompt: "a red fox", size: "1024x1024" },
      toolCallId: "call-1",
    },
  ]);
  assert.match(result.content[0].text, /file-1/);
  assert.doesNotMatch(result.content[0].text, /base64|data:/);
  assert.equal(result.details.files[0].id, "file-1");
});

test("a queued video tells the model not to submit it again", async () => {
  const { tools } = fixture(MEDIA, () => ({
    status: "queued",
    model: "bytedance/seedance-2.5",
    estimatedCostUsd: "0.5",
  }));
  const result = await tool(tools, "generate_video").execute("call-2", {
    prompt: "waves",
    material_ids: ["a"],
    material_use: "frames",
  });
  assert.match(result.content[0].text, /queued|being generated/i);
  assert.match(result.content[0].text, /not .*again/i);
});

test("schemas offer the defaults' choices and the guidelines keep the user's words", () => {
  const { tools } = fixture();
  const image = tool(tools, "generate_image");
  assert.deepEqual(image.parameters.properties.size.enum, MEDIA.image.sizes);
  assert.equal(image.parameters.properties.n.maximum, 4);
  assert.deepEqual(image.parameters.required, ["prompt"]);
  const video = tool(tools, "generate_video");
  assert.equal(video.parameters.properties.duration.minimum, 4);
  assert.equal(video.parameters.properties.duration.maximum, 30);
  assert.deepEqual(video.parameters.properties.material_use.enum, [
    "auto",
    "frames",
    "reference",
  ]);
  const guidelines = tools.flatMap((t) => t.promptGuidelines ?? []).join(" ");
  assert.match(guidelines, /only the parameters the user/i);
  assert.match(guidelines, /faithful|do not add/i);
  assert.match(guidelines, /openai\/gpt-image-2/);
});

test("a model without a style option offers no style argument", () => {
  const { tools } = fixture();
  assert.equal(
    tool(tools, "generate_speech").parameters.properties.style,
    undefined,
  );
  const styled = fixture({ ...MEDIA, speech: { ...MEDIA.speech, style: true } });
  assert.ok(tool(styled.tools, "generate_speech").parameters.properties.style);
});

test("a chip tells the model what the user is about to ask for", () => {
  const { tools } = fixture({ ...MEDIA, hints: ["video"] });
  const guidelines = tools.flatMap((t) => t.promptGuidelines ?? []).join(" ");
  assert.match(guidelines, /turned on video generation/i);
});

test("an edit with more than one image is refused by the schema", () => {
  const { tools } = fixture();
  assert.equal(
    tool(tools, "generate_image").parameters.properties.image_ids.maxItems,
    1,
  );
});
