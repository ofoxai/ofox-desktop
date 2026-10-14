import { useRef } from "react";
import type { KeyboardEvent } from "react";

/** Dismiss a menu and continue in the tab order surrounding its trigger. */
export function useMenuTab(close: () => void) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const tabTarget = useRef<HTMLElement | null>(null);
  return {
    triggerRef,
    onKeyDown: (event: KeyboardEvent) => {
      if (event.key !== "Tab" || !triggerRef.current) return;
      event.preventDefault();
      const trigger = triggerRef.current;
      const root = trigger.closest('[role="dialog"]') ?? document;
      const candidates = Array.from(
        root.querySelectorAll<HTMLElement>(
          "button, input, select, textarea, a[href], [tabindex]",
        ),
      ).filter((node) => {
        const style = getComputedStyle(node);
        return (
          node.tabIndex >= 0 &&
          !node.matches(":disabled, [data-radix-focus-guard]") &&
          !node.closest('[hidden], [aria-hidden="true"]') &&
          !event.currentTarget.contains(node) &&
          style.display !== "none" &&
          style.visibility !== "hidden"
        );
      });
      const index = candidates.indexOf(trigger);
      tabTarget.current =
        candidates[index + (event.shiftKey ? -1 : 1)] ?? trigger;
      close();
    },
    onCloseAutoFocus: (event: Event) => {
      if (tabTarget.current) {
        event.preventDefault();
        tabTarget.current.focus();
        tabTarget.current = null;
      }
    },
  };
}
