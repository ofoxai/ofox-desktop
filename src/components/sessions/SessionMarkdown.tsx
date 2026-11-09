import {
  Children,
  isValidElement,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import Markdown from "react-markdown";
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

/** HTML stays escaped text (no rehype-raw), with no embedded resources or automatic navigation. */
export function SessionMarkdown({
  content,
  highlight = "",
  syntaxHighlight = false,
  math = false,
  previews = false,
}: {
  content: string;
  highlight?: string;
  /** Colours labelled code blocks; external session transcripts leave it off. */
  syntaxHighlight?: boolean;
  /** Draws `$$…$$`, `\(…\)` and `\[…\]` formulas; external session transcripts leave it off. */
  math?: boolean;
  /** Draws Mermaid diagrams and previews HTML blocks; external session transcripts leave it off. */
  previews?: boolean;
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
  const plugins = useMemo(
    () => [
      ...(mathPlugins?.rehype ?? []),
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
    [mathPlugins, syntax, highlight],
  );
  return (
    <div
      className={`${syntaxHighlight ? "markdown-syntax " : ""}break-words text-ui leading-relaxed text-ink-2 [&_p]:my-2 [&_h1]:mb-2 [&_h1]:mt-4 [&_h1]:text-lg [&_h1]:font-semibold [&_h2]:mb-2 [&_h2]:mt-4 [&_h2]:text-base [&_h2]:font-semibold [&_h3]:my-2 [&_h3]:font-semibold [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_blockquote]:border-l-2 [&_blockquote]:border-brand [&_blockquote]:pl-3 [&_blockquote]:text-ink-muted [&_table]:block [&_table]:overflow-x-auto [&_th]:border [&_th]:border-line [&_th]:px-2 [&_td]:border [&_td]:border-line [&_td]:px-2 [&_code]:rounded [&_code]:bg-pill [&_code]:px-1 [&_code]:font-mono [&_code]:text-[12px] [&_.katex-display]:overflow-x-auto [&_.katex-display]:overflow-y-hidden [&_.katex-error]:font-mono [&_.katex-error]:text-[12px]`}
    >
      <Markdown
        remarkPlugins={remarkPlugins}
        rehypePlugins={plugins}
        urlTransform={(url) => {
          try {
            const parsed = new URL(url);
            return parsed.protocol === "http:" || parsed.protocol === "https:"
              ? parsed.href
              : "";
          } catch {
            return "";
          }
        }}
        components={{
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
          input: ({ checked }) => (
            <span aria-hidden>{checked ? "☑" : "☐"}</span>
          ),
          pre: ({ children }) => (
            <CodeBlock
              text={codeText(children).replace(/\n$/, "")}
              preview={previews ? codePreview(codeLanguage(children)) : null}
            >
              {children}
            </CodeBlock>
          ),
        }}
      >
        {source}
      </Markdown>
    </div>
  );
}
