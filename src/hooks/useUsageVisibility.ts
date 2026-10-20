import { useEffect, useState } from "react";

/** Revalidate at the work boundary as native minimize events can arrive late. */
export async function canRefreshUsageNow(): Promise<boolean> {
  if (document.hidden) return false;
  if (!("__TAURI_INTERNALS__" in window)) return true;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const current = getCurrentWindow();
    const [shown, minimized] = await Promise.all([
      current.isVisible(),
      current.isMinimized(),
    ]);
    return !document.hidden && shown && !minimized;
  } catch {
    return false;
  }
}

/** Native visibility is checked as well: a hidden/minimized Tauri window may
 * leave document.visibilityState unchanged on some desktop WebViews. */
export function useUsageVisibility(active: boolean) {
  const [visible, setVisible] = useState(
    () => !("__TAURI_INTERNALS__" in window) && !document.hidden,
  );
  useEffect(() => {
    let disposed = false;
    let revision = 0;
    let nativeWindow:
      | Awaited<
          ReturnType<
            (typeof import("@tauri-apps/api/window"))["getCurrentWindow"]
          >
        >
      | undefined;
    const stops: (() => void)[] = [];
    const check = async () => {
      const current = ++revision;
      let next =
        !document.hidden &&
        (!("__TAURI_INTERNALS__" in window) || !!nativeWindow);
      try {
        if (nativeWindow) {
          const [shown, minimized] = await Promise.all([
            nativeWindow.isVisible(),
            nativeWindow.isMinimized(),
          ]);
          next &&= shown && !minimized;
        }
      } catch {
        next = false;
      }
      if (!disposed && current === revision) setVisible(next);
    };
    document.addEventListener("visibilitychange", check);
    if ("__TAURI_INTERNALS__" in window) {
      void import("@tauri-apps/api/window")
        .then(async ({ getCurrentWindow }) => {
          if (disposed) return;
          nativeWindow = getCurrentWindow();
          const listeners = await Promise.all([
            nativeWindow.onFocusChanged(check),
            nativeWindow.onResized(check),
          ]);
          if (disposed) listeners.forEach((off) => off());
          else stops.push(...listeners);
          await check();
        })
        .catch(() => {
          if (!disposed) setVisible(false);
        });
    } else void check();
    return () => {
      disposed = true;
      revision++;
      document.removeEventListener("visibilitychange", check);
      stops.forEach((off) => off());
    };
  }, []);
  return active && visible;
}
