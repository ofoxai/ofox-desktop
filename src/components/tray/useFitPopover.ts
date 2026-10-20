import { useEffect, type RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * The popover window follows its card's natural height, as magpie's panel
 * does: whatever surrounds the list plus the list at its full size. While it
 * is open it only grows, so switching tabs or folding a row never makes it
 * jump; opened again, it starts from what it shows then. Rust clamps the
 * height and keeps the edge by the tray icon in place (`fit_tray_popover`).
 */
export function useFitPopover(
  card: RefObject<HTMLElement>,
  view: RefObject<HTMLElement>,
  content: RefObject<HTMLElement>,
) {
  useEffect(() => {
    const cardEl = card.current;
    const viewEl = view.current;
    const contentEl = content.current;
    if (!cardEl || !viewEl || !contentEl) return;
    let tallest = 0;
    let sent = 0;
    let frame = 0;
    const measure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const style = getComputedStyle(viewEl);
        const padding =
          (parseFloat(style.paddingTop) || 0) +
          (parseFloat(style.paddingBottom) || 0);
        const wanted = Math.ceil(
          cardEl.offsetHeight -
            viewEl.clientHeight +
            contentEl.offsetHeight +
            padding,
        );
        tallest = Math.max(tallest, wanted);
        if (!tallest || tallest === sent) return;
        sent = tallest;
        void invoke("fit_tray_popover", { height: tallest }).catch((e) =>
          console.error("[tray] fit_tray_popover failed", e),
        );
      });
    };
    const observer = new ResizeObserver(measure);
    observer.observe(cardEl);
    observer.observe(contentEl);
    const reopened = () => {
      tallest = 0;
      measure();
    };
    window.addEventListener("focus", reopened);
    measure();
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
      window.removeEventListener("focus", reopened);
    };
  }, [card, view, content]);
}
