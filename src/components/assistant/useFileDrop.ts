import { useEffect, useRef, useState } from "react";
import { assistantApi } from "@/lib/api/assistant";

/**
 * Whether a modal dialog is open over the page: one that says so (aria-modal),
 * or a Radix layer that keeps the page from taking pointer events, as its
 * modal dialogs do. Read when a drag or a drop arrives: a dialog can open
 * without the page drawing again.
 */
function underModalDialog() {
  return (
    document.querySelector('[aria-modal="true"]') !== null ||
    document.body.style.pointerEvents === "none"
  );
}

/**
 * Files dragged over the window, from the backend's drag and drop events.
 * `dragging` is true from the drag entering until it leaves or drops, but
 * only while `ready` (this page is the one on screen and can take files) and
 * with no modal dialog open: it drives the overlay. A drop is handed to
 * `onDrop` only then too; otherwise its token is left to expire. The
 * listeners are made once and read the latest `ready` and `onDrop` through a
 * ref.
 */
export function useFileDrop({
  ready,
  onDrop,
  onError,
}: {
  ready: boolean;
  /** Imports the files of this drop; `token` is theirs, never a path. */
  onDrop: (token: string) => void;
  onError: (reason: unknown) => void;
}) {
  const [dragging, setDragging] = useState(false);
  const latest = useRef({ ready, onDrop, onError });
  latest.current = { ready, onDrop, onError };

  useEffect(() => {
    let disposed = false;
    const cleanup: (() => void)[] = [];
    const keep = (listening: Promise<() => void>) =>
      listening
        .then((off) => (disposed ? off() : cleanup.push(off)))
        .catch((reason: unknown) => latest.current.onError(reason));
    void keep(
      assistantApi.onDrag((state) =>
        setDragging(state === "enter" && !underModalDialog()),
      ),
    );
    void keep(
      assistantApi.onDrop(({ token }) => {
        // A drop ends the drag, and no leave follows it.
        setDragging(false);
        if (latest.current.ready && !underModalDialog())
          latest.current.onDrop(token);
      }),
    );
    return () => {
      disposed = true;
      cleanup.forEach((off) => off());
    };
  }, []);

  return { dragging: dragging && ready };
}
