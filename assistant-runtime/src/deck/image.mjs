// The natural size of a picture the deck embeds as a data URI, read from
// the file's header without decoding pixels, so the renderer can keep the
// picture's proportions. PNG, JPEG and SVG (the types schema.mjs accepts);
// anything it cannot read is null.

const DATA_URI = /^data:image\/(png|jpeg|svg\+xml);base64,(.*)$/s;

// PNG: the IHDR chunk follows the 8-byte signature; width and height are
// its first two 4-byte fields (bytes 16–23).
function pngSize(bytes) {
  if (bytes.length < 24) return null;
  if (bytes.toString("latin1", 1, 4) !== "PNG") return null;
  if (bytes.toString("latin1", 12, 16) !== "IHDR") return null;
  return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
}

// JPEG: segments follow the SOI marker, each FF <marker> <2-byte length>;
// the first baseline (SOF0), extended (SOF1) or progressive (SOF2) frame
// header holds the height and then the width. Scan data (SOS) or the end
// means there is none.
function jpegSize(bytes) {
  if (bytes[0] !== 0xff || bytes[1] !== 0xd8) return null;
  let at = 2;
  while (at + 3 < bytes.length) {
    if (bytes[at] !== 0xff) return null;
    const marker = bytes[at + 1];
    if (marker === 0xff) {
      at += 1; // a fill byte
    } else if (marker >= 0xc0 && marker <= 0xc2) {
      if (at + 9 > bytes.length) return null;
      return {
        width: bytes.readUInt16BE(at + 7),
        height: bytes.readUInt16BE(at + 5),
      };
    } else if (marker === 0xda || marker === 0xd9) {
      return null;
    } else if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) {
      at += 2; // a marker without a length
    } else {
      at += 2 + bytes.readUInt16BE(at + 2);
    }
  }
  return null;
}

// SVG: the root element's width and height when both are in px or have no
// unit, else its viewBox ("min-x min-y width height").
const attribute = (tag, name) =>
  new RegExp(`\\s${name}\\s*=\\s*(["'])(.*?)\\1`).exec(tag)?.[2];
const pixels = (value) =>
  Number(/^\s*(\d+(?:\.\d+)?)\s*(?:px)?\s*$/.exec(value ?? "")?.[1]);

function svgSize(bytes) {
  const tag = /<svg\b[^>]*>/i.exec(bytes.toString("utf8"))?.[0];
  if (tag === undefined) return null;
  const width = pixels(attribute(tag, "width"));
  const height = pixels(attribute(tag, "height"));
  if (width > 0 && height > 0) return { width, height };
  const box = attribute(tag, "viewBox")
    ?.trim()
    .split(/[\s,]+/)
    .map(Number);
  return box?.length === 4 ? { width: box[2], height: box[3] } : null;
}

const READERS = { png: pngSize, jpeg: jpegSize, "svg+xml": svgSize };

// { width, height } of a data:image/(png|jpeg|svg+xml);base64 URI, in the
// image's own units (pixels, or SVG user units), or null.
export function imageSize(dataUri) {
  const match = typeof dataUri === "string" ? DATA_URI.exec(dataUri) : null;
  if (match === null) return null;
  const size = READERS[match[1]](Buffer.from(match[2], "base64"));
  const usable = (value) => Number.isFinite(value) && value > 0;
  return size && usable(size.width) && usable(size.height) ? size : null;
}
