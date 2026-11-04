import DOMPurify from "dompurify";

type Mermaid = (typeof import("mermaid"))["default"];

/**
 * A reference to anything but the diagram's own parts (`url(#arrow)`):
 * replies load nothing from the network, and neither do their diagrams.
 */
const REMOTE_URL = /url\(\s*(?!['"]?#)[^)]*\)/gi;
const IMPORT = /@import[^;]*;?/gi;

/** Its own instance: the hooks below are not Mermaid's. */
const purify = DOMPurify(window);
purify.addHook("uponSanitizeAttribute", (_node, data) => {
  if (data.attrValue.match(REMOTE_URL)) data.keepAttr = false;
  // A link of a node goes nowhere; a reference to the diagram's own part
  // (`#marker`) stays. The `<a>` itself stays: it carries the node's place.
  if (
    (data.attrName === "href" || data.attrName === "xlink:href") &&
    !data.attrValue.trim().startsWith("#")
  )
    data.keepAttr = false;
});
purify.addHook("uponSanitizeElement", (node, data) => {
  if (data.tagName === "style" && node.textContent)
    node.textContent = node.textContent
      .replace(IMPORT, "")
      .replace(REMOTE_URL, "none");
});

/**
 * Mermaid's SVG as it goes into the page: sanitized again (with the tags
 * Mermaid's own pass keeps), so the page never depends on Mermaid alone, and
 * without what would load or open something — images, link targets, remote
 * CSS.
 */
export function cleanDiagram(svg: string): string {
  return purify.sanitize(svg, {
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
  });
}

/** Markup in a label that could load or open something. */
const RESOURCE_TAGS =
  /<\/?\s*(?:img|image|picture|source|video|audio|track|iframe|frame|object|embed|link|meta|base|script|style|svg|math|form|input|button|textarea|select|a)\b[^>]*>/gi;
/** A tag with attributes: `<b onclick=…>`, `<span style=…>`. */
const TAG_ATTRIBUTES = /<([a-z][a-z0-9]*)\s[^>]*=[^>]*>/gi;

/**
 * The diagram's text without what Mermaid would put in the page while it
 * measures a label, before any sanitizing: tags that load or open something,
 * and every attribute. `<br>`, `<b>`, `<i>` and the like still format.
 */
export function quietDiagramSource(source: string): string {
  // A space, not nothing: Mermaid refuses an empty label.
  return source.replace(RESOURCE_TAGS, " ").replace(TAG_ATTRIBUTES, "<$1>");
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

async function draw(source: string, dark: boolean): Promise<string | null> {
  const mermaid = await loadMermaid();
  // "strict" encodes the diagram's text, turns off its click handlers and
  // sanitizes the SVG; no error diagram is put in the page.
  mermaid.initialize({
    startOnLoad: false,
    securityLevel: "strict",
    suppressErrorRendering: true,
    theme: dark ? "dark" : "default",
    // Labels are measured and drawn in the font the reply is written in.
    fontFamily: getComputedStyle(document.body).fontFamily || undefined,
  });
  const text = quietDiagramSource(source);
  if (!(await mermaid.parse(text, { suppressErrors: true }))) return null;
  count += 1;
  const { svg } = await mermaid.render(`assistant-diagram-${count}`, text);
  return cleanDiagram(svg);
}

/**
 * The SVG of a Mermaid diagram, or null when it does not parse (a reply that
 * is still being written) or Mermaid cannot be loaded. One at a time: the
 * theme is Mermaid's global setting.
 */
export function drawDiagram(
  source: string,
  dark: boolean,
): Promise<string | null> {
  const run = queue.then(() => draw(source, dark)).catch(() => null);
  queue = run;
  return run;
}
