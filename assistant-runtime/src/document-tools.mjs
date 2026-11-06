import { readFile } from "node:fs/promises";
import path from "node:path";
import { Type } from "typebox";
import { withPdf } from "./attachments.mjs";
import { errorMessage } from "./security.mjs";
import { thumbnailFromPng, thumbnailFromSvg } from "./thumbnails.mjs";

// create_document: the model writes the content, the host runs the engine,
// checks the file and keeps it as a generated attachment. The host's reply
// also names the file's path and its first page's preview for this runtime
// alone: from them it adds a PDF's page count and, for a model that sees
// images, a thumbnail. The model never gets either path.

const OFFICE_FORMATS = new Set(["pptx", "docx", "xlsx"]);
const MAX_ISSUE_TEXT = 500;
const PREVIEWS = { ".png": thumbnailFromPng, ".svg": thumbnailFromSvg };

const GUIDELINES = [
  "When the user asks for a PDF, Word, PowerPoint or Excel file, or to write/make a document, produce the file with create_document; never say you cannot attach files.",
  "Before the first document of a format in a conversation, call use_skill on the matching ofox-office-* skill and follow it.",
  "Reply with two or three sentences on the structure first, then call the tool; after the result, summarize the file and mention issues.",
  "At most 3 documents per reply; retry a failed document at most twice, then explain.",
];

function description(formats) {
  const office = formats.filter((format) => OFFICE_FORMATS.has(format));
  const text = formats.filter(
    (format) => !OFFICE_FORMATS.has(format) && format !== "pdf",
  );
  const shapes = [
    office.length &&
      `${office.join(", ")}: content is an array of officecli batch commands`,
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

function issueLine({ severity, message, path: where } = {}) {
  const place = where ? ` at ${where}` : "";
  return `- ${severity ?? "issue"}${place}: ${String(message ?? "").slice(0, MAX_ISSUE_TEXT)}`;
}

function describe(result, format) {
  const { file } = result;
  const issues = Array.isArray(result.issues) ? result.issues : [];
  const facts = [
    format,
    size(file.bytes),
    quantity(file.slides, "slide"),
    quantity(file.pages, "page"),
    quantity(file.sheets, "sheet"),
  ].filter(Boolean);
  return [
    `Created ${file.name} (${facts.join(", ")}) as attachment ${file.id}. Issues: ${issues.length}.`,
    ...issues.map(issueLine),
    "Do not repeat the file contents; tell the user what the file contains.",
  ].join("\n");
}

const skipped = (what, error) =>
  process.stderr.write(
    `create_document ${what} skipped: ${errorMessage(error)}\n`,
  );

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
      format: Type.Union(formats.map((format) => Type.Literal(format))),
      content: Type.Union([
        Type.String({ maxLength: 2_000_000 }),
        Type.Array(Type.Record(Type.String(), Type.Unknown()), {
          maxItems: 2000,
        }),
      ]),
    }),
    async (args, signal, toolCallId) => {
      const problem = contentProblem(args);
      if (problem)
        return {
          content: [{ type: "text", text: problem }],
          details: { code: "assistant_document_content" },
          isError: true,
        };
      const result = await runtime.hostTool(
        context,
        "create_document",
        args,
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
          { type: "text", text: describe(details, args.format) },
          ...(image ? [{ type: "image", ...image }] : []),
        ],
        details,
      };
    },
  );
  return [{ ...tool, promptGuidelines: GUIDELINES }];
}
