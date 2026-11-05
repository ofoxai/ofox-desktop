import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Code2, Eye, Workflow } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { drawDiagram, type DiagramImage } from "@/lib/markdownDiagram";
import { htmlHasScripts, htmlPreviewDocument } from "@/lib/htmlPreview";

/** What a code block of `language` can be shown as besides its code. */
export function codePreview(
  language: string | null | undefined,
): "diagram" | "html" | null {
  if (language === "mermaid") return "diagram";
  if (language === "html") return "html";
  return null;
}

/** Whether the app is in its dark theme, following it as it changes. */
function useDarkTheme(): boolean {
  const [dark, setDark] = useState(() =>
    document.documentElement.classList.contains("dark"),
  );
  useEffect(() => {
    const root = document.documentElement;
    const observer = new MutationObserver(() =>
      setDark(root.classList.contains("dark")),
    );
    observer.observe(root, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, []);
  return dark;
}

/**
 * The diagram of `source`, or null while it is drawn or when it cannot be.
 * A reply still being written changes it often: it is drawn once it has
 * been still for a moment.
 */
function useDiagram(source: string, enabled: boolean): DiagramImage | null {
  const dark = useDarkTheme();
  const [image, setImage] = useState<DiagramImage | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    const timer = window.setTimeout(() => {
      void drawDiagram(source, dark).then((next) => {
        if (live) setImage(next);
      });
    }, 250);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [source, dark, enabled]);
  return enabled ? image : null;
}

/**
 * A fenced code block with its copy button. With `preview`, a Mermaid block
 * is shown as its diagram (its code while it cannot be drawn), and an HTML
 * block can be previewed in a frame that loads nothing and runs no script.
 */
export function CodeBlock({
  text,
  preview,
  children,
}: {
  text: string;
  preview: "diagram" | "html" | null;
  /** The highlighted code. */
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const diagram = useDiagram(text, preview === "diagram");
  // A diagram starts drawn; an HTML page starts as its code.
  const [showCode, setShowCode] = useState(preview === "html");
  const diagramShown = preview === "diagram" && diagram !== null && !showCode;
  const htmlShown = preview === "html" && !showCode;
  const toggle =
    preview === "diagram" && diagram !== null ? (
      <Button
        variant="ghost"
        className="h-6 gap-1 px-2 text-note"
        aria-pressed={!showCode}
        onClick={() => setShowCode(!showCode)}
      >
        {showCode ? (
          <Workflow aria-hidden className="h-3.5 w-3.5" />
        ) : (
          <Code2 aria-hidden className="h-3.5 w-3.5" />
        )}
        {t(
          showCode
            ? "sessionExperience.showDiagram"
            : "sessionExperience.showSource",
        )}
      </Button>
    ) : preview === "html" ? (
      <Button
        variant="ghost"
        className="h-6 gap-1 px-2 text-note"
        aria-pressed={htmlShown}
        onClick={() => setShowCode(!showCode)}
      >
        {htmlShown ? (
          <Code2 aria-hidden className="h-3.5 w-3.5" />
        ) : (
          <Eye aria-hidden className="h-3.5 w-3.5" />
        )}
        {t(
          htmlShown
            ? "sessionExperience.showCode"
            : "sessionExperience.previewHtml",
        )}
      </Button>
    ) : null;
  return (
    <div className="my-3 overflow-hidden rounded-[9px] border border-line-2 bg-canvas">
      <div className="flex items-center justify-end gap-1 border-b border-line-2 px-2 py-1">
        {toggle}
        <CopyButton text={text} aria-label={t("sessionExperience.copyCode")} />
      </div>
      {diagramShown && diagram ? (
        <div className="flex max-w-full justify-center p-3">
          {/* An image: the diagram can load nothing and style nothing outside it. */}
          <img
            src={diagram.src}
            alt={t("sessionExperience.diagram")}
            draggable={false}
            className="h-auto w-full"
            style={diagram.width ? { maxWidth: diagram.width } : undefined}
          />
        </div>
      ) : htmlShown ? (
        <div className="space-y-1 p-2">
          <div className="h-80 min-h-40 resize-y overflow-hidden rounded-ctl border border-line-2">
            <iframe
              title={t("sessionExperience.previewFrame")}
              // No script, form, popup or navigation of the app; an opaque
              // origin; and the page's own policy loads nothing at all.
              sandbox=""
              referrerPolicy="no-referrer"
              srcDoc={htmlPreviewDocument(text)}
              className="h-full w-full bg-white"
            />
          </div>
          {htmlHasScripts(text) && (
            <p className="text-note text-ink-muted">
              {t("sessionExperience.previewNoScripts")}
            </p>
          )}
        </div>
      ) : (
        <pre className="max-w-full overflow-x-auto p-3 font-mono text-[12px] leading-relaxed [&_code]:bg-transparent [&_code]:p-0">
          {children}
        </pre>
      )}
    </div>
  );
}
