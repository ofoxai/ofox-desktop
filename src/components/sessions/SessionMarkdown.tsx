import { Children, isValidElement, useMemo, type ReactNode } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useTranslation } from "react-i18next";
import { settingsApi } from "@/lib/api/settings";
import { CopyButton } from "@/components/ui/copy-button";
import { toast } from "sonner";

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

interface MarkdownNode {
  type: string;
  value?: string;
  children?: MarkdownNode[];
  tagName?: string;
  properties?: Record<string, unknown>;
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
          visit(child);
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

/** HTML stays escaped text (no rehype-raw), with no embedded resources or automatic navigation. */
export function SessionMarkdown({
  content,
  highlight = "",
}: {
  content: string;
  highlight?: string;
}) {
  const { t } = useTranslation();
  const highlights = useMemo(
    () =>
      highlight
        ? [
            [highlightPlugin, { needle: highlight }] as [
              typeof highlightPlugin,
              { needle: string },
            ],
          ]
        : [],
    [highlight],
  );
  return (
    <div className="break-words text-ui leading-relaxed text-ink-2 [&_p]:my-2 [&_h1]:mb-2 [&_h1]:mt-4 [&_h1]:text-lg [&_h1]:font-semibold [&_h2]:mb-2 [&_h2]:mt-4 [&_h2]:text-base [&_h2]:font-semibold [&_h3]:my-2 [&_h3]:font-semibold [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_blockquote]:border-l-2 [&_blockquote]:border-brand [&_blockquote]:pl-3 [&_blockquote]:text-ink-muted [&_table]:block [&_table]:overflow-x-auto [&_th]:border [&_th]:border-line [&_th]:px-2 [&_td]:border [&_td]:border-line [&_td]:px-2 [&_code]:rounded [&_code]:bg-pill [&_code]:px-1 [&_code]:font-mono [&_code]:text-[12px]">
      <Markdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={highlights}
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
            <div className="my-3 overflow-hidden rounded-[9px] border border-line-2 bg-canvas">
              <div className="flex justify-end border-b border-line-2 px-2 py-1">
                <CopyButton
                  text={codeText(children).replace(/\n$/, "")}
                  aria-label={t("sessionExperience.copyCode")}
                />
              </div>
              <pre className="max-w-full overflow-x-auto p-3 font-mono text-[12px] leading-relaxed [&_code]:bg-transparent [&_code]:p-0">
                {children}
              </pre>
            </div>
          ),
        }}
      >
        {content}
      </Markdown>
    </div>
  );
}
