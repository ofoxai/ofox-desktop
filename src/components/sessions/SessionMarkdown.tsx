import {
  Children,
  Fragment,
  isValidElement,
  memo,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import Markdown, { type Components } from "react-markdown";
import remarkParse from "remark-parse";
import { unified, type PluggableList } from "unified";
import remarkGfm from "remark-gfm";
import remarkCjkFriendly from "remark-cjk-friendly";
import remarkCjkFriendlyStrikethrough from "remark-cjk-friendly-gfm-strikethrough";
import { useTranslation } from "react-i18next";
import { settingsApi } from "@/lib/api/settings";
import { CodeBlock, codePreview } from "./CodeBlock";
import { toast } from "sonner";
import {
  loadMarkdownSyntax,
  markdownSyntax,
  markdownSyntaxReady,
} from "@/lib/markdownSyntax";
import {
  hasMathMarkers,
  loadMarkdownMath,
  markdownMathPlugins,
  markdownMathReady,
  rewriteMathDelimiters,
} from "@/lib/markdownMath";
import { rewriteBareUrls } from "@/lib/markdownUrls";
import { splitUrlTail } from "@/lib/urlTail";

function codeText(children: ReactNode): string {
  return Children.toArray(children)
    .map((child) => {
      if (typeof child === "string" || typeof child === "number")
        return String(child);
      return isValidElement<{ children?: ReactNode }>(child)
        ? codeText(child.props.children)
        : "";
    })
    .join("");
}

/** The language of a fenced block's code (`language-…` on its `<code>`). */
function codeLanguage(children: ReactNode): string | null {
  for (const child of Children.toArray(children)) {
    if (!isValidElement<{ className?: string }>(child)) continue;
    const found = /(?:^|\s)language-(\S+)/.exec(child.props.className ?? "");
    if (found) return found[1].toLowerCase();
  }
  return null;
}

interface MarkdownNode {
  type: string;
  value?: string;
  url?: string;
  children?: MarkdownNode[];
  position?: { start: { offset?: number }; end: { offset?: number } };
  tagName?: string;
  properties?: Record<string, unknown>;
}

/**
 * The text of a link remark-gfm made from a bare URL (`https://…`, `www.…`):
 * a link that spans exactly its own text. `[text](url)` and `<url>` are
 * wider than their text and the author's own, so they are not one.
 */
function bareUrlText(node: MarkdownNode): string | null {
  const text = node.children?.[0];
  const start = node.position?.start.offset;
  const end = node.position?.end.offset;
  if (
    node.type !== "link" ||
    node.children?.length !== 1 ||
    text?.type !== "text" ||
    text.value === undefined ||
    start === undefined ||
    end === undefined ||
    text.position?.start.offset !== start ||
    text.position?.end.offset !== end
  )
    return null;
  return node.url === text.value || node.url === `http://${text.value}`
    ? text.value
    : null;
}

/**
 * remark-gfm ends a bare URL only at ASCII punctuation, so text written
 * without spaces ("…northeast1。然后") makes the full stop and what follows
 * part of the link, and the link wrong. Ends such a link at its first
 * full-width punctuation mark and puts the rest after it as text.
 * `rewriteBareUrls` already does this before parsing, and keeps the bold
 * around such a URL, which this cannot; this is the backstop for the URLs it
 * leaves alone, as in a message above its size cap.
 */
function splitBareUrlTail() {
  return (tree: unknown) => {
    const visit = (node: MarkdownNode) => {
      if (!node.children) return;
      node.children = node.children.flatMap((child): MarkdownNode[] => {
        const found = bareUrlText(child);
        if (found === null) {
          visit(child);
          return [child];
        }
        const { url, tail } = splitUrlTail(found);
        if (!tail) return [child];
        return [
          {
            ...child,
            url: child.url?.slice(0, child.url.length - tail.length),
            children: [{ type: "text", value: url }],
          },
          { type: "text", value: tail },
        ];
      });
    };
    visit(tree as MarkdownNode);
  };
}

/** What KaTeX draws: a formula, a display formula, or the text of one it cannot draw. */
const MATH_CLASSES = ["katex", "katex-display", "katex-error"];

function isMath(node: MarkdownNode): boolean {
  const classes = node.properties?.className;
  return (
    Array.isArray(classes) &&
    classes.some((name) => MATH_CLASSES.includes(String(name)))
  );
}

function highlightPlugin({ needle }: { needle: string }) {
  return (tree: unknown) => {
    const pattern = new RegExp(
      `(${needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`,
      "giu",
    );
    const visit = (node: MarkdownNode) => {
      if (!node.children) return;
      node.children = node.children.flatMap((child): MarkdownNode[] => {
        if (child.type !== "text") {
          // A formula is drawn from many small pieces; marking one would break it.
          if (!isMath(child)) visit(child);
          return [child];
        }
        return (child.value ?? "")
          .split(pattern)
          .filter(Boolean)
          .map((value) =>
            value.toLocaleLowerCase() === needle.toLocaleLowerCase()
              ? {
                  type: "element",
                  tagName: "mark",
                  properties: {
                    className: [
                      "rounded-sm",
                      "bg-brand-soft",
                      "text-brand-ink",
                    ],
                  },
                  children: [{ type: "text", value }],
                }
              : { type: "text", value },
          );
      });
    };
    visit(tree as MarkdownNode);
  };
}

/** True once the lazily loaded grammars can highlight code for this caller. */
function useMarkdownSyntax(enabled: boolean) {
  const [ready, setReady] = useState(markdownSyntaxReady);
  useEffect(() => {
    if (!enabled || ready) return;
    let live = true;
    void loadMarkdownSyntax().then(() => {
      if (live) setReady(markdownSyntaxReady());
    });
    return () => {
      live = false;
    };
  }, [enabled, ready]);
  return enabled && ready;
}

/** True once the lazily loaded KaTeX can draw the formulas of this message. */
function useMarkdownMath(enabled: boolean) {
  const [ready, setReady] = useState(markdownMathReady);
  useEffect(() => {
    if (!enabled || ready) return;
    let live = true;
    void loadMarkdownMath().then(() => {
      if (live) setReady(markdownMathReady());
    });
    return () => {
      live = false;
    };
  }, [enabled, ready]);
  return enabled && ready;
}

/** A link opens only a web page; any other address is dropped. */
function webUrl(url: string): string {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:"
      ? parsed.href
      : "";
  } catch {
    return "";
  }
}

interface Block {
  start: number;
  text: string;
}

/**
 * `source` cut into its top-level blocks (paragraphs, lists, tables, formula
 * and code blocks) as `remarkPlugins` read it, so each block is drawn, and
 * kept, on its own. A block that follows the one before it with no blank
 * line stays with it: what it is can hang on that line (a list after a
 * paragraph is read as interrupting it). A reference or footnote definition
 * belongs to the whole text: then it stays one block.
 */
function splitBlocks(source: string, remarkPlugins: PluggableList): Block[] {
  const tree = unified().use(remarkParse).use(remarkPlugins).parse(source);
  if (
    tree.children.some(
      (node) =>
        node.type === "definition" || node.type === "footnoteDefinition",
    )
  )
    return [{ start: 0, text: source }];
  const starts: number[] = [];
  let end = 0;
  for (const node of tree.children) {
    const start = node.position?.start.offset ?? end;
    if (starts.length === 0 || /\n[ \t]*\n/.test(source.slice(end, start)))
      starts.push(start);
    end = node.position?.end.offset ?? start;
  }
  if (starts.length < 2) return [{ start: 0, text: source }];
  return starts.map((start, index) => ({
    start,
    text: source.slice(
      index === 0 ? 0 : start,
      starts[index + 1] ?? source.length,
    ),
  }));
}

// A `$$` that has not closed yet, which `rewriteMathDelimiters` shows as text.
const UNCLOSED_FENCE =
  /^([ \t>]*(?:(?:[-*+]|\d{1,9}[.)])[ \t]+)?)\\\$\\\$[ \t]*([^\n]*)/;

/**
 * The block being written, with a display formula it opens and has not
 * closed shown as its source: an opening fence that runs to the block's end.
 */
const asWritten = (text: string) =>
  text.replace(UNCLOSED_FENCE, (_, containers: string, rest: string) =>
    rest
      ? `${containers}$$\n${containers.replace(/[^\s>]/g, " ")}${rest}`
      : `${containers}$$`,
  );

/** One block, drawn again only when its text or how it is drawn changes. */
const MarkdownBlock = memo(function MarkdownBlock({
  text,
  remarkPlugins,
  rehypePlugins,
  components,
}: {
  text: string;
  remarkPlugins: PluggableList;
  rehypePlugins: PluggableList;
  components: Components;
}) {
  return (
    <Markdown
      remarkPlugins={remarkPlugins}
      rehypePlugins={rehypePlugins}
      urlTransform={webUrl}
      components={components}
    >
      {text}
    </Markdown>
  );
});

/** HTML stays escaped text (no rehype-raw), with no embedded resources or automatic navigation. */
export function SessionMarkdown({
  content,
  highlight = "",
  syntaxHighlight = false,
  math = false,
  previews = false,
  streaming = false,
}: {
  content: string;
  highlight?: string;
  /** Colours labelled code blocks; external session transcripts leave it off. */
  syntaxHighlight?: boolean;
  /**
   * Draws `$$…$$`, `\(…\)` and `\[…\]` formulas, block by block; external
   * session transcripts leave it off.
   */
  math?: boolean;
  /** Draws Mermaid diagrams and previews HTML blocks; external session transcripts leave it off. */
  previews?: boolean;
  /**
   * The text is still arriving: its last block, the one being written, shows
   * its formulas as their source and is the only block drawn again.
   */
  streaming?: boolean;
}) {
  const { t } = useTranslation();
  const syntax = useMarkdownSyntax(syntaxHighlight);
  const drawMath = useMarkdownMath(math && hasMathMarkers(content));
  const mathPlugins = drawMath ? markdownMathPlugins() : null;
  const source = useMemo(() => {
    const text = rewriteBareUrls(content);
    return mathPlugins ? rewriteMathDelimiters(text) : text;
  }, [mathPlugins, content]);
  const remarkPlugins = useMemo(
    () => [
      remarkGfm,
      // CommonMark will not close `**说明：**正文` (punctuation, then a
      // letter): these let bold and strikethrough end beside Chinese text.
      remarkCjkFriendly,
      remarkCjkFriendlyStrikethrough,
      ...(mathPlugins?.remark ?? []),
      splitBareUrlTail,
    ],
    [mathPlugins],
  );
  // Everything but the formulas, which the block being written shows as text.
  const textPlugins = useMemo(
    () => [
      ...(syntax ? [markdownSyntax] : []),
      ...(highlight
        ? [
            [highlightPlugin, { needle: highlight }] as [
              typeof highlightPlugin,
              { needle: string },
            ],
          ]
        : []),
    ],
    [syntax, highlight],
  );
  const plugins = useMemo(
    () => [...(mathPlugins?.rehype ?? []), ...textPlugins],
    [mathPlugins, textPlugins],
  );
  const blocks = useMemo(
    () =>
      math ? splitBlocks(source, remarkPlugins) : [{ start: 0, text: source }],
    [math, source, remarkPlugins],
  );
  const components = useMemo<Components>(
    () => ({
      img: ({ alt }) => (
        <span
          className="inline-flex rounded border border-line-2 px-2 py-1 text-note text-ink-muted"
          title={t("sessionExperience.imageHint")}
        >
          {t("sessionExperience.imageBlocked")}
          {alt ? ` · ${alt}` : ""}
        </span>
      ),
      a: ({ href, children }) =>
        href ? (
          <a
            href={href}
            className="text-brand underline underline-offset-2"
            onClick={(event) => {
              event.preventDefault();
              void settingsApi
                .openExternal(href)
                .catch(() => toast.error(t("common.error")));
            }}
          >
            {children}
          </a>
        ) : (
          <span>{children}</span>
        ),
      input: ({ checked }) => <span aria-hidden>{checked ? "☑" : "☐"}</span>,
      pre: ({ children }) =>
        codeLanguage(children) === "math" ? (
          // A display formula shown as its source while it is written.
          <pre className="my-2 overflow-x-auto whitespace-pre-wrap text-ink-muted">
            {children}
          </pre>
        ) : (
          <CodeBlock
            text={codeText(children).replace(/\n$/, "")}
            preview={previews ? codePreview(codeLanguage(children)) : null}
          >
            {children}
          </CodeBlock>
        ),
    }),
    [t, previews],
  );
  return (
    <div
      className={`${syntaxHighlight ? "markdown-syntax " : ""}break-words text-ui leading-relaxed text-ink-2 [&_p]:my-2 [&_h1]:mb-2 [&_h1]:mt-4 [&_h1]:text-lg [&_h1]:font-semibold [&_h2]:mb-2 [&_h2]:mt-4 [&_h2]:text-base [&_h2]:font-semibold [&_h3]:my-2 [&_h3]:font-semibold [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_blockquote]:border-l-2 [&_blockquote]:border-brand [&_blockquote]:pl-3 [&_blockquote]:text-ink-muted [&_table]:block [&_table]:overflow-x-auto [&_th]:border [&_th]:border-line [&_th]:px-2 [&_td]:border [&_td]:border-line [&_td]:px-2 [&_code]:rounded [&_code]:bg-pill [&_code]:px-1 [&_code]:font-mono [&_code]:text-[12px] [&_.katex-display]:overflow-x-auto [&_.katex-display]:overflow-y-hidden [&_.katex-error]:font-mono [&_.katex-error]:text-[12px]`}
    >
      {/* Blocks sit side by side with no wrapper, a line break between them
          as markdown puts between its blocks, so the page is the same as one
          text drawn whole. */}
      {blocks.map((block, index) => {
        const writing = streaming && index === blocks.length - 1;
        return (
          <Fragment key={block.start}>
            {index > 0 && "\n"}
            <MarkdownBlock
              text={writing ? asWritten(block.text) : block.text}
              remarkPlugins={remarkPlugins}
              rehypePlugins={writing ? textPlugins : plugins}
              components={components}
            />
          </Fragment>
        );
      })}
    </div>
  );
}
