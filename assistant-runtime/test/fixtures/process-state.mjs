import { spawnSync } from "node:child_process";

function exists(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code !== "ESRCH";
  }
}

export function observeProcess(pid) {
  if (!exists(pid)) return { pid, running: false, existsBefore: false };
  if (process.platform === "win32")
    return { pid, running: true, existsBefore: true };
  // kill(pid, 0) also sees unreaped zombies. They cannot execute code and
  // must not be reported as a running server when the owned group has exited.
  const started = performance.now();
  const status = spawnSync("/bin/ps", ["-o", "stat=", "-p", String(pid)], {
    encoding: "utf8",
    timeout: 1000,
  });
  // Reaping may race with ps; unknown observation remains conservative.
  const existsAfterPs = exists(pid);
  const existsAfterFailure = status.status === 0 ? undefined : existsAfterPs;
  return {
    pid,
    running:
      status.status === 0
        ? !/^\s*Z/.test(status.stdout ?? "")
        : existsAfterFailure,
    existsBefore: true,
    psStatus: status.status,
    psSignal: status.signal,
    psError: status.error?.code,
    // This command emits only a short state field. Never record exception
    // messages, command arguments or another process's path in diagnostics.
    psState: (status.stdout ?? "").trim().slice(0, 48),
    psMilliseconds: performance.now() - started,
    existsAfterPs,
    existsAfterFailure,
  };
}

export function isProcessRunning(pid) {
  return observeProcess(pid).running;
}
