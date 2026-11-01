import { useEffect, useState } from "react";
import { startOfLocalDay } from "@/utils/assistantSessionGroups";

/**
 * Local midnight of the current day, which changes by itself while the window
 * stays open: at the next midnight, and whenever the window comes back after
 * the computer slept (a timer that ran while it slept may be late).
 */
export function useStartOfToday(): number {
  const [today, setToday] = useState(() => startOfLocalDay(Date.now()));
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const sync = () => {
      clearTimeout(timer);
      const now = Date.now();
      setToday(startOfLocalDay(now));
      timer = setTimeout(sync, startOfLocalDay(now, 1) - now + 10);
    };
    sync();
    window.addEventListener("focus", sync);
    document.addEventListener("visibilitychange", sync);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("focus", sync);
      document.removeEventListener("visibilitychange", sync);
    };
  }, []);
  return today;
}
