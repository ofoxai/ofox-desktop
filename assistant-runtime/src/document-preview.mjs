// Fixed host-only raster worker. It never receives a model-chosen filesystem
// path and never serves active HTML/SVG to the renderer.
import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { SaxesParser } from "saxes";
import { thumbnailFromSvg } from "./thumbnails.mjs";
import { imageSize } from "./deck/image.mjs";

const MAX_INPUT = 16 * 1024 * 1024;
const MAX_SVG = 4 * 1024 * 1024;
const MAX_JPEG = 200 * 1024;
function boundedEmbeddedImage(value) {
  const match = /^data:image\/(png|jpeg|webp);base64,([A-Za-z0-9+/=\s]+)$/.exec(
    value,
  );
  if (!match) return false;
  let size;
  if (match[1] !== "webp") {
    size = imageSize(`data:image/${match[1]};base64,${match[2]}`);
  } else {
    const bytes = Buffer.from(match[2], "base64");
    if (
      bytes.toString("ascii", 0, 4) !== "RIFF" ||
      bytes.toString("ascii", 8, 12) !== "WEBP"
    )
      return false;
    const kind = bytes.toString("ascii", 12, 16);
    if (kind === "VP8X" && bytes.length >= 30)
      size = {
        width: 1 + bytes.readUIntLE(24, 3),
        height: 1 + bytes.readUIntLE(27, 3),
      };
    else if (kind === "VP8L" && bytes.length >= 25 && bytes[20] === 0x2f)
      size = {
        width: 1 + (bytes[21] | ((bytes[22] & 0x3f) << 8)),
        height:
          1 + ((bytes[22] >> 6) | (bytes[23] << 2) | ((bytes[24] & 15) << 10)),
      };
    else if (
      kind === "VP8 " &&
      bytes.length >= 30 &&
      bytes.subarray(23, 26).equals(Buffer.from([0x9d, 1, 0x2a]))
    )
      size = {
        width: bytes.readUInt16LE(26) & 0x3fff,
        height: bytes.readUInt16LE(28) & 0x3fff,
      };
  }
  return (
    size &&
    size.width > 0 &&
    size.height > 0 &&
    size.width <= 8192 &&
    size.height <= 8192 &&
    size.width * size.height <= 40_000_000
  );
}
export function safeSvg(svg) {
  if (typeof svg !== "string" || Buffer.byteLength(svg) > MAX_SVG) return false;
  try {
    const parser = new SaxesParser();
    let depth = 0;
    parser.on("doctype", () => {
      throw new Error("doctype");
    });
    parser.on("opentag", ({ name, attributes }) => {
      assert.ok(++depth <= 128);
      if (depth === 1) {
        assert.equal(name.split(":").pop().toLowerCase(), "svg");
        const viewBox = attributes.viewBox
          ?.trim()
          .split(/[\s,]+/)
          .map(Number);
        for (const [axis, index] of [
          ["width", 2],
          ["height", 3],
        ]) {
          const raw = attributes[axis];
          const value =
            raw === undefined
              ? viewBox?.[index]
              : /^\d+(?:\.\d+)?(?:px|pt)?$/.test(raw)
                ? Number.parseFloat(raw) * (raw.endsWith("pt") ? 4 / 3 : 1)
                : NaN;
          assert.ok(Number.isFinite(value) && value > 0 && value <= 8192);
        }
        assert.ok(!/\b(?:width|height)\s*:/i.test(attributes.style ?? ""));
      }
      assert.ok(
        !["script", "style", "iframe", "object", "embed"].includes(
          name.split(":").pop().toLowerCase(),
        ),
      );
      for (const [key, value] of Object.entries(attributes)) {
        const attribute = key.split(":").pop().toLowerCase();
        assert.ok(!attribute.startsWith("on"));
        if (["href", "src"].includes(attribute))
          assert.ok(/^#[\w.-]+$/.test(value) || boundedEmbeddedImage(value));
        if (attribute === "style")
          assert.ok(!/url\s*\(|@import|\\/i.test(value));
        else if (/url\s*\(/i.test(value))
          assert.ok(/^url\(#[\w.-]+\)$/.test(value));
      }
    });
    parser.on("closetag", () => {
      depth--;
    });
    parser.write(svg).close();
    return true;
  } catch {
    return false;
  }
}

// Skia omits OfficeCLI's objectBoundingBox PNG patterns. Project only the
// unambiguous, aspect-matched rectangular case to a direct image; retain all
// surrounding transforms and leave every other SVG construct unchanged.
export function normalizeOfficePatternImages(svg) {
  if (typeof svg !== "string" || Buffer.byteLength(svg) > MAX_SVG) return svg;
  const SVG = "http://www.w3.org/2000/svg";
  const XLINK = "http://www.w3.org/1999/xlink";
  const nodes = [],
    stack = [];
  const ids = new Map();
  try {
    const parser = new SaxesParser({ xmlns: true });
    parser.on("doctype", () => {
      throw new Error("doctype");
    });
    parser.on("opentag", (tag) => {
      assert.ok(nodes.length < 30000 && stack.length < 128);
      const node = {
        ...tag,
        start: svg.lastIndexOf("<", parser.position - 1),
        end: parser.position,
        children: [],
        parent: stack.at(-1),
        text: false,
        other: false,
      };
      node.parent?.children.push(node);
      nodes.push(node);
      stack.push(node);
      const id = tag.attributes.id?.value;
      if (id) ids.set(id, (ids.get(id) ?? 0) + 1);
    });
    parser.on("closetag", () => {
      stack.pop().end = parser.position;
    });
    parser.on("text", (text) => {
      if (text.trim() && stack.length) stack.at(-1).text = true;
    });
    parser.on("cdata", () => {
      if (stack.length) stack.at(-1).other = true;
    });
    parser.on("comment", () => {
      if (stack.length) stack.at(-1).other = true;
    });
    parser.on("processinginstruction", () => {
      if (stack.length) stack.at(-1).other = true;
    });
    parser.write(svg).close();
    const plain = (node, name, allowed) =>
      node.name === name &&
      node.uri === SVG &&
      !node.text &&
      !node.other &&
      Object.values(node.attributes).every(
        (a) => a.uri === "" && allowed.includes(a.local),
      );
    const dimension = (value) =>
      /^\d+(?:\.\d+)?$/.test(value ?? "") &&
      Number(value) > 0 &&
      Number(value) <= 8192
        ? Number(value)
        : null;
    const patterns = new Map();
    for (const pattern of nodes) {
      if (
        !plain(pattern, "pattern", ["id", "patternUnits", "width", "height"]) ||
        pattern.parent?.name !== "defs" ||
        pattern.parent.uri !== SVG ||
        pattern.children.length !== 1
      )
        continue;
      const attr = pattern.attributes;
      const id = attr.id?.value;
      if (
        !/^[A-Za-z_][\w.-]*$/.test(id ?? "") ||
        ids.get(id) !== 1 ||
        attr.patternUnits?.value !== "objectBoundingBox" ||
        attr.width?.value !== "1" ||
        attr.height?.value !== "1"
      )
        continue;
      const image = pattern.children[0];
      if (
        image.name !== "image" ||
        image.uri !== SVG ||
        image.text ||
        image.other ||
        image.children.length ||
        image.attributes.width?.value !== "100%" ||
        image.attributes.height?.value !== "100%"
      )
        continue;
      const attributes = Object.values(image.attributes);
      if (
        !attributes.every(
          (a) =>
            (a.uri === "" &&
              ["href", "width", "height", "preserveAspectRatio"].includes(
                a.local,
              )) ||
            (a.uri === XLINK && a.local === "href"),
        )
      )
        continue;
      const sources = attributes.filter(
        (a) => a.local === "href" && (a.uri === "" || a.uri === XLINK),
      );
      if (sources.length !== 1) continue;
      const href = sources[0].value;
      const data = /^data:image\/png;base64,([A-Za-z0-9+/=\s]+)$/.exec(href);
      if (!data) continue;
      const png = Buffer.from(data[1], "base64");
      if (
        png.length < 33 ||
        png.length > MAX_SVG ||
        png.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a" ||
        png.readUInt32BE(8) !== 13 ||
        png.subarray(12, 16).toString() !== "IHDR"
      )
        continue;
      const width = png.readUInt32BE(16),
        height = png.readUInt32BE(20);
      if (
        !width ||
        !height ||
        width > 8192 ||
        height > 8192 ||
        width * height > 16 * 1024 * 1024
      )
        continue;
      const aspect = image.attributes.preserveAspectRatio?.value;
      if (
        aspect !== undefined &&
        !/^(?:none|x(?:Min|Mid|Max)Y(?:Min|Mid|Max)(?: (?:meet|slice))?)$/.test(
          aspect,
        )
      )
        continue;
      patterns.set(id, { href, width, height, aspect });
    }
    const contextAttributes = (node, allowed) =>
      Object.values(node.attributes).every(
        (a) =>
          a.uri === "http://www.w3.org/2000/xmlns/" ||
          (a.uri === "" && allowed.includes(a.local)),
      );
    const drawingContext = (node) => {
      let ancestor = node.parent;
      while (ancestor?.name === "g" && ancestor.uri === SVG) {
        if (!contextAttributes(ancestor, ["transform", "opacity"]))
          return false;
        ancestor = ancestor.parent;
      }
      return (
        ancestor?.name === "svg" &&
        ancestor.uri === SVG &&
        !ancestor.parent &&
        contextAttributes(ancestor, [
          "width",
          "height",
          "viewBox",
          "version",
          "preserveAspectRatio",
          "transform",
          "opacity",
        ])
      );
    };
    const escape = (value) =>
      value
        .replaceAll("&", "&amp;")
        .replaceAll('"', "&quot;")
        .replaceAll("<", "&lt;");
    const replacements = [];
    let size = Buffer.byteLength(svg);
    for (const node of nodes) {
      if (
        !plain(node, "rect", ["width", "height", "fill"]) ||
        node.children.length ||
        !drawingContext(node)
      )
        continue;
      const width = dimension(node.attributes.width?.value),
        height = dimension(node.attributes.height?.value);
      const id = /^url\(#([A-Za-z_][\w.-]*)\)$/.exec(
        node.attributes.fill?.value ?? "",
      )?.[1];
      const image = patterns.get(id);
      if (!width || !height || !image) continue;
      const exactAspect =
        Math.abs(width / height / (image.width / image.height) - 1) <= 1e-6;
      // OfficeCLI rounds each rectangle dimension to two decimal SVG units.
      // Accept only that coordinate uncertainty, not a general aspect margin:
      // some exact proportional size must lie within +/-0.005 of both sides.
      const roundedAspect =
        ["width", "height"].every((key) =>
          /^\d+(?:\.\d{1,2})?$/.test(node.attributes[key].value),
        ) &&
        Math.abs(width * image.height - height * image.width) <=
          0.005 * (image.width + image.height) + 1e-9;
      if (!exactAspect && !roundedAspect) continue;
      const replacement = `<image width="${escape(node.attributes.width.value)}" height="${escape(node.attributes.height.value)}" href="${escape(image.href)}"${image.aspect === undefined ? "" : ` preserveAspectRatio="${escape(image.aspect)}"`}/>`;
      size +=
        Buffer.byteLength(replacement) -
        Buffer.byteLength(svg.slice(node.start, node.end));
      if (size > MAX_SVG) return svg;
      replacements.push({ start: node.start, end: node.end, replacement });
    }
    let result = "",
      at = 0;
    for (const item of replacements) {
      result += svg.slice(at, item.start) + item.replacement;
      at = item.end;
    }
    result += svg.slice(at);
    return Buffer.byteLength(result) <= MAX_SVG && safeSvg(result)
      ? result
      : svg;
  } catch {
    return svg;
  }
}

// Skia also omits SVG markers. Materialize only OfficeCLI's plain line plus
// self-contained triangle markers. Curves, custom marker viewports, clipping,
// inherited paint and resource references remain unchanged; this is not a
// general SVG marker implementation and never edits the source document.
export function normalizeOfficeArrowheads(svg) {
  if (!safeSvg(svg)) return svg;
  const SVG = "http://www.w3.org/2000/svg";
  const nodes = [],
    stack = [],
    ids = new Map();
  try {
    const parser = new SaxesParser({ xmlns: true });
    parser.on("doctype", () => {
      throw new Error("doctype");
    });
    parser.on("opentag", (tag) => {
      assert.ok(nodes.length < 30000 && stack.length < 128);
      const node = {
        ...tag,
        start: svg.lastIndexOf("<", parser.position - 1),
        end: parser.position,
        children: [],
        parent: stack.at(-1),
        content: false,
      };
      node.parent?.children.push(node);
      nodes.push(node);
      stack.push(node);
      for (const attribute of Object.values(tag.attributes))
        if (attribute.local === "id" && attribute.value)
          ids.set(attribute.value, (ids.get(attribute.value) ?? 0) + 1);
    });
    parser.on("closetag", () => {
      stack.pop().end = parser.position;
    });
    parser.on("text", (value) => {
      if (value.trim() && stack.length) stack.at(-1).content = true;
    });
    for (const event of ["cdata", "comment", "processinginstruction"])
      parser.on(event, () => {
        if (stack.length) stack.at(-1).content = true;
      });
    parser.write(svg).close();
    const plain = (node, name, allowed) =>
      node.name === name &&
      node.uri === SVG &&
      !node.content &&
      Object.values(node.attributes).every(
        (a) => a.uri === "" && allowed.includes(a.local),
      );
    const numeric = (value, limit = 8192) => {
      if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(value ?? ""))
        return null;
      const number = Number(value);
      return Number.isFinite(number) && Math.abs(number) <= limit
        ? number
        : null;
    };
    const values = (text) => text.trim().split(/[\s,]+/);
    const finiteTransform = (value) => {
      if (value === undefined) return true;
      if (value.length > 512) return false;
      let at = 0,
        count = 0;
      const commands =
        /\s*(matrix|translate|scale|rotate|skewX|skewY)\s*\(([^()]*)\)\s*,?/gy;
      while (at < value.length) {
        commands.lastIndex = at;
        const match = commands.exec(value);
        if (!match || ++count > 16) return false;
        const numbers = values(match[2]);
        const lengths = {
          matrix: [6],
          translate: [1, 2],
          scale: [1, 2],
          rotate: [1, 3],
          skewX: [1],
          skewY: [1],
        };
        if (
          !lengths[match[1]].includes(numbers.length) ||
          numbers.some((number) => numeric(number, 10000) === null)
        )
          return false;
        at = commands.lastIndex;
      }
      return count > 0;
    };
    const contextAttributes = (node, allowed) =>
      Object.values(node.attributes).every(
        (a) =>
          a.uri === "http://www.w3.org/2000/xmlns/" ||
          (a.uri === "" && allowed.includes(a.local)),
      ) &&
      finiteTransform(node.attributes.transform?.value) &&
      (node.attributes.opacity === undefined ||
        (numeric(node.attributes.opacity.value, 1) !== null &&
          Number(node.attributes.opacity.value) >= 0));
    const drawingContext = (node) => {
      let parent = node.parent;
      while (parent?.name === "g" && parent.uri === SVG) {
        if (!contextAttributes(parent, ["transform", "opacity"])) return false;
        parent = parent.parent;
      }
      return (
        parent?.name === "svg" &&
        parent.uri === SVG &&
        !parent.parent &&
        contextAttributes(parent, [
          "width",
          "height",
          "viewBox",
          "version",
          "preserveAspectRatio",
          "transform",
          "opacity",
        ])
      );
    };
    const markers = new Map();
    for (const node of nodes) {
      if (
        !plain(node, "marker", [
          "id",
          "markerWidth",
          "markerHeight",
          "refX",
          "refY",
          "orient",
          "markerUnits",
        ]) ||
        !node.parent ||
        !plain(node.parent, "defs", []) ||
        node.parent.parent?.name !== "svg" ||
        node.parent.parent.uri !== SVG ||
        node.parent.parent.parent ||
        node.children.length !== 1
      )
        continue;
      const attr = node.attributes;
      const id = attr.id?.value;
      const width = numeric(attr.markerWidth?.value, 64),
        height = numeric(attr.markerHeight?.value, 64),
        refX = numeric(attr.refX?.value),
        refY = numeric(attr.refY?.value);
      const units = attr.markerUnits?.value ?? "strokeWidth";
      if (
        !/^[A-Za-z_][\w.-]*$/.test(id ?? "") ||
        ids.get(id) !== 1 ||
        !width ||
        width < 0 ||
        !height ||
        height < 0 ||
        refX === null ||
        refY === null ||
        refX < 0 ||
        refX > width ||
        refY < 0 ||
        refY > height ||
        attr.orient?.value !== "auto" ||
        !["strokeWidth", "userSpaceOnUse"].includes(units)
      )
        continue;
      const polygon = node.children[0];
      if (
        !plain(polygon, "polygon", ["points", "fill"]) ||
        polygon.children.length ||
        !/^#[\da-f]{6}$/i.test(polygon.attributes.fill?.value ?? "")
      )
        continue;
      const points = values(polygon.attributes.points?.value ?? "").map(
        (value) => numeric(value, 64),
      );
      if (
        points.length !== 6 ||
        points.some(
          (value, i) =>
            value === null || value < 0 || value > (i % 2 ? height : width),
        ) ||
        Math.abs(
          (points[2] - points[0]) * (points[5] - points[1]) -
            (points[4] - points[0]) * (points[3] - points[1]),
        ) < 1e-6
      )
        continue;
      markers.set(id, {
        points,
        refX,
        refY,
        units,
        fill: polygon.attributes.fill.value,
        // OfficeCLI's proven tail-arrow signature anchors the triangle base
        // at the connector endpoint. Its tip then falls under the next node.
        // Native WPS confirms the tip belongs at the endpoint. Correct only
        // this exact exporter signature; arbitrary SVG refX semantics stay.
        officeTail:
          /^arrow\d+$/.test(id) &&
          attr.markerWidth.value === "9" &&
          attr.markerHeight.value === "9" &&
          attr.refX.value === "0" &&
          attr.refY.value === "4.5" &&
          units === "strokeWidth" &&
          points.every((value, i) => value === [0, 0, 9, 4.5, 0, 9][i]),
      });
    }
    const replacements = [];
    let size = Buffer.byteLength(svg);
    for (const node of nodes) {
      if (
        !plain(node, "line", [
          "x1",
          "y1",
          "x2",
          "y2",
          "stroke",
          "stroke-width",
          "marker-start",
          "marker-end",
        ]) ||
        node.children.length ||
        !drawingContext(node) ||
        !/^#[\da-f]{6}$/i.test(node.attributes.stroke?.value ?? "")
      )
        continue;
      const attr = node.attributes;
      const [x1, y1, x2, y2] = ["x1", "y1", "x2", "y2"].map((key) =>
        numeric(attr[key]?.value),
      );
      const stroke = numeric(attr["stroke-width"]?.value, 64);
      if (
        [x1, y1, x2, y2].some((value) => value === null) ||
        !stroke ||
        stroke < 0 ||
        Math.hypot(x2 - x1, y2 - y1) < 1e-6
      )
        continue;
      const heads = ["marker-start", "marker-end"].map((key) => {
        const value = attr[key]?.value;
        if (value === undefined || value === "none") return null;
        const id = /^url\(#([A-Za-z_][\w.-]*)\)$/.exec(value)?.[1];
        return markers.get(id) ?? false;
      });
      if (heads.includes(false) || !heads.some(Boolean)) continue;
      const angle = Math.atan2(y2 - y1, x2 - x1);
      const polygons = heads
        .map((marker, endpoint) => {
          if (!marker) return "";
          const scale = marker.units === "strokeWidth" ? stroke : 1;
          const [x, y] = endpoint ? [x2, y2] : [x1, y1];
          const refX = endpoint && marker.officeTail ? 9 : marker.refX;
          const points = [];
          for (let i = 0; i < 6; i += 2) {
            const dx = (marker.points[i] - refX) * scale,
              dy = (marker.points[i + 1] - marker.refY) * scale;
            const px = x + dx * Math.cos(angle) - dy * Math.sin(angle),
              py = y + dx * Math.sin(angle) + dy * Math.cos(angle);
            if (
              ![px, py].every(
                (value) => Number.isFinite(value) && Math.abs(value) <= 32768,
              )
            )
              throw new Error("marker geometry");
            points.push(`${Number(px.toFixed(6))},${Number(py.toFixed(6))}`);
          }
          return `<polygon points="${points.join(" ")}" fill="${marker.fill}" stroke="none"/>`;
        })
        .join("");
      const original = svg.slice(node.start, node.end);
      const replacement =
        original.replace(
          /\s+marker-(?:start|end)\s*=\s*(?:"[^"]*"|'[^']*')/g,
          "",
        ) + polygons;
      size += Buffer.byteLength(replacement) - Buffer.byteLength(original);
      if (size > MAX_SVG) return svg;
      replacements.push({ start: node.start, end: node.end, replacement });
    }
    let result = "",
      at = 0;
    for (const item of replacements) {
      result += svg.slice(at, item.start) + item.replacement;
      at = item.end;
    }
    result += svg.slice(at);
    return Buffer.byteLength(result) <= MAX_SVG && safeSvg(result)
      ? result
      : svg;
  } catch {
    return svg;
  }
}

export async function rasterPages(
  input,
  {
    render = thumbnailFromSvg,
    write = (name, bytes) =>
      writeFile(name, bytes, { flag: "wx", mode: 0o600 }),
  } = {},
) {
  assert.ok(Array.isArray(input) && input.length <= 60);
  assert.ok(Buffer.byteLength(JSON.stringify(input)) <= MAX_INPUT);
  const pages = [];
  const seen = new Set();
  for (const item of input) {
    assert.ok(
      Number.isInteger(item.page) &&
        item.page >= 1 &&
        item.page <= 1000 &&
        !seen.has(item.page),
    );
    seen.add(item.page);
    if (!safeSvg(item.svg)) continue;
    try {
      const normalized = normalizeOfficeArrowheads(
        normalizeOfficePatternImages(item.svg),
      );
      if (Buffer.byteLength(normalized) > MAX_SVG || !safeSvg(normalized))
        continue;
      const image = await render(Buffer.from(normalized), {
        maxBytes: MAX_JPEG,
        maxEdge: 768,
      });
      assert.equal(image?.mimeType, "image/jpeg");
      const bytes = Buffer.from(image.data, "base64");
      assert.ok(
        bytes.length > 4 &&
          bytes.length <= MAX_JPEG &&
          bytes[0] === 255 &&
          bytes[1] === 216 &&
          bytes.at(-2) === 255 &&
          bytes.at(-1) === 217,
      );
      await write(`document-preview-${item.page}.jpg`, bytes);
      pages.push(item.page);
    } catch {
      /* A failed raster does not invalidate the delivered document. */
    }
  }
  return { mimeType: "image/jpeg", pages };
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  const bytes = await readFile("document-preview-input.json");
  assert.ok(bytes.length <= MAX_INPUT);
  process.stdout.write(
    JSON.stringify(await rasterPages(JSON.parse(bytes.toString("utf8")))),
  );
}
