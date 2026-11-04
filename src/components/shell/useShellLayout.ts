import { useEffect, useState } from "react";

/**
 * How the window is laid out: the top bar ("narrow"), or the left rail and
 * roomier pages ("wide"), as on a laptop's full screen or a large window.
 */
export type ShellLayout = "narrow" | "wide";

/** Wide from this window width. */
export const WIDE_FROM = 1280;
/** Narrow again below this one; in between the layout stays as it was. */
export const NARROW_BELOW = 1240;

function next(previous: ShellLayout | null, width: number): ShellLayout {
  if (width >= WIDE_FROM) return "wide";
  if (width < NARROW_BELOW) return "narrow";
  return previous ?? "narrow";
}

/** The window's layout, following its width. */
export function useShellLayout(): ShellLayout {
  const [layout, setLayout] = useState<ShellLayout>(() =>
    next(null, window.innerWidth),
  );
  useEffect(() => {
    const follow = () => setLayout((old) => next(old, window.innerWidth));
    follow();
    window.addEventListener("resize", follow);
    return () => window.removeEventListener("resize", follow);
  }, []);
  return layout;
}
