import { useEffect, useRef } from "react";

const dayKey = (now: number) => {
  const date = new Date(now);
  return `${date.getFullYear()}/${date.getMonth()}/${date.getDate()}/${date.getTimezoneOffset()}`;
};

/** Calendar boundaries are independent of the collection interval and network. */
export function useUsageCalendar(
  anchor: number,
  enabled: boolean,
  rollover: (now: number) => void | boolean | Promise<void | boolean>,
) {
  const seen = useRef(dayKey(anchor));
  const committed = useRef(seen.current);
  const callback = useRef(rollover);
  callback.current = rollover;
  useEffect(() => {
    seen.current = dayKey(anchor);
    committed.current = seen.current;
  }, [anchor]);
  useEffect(() => {
    if (!enabled) return;
    // A read owned by a prior visible lifetime may still be pending or have
    // been discarded. Only the published anchor can consume its boundary.
    seen.current = committed.current;
    let timer: number;
    let disposed = false;
    const check = () => {
      const now = Date.now();
      const key = dayKey(now);
      if (!document.hidden && key !== seen.current) {
        seen.current = key;
        const complete = (accepted: void | boolean) => {
          if (
            !disposed &&
            accepted === false &&
            seen.current === key &&
            committed.current !== key
          )
            seen.current = committed.current;
        };
        try {
          void Promise.resolve(callback.current(now)).then(complete, () =>
            complete(false),
          );
        } catch {
          complete(false);
        }
      }
    };
    const schedule = () => {
      check();
      const now = Date.now();
      const next = new Date(now);
      next.setHours(24, 0, 0, 0);
      // Also notice clock/timezone changes while the page stays in front.
      timer = window.setTimeout(
        schedule,
        Math.min(60_000, Math.max(1, next.getTime() - now)),
      );
    };
    schedule();
    window.addEventListener("focus", check);
    document.addEventListener("visibilitychange", check);
    return () => {
      disposed = true;
      window.clearTimeout(timer);
      window.removeEventListener("focus", check);
      document.removeEventListener("visibilitychange", check);
    };
  }, [enabled]);
}
