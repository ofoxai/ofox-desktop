import DOMPurify from "dompurify";

type Mermaid = (typeof import("mermaid"))["default"];

/** A drawn diagram: an SVG image, and the width it was drawn for. */
export interface DiagramImage {
  src: string;
  width: number | null;
}

/** Markup in a label that could load or open something. */
const RESOURCE_TAGS =
  /<\/?\s*(?:img|image|picture|source|video|audio|track|iframe|frame|object|embed|link|meta|base|script|style|svg|math|form|input|button|textarea|select|a)\b[^>]*>/gi;
/** A tag with attributes: `<b onclick=…>`, `<span style=…>`. */
const TAG_ATTRIBUTES = /<([a-z][a-z0-9]*)\s[^>]*=[^>]*>/gi;
/** `%%{init: …}%%`: configuration from inside the diagram. */
const DIRECTIVES = /%%\{[\s\S]*?\}%%[ \t]*(?:\r?\n)?/g;
/** Front matter at the top (`---\nconfig: …\n---`): configuration too. */
const FRONT_MATTER = /^\s*---\r?\n[\s\S]*?\r?\n---[ \t]*(?:\r?\n|$)/;
/** A style statement that could reference something: a URL, an escape, an at-rule. */
const STYLE_REFERENCE =
  /^\s*(?:style|classDef|linkStyle)\b.*(?:url|image|\\|@|expression)/i;

/**
 * The diagram's text as Mermaid gets it. Mermaid puts a label's HTML in the
 * page while it measures it, before any sanitizing, so tags that load or
 * open something go first, and every attribute (`<br>`, `<b>`, `<i>` still
 * format). The diagram cannot configure Mermaid (theme, CSS, fonts), and a
 * style that refers to anything is left out.
 */
export function quietDiagramSource(source: string): string {
  return (
    source
      .replace(FRONT_MATTER, "")
      .replace(DIRECTIVES, "")
      .split("\n")
      .filter((line) => !STYLE_REFERENCE.test(line))
      .join("\n")
      // A space, not nothing: Mermaid refuses an empty label.
      .replace(RESOURCE_TAGS, " ")
      .replace(TAG_ATTRIBUTES, "<$1>")
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
    // None of these can be changed from inside a diagram.
    secure: [
      "secure",
      "securityLevel",
      "startOnLoad",
      "maxTextSize",
      "suppressErrorRendering",
      "maxEdges",
      "theme",
      "themeCSS",
      "themeVariables",
      "fontFamily",
      "htmlLabels",
      "darkMode",
    ],
  });
  const text = quietDiagramSource(source);
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
