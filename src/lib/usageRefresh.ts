/** One owner for local sync; automatic ticks never queue more source work. */
export function createUsageRefreshCoordinator(run: () => Promise<unknown>) {
  let active: Promise<void> | undefined;
  let queued = false;
  let disposed = false;
  let enabled = true;
  return {
    request(origin: "manual" | "automatic"): Promise<void> {
      if (disposed || !enabled) return Promise.resolve();
      if (active) {
        if (origin === "manual") queued = true;
        return active;
      }
      const next = async () => {
        let failure: unknown;
        do {
          queued = false;
          try {
            await run();
          } catch (error) {
            failure ??= error;
          }
        } while (queued && enabled && !disposed);
        if (failure !== undefined) throw failure;
      };
      active = next().finally(() => {
        active = undefined;
      });
      return active;
    },
    setEnabled(next: boolean) {
      enabled = next;
      if (!next) queued = false;
    },
    dispose() {
      disposed = true;
      queued = false;
    },
  };
}
