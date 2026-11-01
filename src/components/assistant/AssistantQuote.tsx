import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";

/** Selection is read only from this reply's text, never reasoning or controls. */
export function AssistantQuote({
  children,
  onQuote,
}: {
  children: React.ReactNode;
  onQuote?: (text: string) => void;
}) {
  const { t } = useTranslation();
  const root = useRef<HTMLDivElement>(null);
  const [quote, setQuote] = useState<{
    text: string;
    left: number;
    top: number;
  } | null>(null);
  const update = () => {
    const selection = window.getSelection();
    const element = root.current;
    if (
      !onQuote ||
      !element ||
      !selection?.rangeCount ||
      selection.isCollapsed
    ) {
      setQuote(null);
      return;
    }
    const range = selection.getRangeAt(0);
    if (
      !element.contains(range.startContainer) ||
      !element.contains(range.endContainer)
    ) {
      setQuote(null);
      return;
    }
    const text = selection.toString().trim();
    if (!text || text.length > 128_000) {
      setQuote(null);
      return;
    }
    const rect = range.getBoundingClientRect();
    const parent = element.getBoundingClientRect();
    setQuote({
      text,
      left: Math.max(
        0,
        Math.min(rect.left - parent.left, element.clientWidth - 80),
      ),
      top: rect.bottom - parent.top + 4,
    });
  };
  useEffect(() => {
    if (!quote) return;
    const clear = () => setQuote(null);
    document.addEventListener("scroll", clear, true);
    const change = () => {
      const selection = window.getSelection();
      if (
        !selection?.rangeCount ||
        selection.isCollapsed ||
        !root.current?.contains(selection.anchorNode) ||
        !root.current?.contains(selection.focusNode)
      )
        clear();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        clear();
      }
    };
    document.addEventListener("selectionchange", change);
    document.addEventListener("keydown", key, true);
    return () => {
      document.removeEventListener("scroll", clear, true);
      document.removeEventListener("selectionchange", change);
      document.removeEventListener("keydown", key, true);
    };
  }, [Boolean(quote)]);
  useEffect(() => {
    if (!onQuote) setQuote(null);
  }, [onQuote]);
  if (!onQuote) return <>{children}</>;
  return (
    <div
      ref={root}
      className="relative min-w-0"
      onPointerUp={update}
      onKeyUp={(event) => {
        if (event.key === "Escape") setQuote(null);
        else update();
      }}
    >
      {children}
      {quote && onQuote && (
        <Button
          variant="outline"
          size="sm"
          className="absolute z-20 shadow-md"
          style={{ left: quote.left, top: quote.top }}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => {
            onQuote(quote.text);
            setQuote(null);
            window.getSelection()?.removeAllRanges();
          }}
        >
          {t("assistant.quote")}
        </Button>
      )}
    </div>
  );
}
