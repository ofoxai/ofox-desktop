// Test-only observation of public process/transport APIs, never SDK internals.
import { appendFileSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { isProcessRunning } from "./process-state.mjs";
const record = (value) =>
  appendFileSync(process.env.MCP_EXIT_TRACE, JSON.stringify(value) + "\n");
const originalKill = process.kill.bind(process),
  originalExit = process.exit.bind(process);
process.kill = (pid, signal) => {
  if (pid < 0) record({ kind: "cached_group_signal", pid, signal });
  if (pid === process.pid && signal === "SIGKILL") {
    record({ kind: "self_kill", pid });
    if (process.env.MCP_REJECT_SELF === "true")
      throw Object.assign(new Error("Injected OS refusal"), { code: "EPERM" });
  }
  return originalKill(pid, signal);
};
process.exit = (code) => {
  let childAlive = false;
  try {
    const pid = Number(readFileSync(process.env.MCP_EXIT_CHILD_FILE, "utf8"));
    childAlive = isProcessRunning(pid);
  } catch {}
  record({ kind: "ordinary_exit", code, childAlive });
  return originalExit(code);
};
if (process.env.MCP_DROP_STOP === "true") {
  const { StdioTransport } = await import(
    import.meta.resolve(
      "@earendil-works/pi-mcp",
      pathToFileURL(process.env.MCP_MAIN_ENTRY).href,
    )
  );
  const send = StdioTransport.prototype.send;
  StdioTransport.prototype.send = function (message, options) {
    if (
      message.method === "ofox/private_control" &&
      message.params?.action === "stop"
    ) {
      record({ kind: "injected_stop_failure" });
      return Promise.resolve();
    }
    return send.call(this, message, options);
  };
}
