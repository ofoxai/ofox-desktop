import { useCallback, useEffect, useRef, type RefObject } from "react";

/**
 * Focus an element now, or once the render that shows it has happened: a
 * view that is switching may not have mounted it yet.
 */
export function usePendingFocus() {
  const pending = useRef<RefObject<HTMLElement | null> | null>(null);
  const flush = useCallback(() => {
    const element = pending.current?.current;
    if (!element) return;
    pending.current = null;
    element.focus();
  }, []);
  useEffect(flush);
  return useCallback(
    (target: RefObject<HTMLElement | null>) => {
      pending.current = target;
      flush();
    },
    [flush],
  );
}
