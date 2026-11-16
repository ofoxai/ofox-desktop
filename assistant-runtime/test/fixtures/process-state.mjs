import { spawnSync } from "node:child_process";

function exists(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code !== "ESRCH";
  }
}

export function isProcessRunning(pid) {
  if (!exists(pid)) return false;
  if (process.platform === "win32") return true;
  // kill(pid, 0) also sees unreaped zombies. They cannot execute code and
  // must not be reported as a running server when the owned group has exited.
  const status = spawnSync("/bin/ps", ["-o", "stat=", "-p", String(pid)], {
    encoding: "utf8",
    timeout: 1000,
  });
  if (status.status === 0) return !/^\s*Z/.test(status.stdout ?? "");
  // Reaping may race with ps; unknown observation remains conservative.
  return exists(pid);
}
