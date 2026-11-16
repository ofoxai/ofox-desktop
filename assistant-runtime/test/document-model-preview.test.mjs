import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { createCanvas, loadImage } from "@napi-rs/canvas";
import { documentTools } from "../src/document-tools.mjs";

async function patternPage(t) {
  const directory = await mkdtemp(path.join(tmpdir(), "ofox-model-preview-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  // Like Mathtext output: transparent padding surrounds opaque formula ink.
  const image = createCanvas(16, 4);
  const context = image.getContext("2d");
  context.fillStyle = "#000";
  context.fillRect(2, 1, 12, 2);
  const png = image.toBuffer("image/png").toString("base64");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="1920" height="1080"><defs><pattern id="formula" patternUnits="objectBoundingBox" width="1" height="1"><image href="data:image/png;base64,${png}" width="100%" height="100%" preserveAspectRatio="xMidYMid slice"/></pattern></defs><rect width="1920" height="1080" fill="white"/><g transform="translate(100,100)"><rect width="64" height="16" fill="url(#formula)"/></g></svg>`;
  const file = path.join(directory, "1.svg");
  await writeFile(file, svg);
  return file;
}

function createDocument(result, python = false) {
  return documentTools(
    {
      hostTool: async () => result,
      wrapTool: (_context, name, description, parameters, execute) => ({
        name,
        description,
        parameters,
        execute: (id, args) => execute(args, undefined, id),
      }),
    },
    { params: { documents: { formats: ["pptx"], python } } },
    { vision: true },
  ).find((tool) => tool.name === "create_document");
}

async function assertPatternInk(result, region) {
  const output = result.content.find((item) => item.type === "image");
  assert.ok(output, "the vision model must receive the preview image");
  assert.equal(output.mimeType, "image/jpeg");
  const image = await loadImage(Buffer.from(output.data, "base64"));
  const canvas = createCanvas(image.width, image.height);
  const context = canvas.getContext("2d");
  context.drawImage(image, 0, 0);
  const { data } = context.getImageData(0, 0, image.width, image.height);
  let dark = 0;
  for (
    let y = Math.ceil(region.top * image.height);
    y < region.bottom * image.height;
    y++
  ) {
    for (
      let x = Math.ceil(region.left * image.width);
      x < region.right * image.width;
      x++
    ) {
      const pixel = (y * image.width + x) * 4;
      if (data[pixel] < 80 && data[pixel + 1] < 80 && data[pixel + 2] < 80)
        dark++;
    }
  }
  assert.ok(
    dark >= 20,
    `model preview content lost the pattern image: ${dark} dark pixels`,
  );
}

test("the delivered document thumbnail shows embedded OfficeCLI pattern images to the model", async (t) => {
  const previewPath = await patternPage(t);
  const tool = createDocument({
    status: "completed",
    file: { id: "fixture", name: "formula.pptx", bytes: 100, slides: 1 },
    issues: [],
    previewPath,
  });
  const result = await tool.execute("delivered", {
    name: "formula.pptx",
    format: "pptx",
    content: [{ command: "add", parent: "/", type: "slide" }],
  });
  // The embedded image's content area at the model thumbnail's 768px width.
  await assertPatternInk(result, {
    left: 40 / 768,
    right: 66 / 768,
    top: 40 / 432,
    bottom: 47 / 432,
  });
});

test("the Python preview overview shows embedded OfficeCLI pattern images to the model", async (t) => {
  const page = await patternPage(t);
  const tool = createDocument(
    {
      status: "completed",
      preview: true,
      previewId: "fixture-preview",
      slides: 1,
      issues: [],
      pagePaths: [page],
    },
    true,
  );
  const result = await tool.execute("preview", {
    name: "formula.pptx",
    format: "pptx",
    content: { kind: "python", code: "prs = new_deck()" },
    preview: true,
  });
  // The first slide's image area only; exclude its contact-sheet page label.
  await assertPatternInk(result, {
    left: 64 / 1600,
    right: 90 / 1600,
    top: 96 / 510,
    bottom: 103 / 510,
  });
});
