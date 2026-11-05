import { useEffect, useRef, useState } from "react";

/**
 * Whether a loading indicator should be on screen: only once the load has
 * run for `delayMs` (a quick one shows nothing), and then for at least
 * `minMs` (it never vanishes the moment it appears).
 */
export function useDelayedVisible(
  active: boolean,
  delayMs = 200,
  minMs = 600,
): boolean {
  const [visible, setVisible] = useState(false);
  const shownAt = useRef(0);
  useEffect(() => {
    if (active === visible) return;
    const wait = active
      ? delayMs
      : Math.max(0, minMs - (Date.now() - shownAt.current));
    const timer = window.setTimeout(() => {
      if (active) shownAt.current = Date.now();
      setVisible(active);
    }, wait);
    return () => window.clearTimeout(timer);
  }, [active, visible, delayMs, minMs]);
  return visible;
}
