import { fork } from "node:child_process";
import { fileURLToPath } from "node:url";

/** The document's own outcome: invalid_office, encrypted_office, empty_office or office_expansion. */
export class OfficeError extends Error {
  constructor(reason) {
    super(reason);
    this.name = "OfficeError";
    this.reason = reason;
  }
}

const CHILD = fileURLToPath(new URL("./office-child.mjs", import.meta.url));
// The whole read, start-up included. The child stops reading on its own at
// three quarters of it and returns what it has read (office-core.mjs).
const TIMEOUT = 10000;

/**
 * Reads the text of DOCX, XLSX or PPTX bytes in a child process of the same
 * Node (process.execPath: the bundled node or node.exe in the application).
 * Untrusted XML then can neither block the sidecar's event loop nor grow its
 * heap: the child's old space is capped at 128 MiB (a heap limit of about
 * 176 MiB) whatever heap the sidecar was started with. It gets no path and
 * an empty environment, to which the OS adds only what every process gets
 * (on Windows libuv copies SYSTEMROOT, TEMP and the like), and it is killed
 * on abort, on timeout and once it has answered. Its stdio is ignored so nothing of the document can reach the
 * sidecar's JSONL stdout.
 *
 * Resolves to {text, format, structureCoverage, truncated, warning?,
 * heapLimit}. Rejects with an OfficeError for the document's outcome (a
 * child that dies or times out after it started read too much:
 * office_expansion), with the signal's reason on abort, and with a plain
 * Error when the child cannot start or never reports ready, which is not
 * the document's fault. `timeout`, `childModule` and `onChild` exist for
 * tests.
 */
export function readOffice(
  bytes,
  extension,
  signal,
  { timeout = TIMEOUT, childModule = CHILD, onChild } = {},
) {
  signal?.throwIfAborted();
  return new Promise((resolve, reject) => {
    const child = fork(childModule, [], {
      execArgv: ["--max-old-space-size=128"],
      env: {},
      serialization: "advanced",
      stdio: ["ignore", "ignore", "ignore", "ipc"],
      windowsHide: true,
    });
    onChild?.(child);
    const forked = performance.now();
    let started = false;
    let done = false;
    const finish = (error, result) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
      child.kill("SIGKILL");
      if (error) reject(error);
      else resolve(result);
    };
    const abort = () => finish(signal.reason);
    // Before it reports ready the child has read nothing of the document:
    // ending then (or never starting) is the runtime's failure.
    const stopped = () =>
      finish(
        started
          ? new OfficeError("office_expansion")
          : new Error("The Office reader could not start"),
      );
    const timer = setTimeout(stopped, timeout);
    signal?.addEventListener("abort", abort, { once: true });
    child.on("error", stopped);
    // The channel closes after the child's last message has arrived, unlike
    // "exit", which can come first.
    child.on("disconnect", stopped);
    child.on("message", (message) => {
      if (message?.ready && !started) {
        started = true;
        // Time left until three quarters of the timeout: the rest is for
        // sending the text back.
        const deadline = timeout * 0.75 - (performance.now() - forked);
        child.send({ bytes, extension, deadline });
      } else if (message?.result)
        finish(null, {
          ...message.result,
          text: message.result.text.join(""),
        });
      else if (message?.reason) finish(new OfficeError(message.reason));
    });
  });
}
