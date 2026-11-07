import { readFile } from "node:fs/promises";
import path from "node:path";
import { Type } from "typebox";
import { withPdf } from "./attachments.mjs";
import { deckOf, parseDeck } from "./deck/schema.mjs";
import { renderDeck } from "./deck/render.mjs";
import { themeFor } from "./deck/theme.mjs";
import { errorMessage } from "./security.mjs";
import { thumbnailFromPng, thumbnailFromSvg } from "./thumbnails.mjs";

// create_document: the model writes the content (for a pptx, usually a deck
// object this runtime renders into the batch), the host runs the engine,
// checks the file and keeps it as a generated attachment. The host's reply
// also names the file's path and its first page's preview for this runtime
// alone: from them it adds a PDF's page count and, for a model that sees
// images, a thumbnail. The model never gets either path.

const OFFICE_FORMATS = new Set(["pptx", "docx", "xlsx"]);
const MAX_ISSUE_TEXT = 500;
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
function guidelines(formats) {
  const named = NAMED_FORMATS.filter(([format]) => formats.includes(format));
  const missing = NAMED_FORMATS.filter(([format]) => !formats.includes(format));
  const words = (list) => either(list.map(([, word]) => word));
  return [
    named.length
      ? `When the user asks for a file, or names a ${words(named)} document, produce it with create_document${missing.length ? "." : "; never say you cannot attach files."}`
      : "When the user asks for a file, produce it with create_document.",
    missing.length &&
      `This machine cannot produce ${words(missing)} files; say so if asked.`,
    named.length &&
      `Before the first ${words(named)} document in a conversation, call use_skill on its skill (${either(named.map(([format]) => `ofox-office-${format}`))}) and follow it.`,
    formats.includes("pptx") &&
      "For a pptx, send the deck object the skill describes; do not write slide coordinates yourself.",
    "Reply with two or three sentences on the structure first, then call the tool; after the result, summarize the file and mention issues.",
    "At most 3 documents per reply; retry a failed document at most twice, then explain.",
  ].filter(Boolean);
}

function description(formats) {
  const office = formats.filter((format) => OFFICE_FORMATS.has(format));
  const text = formats.filter(
    (format) => !OFFICE_FORMATS.has(format) && format !== "pdf",
  );
  const batch = office.filter((format) => format !== "pptx");
  const shapes = [
    office.includes("pptx") &&
      'pptx: content is a deck object ({"kind":"deck",…}, see the ofox-office-pptx skill) or, for experts, an array of officecli batch commands',
    batch.length &&
      `${batch.join(", ")}: content is an array of officecli batch commands`,
    formats.includes("pdf") && "pdf: content is the Typst markup of the body",
    text.length && `${text.join(", ")}: content is the file's text`,
  ].filter(Boolean);
  return `Create one file the user can open and save; it appears under your reply. ${shapes.join("; ")}.`;
}

// Office formats take a batch of commands, the others a text.
function contentProblem({ format, content }) {
  if (OFFICE_FORMATS.has(format))
    return Array.isArray(content)
      ? null
      : `${format} content must be an array of officecli batch commands.`;
  if (typeof content === "string") return null;
  return format === "pdf"
    ? "pdf content must be a string of Typst markup."
    : `${format} content must be a string with the file's text.`;
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
    `Created ${file.name} (${facts.join(", ")}) as attachment ${file.id}. Issues: ${issues.length}.${fixed}`,
    ...issues.map(issueLine),
    "Do not repeat the file contents; tell the user what the file contains.",
  ].join("\n");
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

async function thumbnail(previewPath) {
  try {
    const render = PREVIEWS[path.extname(previewPath).toLowerCase()];
    if (!render) throw new Error("unsupported preview type");
    const image = await render(await readFile(previewPath));
    if (!image) throw new Error("over the size budget at the lowest quality");
    return image;
  } catch (error) {
    skipped("preview", error);
    return null;
  }
}

export function documentTools(runtime, context, { vision }) {
  const formats = context.params.documents?.formats;
  if (!Array.isArray(formats) || formats.length === 0) return [];
  const tool = runtime.wrapTool(
    context,
    "create_document",
    description(formats),
    Type.Object({
      name: Type.String({
        minLength: 1,
        maxLength: 120,
        description: "File name with extension",
      }),
      format: Type.String({ enum: formats }),
      content: Type.Union([
        Type.String({ maxLength: 2_000_000 }),
        Type.Array(Type.Object({}, { additionalProperties: true }), {
          maxItems: 2000,
        }),
      ]),
    }),
    async (args, signal, toolCallId) => {
      const deck = args.format === "pptx" ? deckOf(args.content) : null;
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
      const request = rendered ? { ...args, content: rendered.commands } : args;
      const problem = contentProblem(request);
      if (problem)
        return {
          content: [{ type: "text", text: problem }],
          details: { code: "assistant_document_content" },
          isError: true,
        };
      const result = await runtime.hostTool(
        context,
        "create_document",
        request,
        signal,
        toolCallId,
      );
      if (result?.status !== "completed" || !result.file)
        throw new Error(
          result?.error?.message ?? "The host did not create the document",
        );
      // The file exists now: what follows reads only local files and does
      // not stop for an abort, so the result reaches the history either way.
      const { filePath, previewPath, ...details } = result;
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
              describe(details, args.format),
            ].join("\n"),
          },
          ...(image ? [{ type: "image", ...image }] : []),
        ],
        details,
      };
    },
  );
  return [{ ...tool, promptGuidelines: guidelines(formats) }];
}
