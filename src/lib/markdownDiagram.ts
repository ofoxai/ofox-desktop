import DOMPurify from "dompurify";

type Mermaid = (typeof import("mermaid"))["default"];

/** A drawn diagram: an SVG image, and the width it was drawn for. */
export interface DiagramImage {
  src: string;
  width: number | null;
}

/** Front matter at the top (`---\nconfig: …\n---`), as loosely as Mermaid reads it. */
const FRONT_MATTER = /^\s*-{3}\s*[\n\r][\s\S]*?[\n\r]-{3}\s*(?:[\n\r]+|$)/;
/** `%%{init: …}%%`, closed or not, as Mermaid reads it. */
const DIRECTIVES = /%%\{(?:(?!\}%%)[\s\S])*(?:\}%%)?/g;
/**
 * What only a diagram that reaches for something has: a URL, an image, an
 * at-rule, a CSS expression, an image shape (`img:`), and a backslash, which
 * CSS reads as an escape (`u\72l(`); `\n` alone can spell none of these.
 */
const REFERENCE =
  /url\s*\(|image-set\s*\(|@import|expression\s*\(|\bimg\s*:|\\(?!n)/i;
/** Something an HTML parser would take for a tag, and `<<annotations>>`. */
const TAG_LIKE = /<<[^<>]*>>|<[a-z!?/][^>]*>?/gi;
/** Formatting a label may keep: a bare tag, no attribute. */
const FORMAT_TAG = /^<\/?(?:br|b|i|em|strong|u|s|sub|sup|small)\s*\/?>$/i;
const ANNOTATION = /^<<[\w\s-]+>>$/;

/**
 * The diagram's text as Mermaid gets it, or null when it is not drawn but
 * shown as its code. It cannot configure Mermaid (no front matter, no
 * directive), cannot reach for anything, and a tag that is not plain
 * formatting turns into text: `‹` takes the place of its `<`.
 */
export function diagramSource(source: string): string | null {
  const text = source.replace(FRONT_MATTER, "").replace(DIRECTIVES, "");
  if (/%%\{/.test(text) || /^\s*-{3}/.test(text) || REFERENCE.test(text))
    return null;
  return text.replace(TAG_LIKE, (tag) =>
    FORMAT_TAG.test(tag) || ANNOTATION.test(tag) ? tag : `‹${tag.slice(1)}`,
  );
}

const SANITIZE = {
  // The tags Mermaid's own pass keeps.
  ADD_TAGS: ["foreignobject"],
  ADD_ATTR: ["dominant-baseline"],
  HTML_INTEGRATION_POINTS: { foreignobject: true },
  FORBID_TAGS: [
    "img",
    "image",
    "picture",
    "source",
    "video",
    "audio",
    "track",
    "form",
    "input",
    "button",
  ],
};

/** Its own instance: the hook below is not Mermaid's. */
const purify = DOMPurify(window);
purify.addHook("uponSanitizeAttribute", (_node, data) => {
  // A node's link goes nowhere; a reference to the diagram's own part
  // (`#marker`) stays. The `<a>` itself stays: it carries the node's place.
  if (
    (data.attrName === "href" || data.attrName === "xlink:href") &&
    !data.attrValue.trim().startsWith("#")
  )
    data.keepAttr = false;
});

/** Mermaid's SVG sanitized again, so the page never depends on Mermaid alone. */
export function cleanDiagram(svg: string): string {
  return purify.sanitize(svg, SANITIZE);
}

/**
 * The diagram as an image. An SVG shown as an image runs nothing, loads
 * nothing, and its styles stay inside it: whatever a diagram says, it cannot
 * reach the network or the app's page.
 */
export function diagramImage(svg: string): DiagramImage | null {
  const fragment = purify.sanitize(svg, {
    ...SANITIZE,
    RETURN_DOM_FRAGMENT: true,
  });
  const root = fragment.querySelector("svg");
  if (!root) return null;
  root.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  const width = Number(root.getAttribute("viewBox")?.split(/[\s,]+/)[2]);
  return {
    src: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(
      new XMLSerializer().serializeToString(root),
    )}`,
    width: Number.isFinite(width) && width > 0 ? width : null,
  };
}

let loading: Promise<Mermaid> | null = null;
let queue: Promise<unknown> = Promise.resolve();
let count = 0;

/**
 * Mermaid is loaded the first time a reply holds a diagram, so none of it
 * weighs on the app's first paint. A failed load leaves the diagram as its
 * source and is retried on the next one.
 */
function loadMermaid(): Promise<Mermaid> {
  loading ??= import("mermaid")
    .then(({ default: mermaid }) => mermaid)
    .catch((error) => {
      loading = null;
      throw error;
    });
  return loading;
}

async function draw(
  source: string,
  dark: boolean,
): Promise<DiagramImage | null> {
  const mermaid = await loadMermaid();
  mermaid.initialize({
    startOnLoad: false,
    // "strict" encodes the diagram's text, turns off its click handlers and
    // sanitizes the SVG; no error diagram is put in the page.
    securityLevel: "strict",
    suppressErrorRendering: true,
    theme: dark ? "dark" : "default",
    // Labels are measured and drawn in the font the reply is written in (a
    // system font, which an SVG image can use too).
    fontFamily: getComputedStyle(document.body).fontFamily || undefined,
    // Labels are SVG text: a label's HTML is never parsed, not even while
    // Mermaid measures it in the page.
    htmlLabels: false,
    // None of these can be changed from inside a diagram, at any depth
    // (Mermaid removes them from nested settings too, e.g. flowchart's).
    secure: [
      "secure",
      "securityLevel",
      "startOnLoad",
      "maxTextSize",
      "suppressErrorRendering",
      "maxEdges",
      "dompurifyConfig",
      "theme",
      "themeCSS",
      "themeVariables",
      "fontFamily",
      "altFontFamily",
      "htmlLabels",
      "darkMode",
    ],
  });
  const text = diagramSource(source);
  if (text === null) return null;
  if (!(await mermaid.parse(text, { suppressErrors: true }))) return null;
  count += 1;
  const { svg } = await mermaid.render(`assistant-diagram-${count}`, text);
  return diagramImage(svg);
}

/**
 * A Mermaid diagram as an image, or null when it does not parse (a reply
 * that is still being written) or Mermaid cannot be loaded. One at a time:
 * the theme is Mermaid's global setting.
 */
export function drawDiagram(
  source: string,
  dark: boolean,
): Promise<DiagramImage | null> {
  const run = queue.then(() => draw(source, dark)).catch(() => null);
  queue = run;
  return run;
}
