import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";
import { AssistantRuntime } from "./runtime.mjs";
import { errorMessage, redact } from "./security.mjs";

const pending = new Map();
const write = (value) =>
  process.stdout.write(
    JSON.stringify(redact(value, [...runtime.secrets])) + "\n",
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
input.on("close", () => {
  void runtime.close().finally(() => {
    for (const request of pending.values())
      request.reject(new Error("Host disconnected"));
    pending.clear();
  });
});
process.on("SIGTERM", () => {
  void runtime.close().finally(() => process.exit(0));
});
process.on("SIGINT", () => {
  void runtime.close().finally(() => process.exit(0));
});

async function handle(line) {
  let request;
  try {
    if (Buffer.byteLength(line) > 1024 * 1024)
      throw new Error("IPC request exceeds 1 MiB");
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
      result = { protocolVersion: 1, sdkVersion: "1.4.0" };
    else if (request.method === "load")
      result = await runtime.load(request.params ?? {});
    else if (request.method === "run")
      result = await runtime.run(request.params ?? {});
    else if (request.method === "compact")
      result = await runtime.run(request.params ?? {}, true);
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
