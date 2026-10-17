import { useCallback, useEffect, useRef, useState } from "react";
import {
  libraryApi,
  type LibraryInventory,
  type LibraryKind,
} from "@/lib/api/library";

/** Own the scan ticket for as long as its observations are displayed. */
export function useLibraryInventory(
  kind: LibraryKind,
  epoch: number | undefined,
  revision: string,
  active: boolean,
) {
  const [snapshot, setSnapshot] = useState<{
    key: string;
    inventory: LibraryInventory;
  }>();
  const [error, setError] = useState<unknown>();
  const [loading, setLoading] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [expired, setExpired] = useState(false);
  const alive = useRef(false);
  const generation = useRef(0);
  const current = useRef<{ key: string; ticket?: string; pending: boolean }>();
  const key = `${kind}:${epoch}:${revision}:${refresh}`;
  const discard = (ticket: string) =>
    void libraryApi.discardInventory(ticket).catch(() => undefined);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      // Delay cleanup through StrictMode's synchronous effect replay.
      queueMicrotask(() => {
        if (alive.current) return;
        generation.current++;
        if (current.current?.ticket) discard(current.current.ticket);
        current.current = undefined;
      });
    };
  }, []);
  useEffect(() => {
    if (
      !active ||
      epoch === undefined ||
      epoch < 0 ||
      current.current?.key === key
    )
      return;
    const request = ++generation.current;
    if (current.current?.ticket) discard(current.current.ticket);
    current.current = { key, pending: true };
    setSnapshot(undefined);
    setError(undefined);
    setExpired(false);
    setLoading(true);
    void libraryApi
      .inventory(kind)
      .then((inventory) => {
        if (!alive.current || request !== generation.current) {
          discard(inventory.ticketId);
          return;
        }
        current.current = { key, ticket: inventory.ticketId, pending: false };
        setSnapshot({ key, inventory });
      })
      .catch((reason) => {
        if (alive.current && request === generation.current) setError(reason);
      })
      .finally(() => {
        if (alive.current && request === generation.current) setLoading(false);
      });
  }, [active, epoch, key, kind]);
  const valid =
    snapshot?.key === key && !expired ? snapshot.inventory : undefined;
  useEffect(() => {
    if (!valid) return;
    const timer = setTimeout(
      () => {
        setExpired(true);
        if (current.current?.ticket === valid.ticketId) {
          discard(valid.ticketId);
          current.current = undefined;
        }
      },
      Math.max(0, valid.expiresAtMs - Date.now()),
    );
    return () => clearTimeout(timer);
  }, [valid]);
  const rescan = useCallback(() => setRefresh((value) => value + 1), []);
  return { inventory: valid, error, loading, expired, rescan };
}
