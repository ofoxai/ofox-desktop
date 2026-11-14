import { readFile } from "node:fs/promises";
import path from "node:path";
import { Type } from "typebox";
import { withPdf } from "./attachments.mjs";
import { deckOf, parseDeck } from "./deck/schema.mjs";
import { renderDeck } from "./deck/render.mjs";
import { themeFor, THEME_NAMES } from "./deck/theme.mjs";
import { presentationStyleTool } from "./presentation-styles.mjs";
import {
  createPresentationPlanning,
  planDrift,
} from "./presentation-planning.mjs";
import { errorMessage } from "./security.mjs";
import {
  normalizeOfficeArrowheads,
  normalizeOfficePatternImages,
  safeSvg,
} from "./document-preview.mjs";
import {
  contactSheet,
  thumbnailFromPng,
  thumbnailFromSvg,
} from "./thumbnails.mjs";

// create_document: the model writes the content (for a pptx, Python deck
// code the host runs, or a deck object this runtime renders into the batch),
// the host runs the engine, checks the file and keeps it as a generated
// attachment. The host's reply also names the file's path and its first
// page's preview for this runtime alone: from them it adds a PDF's page
// count and, for a model that sees images, a thumbnail. A preview (the host
// makes and checks the file and delivers nothing) answers with every slide's
// SVG path instead, which become one overview image. The model never gets
// any of these paths.

const OFFICE_FORMATS = new Set(["pptx", "docx", "xlsx"]);
const MAX_ISSUE_TEXT = 500;
// The most Python code the schema lets through; the host counts the bytes
// and its limit is the authority.
const MAX_PYTHON_CHARACTERS = 200_000;
// The most slides one overview image shows: a taller sheet is downscaled
// beyond reading, or refused by a provider.
const OVERVIEW_SLIDES = 24;
const PREVIEWS = { ".png": thumbnailFromPng, ".svg": thumbnailFromSvg };

// The formats a user names, in the words they use; each has its skill.
const NAMED_FORMATS = [
  ["pdf", "PDF"],
  ["docx", "Word"],
  ["pptx", "PowerPoint"],
  ["xlsx", "Excel"],
];

const either = (words) =>
  words.length > 1
    ? `${words.slice(0, -1).join(", ")} or ${words.at(-1)}`
    : words[0];

// What the model is told depends on what this machine can make: a format
// whose engine is missing is one the model says it cannot produce.
function guidelines(
  formats,
  python,
  { planned = false, primedPptx = false } = {},
) {
  const named = NAMED_FORMATS.filter(([format]) => formats.includes(format));
  const needSkill = named.filter(
    ([format]) => format !== "pptx" || !primedPptx,
  );
  const missing = NAMED_FORMATS.filter(([format]) => !formats.includes(format));
  const words = (list) => either(list.map(([, word]) => word));
  return [
    named.length
      ? `When the user asks for a file, or names a ${words(named)} document, produce it with create_document${missing.length ? "." : "; never say you cannot attach files."}`
      : "When the user asks for a file, produce it with create_document.",
    missing.length &&
      `This machine cannot produce ${words(missing)} files; say so if asked.`,
    needSkill.length &&
      `Before the first ${words(needSkill)} document in a conversation, call use_skill on its skill (${either(needSkill.map(([format]) => `ofox-office-${format}`))}) and follow it.`,
    primedPptx &&
      "The Host has already verified and loaded the builtin ofox-office-pptx instructions for this current run. Follow them; do not call use_skill merely to load that same builtin again. No other skill is primed by this marker.",
    formats.includes("html") &&
      "Before creating an HTML page, call use_skill on ofox-office-html. Plan the user's task and real interactions, then build a complete responsive page with keyboard access and visible state changes. Saving HTML does not test it in a browser.",
    formats.includes("pptx") &&
      (planned
        ? "For planned PPTX, call get_presentation_candidates first. Choose only an eligible ID, with manual choice taking priority, then call get_presentation_style for that ID's recipe. Reuse the complete candidate requirements when freezing, including the required overall density. Freeze one plan containing every page with freeze_presentation_plan, then create_document. Plan each slide's claim, evidence, visual, composition and editing requirements independently."
        : "Before designing a pptx, call get_presentation_style to select a coherent style by audience, purpose and density, then read that style's recipe. Plan each slide's claim, evidence, visual, composition and editing requirements independently before writing the deck."),
    formats.includes("pptx") &&
      (python
        ? 'For a pptx, send the Python deck code ({"kind":"python","code":"…"}) or the deck object the skill describes; do not write slide coordinates yourself.'
        : "For a pptx, send the deck object the skill describes; do not write slide coordinates yourself."),
    python &&
      'Send a pptx with preview: true first: look at the overview and the issues, fix what is wrong, then deliver under the same file name by calling again without preview (the code may change) or with fromPreview: "<previewId>" and no content. If you end the reply without delivering, the latest preview is delivered automatically.',
    "Reply with two or three sentences on the structure first, then call the tool; after the result, summarize the file and mention issues.",
    "Write explanations, status updates and completion summaries in the user's language. Keep tool retries concise. An automated count of zero findings does not verify visual quality, formulas, interactions or editing; state what was actually checked and what remains unverified.",
    formats.includes("pptx") &&
      "For PPTX, finish concisely: deliver the file, state the selected style and its reason, then actual checks and remaining issues. Do not include attachment IDs, plan IDs, revisions, internal protocol or helper names unless the user specifically asks for those details. Fix ordinary layout overlaps, clipping and text overflow within the existing task without asking for approval. Do not infer visual quality or target-application editing from structural checks. Use a longer explanation only when requested.",
    "At most 3 documents per reply; retry a failed document at most twice, then explain.",
  ].filter(Boolean);
}

function description(formats, python, planned = false) {
  const office = formats.filter((format) => OFFICE_FORMATS.has(format));
  const text = formats.filter(
    (format) => !OFFICE_FORMATS.has(format) && format !== "pdf",
  );
  const batch = office.filter((format) => format !== "pptx");
  const shapes = [
    office.includes("pptx") &&
      (planned
        ? python
          ? 'pptx: content is {"kind":"python","code":"…"} or a structured deck object ({"kind":"deck",…}); raw OfficeCLI batches cannot enforce the frozen presentation plan'
          : 'pptx: content is a structured deck object ({"kind":"deck",…}); raw OfficeCLI batches cannot enforce the frozen presentation plan'
        : python
          ? 'pptx: content is {"kind":"python","code":"…"} (the Python deck code the ofox-office-pptx skill describes), a deck object ({"kind":"deck",…}) or, for experts, an array of officecli batch commands'
          : 'pptx: content is a deck object ({"kind":"deck",…}, see the ofox-office-pptx skill) or, for experts, an array of officecli batch commands'),
    batch.length &&
      `${batch.join(", ")}: content is an array of officecli batch commands`,
    formats.includes("pdf") && "pdf: content is the Typst markup of the body",
    text.length && `${text.join(", ")}: content is the file's text`,
  ].filter(Boolean);
  const preview = python
    ? " A pptx can be previewed first (preview: true): the host makes and checks it, and you get its previewId, its issues and an overview of its slides; nothing is delivered until you call again without preview, or send fromPreview with that previewId and no content, under the same file name."
    : "";
  return `Create one file the user can open and save; it appears under your reply. ${shapes.join("; ")}.${preview}`;
}

const isObject = (value) =>
  value !== null && typeof value === "object" && !Array.isArray(value);

// A pptx's Python deck code, as the host reads it: the object
// {"kind":"python","code":"…"}, Pi's list of one around it (Pi wraps an
// object no schema variant matched, such as code over the length limit), or
// its JSON text. Returns what the host should be sent, which is the code as
// it came, or undefined when the content is not Python; the host checks the
// code and its size.
function pythonContent(content) {
  let given = content;
  if (typeof given === "string") {
    try {
      given = JSON.parse(given);
    } catch {
      return undefined;
    }
  }
  const candidate =
    Array.isArray(given) && given.length === 1 ? given[0] : given;
  if (!isObject(candidate) || candidate.kind !== "python") return undefined;
  return typeof content === "string" ? content : candidate;
}

// Office formats take a batch of commands (a pptx usually a deck object,
// which is rendered into one first), the others a text.
function contentProblem({ format, content }, python) {
  if (OFFICE_FORMATS.has(format)) {
    if (Array.isArray(content)) return null;
    if (format !== "pptx")
      return `${format} content must be an array of officecli batch commands.`;
    return python
      ? 'pptx content must be {"kind":"python","code":"…"}, a deck object ({"kind":"deck",…}) or, for experts, an array of officecli batch commands.'
      : 'pptx content must be a deck object ({"kind":"deck",…}) or, for experts, an array of officecli batch commands.';
  }
  if (typeof content === "string") return null;
  return format === "pdf"
    ? "pdf content must be a string of Typst markup."
    : `${format} content must be a string with the file's text.`;
}

// The host refuses a batch with this code (create.rs FAILED), whether its
// checks or officecli turned it down.
const DOCUMENT_FAILED = "assistant_document_failed";

// A batch this runtime rendered from a deck and the host refused is the
// renderer's fault: the model is told so, or it would rewrite a deck that
// was fine.
const RENDER_REFUSED =
  "The rendered batch was refused by the host (this is a renderer problem, not your deck): ";

// Whether a host failure is its refusal of the batch: its code, or the
// code its message starts with (a refusal reads "<code>: <why>").
const isRefusal = (code, message) =>
  code === DOCUMENT_FAILED || String(message).startsWith(DOCUMENT_FAILED);

// A thrown host failure, prefixed with RENDER_REFUSED when it refused a
// rendered batch; any other failure is left as it is.
function refusedRender(error) {
  if (!isRefusal(error?.code, error?.message)) return error;
  return Object.assign(new Error(`${RENDER_REFUSED}${error.message}`), {
    code: error.code,
    outcomeUnknown: error.outcomeUnknown,
  });
}

// A deck object becomes the batch that draws it, or the problems that stop
// it. The renderer throws only for what parseDeck should have refused.
function renderedDeck(content) {
  const parsed = parseDeck(content);
  if (parsed.problems) return { problems: parsed.problems };
  const { deck, warnings } = parsed;
  try {
    const { commands, slides } = renderDeck(deck, themeFor(deck.theme));
    return {
      commands,
      told: [
        `Rendered ${slides} slides from the deck with theme ${deck.theme}.`,
        warnings.length > 0 && `Ignored: ${warnings.join("; ")}.`,
      ].filter(Boolean),
    };
  } catch (error) {
    return { problems: [errorMessage(error)] };
  }
}

const quantity = (count, unit) =>
  Number.isSafeInteger(count)
    ? `${count} ${unit}${count === 1 ? "" : "s"}`
    : "";

function size(bytes) {
  if (!Number.isSafeInteger(bytes)) return "";
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

// The host's issues as objects: a bare string is its message, and anything
// else is left out. The file exists by now, so no issue may fail the call.
function issueList(issues) {
  if (!Array.isArray(issues)) return [];
  return issues.flatMap((issue) => {
    if (typeof issue === "string") return [{ message: issue }];
    if (issue && typeof issue === "object" && !Array.isArray(issue))
      return [issue];
    return [];
  });
}

function issueLine({ severity, message, path: where }) {
  const place = where ? ` at ${String(where)}` : "";
  return `- ${String(severity ?? "issue")}${place}: ${String(message ?? "").slice(0, MAX_ISSUE_TEXT)}`;
}

function describe(result, format) {
  const { file } = result;
  const issues = issueList(result.issues);
  const facts = [
    format,
    size(file.bytes),
    quantity(file.slides, "slide"),
    quantity(file.pages, "page"),
    quantity(file.sheets, "sheet"),
  ].filter(Boolean);
  // The host adds `fixed` for Office files: layout problems it repaired.
  const repaired =
    result.fixed > 0 ? quantity(result.fixed, "layout issue") : "";
  const fixed = repaired && ` Auto-fixed ${repaired}.`;
  return [
    `Created ${file.name} (${facts.join(", ")}) as attachment ${file.id}.${OFFICE_FORMATS.has(format) ? ` Automated check findings: ${issues.length}.${fixed}` : ""}`,
    OFFICE_FORMATS.has(format)
      ? "Check scope: automated structure/layout checks only. Visual appearance, formulas and editing in the target application are not verified by this count; zero findings do not certify those results."
      : format === "html"
        ? "HTML was saved; browser rendering and interactions were not verified. Do not claim buttons, keyboard behavior or layout were tested without actual browser evidence."
        : "Saving or compiling the file does not certify its visual appearance or content accuracy.",
    ...issues.map(issueLine),
    "Do not repeat the file contents; tell the user what the file contains.",
  ].join("\n");
}

// What a preview tells the model: its id, what it holds and its issues. The
// next lines (see overviewNote and previewNext) say what to do with it.
function describePreview(result, { name, format }) {
  const issues = issueList(result.issues);
  const facts = [
    format,
    quantity(result.slides, "slide"),
    quantity(result.pages, "page"),
    quantity(result.sheets, "sheet"),
  ].filter(Boolean);
  return [
    `Previewed ${name} (${facts.join(", ")}) as previewId ${result.previewId}; nothing is delivered yet. Automated check findings: ${issues.length}.`,
    "This count covers automated structure/layout findings, not visual or formula acceptance. Inspect the overview; mathematical display and target-application editing need separate verification.",
    ...issues.map(issueLine),
  ].join("\n");
}

const previewNext = (previewId) =>
  `Fix what the issues and the slides show and call create_document again under the same file name: with preview: true for another look (a file may be previewed twice per reply), or without preview to deliver the changed code. To keep this file as it is, deliver with fromPreview: "${previewId}" and no content. If you end the reply without delivering, the latest preview is delivered automatically.`;

// What the overview image is, or why there is none; nothing when a model
// that cannot see images was shown none. `slides` is the deck's count: a
// slide the host drew no page of is not in the image.
function overviewNote({ vision, pages, slides, image, targetPageNumbers }) {
  if (!vision || pages === 0) return [];
  if (!image)
    return [
      "No overview of the slides could be drawn; visual and formula rendering remain unverified. The automated findings alone cannot establish visual quality.",
    ];
  const total = Number.isSafeInteger(slides) && slides > pages ? slides : pages;
  if (
    Array.isArray(targetPageNumbers) &&
    targetPageNumbers.length > 0 &&
    targetPageNumbers.length <= 30 &&
    new Set(targetPageNumbers).size === targetPageNumbers.length &&
    targetPageNumbers.every(
      (page) => Number.isInteger(page) && page >= 1 && page <= 1000,
    )
  )
    return [
      "The image is an overview of targeted source pages: " +
        targetPageNumbers.join(", ") +
        ". It contains " +
        Math.min(pages, OVERVIEW_SLIDES) +
        " of " +
        targetPageNumbers.length +
        " targeted page previews, each under its actual slide number. The original deck has " +
        total +
        " slides; untargeted pages are not included.",
    ];
  const shown =
    pages > OVERVIEW_SLIDES
      ? ` It shows the first ${OVERVIEW_SLIDES} of ${total} slides.`
      : pages < total
        ? ` It shows ${pages} of ${total} slides; the issues say which have no picture.`
        : "";
  return [
    `The image is an overview of the slides, each under its slide number.${shown}`,
  ];
}

// Through console: main.mjs writes it to stderr without the run's credentials.
const skipped = (what, error) =>
  console.warn(`create_document ${what} skipped: ${errorMessage(error)}`);

async function pageCount(filePath) {
  try {
    const bytes = await readFile(filePath);
    return await withPdf(bytes, (document) => document.numPages);
  } catch (error) {
    skipped("page count", error);
    return undefined;
  }
}

// The image `draw` makes, or null with a warning when it cannot be made: a
// file that exists must not fail for want of its picture.
async function shown(what, draw) {
  try {
    const image = await draw();
    if (!image) throw new Error("over the size budget at the lowest quality");
    return image;
  } catch (error) {
    skipped(what, error);
    return null;
  }
}

const thumbnail = (previewPath) =>
  shown("preview", async () => {
    const extension = path.extname(previewPath).toLowerCase();
    const render = PREVIEWS[extension];
    if (!render) throw new Error("unsupported preview type");
    const bytes = await readFile(previewPath);
    return render(extension === ".svg" ? modelPreviewSvg(bytes) : bytes);
  });

// Model thumbnails and overviews need the same bounded compatibility adapter
// as the user's persisted JPEG pages. Never rewrite the host's SVG file.
const modelPreviewSvg = (bytes) => {
  const svg = bytes.toString("utf8");
  if (!safeSvg(svg)) throw new Error("unsafe model preview SVG");
  const normalized = normalizeOfficeArrowheads(
    normalizeOfficePatternImages(svg),
  );
  if (!safeSvg(normalized))
    throw new Error("unsafe normalized model preview SVG");
  return Buffer.from(normalized);
};

// A page's slide number: the host writes slide n as pages/<n>.svg.
const slideNumber = (page, index) =>
  /^(\d+)\.svg$/i.exec(path.basename(page))?.[1] ?? String(index + 1);

// One image of a preview's slides, the first OVERVIEW_SLIDES of them, each
// under its own number; a page that cannot be read is a placeholder.
const overview = (pagePaths) =>
  shown("overview", async () =>
    contactSheet(
      await Promise.all(
        pagePaths.slice(0, OVERVIEW_SLIDES).map(async (page, index) => ({
          label: slideNumber(page, index),
          svg: await readFile(page)
            .then(modelPreviewSvg)
            .catch(() => null),
        })),
      ),
    ),
  );

export function documentTools(runtime, context, { vision }) {
  const formats = context.params.documents?.formats;
  if (!Array.isArray(formats) || formats.length === 0) return [];
  // Python decks (and previews, which exist for them) are a pptx's, offered
  // only when the host can run the engine.
  const python =
    context.params.documents?.python === true && formats.includes("pptx");
  const planning = createPresentationPlanning(runtime, context);
  const primedPptx =
    Boolean(planning) &&
    Array.isArray(context.params.documents?.primedBuiltinSkills) &&
    context.params.documents.primedBuiltinSkills.includes("ofox-office-pptx");
  // Derived per preview, never a mutable shared "last fallback" result.
  const previewFallbacks = new Map();
  const content = Type.Union([
    Type.String({ maxLength: 2_000_000 }),
    Type.Array(Type.Object({}, { additionalProperties: true }), {
      maxItems: 2000,
    }),
    ...(python
      ? [
          Type.Object({
            kind: Type.String({ enum: ["python"] }),
            code: Type.String({ maxLength: MAX_PYTHON_CHARACTERS }),
          }),
        ]
      : []),
  ]);
  const tool = runtime.wrapTool(
    context,
    "create_document",
    description(formats, python, Boolean(planning)),
    Type.Object({
      name: Type.String({
        minLength: 1,
        maxLength: 120,
        description: "File name with extension",
      }),
      format: Type.String({ enum: formats }),
      content: python ? Type.Optional(content) : content,
      ...(planning
        ? {
            planId: Type.Optional(
              Type.String({
                pattern: "^[A-Za-z0-9_-]{1,80}$",
                description:
                  "Optional reference to this run's Host-frozen PPTX plan. Omit it to bind the current plan automatically; if supplied it must match. Passing an ID cannot create or replace a plan.",
              }),
            ),
            revision: Type.Optional(
              Type.Integer({
                minimum: 1,
                description:
                  "Optional revision of this run's Host-frozen PPTX plan. Omit it to bind the current revision automatically; an explicit revision must match.",
              }),
            ),
          }
        : {}),
      ...(python
        ? {
            preview: Type.Optional(
              Type.Boolean({
                description:
                  "Make and check the file without delivering it; the result has a previewId and an overview of the slides",
              }),
            ),
            fromPreview: Type.Optional(
              Type.String({
                description:
                  "A previewId of this reply: deliver that preview as it is, with no content",
              }),
            ),
          }
        : {}),
    }),
    async (given, signal, toolCallId) => {
      const args = planning ? planning.bindDocument(given) : given;
      const planProblem = planning?.problem(args);
      if (planProblem)
        return {
          content: [{ type: "text", text: planProblem }],
          details: { code: "assistant_presentation_plan_invalid" },
          isError: true,
        };
      // This value is frozen by the Host from the user's picker, never a
      // create_document argument. Auto retains the model's normal choice.
      const chosen =
        planning?.current()?.effectiveStyle ??
        context.params.documents?.selectedStyle;
      const selectedStyle = chosen === "auto" ? undefined : chosen;
      if (selectedStyle !== undefined && !THEME_NAMES.includes(selectedStyle))
        return {
          content: [
            {
              type: "text",
              text: "The selected presentation style is unavailable.",
            },
          ],
          details: { code: "assistant_deck_invalid" },
          isError: true,
        };
      // The host reads Python code and a preview's file itself: both go to it
      // as they came, with no deck to render and no batch to check here.
      const fromPreview = args.fromPreview !== undefined;
      const code =
        !fromPreview && args.format === "pptx"
          ? pythonContent(args.content)
          : undefined;
      const hostReads = fromPreview || code !== undefined;
      let deck =
        !hostReads && args.format === "pptx" ? deckOf(args.content) : null;
      let presentationFallbacks = fromPreview
        ? (previewFallbacks.get(args.fromPreview) ?? [])
        : [];
      if (deck && planning) {
        if (
          !Array.isArray(deck.slides) ||
          deck.slides.length !== planning.current().slides.length
        )
          throw new Error(
            "assistant_presentation_plan_invalid: Deck count differs from the frozen page plan",
          );
        const hydrated = await planning.hydrate(deck, args, signal, toolCallId);
        deck = hydrated.deck;
        presentationFallbacks = hydrated.fallbacks;
      }
      if (args.format === "pptx" && selectedStyle && !hostReads && !deck)
        return {
          content: [
            {
              type: "text",
              text: planning
                ? "A frozen presentation plan requires a structured deck object or Python deck code; raw OfficeCLI batches cannot enforce its style and page plan."
                : "A manually selected style requires a deck object or Python deck code, not raw commands.",
            },
          ],
          details: { code: "assistant_deck_invalid" },
          isError: true,
        };
      if (deck && selectedStyle) deck = { ...deck, theme: selectedStyle };
      const rendered = deck ? renderedDeck(deck) : null;
      if (rendered?.problems)
        return {
          content: [
            {
              type: "text",
              text: `The deck was not rendered; fix these and send the whole deck again:\n- ${rendered.problems.join("\n- ")}`,
            },
          ],
          details: { code: "assistant_deck_invalid" },
          isError: true,
        };
      const request = rendered
        ? { ...args, content: rendered.commands }
        : code !== undefined
          ? { ...args, content: code }
          : args;
      const problem = hostReads ? null : contentProblem(request, python);
      if (problem)
        return {
          content: [{ type: "text", text: problem }],
          details: { code: "assistant_document_content" },
          isError: true,
        };
      let result;
      try {
        result = await runtime.hostTool(
          context,
          "create_document",
          request,
          signal,
          toolCallId,
        );
      } catch (error) {
        throw rendered ? refusedRender(error) : error;
      }
      // The deck helper's built-page receipt never reaches the model as an
      // issue; pages that differ from the frozen plan are reported instead.
      if (result && Array.isArray(result.issues))
        result = {
          ...result,
          issues: planDrift(result.issues, planning?.current()).issues,
        };
      if (
        result?.status === "completed" &&
        result.preview === true &&
        typeof result.previewId === "string"
      ) {
        previewFallbacks.set(result.previewId, presentationFallbacks);
        if (previewFallbacks.size > 12)
          previewFallbacks.delete(previewFallbacks.keys().next().value);
        // The paths are for this runtime alone, like a file's.
        const { pagePaths, filePath, previewPath, ...details } = result;
        if (presentationFallbacks.length)
          details.presentationFallbacks = presentationFallbacks;
        const pages = Array.isArray(pagePaths)
          ? pagePaths.filter((page) => typeof page === "string")
          : [];
        const image = vision && pages.length > 0 ? await overview(pages) : null;
        return {
          content: [
            {
              type: "text",
              text: [
                ...(rendered?.told ?? []),
                ...presentationFallbacks.map(
                  (item) =>
                    `Page ${item.page}: planned image ${item.assetId} was not supplied; declared ${item.fallback} fallback used. Image requirement is not satisfied.`,
                ),
                describePreview(details, args),
                ...overviewNote({
                  vision,
                  pages: pages.length,
                  slides: details.slides,
                  image,
                  targetPageNumbers: details.targetPageNumbers,
                }),
                previewNext(details.previewId),
              ].join("\n"),
            },
            ...(image ? [{ type: "image", ...image }] : []),
          ],
          details,
        };
      }
      if (result?.status !== "completed" || !result.file) {
        const message =
          result?.error?.message ?? "The host did not create the document";
        throw new Error(
          rendered && isRefusal(result?.error?.code, message)
            ? `${RENDER_REFUSED}${message}`
            : message,
        );
      }
      // The file exists now: what follows reads only local files and does
      // not stop for an abort, so the result reaches the history either way.
      const { filePath, previewPath, ...details } = result;
      if (presentationFallbacks.length)
        details.presentationFallbacks = presentationFallbacks;
      if (
        args.format === "pdf" &&
        filePath &&
        details.file.pages === undefined
      ) {
        const pages = await pageCount(filePath);
        if (pages !== undefined) details.file = { ...details.file, pages };
      }
      const image = vision && previewPath ? await thumbnail(previewPath) : null;
      return {
        content: [
          {
            type: "text",
            text: [
              ...(rendered?.told ?? []),
              ...presentationFallbacks.map(
                (item) =>
                  `Page ${item.page}: planned image ${item.assetId} was not supplied; declared ${item.fallback} fallback used. Image requirement is not satisfied.`,
              ),
              describe(details, args.format),
            ].join("\n"),
          },
          ...(image ? [{ type: "image", ...image }] : []),
        ],
        details,
      };
    },
  );
  return [
    {
      ...tool,
      promptGuidelines: [
        ...guidelines(formats, python, {
          planned: Boolean(planning),
          primedPptx,
        }),
        ...(planning
          ? [
              "This run uses planned PPTX creation: understand the requirements, call get_presentation_candidates, choose an eligible style with design judgment, and freeze_presentation_plan once with all pages before create_document. Current-run Host-frozen planId and revision bind automatically when omitted, including fromPreview delivery; explicit references must match. Passing references never creates a plan. Do not freeze again after an accepted plan. Manual user style has priority.",
              "For a frozen source edit, use prs=source_deck() and edit target_slides(prs) only. Source version is the attachment's complete SHA256 and pageNumbers are actual 1-based pages; slideCount is the target count. Preserve the rest of the deck. Do not rebuild with new_deck or a Node deck object.",
              "Understand purpose, audience, language, speaking duration, density, data/formula/image proportions, brand colors/fonts and native editing needs. Infer reasonable design preferences from the request without an unnecessary questionnaire. Record uncertain duration/contentMix/brand as null, and distinguish unknowns and inferred choices briefly in decisionSummary; never invent facts or spending authorization.",
              'Plan image visuals only when they explain the page. A plan never authorizes spending; read the frozen result visualBudget, a trusted Host freeze-time snapshot rather than a reservation. If canAttempt=true, the user already authorized the shown limits: do not ask again. Call generate_image for a needed planned asset with its frozen references and the user-named model if any; the Host still checks price, remaining budget and identity on every attempt. If unavailable/refused use native visuals, supplied assets or the declared fallback and disclose the missing image. Node showcase images may use media:{assetId:"planned-id"}; Python uses image(..., asset_id=...). Never put private asset bytes or host paths in your reply.',
            ]
          : []),
      ],
    },
    ...(planning?.tools ?? []),
    ...(formats.includes("pptx")
      ? [presentationStyleTool(runtime, context)]
      : []),
  ];
}
