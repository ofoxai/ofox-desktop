import assert from "node:assert/strict";
import {
  mkdtemp,
  mkdir,
  writeFile,
  symlink,
  rm,
  realpath,
  truncate,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { loadImage } from "@napi-rs/canvas";
import { AttachmentStore, prepareAttachments } from "../src/attachments.mjs";
import {
  chinesePdf,
  imagePdf,
  noisePixels,
  passwordPdf,
  pdfPages,
} from "./fixtures/pdf.mjs";

const unsafePath = (error) => error.details?.reason === "unsafe_path";

const onePixelPng = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
  "base64",
);

async function folder(t, prefix = "ofox-attachments-") {
  const root = await mkdtemp(path.join(tmpdir(), prefix));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

// Writes each file under root and returns a store describing all of them.
async function storeWith(t, files) {
  const root = await folder(t);
  const attachments = [];
  for (const [name, bytes, mimeType] of files) {
    await writeFile(path.join(root, name), bytes);
    attachments.push({ id: name, path: name, name, mimeType });
  }
  return new AttachmentStore(root, attachments);
}

test("attachments are bounded, paged and confined to their imported root", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-attachments-"));
  try {
    await mkdir(path.join(root, "files"));
    await writeFile(
      path.join(root, "files", "notes.txt"),
      "line\n".repeat(3000),
    );
    const store = new AttachmentStore(root, [
      {
        id: "notes",
        path: "files/notes.txt",
        name: "notes.txt",
        mimeType: "text/plain",
      },
    ]);
    const first = await store.read("notes", 0, 50);
    assert.equal(first.text.length, 50);
    assert.equal(first.hasMore, true);
    await assert.rejects(() => store.read("../../elsewhere"), /attachment/i);
    const traversal = new AttachmentStore(root, [
      { id: "bad", path: "../escape", name: "x.txt" },
    ]);
    await assert.rejects(() => traversal.read("bad"), /root|path/i);
    await assert.rejects(() => traversal.read("bad"), unsafePath);
    const directory = new AttachmentStore(root, [
      { id: "folder", path: "files", name: "folder.txt" },
    ]);
    await assert.rejects(() => directory.read("folder"), /regular file/);
    await assert.rejects(() => directory.read("folder"), unsafePath);
    if (process.platform !== "win32") {
      await symlink(path.dirname(root), path.join(root, "outside"));
      const link = new AttachmentStore(root, [
        { id: "link", path: "outside/missing", name: "x.txt" },
      ]);
      await assert.rejects(() => link.read("link"), /symlink/);
      await assert.rejects(() => link.read("link"), unsafePath);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test(
  "a host attachment root alias accepts its own absolute paths without allowing nested symlinks",
  { skip: process.platform === "win32" },
  async (t) => {
    const temporary = await mkdtemp(path.join(tmpdir(), "ofox-root-alias-"));
    t.after(() => rm(temporary, { recursive: true, force: true }));
    const canonical = await realpath(temporary);
    const root = path.join(canonical, "imported");
    const alias = path.join(canonical, "alias");
    await mkdir(root);
    await symlink(root, alias, "dir");
    await writeFile(path.join(root, "note.txt"), "confirmation: MANDARIN");
    for (const requested of ["note.txt", path.join(alias, "note.txt")]) {
      const store = new AttachmentStore(alias, [
        { id: "note", path: requested, name: "note.txt" },
      ]);
      assert.match((await store.read("note")).text, /MANDARIN/);
    }
    await writeFile(path.join(canonical, "outside.txt"), "private");
    await symlink(
      path.join(canonical, "outside.txt"),
      path.join(root, "linked.txt"),
    );
    for (const requested of [
      path.join(canonical, "outside.txt"),
      "../outside.txt",
      path.join(alias, "linked.txt"),
    ]) {
      const store = new AttachmentStore(alias, [
        { id: "outside", path: requested, name: "outside.txt" },
      ]);
      await assert.rejects(() => store.read("outside"), /root|symlink/i);
      await assert.rejects(() => store.read("outside"), unsafePath);
    }
  },
);

test("the host must declare an absolute root and well-formed attachments", () => {
  const attachment = { id: "a", path: "a.txt", name: "a.txt" };
  assert.throws(() => new AttachmentStore(undefined, [attachment]), /root/i);
  assert.throws(() => new AttachmentStore("relative", [attachment]), /root/i);
  assert.throws(
    () => new AttachmentStore(tmpdir(), [{ ...attachment, path: 7 }]),
    /attachment/i,
  );
  assert.throws(
    () => new AttachmentStore(tmpdir(), [attachment, attachment]),
    /attachment/i,
  );
  assert.equal(new AttachmentStore(undefined, []).size, 0);
});

test("scanned PDFs render bounded images without pretending to extract text", async (t) => {
  const store = await storeWith(t, [["scan.pdf", pdfPages([null])]]);
  const text = await store.read("scan.pdf");
  assert.equal(text.scanned, true);
  assert.equal(text.pageCount, 1);
  assert.deepEqual(text.pagesWithoutText, [1]);
  assert.equal(text.textCoverage, "none");
  const image = await store.image("scan.pdf", 1);
  assert.equal(image.mimeType, "image/png");
  assert.equal(
    Buffer.from(image.data, "base64").subarray(1, 4).toString(),
    "PNG",
  );
  await assert.rejects(() => store.image("scan.pdf", 2), /page/i);
});

test("mixed PDFs disclose each page whose visual content was not extracted", async (t) => {
  const store = await storeWith(t, [
    ["mixed.pdf", pdfPages(["Cover only", null, null])],
  ]);
  const result = await store.read("mixed.pdf");
  assert.match(result.text, /Cover only/);
  assert.equal(result.scanned, false);
  assert.deepEqual(result.pagesWithoutText, [2, 3]);
  assert.equal(result.textCoverage, "partial");
  assert.match(result.warning, /2, 3.*no extractable text/i);
  assert.match(result.warning, /not.*read|unread/i);
  const end = await store.read("mixed.pdf", result.nextOffset);
  assert.equal(end.hasMore, false);
  assert.deepEqual(end.pagesWithoutText, [2, 3]);
  assert.match(end.warning, /complete document/i);
});

test("fully textual PDFs preserve complete text coverage", async (t) => {
  const store = await storeWith(t, [
    ["text.pdf", pdfPages(["First", "Second"])],
  ]);
  const result = await store.read("text.pdf");
  assert.deepEqual(result.pagesWithoutText, []);
  assert.equal(result.textCoverage, "complete");
  assert.equal(result.warning, undefined);
  assert.match(result.text, /First[\s\S]*Second/);
});

test("Chinese PDF text in predefined CMaps is extracted, not reported as a page without text", async (t) => {
  const store = await storeWith(t, [["cn.pdf", chinesePdf("中文摘要")]]);
  const result = await store.read("cn.pdf");
  assert.match(result.text, /中文摘要/);
  assert.deepEqual(result.pagesWithoutText, []);
  assert.equal(result.textCoverage, "complete");
});

test(
  "a PDF final page exceeding the extraction limit has partial text coverage",
  { timeout: 45000 },
  async (t) => {
    // Keep the long text inside the page; PDF.js omits off-page text chunks.
    const scale = "0.000001 0 0 1 0 0 cm\n";
    let source = pdfPages(["A".repeat(16 * 1024 * 1024 + 64)]).toString();
    source = source
      .replace(
        /\/Length (\d+)/,
        (_, length) => `/Length ${Number(length) + scale.length}`,
      )
      .replace("stream\n", `stream\n${scale}`);
    // The single page's stream is the final object, so only startxref moves.
    source = source.replace(
      /startxref\n\d+/,
      `startxref\n${source.indexOf("\nxref\n") + 1}`,
    );
    const pdf = Buffer.from(source);
    assert.ok(pdf.length <= 25 * 1024 * 1024, "fixture must be importable");
    const store = await storeWith(t, [["large.pdf", pdf]]);
    const result = await store.read("large.pdf");
    assert.equal(result.pageCount, 1);
    assert.deepEqual(result.pagesWithoutText, []);
    assert.equal(result.truncated, true);
    assert.equal(result.textCoverage, "partial");
    assert.match(result.warning, /unread|limit/i);
  },
);

test("long text can page beyond the initial extraction prefix", async (t) => {
  const text = "a".repeat(250000) + "TAIL-MUST-BE-READABLE";
  const store = await storeWith(t, [["long.txt", Buffer.from(text)]]);
  const extracted = await store.extract("long.txt");
  const tailOffset = extracted.text.indexOf("TAIL-MUST-BE-READABLE");
  assert.ok(tailOffset >= 250000, "text must retain the tail for paging");
  const tail = await store.read("long.txt", tailOffset, 100);
  assert.match(tail.text, /TAIL-MUST-BE-READABLE/);
  assert.equal(tail.hasMore, false);
});

test("extraction limits disclose unread content instead of claiming a complete document", async (t) => {
  const store = await storeWith(t, [
    ["large.txt", Buffer.alloc(17 * 1024 * 1024, "a")],
  ]);
  const first = await store.read("large.txt");
  assert.equal(first.truncated, true);
  assert.match(first.warning, /limit|truncat/i);
  assert.equal(first.extractionLimit, 16 * 1024 * 1024);
});

// prepareAttachments turns the user's selection into prompt text, native
// images and one structured outcome per attachment.
async function prepared(t, files, options = {}) {
  const store = await storeWith(t, files);
  return await prepareAttachments(
    store,
    options.ids ?? files.map(([name]) => name),
    { vision: false, ...options },
  );
}

test("a readable text attachment is ready and inlined under its header", async (t) => {
  const result = await prepared(t, [
    ["notes.txt", "Quarterly revenue grew", "text/plain"],
  ]);
  assert.deepEqual(result.outcomes, [
    {
      id: "notes.txt",
      status: "ready",
      code: null,
      details: {
        format: "text",
        characters: 22,
        inlineCharacters: 22,
        hasMore: false,
        truncated: false,
      },
    },
  ]);
  assert.equal(result.blocked, null);
  assert.deepEqual(result.images, []);
  assert.match(
    result.text,
    /\[User attachment: "notes\.txt"; id=notes\.txt\]\nQuarterly revenue grew/,
  );
});

test("text longer than the inline window stays ready and points at read_attachment", async (t) => {
  const result = await prepared(t, [["long.md", "b".repeat(9000)]]);
  assert.equal(result.outcomes[0].status, "ready");
  assert.equal(result.outcomes[0].details.hasMore, true);
  assert.equal(result.outcomes[0].details.inlineCharacters, 8000);
  assert.match(result.text, /use read_attachment id=long\.md, offset=8000/);
});

test("text cut at the extraction limit is partial and says so", async (t) => {
  const result = await prepared(t, [
    ["large.txt", Buffer.alloc(17 * 1024 * 1024, "a")],
  ]);
  const [outcome] = result.outcomes;
  assert.equal(outcome.status, "partial");
  assert.equal(outcome.code, "assistant_attachment_partial");
  assert.equal(outcome.details.truncated, true);
  assert.equal(outcome.details.extractionLimit, 16 * 1024 * 1024);
  assert.equal(result.blocked, null);
  assert.match(result.text, /Extraction warning:.*limit/);
});

test("invalid UTF-8 text is unreadable without a decoder message", async (t) => {
  const result = await prepared(t, [
    ["broken.txt", Buffer.from([0x61, 0xff, 0xfe, 0x62])],
  ]);
  assert.deepEqual(result.outcomes[0], {
    id: "broken.txt",
    status: "unreadable",
    code: "assistant_attachment_unreadable",
    details: { format: "text", reason: "invalid_utf8" },
  });
  assert.match(result.blocked.message, /^assistant_attachment_unreadable: /);
  assert.doesNotMatch(result.blocked.message, /encoded data|TextDecoder/i);
});

test("an unsupported format names only the readable formats", async (t) => {
  const result = await prepared(t, [
    ["report.docx", "PK", "application/octet-stream"],
  ]);
  const [outcome] = result.outcomes;
  assert.equal(outcome.status, "unsupported");
  assert.equal(outcome.code, "assistant_attachment_unsupported");
  assert.deepEqual(outcome.details, {
    extension: ".docx",
    mimeType: "application/octet-stream",
  });
  assert.match(result.blocked.message, /image.*PDF.*UTF-8 text/i);
  assert.doesNotMatch(result.blocked.message, /DOCX|XLSX|PPTX|Office/i);
});

test("unsupported image types are not sent to the model", async (t) => {
  const result = await prepared(
    t,
    [["vector.svg", "<svg/>", "image/svg+xml"]],
    { vision: true },
  );
  assert.equal(result.outcomes[0].status, "unsupported");
  assert.deepEqual(result.images, []);
});

test("files over 25 MiB and PDFs over 100 pages are too large", async (t) => {
  const root = await folder(t);
  await writeFile(path.join(root, "huge.txt"), "");
  await truncate(path.join(root, "huge.txt"), 25 * 1024 * 1024 + 1);
  await writeFile(
    path.join(root, "long.pdf"),
    pdfPages(Array.from({ length: 101 }, (_, index) => `Page ${index}`)),
  );
  const store = new AttachmentStore(root, [
    { id: "huge", path: "huge.txt", name: "huge.txt" },
    { id: "long", path: "long.pdf", name: "long.pdf" },
  ]);
  const result = await prepareAttachments(store, ["huge", "long"], {
    vision: true,
  });
  assert.deepEqual(
    result.outcomes.map(({ status, code, details }) => ({
      status,
      code,
      details,
    })),
    [
      {
        status: "too_large",
        code: "assistant_attachment_too_large",
        details: {
          format: "text",
          limit: "file_size",
          maxBytes: 25 * 1024 * 1024,
        },
      },
      {
        status: "too_large",
        code: "assistant_attachment_too_large",
        details: {
          format: "pdf",
          limit: "pdf_pages",
          maxPages: 100,
          pageCount: 101,
        },
      },
    ],
  );
});

test("an image needs a vision model; with one it is sent natively", async (t) => {
  const files = [["photo.png", onePixelPng, "image/png"]];
  const textOnly = await prepared(t, files);
  assert.deepEqual(textOnly.outcomes[0], {
    id: "photo.png",
    status: "model_unsupported",
    code: "assistant_vision_model_required",
    details: { format: "image", mimeType: "image/png" },
  });
  assert.deepEqual(textOnly.images, []);
  assert.match(textOnly.blocked.message, /^assistant_vision_model_required: /);
  const vision = await prepared(t, files, { vision: true });
  assert.equal(vision.outcomes[0].status, "ready");
  assert.equal(vision.blocked, null);
  assert.deepEqual(vision.images, [
    {
      type: "image",
      data: onePixelPng.toString("base64"),
      mimeType: "image/png",
    },
  ]);
  assert.match(
    vision.text,
    /"photo\.png"; id=photo\.png; attached as an image/,
  );
});

test("an image file whose bytes are not that image is unreadable", async (t) => {
  const result = await prepared(
    t,
    [["fake.png", "this is not a picture", "image/png"]],
    { vision: true },
  );
  assert.equal(result.outcomes[0].status, "unreadable");
  assert.equal(result.outcomes[0].details.reason, "invalid_image");
});

test("a message holds at most the allowed number of images", async (t) => {
  const result = await prepared(
    t,
    [
      ["one.png", onePixelPng, "image/png"],
      ["two.png", onePixelPng, "image/png"],
    ],
    { vision: true, imageLimit: 1 },
  );
  assert.deepEqual(
    result.outcomes.map((outcome) => outcome.status),
    ["ready", "too_large"],
  );
  assert.deepEqual(result.outcomes[1].details, {
    format: "image",
    mimeType: "image/png",
    limit: "message_images",
    maxImages: 1,
  });
  assert.equal(result.images.length, 1);
});

test("a vision model gets the first PDF pages without text as images; the rest are partial", async (t) => {
  const result = await prepared(
    t,
    [["mixed.pdf", pdfPages(["Cover only", null, null, null])]],
    { vision: true },
  );
  const [outcome] = result.outcomes;
  assert.equal(outcome.status, "partial");
  assert.equal(outcome.code, "assistant_attachment_partial");
  assert.deepEqual(outcome.details.pagesWithoutText, [2, 3, 4]);
  assert.deepEqual(outcome.details.pagesAttachedAsImages, [2, 3]);
  assert.equal(outcome.details.textCoverage, "partial");
  assert.equal(result.images.length, 2);
  assert.match(result.text, /pages 2, 3 are attached as images/i);
  assert.match(result.text, /remaining.*4.*read_attachment/i);
});

test("a PDF whose pages without text all fit as images is ready", async (t) => {
  const result = await prepared(
    t,
    [["mixed.pdf", pdfPages(["Cover only", null])]],
    { vision: true },
  );
  assert.equal(result.outcomes[0].status, "ready");
  assert.deepEqual(result.outcomes[0].details.pagesAttachedAsImages, [2]);
  assert.equal(result.images.length, 1);
});

test("a text-only model reads a PDF's text and reports its pages without text as partial", async (t) => {
  const result = await prepared(t, [
    ["mixed.pdf", pdfPages(["Cover only", null])],
  ]);
  const [outcome] = result.outcomes;
  assert.equal(outcome.status, "partial");
  assert.equal(outcome.code, "assistant_attachment_partial");
  assert.deepEqual(outcome.details.pagesWithoutText, [2]);
  assert.equal(outcome.details.textCoverage, "partial");
  assert.equal(outcome.details.pagesAttachedAsImages, undefined);
  assert.equal(result.blocked, null);
  assert.deepEqual(result.images, []);
  assert.match(result.text, /Cover only/);
  assert.match(
    result.text,
    /Extraction warning: PDF pages 2 have no extractable text; their visual content has not been read/,
  );
});

test("a text-only model cannot take a PDF without any text", async (t) => {
  const result = await prepared(t, [["scan.pdf", pdfPages([null, null])]]);
  assert.deepEqual(result.outcomes[0], {
    id: "scan.pdf",
    status: "model_unsupported",
    code: "assistant_vision_model_required",
    details: { format: "pdf", pageCount: 2, pagesWithoutText: [1, 2] },
  });
  assert.match(
    result.blocked.message,
    /^assistant_vision_model_required: .*no extractable text.*vision model/,
  );
});

test("damaged and password-protected PDFs are unreadable", async (t) => {
  const result = await prepared(t, [
    ["damaged.pdf", "%PDF-1.4 nothing else"],
    ["locked.pdf", passwordPdf()],
  ]);
  assert.deepEqual(
    result.outcomes.map(({ status, details }) => [status, details.reason]),
    [
      ["unreadable", "invalid_pdf"],
      ["unreadable", "password_protected"],
    ],
  );
});

test("unknown and missing attachments are not found", async (t) => {
  const root = await folder(t);
  const store = new AttachmentStore(root, [
    { id: "gone", path: "gone.txt", name: "gone.txt" },
  ]);
  const result = await prepareAttachments(store, ["unknown", "gone"], {
    vision: false,
  });
  assert.deepEqual(
    result.outcomes.map(({ id, status, code, details }) => [
      id,
      status,
      code,
      details.reason,
    ]),
    [
      ["unknown", "unreadable", "assistant_attachment_not_found", "unknown_id"],
      ["gone", "unreadable", "assistant_attachment_not_found", "missing"],
    ],
  );
});

test("every selected attachment gets an outcome and the first failure blocks", async (t) => {
  const result = await prepared(
    t,
    [
      ["photo.png", onePixelPng, "image/png"],
      ["broken.txt", Buffer.from([0xc3, 0x28])],
      ["notes.txt", "fine"],
    ],
    { ids: ["photo.png", "broken.txt", "notes.txt", "notes.txt"] },
  );
  assert.deepEqual(
    result.outcomes.map((outcome) => outcome.status),
    ["model_unsupported", "unreadable", "ready"],
  );
  assert.match(result.blocked.message, /^assistant_vision_model_required/);
});

async function centerPixel(image) {
  const picture = await loadImage(Buffer.from(image.data, "base64"));
  const { createCanvas } = await import("@napi-rs/canvas");
  const canvas = createCanvas(picture.width, picture.height);
  const context = canvas.getContext("2d");
  context.drawImage(picture, 0, 0);
  return [
    ...context.getImageData(
      Math.floor(picture.width / 2),
      Math.floor(picture.height / 2),
      1,
      1,
    ).data,
  ];
}

test("a scanned page whose image is over 4 megapixels renders its content", async (t) => {
  // 2100 x 2100 is below a 300-dpi A4 scan (2480 x 3508).
  const red = Buffer.alloc(2100 * 2100 * 3);
  for (let index = 0; index < red.length; index += 3) red[index] = 255;
  const store = await storeWith(t, [["scan.pdf", imagePdf(2100, 2100, red)]]);
  assert.deepEqual(
    await centerPixel(await store.image("scan.pdf", 1)),
    [255, 0, 0, 255],
  );
});

test("a page whose image is over 40 megapixels is left unattached, not sent blank", async (t) => {
  // PDF.js checks an image's declared size before decoding it, so a few
  // bytes of image data are enough. 42 and 110 megapixels.
  for (const [width, height] of [
    [7000, 6000],
    [11000, 10000],
  ]) {
    const name = `${width}x${height}`;
    const store = await storeWith(t, [
      ["poster.pdf", imagePdf(width, height, Buffer.alloc(3))],
    ]);
    await assert.rejects(
      () => store.image("poster.pdf", 1),
      (error) =>
        error.code === "assistant_attachment_too_large" &&
        error.details.limit === "pdf_image" &&
        error.details.maxPixels === 40000000 &&
        error.details.page === 1,
      name,
    );
    const result = await prepareAttachments(store, ["poster.pdf"], {
      vision: true,
    });
    assert.equal(result.blocked, null, name);
    assert.equal(result.outcomes[0].status, "partial", name);
    assert.deepEqual(
      result.outcomes[0].details.pagesAttachedAsImages,
      [],
      name,
    );
    assert.deepEqual(result.images, [], name);
    assert.match(
      result.text,
      /Remaining pages without extractable text: 1/,
      name,
    );
  }
});

test("a page with content PDF.js cannot draw still renders the rest", async (t) => {
  const red = Buffer.alloc(300 * 200 * 3);
  for (let index = 0; index < red.length; index += 3) red[index] = 255;
  const store = await storeWith(t, [
    ["odd.pdf", imagePdf(300, 200, red, { before: "/Missing Do " })],
  ]);
  assert.deepEqual(
    await centerPixel(await store.image("odd.pdf", 1)),
    [255, 0, 0, 255],
  );
});

test("a rendered page too large as PNG is sent as JPEG, or else left unattached", async (t) => {
  const root = await folder(t);
  await writeFile(
    path.join(root, "photo.pdf"),
    imagePdf(300, 200, noisePixels(300, 200)),
  );
  const attachments = [{ id: "photo", path: "photo.pdf", name: "photo.pdf" }];
  const png = await new AttachmentStore(root, attachments).image("photo", 1);
  assert.equal(png.mimeType, "image/png");
  const pngBytes = Buffer.from(png.data, "base64").length;
  const jpeg = await new AttachmentStore(root, attachments, {
    maxRenderedPage: pngBytes - 1,
  }).image("photo", 1);
  assert.equal(jpeg.mimeType, "image/jpeg");
  assert.ok(Buffer.from(jpeg.data, "base64").length < pngBytes);
  const tiny = new AttachmentStore(root, attachments, {
    maxRenderedPage: 1000,
  });
  await assert.rejects(
    () => tiny.image("photo", 1),
    (error) =>
      error.code === "assistant_attachment_too_large" &&
      error.details.limit === "rendered_page",
  );
  // Inline, a page that cannot be rendered stays for read_attachment.
  const result = await prepareAttachments(tiny, ["photo"], { vision: true });
  assert.equal(result.blocked, null);
  assert.equal(result.outcomes[0].status, "partial");
  assert.deepEqual(result.outcomes[0].details.pagesAttachedAsImages, []);
  assert.deepEqual(result.images, []);
  assert.match(result.text, /Remaining pages without extractable text: 1/);
});

test("inline page renders read and parse a PDF once", async (t) => {
  const root = await folder(t);
  await writeFile(path.join(root, "scan.pdf"), pdfPages([null, null, null]));
  let reads = 0;
  class CountingStore extends AttachmentStore {
    async file(id) {
      reads++;
      return await super.file(id);
    }
  }
  const store = new CountingStore(root, [
    { id: "scan", path: "scan.pdf", name: "scan.pdf" },
  ]);
  const result = await prepareAttachments(store, ["scan"], { vision: true });
  assert.equal(result.images.length, 2);
  // One read for the text, one for both rendered pages.
  assert.equal(reads, 2);
});

test("PDFs over 100 pages cannot be rendered either", async (t) => {
  const store = await storeWith(t, [
    ["long.pdf", pdfPages(Array.from({ length: 101 }, () => null))],
  ]);
  await assert.rejects(
    () => store.image("long.pdf", 1),
    (error) =>
      error.code === "assistant_attachment_too_large" &&
      error.details.limit === "pdf_pages",
  );
});

test("an abort stops a long PDF between pages", async (t) => {
  const store = await storeWith(t, [
    [
      "long.pdf",
      pdfPages(Array.from({ length: 100 }, (_, index) => `Page ${index}`)),
    ],
  ]);
  const controller = new AbortController();
  const reading = store.read("long.pdf", 0, 8000, {
    signal: controller.signal,
  });
  // PDF.js would otherwise finish in one run of microtasks; these turns of
  // the event loop only happen because extraction yields between pages.
  let turns = 0;
  const turn = () => {
    if (++turns === 20) controller.abort();
    else setImmediate(turn);
  };
  setImmediate(turn);
  await assert.rejects(reading, { name: "AbortError" });
  const aborted = new AbortController();
  aborted.abort();
  await assert.rejects(
    () => store.image("long.pdf", 1, { signal: aborted.signal }),
    { name: "AbortError" },
  );
  // Nothing partial was cached by the aborted read.
  assert.equal((await store.read("long.pdf")).pageCount, 100);
});
