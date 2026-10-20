import { useCallback, useSyncExternalStore, type SetStateAction } from "react";

/** State belongs to the retained page; suspended portals may unmount freely. */
export class DraftStore {
  values = new Map<string, unknown>();
  listeners = new Set<() => void>();
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
}
export function useDraftState<T>(
  store: DraftStore,
  key: string,
  initial: T | (() => T),
): [T, (value: SetStateAction<T>) => void] {
  if (!store.values.has(key))
    store.values.set(
      key,
      typeof initial === "function" ? (initial as () => T)() : initial,
    );
  const value = useSyncExternalStore(
    store.subscribe,
    () => store.values.get(key) as T,
  );
  const setValue = useCallback(
    (next: SetStateAction<T>) => {
      const previous = store.values.get(key) as T;
      const resolved =
        typeof next === "function"
          ? (next as (before: T) => T)(previous)
          : next;
      if (Object.is(previous, resolved)) return;
      store.values.set(key, resolved);
      store.listeners.forEach((listener) => listener());
    },
    [store, key],
  );
  return [value, setValue];
}
