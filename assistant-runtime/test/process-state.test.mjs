import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { createInterface } from "node:readline";
import test from "node:test";
import { isProcessRunning } from "./fixtures/process-state.mjs";

test(
  "exit observation distinguishes a live owned process from its unreaped dead child",
  { skip: process.platform === "win32" },
  async (t) => {
    const parent = spawn(
      process.execPath,
      [
        "-e",
        `const { spawn } = require('node:child_process');
       const child = spawn(process.execPath, ['-e', 'process.exit(0)'], { stdio: 'ignore' });
       console.log(child.pid);
       Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 15000);`,
      ],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    const closed = once(parent, "close");
    t.after(async () => {
      parent.kill("SIGKILL");
      await closed;
    });
    const lines = createInterface({ input: parent.stdout });
    const [line] = await once(lines, "line");
    const pid = Number(line);
    assert.ok(Number.isInteger(pid) && pid > 0);
    const deadline = Date.now() + 5000;
    let zombie = false;
    while (Date.now() < deadline) {
      const status = spawnSync("/bin/ps", ["-o", "stat=", "-p", String(pid)], {
        encoding: "utf8",
        timeout: 1000,
      });
      if (/^\s*Z/.test(status.stdout ?? "")) {
        zombie = true;
        break;
      }
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    assert.ok(zombie, "the blocked parent leaves a genuine zombie to observe");
    assert.doesNotThrow(
      () => process.kill(pid, 0),
      "kill(0) still sees the zombie PID",
    );
    assert.equal(
      isProcessRunning(parent.pid),
      true,
      "the owned parent is live",
    );
    assert.equal(isProcessRunning(pid), false, "a zombie cannot execute code");
  },
);
