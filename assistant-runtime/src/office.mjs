import { Worker } from "node:worker_threads";

export class OfficeError extends Error {
  constructor(reason) {
    super(reason);
    this.name = "OfficeError";
    this.reason = reason;
  }
}

// Parsing untrusted XML must not monopolize the assistant's event loop or
// heap. Cancellation terminates this worker; it never has a Key or host paths.
export function readOffice(bytes, extension, signal) {
  signal?.throwIfAborted();
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL("./office-worker.mjs", import.meta.url), {
      workerData: { bytes, extension },
      env: {},
      execArgv: [],
      resourceLimits: { maxOldGenerationSizeMb: 128, stackSizeMb: 4 },
    });
    let done = false;
    const finish = (error, result) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
      void worker.terminate();
      if (error) reject(error);
      else resolve(result);
    };
    const abort = () => finish(signal.reason);
    const timer = setTimeout(
      () => finish(new OfficeError("office_expansion")),
      10000,
    );
    signal?.addEventListener("abort", abort, { once: true });
    worker.once("message", ({ result, reason }) =>
      finish(reason ? new OfficeError(reason) : null, result),
    );
    worker.once("error", (error) =>
      finish(
        new OfficeError(
          error.code === "ERR_WORKER_OUT_OF_MEMORY"
            ? "office_expansion"
            : "invalid_office",
        ),
      ),
    );
    worker.once("exit", () => finish(new OfficeError("invalid_office")));
    if (signal?.aborted) abort();
  });
}
