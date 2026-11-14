// A trusted process-group leader. The external service never receives our
// control nonce, and never owns the launcher lifetime. No cached PID is killed.
import { spawn, execFile } from "node:child_process";
import crossSpawn from "cross-spawn";

const nonce = process.env.OFOX_MCP_CONTROL;
delete process.env.OFOX_MCP_CONTROL;
if (!nonce || nonce.length < 32) process.exit(1);
const unix = process.platform !== "win32";
if (unix) {
  const ownGroup = await new Promise((resolve) =>
    execFile(
      "/bin/ps",
      ["-o", "pgid=", "-p", String(process.pid)],
      { timeout: 1000 },
      (error, stdout) =>
        resolve(!error && Number(stdout.trim()) === process.pid),
    ),
  );
  if (!ownGroup) process.exit(1);
}
let child;
let stopping = false;
let started = false;
let buffer = Buffer.alloc(0);
const MAX_INPUT = 8 * 1024 * 1024;
function shutdown() {
  if (stopping) return;
  stopping = true;
  child?.stdin?.end();
  if (unix) {
    // Remain the live leader during both signals. kill(0) addresses this
    // process's current group, never a numeric PID saved before an exit.
    setTimeout(() => process.kill(0, "SIGTERM"), 30);
    setTimeout(() => process.kill(0, "SIGKILL"), 150);
  } else {
    // The Host's JobObject owns the entire Windows run tree. The direct child
    // is an owned ChildProcess handle; descendants are reaped at run teardown.
    setTimeout(() => child?.kill(), 30);
    setTimeout(() => process.exit(0), 150);
  }
}
process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);
process.on("uncaughtException", shutdown);
process.on("unhandledRejection", shutdown);
process.stdin.on("end", shutdown);
process.stdin.on("error", shutdown);
process.stdout.on("error", shutdown);
process.stderr.on("error", shutdown);
process.stdin.on("data", (chunk) => {
  buffer = Buffer.concat([buffer, chunk]);
  if (buffer.length > MAX_INPUT) return shutdown();
  let newline;
  while ((newline = buffer.indexOf(10)) >= 0) {
    const line = buffer.subarray(0, newline);
    buffer = buffer.subarray(newline + 1);
    let frame;
    try {
      frame = JSON.parse(line.toString("utf8"));
    } catch {
      return shutdown();
    }
    if (!frame || typeof frame !== "object" || Array.isArray(frame))
      return shutdown();
    if (frame.method === "ofox/private_control") {
      if (frame.params?.nonce !== nonce) return shutdown();
      if (frame.params.action === "stop") return shutdown();
      if (frame.params.action !== "start" || started) return shutdown();
      const { command, args, cwd } = frame.params;
      if (
        typeof command !== "string" ||
        !Array.isArray(args) ||
        args.some((arg) => typeof arg !== "string")
      )
        return shutdown();
      started = true;
      // The runtime's pinned cross-spawn dependency supplies Windows .cmd
      // quoting without invoking a constructed shell command.
      const spawnService = unix ? spawn : crossSpawn;
      child = spawnService(command, args, {
        cwd,
        env: process.env,
        detached: false,
        windowsHide: true,
        stdio: ["pipe", "pipe", "pipe"],
      });
      child.stdout.pipe(process.stdout);
      child.stderr.pipe(process.stderr);
      child.on("error", shutdown);
      child.on("exit", shutdown);
      child.stdin.on("error", shutdown);
      continue;
    }
    if (!started || stopping) continue;
    // Backpressure from a stuck server cannot prevent the independent control
    // parser from processing stop. Bound queued writes before accepting more.
    if (child.stdin.writableLength + line.length > MAX_INPUT) return shutdown();
    child.stdin.write(Buffer.concat([line, Buffer.from("\n")]));
  }
});
