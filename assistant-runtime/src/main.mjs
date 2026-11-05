import { createInterface } from "node:readline";
import { readFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { AssistantRuntime } from "./runtime.mjs";
import { LINE_LIMIT, oversizedReplyId } from "./ipc.mjs";
import { errorMessage, redact, scrubSecrets } from "./security.mjs";

const { version: sdkVersion } = JSON.parse(
  await readFile(
    new URL(
      "../node_modules/@earendil-works/pi-coding-agent/package.json",
      import.meta.url,
    ),
    "utf8",
  ),
);

const pending = new Map();
// Frames carry conversation content: only the exact run credentials go.
// Error text in them was already scrubbed as a diagnostic.
const write = (value) =>
  process.stdout.write(
    JSON.stringify(scrubSecrets(value, [...runtime.secrets])) + "\n",
  );
const runtime = new AssistantRuntime({
  emit: (event) => write({ method: "event", params: event }),
  hostCall: (params, signal) =>
    new Promise((resolve, reject) => {
      const id = `tool-${randomUUID()}`;
      const cancel = () => {
        pending.delete(id);
        reject(
          Object.assign(
            new Error(
              "Host operation cancelled; its outcome may need checking",
            ),
            { code: "unknown_outcome" },
          ),
        );
      };
      if (signal?.aborted) {
        cancel();
        return;
      }
      signal?.addEventListener("abort", cancel, { once: true });
      pending.set(id, {
        resolve: (result) => {
          signal?.removeEventListener("abort", cancel);
          resolve(result);
        },
        reject: (error) => {
          signal?.removeEventListener("abort", cancel);
          reject(error);
        },
      });
      write({ id, method: "tool", params });
    }),
});

// stdout is exclusively the private JSON-RPC pipe. Third-party libraries may use
// console; route diagnostics through source redaction instead of corrupting IPC.
for (const method of ["log", "info", "debug", "warn", "error"])
  console[method] = (...values) => {
    const safe = values
      .map((value) =>
        value instanceof Error
          ? errorMessage(value, [...runtime.secrets])
          : typeof value === "string"
            ? redact(value, [...runtime.secrets])
            : JSON.stringify(redact(value, [...runtime.secrets])),
      )
      .join(" ");
    process.stderr.write(`${safe.slice(0, 4000)}\n`);
  };

const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on("line", (line) => {
  void handle(line);
});
let terminating;
function terminate() {
  return (terminating ??= (async () => {
    const closing = runtime.close().catch(() => false);
    for (const request of pending.values())
      request.reject(new Error("Host disconnected"));
    pending.clear();
    const confirmed = await closing;
    if (confirmed) {
      process.exit(0);
    } else {
      // Do not run the SDK's process-exit hook with live cached process-group
      // IDs. Only this live process is targeted; launcher EOF / the Host's
      // owned JobObject performs descendant teardown.
      process.stderr.write(
        "MCP cleanup incomplete; terminating runtime safely\n",
      );
      try {
        process.kill(process.pid, "SIGKILL");
      } catch {
        // If the OS refuses even self-termination, remain alive for the Host's
        // owned process/job teardown rather than run unsafe exit hooks.
        process.stderr.write("Runtime awaiting owned Host termination\n");
        setInterval(() => {}, 1000);
      }
    }
  })());
}
input.on("close", () => {
  void terminate();
});
process.on("SIGTERM", () => {
  void terminate();
});
process.on("SIGINT", () => {
  void terminate();
});

async function handle(line) {
  let request;
  try {
    if (Buffer.byteLength(line) > LINE_LIMIT) {
      const id = oversizedReplyId(line);
      const call = id && pending.get(id);
      if (call) {
        pending.delete(id);
        call.reject(
          Object.assign(new Error("assistant_result_too_large"), {
            code: "assistant_result_too_large",
          }),
        );
        return;
      }
      throw new Error("IPC request exceeds 1 MiB");
    }
    request = JSON.parse(line);
    if (!request || typeof request !== "object")
      throw new Error("Invalid JSON-RPC request");
    if (!request.method && pending.has(request.id)) {
      const call = pending.get(request.id);
      pending.delete(request.id);
      if (request.error)
        call.reject(
          Object.assign(
            new Error(
              errorMessage(request.error.message ?? request.error, [
                ...runtime.secrets,
              ]),
            ),
            {
              code: request.error.code,
              outcomeUnknown: request.error.data?.outcomeUnknown === true,
            },
          ),
        );
      else call.resolve(request.result);
      return;
    }
    let result;
    if (request.method === "health")
      result = { protocolVersion: 1, sdkVersion };
    else if (request.method === "load")
      result = await runtime.load(request.params ?? {});
    else if (request.method === "run")
      result = await runtime.run(request.params ?? {});
    else if (request.method === "compact")
      result = await runtime.run(request.params ?? {}, true);
    else if (request.method === "title")
      result = await runtime.title(request.params ?? {});
    else if (request.method === "navigate")
      result = await runtime.navigate(request.params ?? {});
    else if (request.method === "append_media")
      result = await runtime.appendMedia(request.params ?? {});
    else if (request.method === "fork")
      result = await runtime.fork(request.params ?? {});
    else if (request.method === "steer")
      result = await runtime.steer(request.params ?? {});
    else if (request.method === "stop")
      result = await runtime.stop(request.params ?? {});
    else throw new Error("Unknown assistant runtime method");
    if (request.id !== undefined) write({ id: request.id, result });
  } catch (error) {
    write({
      id: request?.id ?? null,
      error: {
        code: "runtime_error",
        message:
          error instanceof SyntaxError
            ? "Invalid JSON-RPC JSON"
            : errorMessage(error, [...runtime.secrets]),
      },
    });
  }
}
