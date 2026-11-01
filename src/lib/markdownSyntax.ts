type Lowlight = ReturnType<(typeof import("lowlight"))["createLowlight"]>;

const MAX_BLOCK = 32 * 1024;
const MAX_MESSAGE = 64 * 1024;

let lowlight: Lowlight | null = null;
let loading: Promise<void> | null = null;

/**
 * Loads the grammars once, in their own chunk, the first time a highlighted
 * message is shown, so they never weigh on the app's first paint. A failed
 * load leaves code as plain text and is retried on the next message.
 */
export function loadMarkdownSyntax(): Promise<void> {
  loading ??= import("lowlight")
    .then(({ common, createLowlight }) => {
      lowlight = createLowlight(common);
    })
    .catch((error) => {
      loading = null;
      console.warn("[markdownSyntax] grammars failed to load", error);
    });
  return loading;
}

export const markdownSyntaxReady = () => lowlight !== null;

interface Node {
  type: string;
  tagName?: string;
  value?: string;
  properties?: Record<string, unknown>;
  children?: Node[];
}

/** Only labelled fenced code is parsed. Large/unknown blocks retain all text. */
export function markdownSyntax() {
  return (tree: unknown) => {
    const grammars = lowlight;
    if (!grammars) return;
    let remaining = MAX_MESSAGE;
    const visit = (node: Node, parent?: Node) => {
      if (node.tagName === "code" && parent?.tagName === "pre") {
        const classes = Array.isArray(node.properties?.className)
          ? node.properties.className.filter(
              (value): value is string => typeof value === "string",
            )
          : [];
        const language = classes
          .find((value) => /^language-\S+$/.test(value))
          ?.slice("language-".length);
        if (!language || !grammars.registered(language)) return;
        if (node.children?.some((child) => child.type !== "text")) return;
        const source = (node.children ?? [])
          .map((child) => child.value ?? "")
          .join("");
        if (source.length > MAX_BLOCK || source.length > remaining) return;
        remaining -= source.length;
        try {
          const highlighted = grammars.highlight(language, source);
          node.children = highlighted.children as Node[];
          node.properties = {
            ...node.properties,
            className: [...classes, "hljs"],
          };
        } catch {
          // Incomplete streamed code must remain readable if a grammar fails.
        }
        return;
      }
      for (const child of node.children ?? []) visit(child, node);
    };
    visit(tree as Node);
  };
}
