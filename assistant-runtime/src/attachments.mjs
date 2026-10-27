import { lstat, open, realpath, stat } from "node:fs/promises";
import { constants } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { Type } from "typebox";

const MAX_FILE = 25 * 1024 * 1024;
const MAX_IMAGE = 10 * 1024 * 1024;
const MAX_RENDERED_PAGE = 5 * 1024 * 1024;
const MAX_TEXT = 16 * 1024 * 1024;
const MAX_CACHED_TEXT = MAX_TEXT * 2;
const MAX_PDF_PAGES = 100;
// Embedded images above this many pixels are skipped by PDF.js (it guards
// against decompression bombs). A 600-dpi A3 scan has about 70 MP; the
// rendered output is bounded separately.
const MAX_PDF_IMAGE_PIXELS = 100 * 1000 * 1000;
const MAX_READ = 20000;
const INLINE_TEXT = 8000;
// Pages without text attached to the user turn; the model renders the rest.
const INLINE_PDF_PAGES = 2;
export const MAX_MESSAGE_IMAGES = 10;

const TEXT_EXTENSIONS = new Set([
  ".txt",
  ".md",
  ".csv",
  ".json",
  ".log",
  ".yaml",
  ".yml",
  ".toml",
  ".xml",
  ".html",
  ".js",
  ".ts",
  ".py",
  ".rs",
  ".go",
]);
const IMAGE_EXTENSIONS = new Set([".png", ".jpg", ".jpeg", ".webp", ".gif"]);

/**
 * A per-attachment failure with the outcome the host shows for it. The
 * message starts with the code, like other assistant_* errors.
 */
export class AttachmentError extends Error {
  constructor(status, code, message, details = {}) {
    super(`${code}: ${message}`);
    this.name = "AttachmentError";
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

const unreadable = (reason, message, details = {}) =>
  new AttachmentError(
    "unreadable",
    "assistant_attachment_unreadable",
    message,
    { ...details, reason },
  );
const notFound = (reason, details = {}) =>
  new AttachmentError(
    "unreadable",
    "assistant_attachment_not_found",
    "The attachment is no longer available",
    { ...details, reason },
  );
const tooLarge = (message, details) =>
  new AttachmentError(
    "too_large",
    "assistant_attachment_too_large",
    message,
    details,
  );
const visionRequired = (message, details) =>
  new AttachmentError(
    "model_unsupported",
    "assistant_vision_model_required",
    message,
    details,
  );

// PDF.js reads CMaps, ICC profiles and WebAssembly decoders from these
// directories in Node. Without them CJK text in predefined CMaps is lost.
const pdfjsRoot = path
  .dirname(createRequire(import.meta.url).resolve("pdfjs-dist/package.json"))
  .split(path.sep)
  .join("/");

// PDF.js parses in one long chain of microtasks. Yielding between pages lets a
// stop request from the host arrive and abort the work.
const yieldToHost = () => new Promise((resolve) => setImmediate(resolve));

const pageLimit = (document) => {
  if (document.numPages > MAX_PDF_PAGES)
    throw tooLarge("PDF attachments are limited to 100 pages", {
      format: "pdf",
      limit: "pdf_pages",
      maxPages: MAX_PDF_PAGES,
      pageCount: document.numPages,
    });
};

// Opens a PDF for read() and maps PDF.js failures to outcomes; our own errors
// pass through. Aborting the signal destroys the loading task, which is also
// destroyed when opening fails.
async function withPdf(bytes, read, signal) {
  signal?.throwIfAborted();
  const { getDocument } = await import("pdfjs-dist/legacy/build/pdf.mjs");
  const task = getDocument({
    data: new Uint8Array(bytes),
    isEvalSupported: false,
    useSystemFonts: false,
    disableFontFace: true,
    maxImageSize: MAX_PDF_IMAGE_PIXELS,
    canvasMaxAreaInBytes: 16 * 1024 * 1024,
    cMapUrl: `${pdfjsRoot}/cmaps/`,
    cMapPacked: true,
    iccUrl: `${pdfjsRoot}/iccs/`,
    wasmUrl: `${pdfjsRoot}/wasm/`,
    verbosity: 0,
  });
  const stop = () => void task.destroy();
  signal?.addEventListener("abort", stop, { once: true });
  try {
    return await read(await task.promise);
  } catch (error) {
    if (signal?.aborted) throw signal.reason;
    if (error instanceof AttachmentError || error?.invalidPage) throw error;
    if (error?.name === "PasswordException")
      throw unreadable("password_protected", "The PDF is password-protected", {
        format: "pdf",
      });
    throw unreadable("invalid_pdf", "The PDF is damaged or not a PDF", {
      format: "pdf",
    });
  } finally {
    signal?.removeEventListener("abort", stop);
    await task.destroy();
  }
}

function boundedText(text, metadata, unreadPages = false) {
  const truncated = unreadPages || text.length > MAX_TEXT;
  const warning = [
    metadata.warning,
    truncated
      ? "Document extraction reached its safety limit. Unread content remains; do not treat this as the complete document. Ask the user for a smaller document or a specific excerpt."
      : "",
  ]
    .filter(Boolean)
    .join(" ");
  return {
    ...metadata,
    text: text.slice(0, MAX_TEXT),
    truncated,
    extractionLimit: MAX_TEXT,
    ...(warning ? { warning } : {}),
  };
}

// The declared MIME type can be wrong; only these signatures are sent on.
function imageType(bytes) {
  if (bytes.subarray(0, 8).equals(Buffer.from("89504e470d0a1a0a", "hex")))
    return "image/png";
  if (bytes.subarray(0, 3).equals(Buffer.from("ffd8ff", "hex")))
    return "image/jpeg";
  if (/^GIF8[79]a$/.test(bytes.subarray(0, 6).toString("latin1")))
    return "image/gif";
  if (
    bytes.subarray(0, 4).toString("latin1") === "RIFF" &&
    bytes.subarray(8, 12).toString("latin1") === "WEBP"
  )
    return "image/webp";
  return null;
}

function validAttachment(attachment) {
  return (
    attachment &&
    typeof attachment === "object" &&
    typeof attachment.id === "string" &&
    attachment.id.length > 0 &&
    attachment.id.length <= 200 &&
    typeof attachment.name === "string" &&
    typeof attachment.path === "string" &&
    attachment.path.length > 0 &&
    (attachment.mimeType === undefined ||
      attachment.mimeType === null ||
      typeof attachment.mimeType === "string")
  );
}

export class AttachmentStore {
  // maxRenderedPage is configurable for tests of the encoding fallback.
  constructor(
    root,
    attachments = [],
    { maxRenderedPage = MAX_RENDERED_PAGE } = {},
  ) {
    if (!Array.isArray(attachments) || attachments.length > 200)
      throw new Error("Invalid conversation attachments");
    if (
      attachments.length &&
      (typeof root !== "string" || !path.isAbsolute(root))
    )
      throw new Error("The attachment root must be an absolute path");
    this.root = root;
    this.attachments = new Map();
    for (const attachment of attachments) {
      if (!validAttachment(attachment) || this.attachments.has(attachment.id))
        throw new Error("Invalid conversation attachment");
      this.attachments.set(attachment.id, attachment);
    }
    this.cache = new Map();
    this.cachedCharacters = 0;
    this.maxRenderedPage = maxRenderedPage;
  }

  get size() {
    return this.attachments.size;
  }

  attachment(id) {
    const attachment = this.attachments.get(id);
    if (!attachment) throw notFound("unknown_id");
    return attachment;
  }

  // "text", "pdf" or "image" by name and MIME type; null is unsupported.
  format(id) {
    const attachment = this.attachment(id);
    const extension = path.extname(attachment.name).toLowerCase();
    const mimeType = attachment.mimeType ?? "";
    if (extension === ".pdf" || mimeType === "application/pdf") return "pdf";
    if (IMAGE_EXTENSIONS.has(extension) || mimeType.startsWith("image/"))
      return "image";
    if (
      TEXT_EXTENSIONS.has(extension) ||
      mimeType.startsWith("text/") ||
      mimeType === "application/json"
    )
      return "text";
    return null;
  }

  supportedFormat(id) {
    const format = this.format(id);
    if (format) return format;
    const attachment = this.attachment(id);
    throw new AttachmentError(
      "unsupported",
      "assistant_attachment_unsupported",
      "Unsupported attachment format; use an image (PNG, JPEG, WebP or GIF), a PDF or a UTF-8 text file",
      {
        extension: path.extname(attachment.name).toLowerCase(),
        mimeType: attachment.mimeType ?? null,
      },
    );
  }

  async file(id) {
    const attachment = this.attachment(id);
    const format = this.format(id);
    const known = format ? { format } : {};
    try {
      return { attachment, bytes: await this.#confinedRead(attachment) };
    } catch (error) {
      if (error instanceof AttachmentError) {
        error.details = { ...known, ...error.details };
        throw error;
      }
      if (["ENOENT", "ENOTDIR"].includes(error?.code))
        throw notFound("missing", known);
      if (error?.code === "ELOOP")
        throw unreadable(
          "unsafe_path",
          "Attachment path may not contain symlinks",
          known,
        );
      throw unreadable(
        "read_failed",
        "The attachment could not be read",
        known,
      );
    }
  }

  async #confinedRead(attachment) {
    const unsafe = (message) => unreadable("unsafe_path", message);
    const declaredRoot = path.resolve(this.root);
    const root = await realpath(declaredRoot);
    const relative = path.relative(
      declaredRoot,
      path.resolve(declaredRoot, attachment.path),
    );
    if (
      !relative ||
      relative === ".." ||
      relative.startsWith(`..${path.sep}`) ||
      path.isAbsolute(relative)
    )
      throw unsafe("Attachment path escapes its imported root");
    // The host root can use a system alias such as macOS /var -> /private/var.
    // Resolve that trusted prefix only; descendants must still be symlink-free.
    const requested = path.resolve(root, relative);
    let component = root;
    for (const part of relative.split(path.sep)) {
      component = path.join(component, part);
      if ((await lstat(component)).isSymbolicLink())
        throw unsafe("Attachment path may not contain symlinks");
    }
    const actual = await realpath(requested);
    if (actual !== requested) throw unsafe("Attachment path changed");
    const info = await stat(actual);
    if (!info.isFile()) throw unsafe("Attachment must be a regular file");
    if (info.size > MAX_FILE)
      throw tooLarge("Attachments are limited to 25 MiB", {
        limit: "file_size",
        maxBytes: MAX_FILE,
      });
    const handle = await open(
      actual,
      constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0),
    );
    try {
      const opened = await handle.stat();
      if (
        opened.dev !== info.dev ||
        opened.ino !== info.ino ||
        !opened.isFile() ||
        (await realpath(requested)) !== actual
      )
        throw unsafe("Attachment path changed while opening");
      if (opened.size > MAX_FILE)
        throw tooLarge("Attachments are limited to 25 MiB", {
          limit: "file_size",
          maxBytes: MAX_FILE,
        });
      const bytes = await handle.readFile();
      if (bytes.length > MAX_FILE)
        throw tooLarge("Attachments are limited to 25 MiB", {
          limit: "file_size",
          maxBytes: MAX_FILE,
        });
      return bytes;
    } finally {
      await handle.close();
    }
  }

  async extract(id, { signal } = {}) {
    if (this.cache.has(id)) {
      const cached = this.cache.get(id);
      this.cache.delete(id);
      this.cache.set(id, cached);
      return cached;
    }
    const format = this.supportedFormat(id);
    let result;
    if (format === "image") {
      const { mimeType } = await this.#imageBytes(id);
      result = { format, text: "", mimeType };
    } else if (format === "pdf") {
      const { bytes } = await this.file(id);
      result = await withPdf(
        bytes,
        (document) => this.#pdfText(document, signal),
        signal,
      );
    } else {
      const { bytes } = await this.file(id);
      let text;
      try {
        text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      } catch {
        throw unreadable("invalid_utf8", "The attachment is not UTF-8 text", {
          format,
        });
      }
      result = boundedText(text, { format });
    }
    while (
      this.cache.size &&
      this.cachedCharacters + result.text.length > MAX_CACHED_TEXT
    ) {
      const oldest = this.cache.keys().next().value;
      this.cachedCharacters -= this.cache.get(oldest).text.length;
      this.cache.delete(oldest);
    }
    this.cache.set(id, result);
    this.cachedCharacters += result.text.length;
    return result;
  }

  async #pdfText(document, signal) {
    pageLimit(document);
    let text = "";
    let processedPages = 0;
    let textPages = 0;
    const pagesWithoutText = [];
    for (
      let number = 1;
      number <= document.numPages && text.length < MAX_TEXT;
      number++
    ) {
      await yieldToHost();
      signal?.throwIfAborted();
      const page = await document.getPage(number);
      const content = await page.getTextContent();
      const pageText = content.items.map((item) => item.str ?? "").join(" ");
      if (pageText.trim()) textPages++;
      else pagesWithoutText.push(number);
      text += `[Page ${number}]\n${pageText.trim() ? pageText : "[No extractable text; visual content has not been read.]"}\n`;
      page.cleanup();
      processedPages = number;
    }
    const result = boundedText(
      text,
      {
        format: "pdf",
        pageCount: document.numPages,
        scanned: textPages === 0,
        pagesWithoutText,
        textCoverage:
          !pagesWithoutText.length &&
          processedPages === document.numPages &&
          text.length <= MAX_TEXT
            ? "complete"
            : textPages
              ? "partial"
              : "none",
      },
      processedPages < document.numPages,
    );
    if (pagesWithoutText.length)
      result.warning = [
        result.warning,
        `PDF pages ${pagesWithoutText.join(", ")} have no extractable text; their visual content has not been read and needs a vision model. Do not treat the extracted text as the complete document.`,
      ]
        .filter(Boolean)
        .join(" ");
    return result;
  }

  async read(id, offset = 0, length = INLINE_TEXT, { signal } = {}) {
    if (
      !Number.isSafeInteger(offset) ||
      offset < 0 ||
      !Number.isSafeInteger(length) ||
      length < 1 ||
      length > MAX_READ
    )
      throw new Error("Invalid attachment page range");
    const result = await this.extract(id, { signal });
    return {
      ...result,
      characters: result.text.length,
      text: result.text.slice(offset, offset + length),
      offset,
      nextOffset: Math.min(result.text.length, offset + length),
      hasMore: offset + length < result.text.length,
    };
  }

  async #imageBytes(id) {
    const attachment = this.attachment(id);
    const details = { format: "image", mimeType: attachment.mimeType ?? null };
    if (
      !["image/png", "image/jpeg", "image/webp", "image/gif"].includes(
        attachment.mimeType,
      ) &&
      !IMAGE_EXTENSIONS.has(path.extname(attachment.name).toLowerCase())
    )
      throw new AttachmentError(
        "unsupported",
        "assistant_attachment_unsupported",
        "Unsupported image format; use PNG, JPEG, WebP or GIF",
        details,
      );
    const { bytes } = await this.file(id);
    if (bytes.length > MAX_IMAGE)
      throw tooLarge("Images are limited to 10 MiB", {
        ...details,
        limit: "image_size",
        maxBytes: MAX_IMAGE,
      });
    const mimeType = imageType(bytes);
    if (!mimeType)
      throw unreadable(
        "invalid_image",
        "The image file is damaged or not a PNG, JPEG, WebP or GIF image",
        details,
      );
    return { mimeType, bytes };
  }

  // The image itself, or one PDF page rendered as an image.
  async image(id, pageNumber, { signal } = {}) {
    if (this.supportedFormat(id) !== "pdf") {
      const { mimeType, bytes } = await this.#imageBytes(id);
      return { type: "image", data: bytes.toString("base64"), mimeType };
    }
    const [rendered] = await this.renderPages(id, [pageNumber], { signal });
    if (rendered.error) throw rendered.error;
    return rendered.image;
  }

  /**
   * Renders PDF pages from one parse of the document. Each entry has the
   * page's image, or the AttachmentError that kept it from rendering.
   */
  async renderPages(id, pageNumbers, { signal } = {}) {
    const { bytes } = await this.file(id);
    // Loaded outside withPdf: a missing native addon is not a damaged PDF.
    const { createCanvas } = await import("@napi-rs/canvas");
    return await withPdf(
      bytes,
      async (document) => {
        pageLimit(document);
        for (const number of pageNumbers)
          if (
            !Number.isSafeInteger(number) ||
            number < 1 ||
            number > document.numPages
          )
            throw Object.assign(
              new Error(
                `Invalid PDF page; this PDF has ${document.numPages} page(s)`,
              ),
              { invalidPage: true },
            );
        const rendered = [];
        for (const number of pageNumbers) {
          await yieldToHost();
          signal?.throwIfAborted();
          try {
            rendered.push({
              page: number,
              image: await this.#renderPage(document, number, createCanvas),
            });
          } catch (error) {
            if (signal?.aborted) throw signal.reason;
            rendered.push({
              page: number,
              error:
                error instanceof AttachmentError
                  ? error
                  : unreadable(
                      "invalid_pdf",
                      "The PDF page could not be rendered",
                      { format: "pdf", page: number },
                    ),
            });
          }
        }
        return rendered;
      },
      signal,
    );
  }

  // PNG keeps text sharp; a page too large as PNG, such as a photo, is sent
  // as JPEG instead.
  async #renderPage(document, number, createCanvas) {
    const page = await document.getPage(number);
    const original = page.getViewport({ scale: 1 });
    const scale = Math.min(
      1.5,
      2000 / original.width,
      2000 / original.height,
      Math.sqrt(4000000 / (original.width * original.height)),
    );
    if (!Number.isFinite(scale) || scale <= 0)
      throw unreadable("invalid_pdf", "The PDF page has invalid dimensions", {
        format: "pdf",
        page: number,
      });
    const viewport = page.getViewport({ scale });
    const canvas = createCanvas(
      Math.ceil(viewport.width),
      Math.ceil(viewport.height),
    );
    await page.render({ canvasContext: canvas.getContext("2d"), viewport })
      .promise;
    page.cleanup();
    for (const [mimeType, quality] of [["image/png"], ["image/jpeg", 85]]) {
      const image = canvas.toBuffer(mimeType, quality);
      if (image.length <= this.maxRenderedPage)
        return { type: "image", data: image.toString("base64"), mimeType };
    }
    throw tooLarge("A rendered PDF page exceeds the image size limit", {
      format: "pdf",
      limit: "rendered_page",
      maxBytes: this.maxRenderedPage,
      page: number,
    });
  }
}

// One selected attachment as prompt text, native images and its outcome.
async function prepareOne(
  store,
  id,
  { vision, room, imageLimit, scrub, signal },
) {
  const attachment = store.attachment(id);
  const header = `\n\n[User attachment: ${JSON.stringify(scrub(attachment.name))}; id=${id}`;
  const format = store.supportedFormat(id);
  if (format === "image") {
    const image = await store.image(id, undefined, { signal });
    const details = { format, mimeType: image.mimeType };
    if (!vision)
      throw visionRequired(
        "This model cannot read images; choose a vision model",
        details,
      );
    if (room < 1)
      throw tooLarge(
        `A message holds at most ${imageLimit} images or rendered PDF pages`,
        { ...details, limit: "message_images", maxImages: imageLimit },
      );
    return {
      prompt: `${header}; attached as an image]`,
      attached: [image],
      status: "ready",
      details,
    };
  }
  const result = await store.read(id, 0, INLINE_TEXT, { signal });
  const details = {
    format,
    characters: result.characters,
    inlineCharacters: result.text.length,
    hasMore: result.hasMore,
    truncated: result.truncated,
    ...(result.truncated ? { extractionLimit: result.extractionLimit } : {}),
    ...(format === "pdf"
      ? {
          pageCount: result.pageCount,
          textCoverage: result.textCoverage,
          pagesWithoutText: result.pagesWithoutText,
        }
      : {}),
  };
  let prompt = `${header}]\n${scrub(result.text)}`;
  if (result.hasMore)
    prompt += `\n[More content: use read_attachment id=${id}, offset=${result.nextOffset}]`;
  if (result.warning)
    prompt += `\n[Extraction warning: ${scrub(result.warning)}]`;
  const visualPages = format === "pdf" ? result.pagesWithoutText : [];
  const attached = [];
  // Without image input a PDF that has some text is still read, as partial:
  // the extraction warning names the pages that were not read.
  if (visualPages.length && !vision && result.textCoverage === "none")
    throw visionRequired(
      "This PDF has no extractable text; choose a vision model to read its pages",
      { format, pageCount: result.pageCount, pagesWithoutText: visualPages },
    );
  if (visualPages.length && vision) {
    // A page that fails to render stays for read_attachment with page.
    const wanted = visualPages.slice(0, Math.min(INLINE_PDF_PAGES, room));
    const rendered = wanted.length
      ? await store.renderPages(id, wanted, { signal })
      : [];
    const shown = [];
    for (const { page, image } of rendered)
      if (image) {
        shown.push(page);
        attached.push(image);
      }
    const remaining = visualPages.filter((page) => !shown.includes(page));
    details.pagesAttachedAsImages = shown;
    prompt += `\n[${shown.length ? `PDF pages ${shown.join(", ")} are attached as images. ` : ""}${
      remaining.length
        ? `Remaining pages without extractable text: ${remaining.join(", ")}; use read_attachment id=${id} with page to inspect them before claiming to have read the complete document.`
        : "All pages without extractable text are included as images."
    }]`;
  }
  const complete = !result.truncated && attached.length === visualPages.length;
  return {
    prompt,
    attached,
    status: complete ? "ready" : "partial",
    details,
  };
}

/**
 * Reads the attachments the user selected for this turn. Each gets one
 * outcome, passed to onOutcome as soon as it is known; the first outcome that
 * is not ready or partial is returned as `blocked` so the caller can stop
 * before any model request. An abort rejects with the signal's reason.
 */
export async function prepareAttachments(
  store,
  ids,
  {
    vision = false,
    imageLimit = MAX_MESSAGE_IMAGES,
    scrub = (value) => value,
    signal,
    onOutcome = () => {},
  } = {},
) {
  const outcomes = [];
  const images = [];
  let text = "";
  let blocked = null;
  for (const id of new Set(ids)) {
    signal?.throwIfAborted();
    try {
      const prepared = await prepareOne(store, id, {
        vision,
        room: imageLimit - images.length,
        imageLimit,
        scrub,
        signal,
      });
      text += prepared.prompt;
      images.push(...prepared.attached);
      outcomes.push({
        id,
        status: prepared.status,
        code:
          prepared.status === "partial" ? "assistant_attachment_partial" : null,
        details: prepared.details,
      });
    } catch (error) {
      if (signal?.aborted) throw signal.reason;
      if (!(error instanceof AttachmentError)) throw error;
      outcomes.push({
        id,
        status: error.status,
        code: error.code,
        details: error.details,
      });
      blocked ??= error;
    }
    onOutcome(outcomes.at(-1));
  }
  return { text, images, outcomes, blocked };
}

// The model-facing description of exactly what this run can read.
function attachmentGuidelines(vision) {
  return [
    'The user can import attachments into this conversation; a message names each one as [User attachment: "name"; id=...].',
    "read_attachment reads UTF-8 text files and the text of PDFs of up to 100 pages, at most 20,000 characters per call.",
    vision
      ? "It also shows PNG, JPEG, WebP and GIF images, and renders one PDF page as an image when given page, for scanned pages, charts and other visual content."
      : "This model cannot view images: image attachments and PDF pages without extractable text cannot be read; ask the user to choose a vision model instead of guessing what they show.",
    "If a result is truncated or lists pagesWithoutText you have not viewed, tell the user which part you did not read. Other file formats cannot be read.",
  ];
}

/** read_attachment for a run whose conversation has attachments. */
export function readAttachmentTool(runtime, context, { vision }) {
  const store = context.attachments;
  const parameters = Type.Object({
    id: Type.String({ minLength: 1, maxLength: 200 }),
    offset: Type.Optional(Type.Integer({ minimum: 0 })),
    length: Type.Optional(Type.Integer({ minimum: 1, maximum: MAX_READ })),
    ...(vision
      ? {
          page: Type.Optional(
            Type.Integer({ minimum: 1, maximum: MAX_PDF_PAGES }),
          ),
        }
      : {}),
  });
  const description = vision
    ? "Read an attachment imported into this conversation by id: text in pages of up to 20,000 characters (follow nextOffset while hasMore is true), an image attachment as an image, or with page one PDF page rendered as an image."
    : "Read an attachment imported into this conversation by id: text in pages of up to 20,000 characters (follow nextOffset while hasMore is true).";
  const tool = runtime.wrapTool(
    context,
    "read_attachment",
    description,
    parameters,
    async ({ id, offset, length, page }, signal) => {
      const format = store.supportedFormat(id);
      if (page !== undefined || format === "image") {
        if (!vision)
          throw visionRequired(
            "This model cannot view images; ask the user to choose a vision model",
            { format },
          );
        if (page !== undefined && format !== "pdf")
          throw new Error("page applies only to PDF attachments");
        return {
          content: [await store.image(id, page, { signal })],
          details: { id, ...(page === undefined ? {} : { page }) },
        };
      }
      return await store.read(id, offset, length, { signal });
    },
  );
  return { ...tool, promptGuidelines: attachmentGuidelines(vision) };
}
