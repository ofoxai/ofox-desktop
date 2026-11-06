// Images a model is shown of a file: a rendered PDF page, or a thumbnail of
// a generated document's first page or slide. The native canvas addon loads
// on first use, not with the runtime.

const THUMBNAIL_EDGE = 768;
const MAX_THUMBNAIL_BYTES = 200 * 1024;
const THUMBNAIL_ENCODINGS = [85, 60, 40].map((quality) => [
  "image/jpeg",
  quality,
]);

/**
 * The first of `encodings` ([mimeType, quality?] pairs, tried in order) that
 * encodes the canvas in at most maxBytes, as { data: base64, mimeType }, or
 * null when none does.
 */
export function encodeWithin(canvas, encodings, maxBytes) {
  for (const [mimeType, quality] of encodings) {
    const bytes = canvas.toBuffer(mimeType, quality);
    if (bytes.length <= maxBytes)
      return { data: bytes.toString("base64"), mimeType };
  }
  return null;
}

/**
 * A JPEG of a canvas or loaded image, scaled down to maxEdge on its long side
 * over white (JPEG has no transparency), at the best quality step within
 * maxBytes; null when even the lowest step is over.
 */
export async function encodeThumbnail(
  source,
  { maxBytes = MAX_THUMBNAIL_BYTES, maxEdge = THUMBNAIL_EDGE } = {},
) {
  const { createCanvas } = await import("@napi-rs/canvas");
  const scale = Math.min(1, maxEdge / Math.max(source.width, source.height));
  const width = Math.max(1, Math.round(source.width * scale));
  const height = Math.max(1, Math.round(source.height * scale));
  const canvas = createCanvas(width, height);
  const context = canvas.getContext("2d");
  context.fillStyle = "#fff";
  context.fillRect(0, 0, width, height);
  context.drawImage(source, 0, 0, width, height);
  return encodeWithin(canvas, THUMBNAIL_ENCODINGS, maxBytes);
}

/** A thumbnail of PNG bytes; rejects bytes the canvas cannot decode. */
export async function thumbnailFromPng(bytes, options) {
  const { loadImage } = await import("@napi-rs/canvas");
  return encodeThumbnail(await loadImage(bytes), options);
}

/**
 * A thumbnail of SVG bytes, drawn at the full long edge: a vector loses
 * nothing when enlarged, and an SVG sized only by its viewBox loads small.
 * Rejects when this canvas build cannot read the SVG.
 */
export async function thumbnailFromSvg(bytes, options = {}) {
  const { loadImage } = await import("@napi-rs/canvas");
  const image = await loadImage(bytes);
  const maxEdge = options.maxEdge ?? THUMBNAIL_EDGE;
  const scale = maxEdge / Math.max(image.width, image.height);
  if (!Number.isFinite(scale) || scale <= 0)
    throw new Error("The SVG has no size");
  const width = Math.max(1, Math.round(image.width * scale));
  const height = Math.max(1, Math.round(image.height * scale));
  image.width = width;
  image.height = height;
  return encodeThumbnail(image, options);
}
