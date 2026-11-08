import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { createCanvas, loadImage } from "@napi-rs/canvas";
import { validateToolArguments } from "@earendil-works/pi-ai";
import { documentTools } from "../src/document-tools.mjs";
import { pdfPages } from "./fixtures/pdf.mjs";

const FORMATS = [
  "md",
  "txt",
  "csv",
  "html",
  "json",
  "pptx",
  "docx",
  "xlsx",
  "pdf",
];

const completed = (extra = {}) => ({
  status: "completed",
  file: {
    id: "att-deck",
    name: "deck.pptx",
    mimeType:
      "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    bytes: 12_400,
    slides: 3,
  },
  issues: [],
  ...extra,
});

function fixture({
  formats = FORMATS,
  python = false,
  vision = true,
  reply = () => completed(),
} = {}) {
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
  return {
    calls,
    tools: documentTools(
      runtime,
      { params: { documents: { formats, python } } },
      { vision },
    ),
  };
}

const createDocument = (options) => {
  const { calls, tools } = fixture(options);
  const tool = tools.find((t) => t.name === "create_document");
  return { calls, tool };
};

async function folder(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "ofox-documents-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}

/** A PNG of the given size with a gradient, as a preview would be. */
async function writePng(file, width, height) {
  const canvas = createCanvas(width, height);
  const context = canvas.getContext("2d");
  const gradient = context.createLinearGradient(0, 0, width, height);
  gradient.addColorStop(0, "#2a6");
  gradient.addColorStop(1, "#a26");
  context.fillStyle = gradient;
  context.fillRect(0, 0, width, height);
  context.fillStyle = "#fff";
  context.font = `${Math.ceil(height / 12)}px sans-serif`;
  context.fillText("Annual report", width / 10, height / 2);
  await writeFile(file, canvas.toBuffer("image/png"));
  return file;
}

const DECK = [
  { command: "add", parent: "/", type: "slide" },
  {
    command: "set",
    path: "/slide[1]/shape[1]",
    props: { text: "Quarterly results" },
  },
];

// What the pptx skill teaches the model to send instead of a batch.
const DECK_OBJECT = {
  kind: "deck",
  title: "Quarterly results",
  language: "en",
  slides: [
    {
      type: "hero",
      title: "Quarterly results",
      subtitle: "Revenue, margin and what comes next",
    },
    {
      type: "pillars",
      title: "Three things to know",
      pillars: [
        { heading: "Growth", body: "Revenue rose 20% on the quarter" },
        { heading: "Margin", body: "Gross margin held at 62%" },
        { heading: "Outlook", body: "Two launches are planned for Q4" },
      ],
    },
    { type: "cta", title: "Questions?", subtitle: "team@example.com" },
  ],
};

test("registers create_document with the host formats", () => {
  const { tools } = fixture({ formats: ["pdf", "md"] });
  assert.deepEqual(
    tools.map((t) => t.name),
    ["create_document"],
  );
  const [tool] = tools;
  for (const format of ["pdf", "md"]) {
    const args = { name: `notes.${format}`, format, content: "= Notes" };
    assert.deepEqual(
      validateToolArguments(tool, { name: tool.name, arguments: args }),
      args,
    );
  }
  for (const format of ["pptx", "docx", "exe"])
    assert.throws(
      () =>
        validateToolArguments(tool, {
          name: tool.name,
          arguments: { name: "deck.pptx", format, content: DECK },
        }),
      /Validation failed/,
      format,
    );
});

test("a batch array and a text body both pass the schema", () => {
  const { tool } = createDocument();
  for (const args of [
    { name: "deck.pptx", format: "pptx", content: DECK },
    { name: "notes.md", format: "md", content: "# Notes" },
  ])
    assert.deepEqual(
      validateToolArguments(tool, { name: tool.name, arguments: args }),
      args,
    );
  assert.throws(
    () =>
      validateToolArguments(tool, {
        name: tool.name,
        arguments: { name: "", format: "md", content: "x" },
      }),
    /Validation failed/,
  );
});

test("no tool when the host names no format", () => {
  assert.deepEqual(fixture({ formats: [] }).tools, []);
});

test("forwards office content as a batch array", async () => {
  const { calls, tool } = createDocument();
  const args = { name: "deck.pptx", format: "pptx", content: DECK };
  const result = await tool.execute("c1", args);
  assert.deepEqual(calls, [
    { name: "create_document", args, toolCallId: "c1" },
  ]);
  assert.notEqual(result.isError, true);
  assert.match(result.content[0].text, /attachment att-deck/);
  assert.match(result.content[0].text, /deck\.pptx \(pptx, [^)]*3 slides\)/);
  assert.match(result.content[0].text, /Issues: 0\./);
  assert.match(result.content[0].text, /Do not repeat the file contents/);
  assert.equal(result.details.file.id, "att-deck");
});

test("a deck object is rendered before it reaches the host", async () => {
  const { calls, tool } = createDocument();
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: DECK_OBJECT,
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].name, "create_document");
  assert.equal(calls[0].toolCallId, "c1");
  assert.equal(calls[0].args.name, "deck.pptx");
  assert.equal(calls[0].args.format, "pptx");
  assert.ok(Array.isArray(calls[0].args.content));
  assert.equal(calls[0].args.content[0].type, "slide");
  assert.notEqual(result.isError, true);
  const text = result.content[0].text;
  assert.match(text, /^Rendered 3 slides from the deck with theme navy\.\n/);
  assert.doesNotMatch(text, /Ignored:/);
  assert.match(text, /Created deck\.pptx .* as attachment att-deck/);
});

test("a one-element array holding the deck is a deck too", async () => {
  const { calls, tool } = createDocument();
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: [DECK_OBJECT],
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].args.content[0].type, "slide");
  assert.match(result.content[0].text, /^Rendered 3 slides /);
});

test("a deck sent as JSON text, without kind or as Deck is rendered", async () => {
  const { kind, ...unkinded } = DECK_OBJECT;
  assert.equal(kind, "deck");
  for (const content of [
    JSON.stringify(DECK_OBJECT),
    unkinded,
    [unkinded],
    { ...DECK_OBJECT, kind: "Deck" },
  ]) {
    const { calls, tool } = createDocument();
    const result = await tool.execute("c1", {
      name: "deck.pptx",
      format: "pptx",
      content,
    });
    assert.notEqual(result.isError, true, JSON.stringify(content));
    assert.equal(calls.length, 1);
    assert.equal(calls[0].args.content[0].type, "slide");
    assert.match(result.content[0].text, /^Rendered 3 slides /);
  }
});

test("pptx text that is no deck is told the deck object comes first", async () => {
  const { calls, tool } = createDocument();
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: "Slide 1: Quarterly results",
  });
  assert.equal(result.isError, true);
  assert.equal(result.details.code, "assistant_document_content");
  assert.equal(
    result.content[0].text,
    'pptx content must be a deck object ({"kind":"deck",…}) or, for experts, an array of officecli batch commands.',
  );
  assert.deepEqual(calls, []);
});

const RENDER_REFUSED =
  "The rendered batch was refused by the host (this is a renderer problem, not your deck): ";

test("a host refusal of a rendered deck is blamed on the renderer", async () => {
  const refusal =
    'assistant_document_failed: item 52: "props.data" must be the table\'s rows inline (CSV); files and addresses cannot be used';
  // The host refuses by failing the call (the IPC error's message is the
  // refusal), or with a failed reply.
  const replies = [
    () => {
      throw Object.assign(new Error(refusal), { code: refusal });
    },
    () => ({
      status: "failed",
      error: { code: "assistant_document_failed", message: refusal },
    }),
  ];
  for (const reply of replies) {
    const { tool } = createDocument({ reply });
    await assert.rejects(
      tool.execute("c1", {
        name: "deck.pptx",
        format: "pptx",
        content: DECK_OBJECT,
      }),
      (error) => {
        assert.equal(error.message, `${RENDER_REFUSED}${refusal}`);
        return true;
      },
    );
    // A batch the model wrote is its own: no prefix.
    const own = createDocument({ reply }).tool;
    await assert.rejects(
      own.execute("c2", { name: "deck.pptx", format: "pptx", content: DECK }),
      (error) => {
        assert.equal(error.message, refusal);
        return true;
      },
    );
  }
  // Other failures of a rendered deck keep their own words.
  const limit = "assistant_document_limit: at most 3 documents per reply";
  const { tool } = createDocument({
    reply: () => {
      throw Object.assign(new Error(limit), { code: limit });
    },
  });
  await assert.rejects(
    tool.execute("c3", {
      name: "deck.pptx",
      format: "pptx",
      content: DECK_OBJECT,
    }),
    (error) => {
      assert.equal(error.message, limit);
      return true;
    },
  );
});

test("what the deck ignored is told to the model", async () => {
  const { tool } = createDocument();
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: { ...DECK_OBJECT, theme: "neon" },
  });
  const [rendered, ignored] = result.content[0].text.split("\n");
  assert.equal(rendered, "Rendered 3 slides from the deck with theme navy.");
  assert.match(ignored, /^Ignored: deck: unknown theme "neon"; using navy/);
});

test("an invalid deck is refused without calling the host", async () => {
  const { calls, tool } = createDocument();
  const slides = [...DECK_OBJECT.slides];
  slides[1] = { type: "bullets", title: "Three things to know" };
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: { ...DECK_OBJECT, slides },
  });
  assert.equal(result.isError, true);
  assert.equal(result.details.code, "assistant_deck_invalid");
  assert.match(
    result.content[0].text,
    /^The deck was not rendered; fix these and send the whole deck again:\n- /,
  );
  assert.match(result.content[0].text, /unknown type "bullets"/);
  assert.deepEqual(calls, []);
});

test("a deck is only read for a pptx", async () => {
  const { calls, tool } = createDocument();
  const result = await tool.execute("c1", {
    name: "deck.json",
    format: "json",
    content: [DECK_OBJECT],
  });
  assert.equal(result.isError, true);
  assert.equal(result.details.code, "assistant_document_content");
  assert.deepEqual(calls, []);
});

test("a batch array still goes through unchanged", async () => {
  const { calls, tool } = createDocument();
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: DECK,
  });
  assert.deepEqual(calls[0].args.content, DECK);
  assert.doesNotMatch(result.content[0].text, /Rendered|Ignored/);
  assert.match(result.content[0].text, /^Created deck\.pptx /);
});

test("the host's layout fixes are told to the model", async () => {
  const text = async (extra) => {
    const { tool } = createDocument({ reply: () => completed(extra) });
    const result = await tool.execute("c1", {
      name: "deck.pptx",
      format: "pptx",
      content: DECK,
    });
    return result.content[0].text;
  };
  assert.match(
    await text({ fixed: 3 }),
    /^Created .* Issues: 0\. Auto-fixed 3 layout issues\./,
  );
  assert.match(
    await text({ fixed: 1 }),
    /^Created .* Issues: 0\. Auto-fixed 1 layout issue\.\n/,
  );
  for (const extra of [{}, { fixed: 0 }, { fixed: "3" }])
    assert.doesNotMatch(await text(extra), /Auto-fixed/, JSON.stringify(extra));
});

test("rejects string content for office formats and arrays for pdf", async () => {
  const { calls, tool } = createDocument();
  const office = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: "Slide 1: Quarterly results",
  });
  assert.equal(office.isError, true);
  assert.match(office.content[0].text, /array/);
  const pdf = await tool.execute("c2", {
    name: "report.pdf",
    format: "pdf",
    content: DECK,
  });
  assert.equal(pdf.isError, true);
  assert.match(pdf.content[0].text, /string/);
  const text = await tool.execute("c3", {
    name: "notes.md",
    format: "md",
    content: [{ text: "# Notes" }],
  });
  assert.equal(text.isError, true);
  assert.match(text.content[0].text, /string/);
  assert.deepEqual(calls, []);
});

test("attaches a thumbnail for vision models and strips paths", async (t) => {
  const directory = await folder(t);
  const previewPath = await writePng(
    path.join(directory, "preview-1.png"),
    64,
    64,
  );
  const reply = () =>
    completed({ filePath: path.join(directory, "deck.pptx"), previewPath });
  const args = { name: "deck.pptx", format: "pptx", content: DECK };

  const seeing = await createDocument({ vision: true, reply }).tool.execute(
    "c1",
    args,
  );
  assert.equal(seeing.content.length, 2);
  assert.equal(seeing.content[1].type, "image");
  assert.equal(seeing.content[1].mimeType, "image/jpeg");
  assert.ok(seeing.content[1].data.length > 0);
  assert.equal(seeing.details.previewPath, undefined);
  assert.equal(seeing.details.filePath, undefined);
  assert.doesNotMatch(JSON.stringify(seeing), new RegExp(directory));

  const blind = await createDocument({ vision: false, reply }).tool.execute(
    "c2",
    args,
  );
  assert.deepEqual(
    blind.content.map((part) => part.type),
    ["text"],
  );
  assert.equal(blind.details.previewPath, undefined);
  assert.equal(blind.details.filePath, undefined);
});

test("thumbnail stays within budget", async (t) => {
  const directory = await folder(t);
  const previewPath = await writePng(
    path.join(directory, "preview-1.png"),
    2000,
    2000,
  );
  const { tool } = createDocument({
    reply: () => completed({ previewPath }),
  });
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: DECK,
  });
  assert.equal(result.content[1].type, "image");
  assert.ok(Buffer.from(result.content[1].data, "base64").length <= 200 * 1024);
  assert.ok(JSON.stringify(result).length < 1024 * 1024);
});

test("a pdf result reports its page count from the file", async (t) => {
  const directory = await folder(t);
  const filePath = path.join(directory, "report.pdf");
  await writeFile(filePath, pdfPages(["One", "Two", "Three"]));
  const { tool } = createDocument({
    vision: false,
    reply: () =>
      completed({
        file: {
          id: "att-report",
          name: "report.pdf",
          mimeType: "application/pdf",
          bytes: 900,
        },
        filePath,
      }),
  });
  const result = await tool.execute("c1", {
    name: "report.pdf",
    format: "pdf",
    content: "= Report\nThree pages.",
  });
  assert.equal(result.details.file.pages, 3);
  assert.match(result.content[0].text, /report\.pdf \(pdf, [^)]*3 pages\)/);
  assert.equal(result.details.filePath, undefined);
});

test("an SVG slide preview is attached when the canvas reads SVG", async (t) => {
  const directory = await folder(t);
  const previewPath = path.join(directory, "preview-1.svg");
  await writeFile(
    previewPath,
    '<svg xmlns="http://www.w3.org/2000/svg" width="960" height="540"><rect width="960" height="540" fill="#fff"/><text x="40" y="100" font-size="60">Quarterly results</text></svg>',
  );
  const warn = t.mock.method(console, "warn", () => {});
  const { tool } = createDocument({
    reply: () => completed({ previewPath }),
  });
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: DECK,
  });
  assert.match(result.content[0].text, /attachment att-deck/);
  // Without SVG support in this canvas build the preview is skipped.
  if (result.content.length === 1) {
    assert.equal(warn.mock.callCount(), 1);
    return;
  }
  assert.equal(result.content[1].type, "image");
  assert.equal(result.content[1].mimeType, "image/jpeg");
});

// Warnings go through console, which main.mjs sends to stderr without the
// run's credentials.
test("an unreadable preview is skipped with one warning", async (t) => {
  const directory = await folder(t);
  const broken = path.join(directory, "preview-1.svg");
  await writeFile(broken, "this is not an svg");
  for (const previewPath of [broken, path.join(directory, "missing.png")]) {
    const warn = t.mock.method(console, "warn", () => {});
    const { tool } = createDocument({
      reply: () => completed({ previewPath }),
    });
    const result = await tool.execute("c1", {
      name: "deck.pptx",
      format: "pptx",
      content: DECK,
    });
    warn.mock.restore();
    assert.deepEqual(
      result.content.map((part) => part.type),
      ["text"],
    );
    assert.equal(result.details.previewPath, undefined);
    assert.equal(warn.mock.callCount(), 1);
    assert.match(String(warn.mock.calls[0].arguments[0]), /preview/i);
  }
});

test("the host's issues are listed for the model to mention", async () => {
  const { tool } = createDocument({
    vision: false,
    reply: () =>
      completed({
        issues: [
          {
            severity: "warning",
            message: "Text overflows its box",
            path: "/slide[2]/shape[1]",
          },
          { severity: "error", message: "Missing title" },
        ],
      }),
  });
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: DECK,
  });
  const text = result.content[0].text;
  assert.match(text, /Issues: 2\./);
  assert.match(
    text,
    /warning at \/slide\[2\]\/shape\[1\]: Text overflows its box/,
  );
  assert.match(text, /error: Missing title/);
});

test("malformed issues from the host never fail a created file", async () => {
  const { calls, tool } = createDocument({
    vision: false,
    reply: () => completed({ issues: [null, "text", { message: "x" }, 7] }),
  });
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: DECK,
  });
  assert.notEqual(result.isError, true);
  assert.equal(calls.length, 1);
  const text = result.content[0].text;
  assert.match(text, /attachment att-deck/);
  assert.match(text, /Issues: 2\./);
  assert.match(text, /- issue: text\n- issue: x\n/);
});

test("a reply that is not a completed document fails the call", async () => {
  const { tool } = createDocument({
    reply: () => ({
      status: "failed",
      error: {
        code: "assistant_document_failed",
        message: "batch item 2 failed",
      },
    }),
  });
  await assert.rejects(
    tool.execute("c1", { name: "deck.pptx", format: "pptx", content: DECK }),
    /batch item 2 failed/,
  );
});

test("the guidelines tell the model to make files and read the skill first", () => {
  const guidelines = fixture().tools[0].promptGuidelines.join("\n");
  assert.match(
    guidelines,
    /When the user asks for a file, or names a PDF, Word, PowerPoint or Excel document, produce it with create_document; never say you cannot attach files\./,
  );
  assert.match(
    guidelines,
    /Before the first PDF, Word, PowerPoint or Excel document in a conversation, call use_skill on its skill \(ofox-office-pdf, ofox-office-docx, ofox-office-pptx or ofox-office-xlsx\)/,
  );
  assert.doesNotMatch(guidelines, /cannot produce/);
  assert.match(guidelines, /two or three sentences on the structure first/);
  assert.match(
    guidelines,
    /At most 3 documents per reply; retry a failed document at most twice/,
  );
});

test("without the engines the guidelines say which files cannot be made", () => {
  const textOnly = fixture({ formats: ["md", "txt", "csv", "html", "json"] })
    .tools[0].promptGuidelines;
  const text = textOnly.join("\n");
  assert.match(
    text,
    /When the user asks for a file, produce it with create_document\./,
  );
  assert.match(
    text,
    /This machine cannot produce PDF, Word, PowerPoint or Excel files; say so if asked\./,
  );
  assert.doesNotMatch(
    text,
    /never say you cannot attach|use_skill|ofox-office/,
  );

  const pdfOnly = fixture({
    formats: ["md", "txt", "pdf"],
  }).tools[0].promptGuidelines.join("\n");
  assert.match(
    pdfOnly,
    /or names a PDF document, produce it with create_document\./,
  );
  assert.match(
    pdfOnly,
    /This machine cannot produce Word, PowerPoint or Excel files; say so if asked\./,
  );
  assert.match(pdfOnly, /call use_skill on its skill \(ofox-office-pdf\)/);
  assert.doesNotMatch(
    pdfOnly,
    /never say you cannot attach|ofox-office-(md|txt|docx)/,
  );
});

test("the description and guidelines teach the deck object", () => {
  const [tool] = fixture().tools;
  assert.match(
    tool.description,
    /pptx: content is a deck object \(\{"kind":"deck",…\}, see the ofox-office-pptx skill\) or, for experts, an array of officecli batch commands/,
  );
  assert.match(
    tool.promptGuidelines.join("\n"),
    /For a pptx, send the deck object the skill describes; do not write slide coordinates yourself\./,
  );
});

test("the schema is plain JSON Schema: an enum and a string or object list", () => {
  const { tool } = createDocument({ formats: ["md", "pptx"] });
  const { format, content } = tool.parameters.properties;
  assert.deepEqual(format, { type: "string", enum: ["md", "pptx"] });
  assert.deepEqual(content, {
    anyOf: [
      { type: "string", maxLength: 2_000_000 },
      {
        type: "array",
        items: { type: "object", properties: {}, additionalProperties: true },
        maxItems: 2000,
      },
    ],
  });
  assert.doesNotMatch(
    JSON.stringify(tool.parameters),
    /patternProperties|"const"/,
  );
  // Pi coerces a scalar to a string and one object to a list of it; a
  // list of anything but objects, or one over the limits, fails.
  for (const content of [
    ["text"],
    [[DECK[0]]],
    [5],
    Array(2001).fill({}),
    "x".repeat(2_000_001),
  ])
    assert.throws(
      () =>
        validateToolArguments(tool, {
          name: tool.name,
          arguments: { name: "deck.pptx", format: "pptx", content },
        }),
      /Validation failed/,
      JSON.stringify(content),
    );
});

// ---- Python decks and previews (the host offers them with documents.python)

const PYTHON = {
  kind: "python",
  code: 'from deck import *\nprs = new_deck()\ncover_slide(prs, "Quarterly results")\n',
};

const previewed = (extra = {}) => ({
  status: "completed",
  preview: true,
  previewId: "doc-9",
  slides: 3,
  issues: [],
  pagePaths: [],
  ...extra,
});

// Whether this build of the canvas reads SVG at all, so only that skips a
// test and a failure in the overview code fails it.
async function readsSvg() {
  try {
    await loadImage(
      Buffer.from(
        '<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>',
      ),
    );
    return true;
  } catch {
    return false;
  }
}

/** `count` slide SVGs in `directory`, as the host's pages/N.svg; their paths. */
async function writePages(directory, count) {
  const paths = [];
  for (let page = 1; page <= count; page++) {
    const file = path.join(directory, `${page}.svg`);
    await writeFile(
      file,
      `<svg xmlns="http://www.w3.org/2000/svg" width="960" height="540"><rect width="960" height="540" fill="#1E3A5F"/><text x="40" y="100" font-size="60" fill="#fff">Slide ${page}</text></svg>`,
    );
    paths.push(file);
  }
  return paths;
}

test("mentions python only when the host offers it", () => {
  const offered = fixture({ python: true }).tools[0];
  const words = [offered.description, ...offered.promptGuidelines];
  for (const text of [words[0], words.slice(1).join("\n")]) {
    assert.match(text, /\{"kind":"python"/);
    assert.match(text, /preview/);
    assert.match(text, /fromPreview/);
  }
  assert.match(offered.description, /deck object/);

  const absent = fixture({ python: false }).tools[0];
  const plain = [absent.description, ...absent.promptGuidelines].join("\n");
  assert.doesNotMatch(plain, /python|preview|fromPreview/i);
  assert.equal(
    absent.description,
    fixture({}).tools[0].description,
    "documents.python missing reads like false",
  );
});

test("the guidelines tell a python model to preview, then deliver", () => {
  const text = fixture({ python: true }).tools[0].promptGuidelines.join("\n");
  assert.match(text, /preview: true/);
  assert.match(text, /fromPreview: "<previewId>"/);
  assert.match(text, /ends? the reply without delivering/);
  assert.match(text, /At most 3 documents per reply/);
  // Without pptx there is nothing to write Python for.
  const textOnly = fixture({ python: true, formats: ["md", "pdf"] }).tools[0];
  assert.doesNotMatch(
    [textOnly.description, ...textOnly.promptGuidelines].join("\n"),
    /python/i,
  );
});

test("python content and a preview pass the schema only when offered", () => {
  const offered = createDocument({ python: true }).tool;
  const args = { name: "deck.pptx", format: "pptx", content: PYTHON };
  // Pi must not wrap the one object into a list.
  for (const extra of [{}, { preview: true }]) {
    assert.deepEqual(
      validateToolArguments(offered, {
        name: offered.name,
        arguments: { ...args, ...extra },
      }),
      { ...args, ...extra },
    );
  }
  // fromPreview delivers a file made already: no content.
  const delivery = {
    name: "deck.pptx",
    format: "pptx",
    fromPreview: "doc-9",
  };
  assert.deepEqual(
    validateToolArguments(offered, { name: offered.name, arguments: delivery }),
    delivery,
  );
  const { content } = offered.parameters.properties;
  const [, , code] = content.anyOf;
  assert.equal(code.properties.code.maxLength, 200_000);

  const plain = createDocument({ python: false }).tool;
  assert.deepEqual(Object.keys(plain.parameters.properties), [
    "name",
    "format",
    "content",
  ]);
  assert.deepEqual(plain.parameters.required, ["name", "format", "content"]);
  assert.doesNotMatch(
    JSON.stringify(offered.parameters),
    /patternProperties|"const"/,
  );
});

test("passes python content to the host unchanged", async () => {
  // As the model sent it: the object, Pi's one-element list around it, or
  // its JSON text. Never a deck: the renderer must not touch the code.
  const huge = { ...PYTHON, code: "x = 1\n".repeat(40_000) };
  for (const content of [
    PYTHON,
    [PYTHON],
    JSON.stringify(PYTHON),
    { kind: "python" },
  ]) {
    const { calls, tool } = createDocument({ python: true });
    const args = { name: "deck.pptx", format: "pptx", content };
    const result = await tool.execute("c1", { ...args, preview: true });
    const sent = Array.isArray(content) ? content[0] : content;
    assert.deepEqual(
      calls,
      [
        {
          name: "create_document",
          args: { ...args, content: sent, preview: true },
          toolCallId: "c1",
        },
      ],
      JSON.stringify(content),
    );
    assert.doesNotMatch(result.content[0].text, /Rendered|Ignored/);
  }
  // Over the schema's length the schema lets it through as a list, and the
  // host's byte limit, not this runtime, refuses it.
  const { calls, tool } = createDocument({ python: true });
  await tool.execute("c2", {
    name: "deck.pptx",
    format: "pptx",
    content: [huge],
  });
  assert.equal(calls[0].args.content, huge);
});

test("python content is the host's to refuse when it offers no python", async () => {
  const refusal =
    "assistant_document_failed: Python content cannot run on this computer; send a deck object instead";
  const { tool } = createDocument({
    python: false,
    reply: () => {
      throw Object.assign(new Error(refusal), { code: refusal });
    },
  });
  await assert.rejects(
    tool.execute("c1", { name: "deck.pptx", format: "pptx", content: PYTHON }),
    (error) => {
      assert.equal(error.message, refusal);
      return true;
    },
  );
});

test("python content for another format is the host's to refuse, text is its own", async () => {
  const { calls, tool } = createDocument({ python: true });
  const docx = await tool.execute("c1", {
    name: "a.docx",
    format: "docx",
    content: PYTHON,
  });
  assert.equal(docx.isError, true);
  assert.equal(docx.details.code, "assistant_document_content");
  const text = JSON.stringify(PYTHON);
  await tool.execute("c2", { name: "a.json", format: "json", content: text });
  assert.deepEqual(
    calls.map((call) => call.args.content),
    [text],
  );
});

test("text that is no deck or python is told what a pptx takes", async () => {
  const content = async (python) => {
    const { tool } = createDocument({ python });
    const result = await tool.execute("c1", {
      name: "deck.pptx",
      format: "pptx",
      content: "Slide 1: Quarterly results",
    });
    return result.content[0].text;
  };
  assert.equal(
    await content(true),
    'pptx content must be {"kind":"python","code":"…"}, a deck object ({"kind":"deck",…}) or, for experts, an array of officecli batch commands.',
  );
  assert.match(await content(false), /^pptx content must be a deck object /);
});

test("a call with neither content nor fromPreview is refused without the host", async () => {
  const { calls, tool } = createDocument({ python: true });
  for (const [format, name] of [
    ["pptx", "deck.pptx"],
    ["md", "notes.md"],
  ]) {
    const result = await tool.execute("c1", { name, format });
    assert.equal(result.isError, true);
    assert.equal(result.details.code, "assistant_document_content");
  }
  assert.deepEqual(calls, []);
});

test("fromPreview delivers without content and is described as a created file", async () => {
  const { calls, tool } = createDocument({ python: true });
  const args = { name: "deck.pptx", format: "pptx", fromPreview: "doc-9" };
  const result = await tool.execute("c1", args);
  assert.deepEqual(calls, [
    { name: "create_document", args, toolCallId: "c1" },
  ]);
  assert.notEqual(result.isError, true);
  assert.match(
    result.content[0].text,
    /^Created deck\.pptx .* attachment att-deck/,
  );
  // Content next to it is the host's to refuse: it is not rendered.
  await tool.execute("c2", { ...args, content: DECK_OBJECT });
  assert.deepEqual(calls[1].args.content, DECK_OBJECT);
});

test("a preview result carries one contact sheet", async (t) => {
  if (!(await readsSvg())) {
    t.skip("SVG unsupported");
    return;
  }
  const directory = await folder(t);
  const pagePaths = await writePages(directory, 3);
  const reply = () =>
    previewed({
      pagePaths,
      issues: [
        {
          severity: "warning",
          message: "Text overflows its box",
          path: "/slide[2]/shape[1]",
        },
      ],
    });
  const { calls, tool } = createDocument({ python: true, reply });
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: PYTHON,
    preview: true,
  });
  assert.notEqual(result.isError, true);
  assert.deepEqual(
    result.content.map((part) => part.type),
    ["text", "image"],
  );
  const image = await loadImage(Buffer.from(result.content[1].data, "base64"));
  assert.equal(result.content[1].mimeType, "image/jpeg");
  assert.equal(image.width, 1600);
  const text = result.content[0].text;
  assert.match(text, /previewId doc-9/);
  assert.match(text, /\(pptx, 3 slides\)/);
  assert.match(text, /Issues: 1\./);
  assert.match(
    text,
    /warning at \/slide\[2\]\/shape\[1\]: Text overflows its box/,
  );
  assert.match(text, /deliver with fromPreview: "doc-9"/);
  assert.match(text, /delivered automatically/);
  assert.match(text, /without preview/);
  assert.doesNotMatch(text, /Created|attachment/);
  // The host's paths are for this runtime alone.
  assert.equal(result.details.pagePaths, undefined);
  assert.equal(result.details.previewId, "doc-9");
  assert.equal(result.details.preview, true);
  assert.doesNotMatch(JSON.stringify(result), new RegExp(directory));
  assert.doesNotMatch(JSON.stringify(result), /pagePaths/);
  assert.equal(calls.length, 1);
});

test("a preview shown to a model that cannot see images is text only", async (t) => {
  const directory = await folder(t);
  const pagePaths = await writePages(directory, 2);
  const { tool } = createDocument({
    python: true,
    vision: false,
    reply: () => previewed({ pagePaths, slides: 2 }),
  });
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: PYTHON,
    preview: true,
  });
  assert.deepEqual(
    result.content.map((part) => part.type),
    ["text"],
  );
  const text = result.content[0].text;
  assert.match(text, /previewId doc-9/);
  assert.match(text, /2 slides/);
  assert.match(text, /Issues: 0\./);
  assert.match(text, /deliver with fromPreview: "doc-9"/);
  assert.doesNotMatch(JSON.stringify(result), new RegExp(directory));
  assert.equal(result.details.pagePaths, undefined);
});

test("a preview whose pages cannot be drawn is text only, with one warning", async (t) => {
  const directory = await folder(t);
  const broken = path.join(directory, "1.svg");
  await writeFile(broken, "this is not an svg");
  for (const pagePaths of [[broken], [path.join(directory, "missing.svg")]]) {
    const warn = t.mock.method(console, "warn", () => {});
    const { tool } = createDocument({
      python: true,
      reply: () => previewed({ pagePaths }),
    });
    const result = await tool.execute("c1", {
      name: "deck.pptx",
      format: "pptx",
      content: PYTHON,
      preview: true,
    });
    warn.mock.restore();
    assert.deepEqual(
      result.content.map((part) => part.type),
      ["text"],
    );
    assert.match(result.content[0].text, /previewId doc-9/);
    assert.match(result.content[0].text, /No overview of the slides/);
    assert.equal(warn.mock.callCount(), 1);
    assert.match(String(warn.mock.calls[0].arguments[0]), /overview/i);
  }
  // No pages at all (a text file's preview): nothing to draw, nothing to warn.
  const warn = t.mock.method(console, "warn", () => {});
  const { tool } = createDocument({ python: true, reply: () => previewed() });
  const result = await tool.execute("c2", {
    name: "notes.md",
    format: "md",
    content: "# Notes",
    preview: true,
  });
  assert.equal(result.content.length, 1);
  assert.equal(warn.mock.callCount(), 0);
});

test("the overview shows the first 24 slides and says how many there are", async (t) => {
  const directory = await folder(t);
  const pagePaths = await writePages(directory, 25);
  const { tool } = createDocument({
    python: true,
    vision: false,
    reply: () => previewed({ pagePaths, slides: 25 }),
  });
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: PYTHON,
    preview: true,
  });
  // Without vision nothing is read or drawn, and nothing is said about it.
  assert.doesNotMatch(result.content[0].text, /first 24/);
  if (!(await readsSvg())) return;
  const seeing = createDocument({
    python: true,
    reply: () => previewed({ pagePaths, slides: 25 }),
  }).tool;
  const shown = await seeing.execute("c2", {
    name: "deck.pptx",
    format: "pptx",
    content: PYTHON,
    preview: true,
  });
  assert.match(shown.content[0].text, /It shows the first 24 of 25 slides\./);
  const image = await loadImage(Buffer.from(shown.content[1].data, "base64"));
  // Twelve rows of 16:9 cells.
  assert.equal(image.height, 24 + 12 * (32 + 430 + 24));
});

test("a deck object can be previewed too, with what was rendered first", async () => {
  const { tool } = createDocument({
    python: true,
    vision: false,
    reply: () => previewed(),
  });
  const result = await tool.execute("c1", {
    name: "deck.pptx",
    format: "pptx",
    content: DECK_OBJECT,
    preview: true,
  });
  assert.match(
    result.content[0].text,
    /^Rendered 3 slides from the deck with theme navy\.\nPreview/,
  );
});

test("a completed reply with neither a file nor a preview still fails the call", async () => {
  for (const reply of [
    { status: "completed", issues: [] },
    { status: "completed", preview: true, issues: [], pagePaths: [] },
    { status: "completed", preview: false, previewId: "doc-9" },
  ]) {
    const { tool } = createDocument({ python: true, reply: () => reply });
    await assert.rejects(
      tool.execute("c1", { name: "deck.pptx", format: "pptx", content: DECK }),
      /The host did not create the document/,
    );
  }
});
