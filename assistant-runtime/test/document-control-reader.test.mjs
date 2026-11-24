import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm, writeFile, readFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import test from "node:test";

const pwsh =
  process.env.OFOX_DOCUMENT_TEST_PWSH ??
  (process.platform === "win32" ? "pwsh" : null);
if (process.env.OFOX_DOCUMENT_CONTROL_REQUIRED === "1") {
  assert.ok(pwsh?.trim(), "PowerShell is required for the control preflight");
}
const fixture = fileURLToPath(
  new URL("./fixtures/document-control-reader.ps1", import.meta.url),
);

async function controller(mode, run) {
  const root = await mkdtemp(path.join(os.tmpdir(), "ofox-control-reader-"));
  const child = spawn(
    pwsh,
    [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-File",
      fixture,
      "-Mode",
      mode,
      "-SignalDirectory",
      root,
    ],
    {
      env: {
        ...process.env,
        HOME: root,
        USERPROFILE: root,
        APPDATA: root,
        LOCALAPPDATA: root,
        XDG_CONFIG_HOME: root,
        XDG_CACHE_HOME: root,
        POWERSHELL_TELEMETRY_OPTOUT: "1",
      },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  const exit = once(child, "close");
  const reader = createInterface({ input: child.stdout, crlfDelay: Infinity });
  const messages = [];
  let waiter = null;
  reader.on("line", (line) => {
    if (line.length > 1024) return;
    try {
      const message = JSON.parse(line);
      messages.push(message);
      waiter?.();
    } catch {}
  });
  child.stdin.on("error", () => {});
  child.stderr.on("data", () => {});
  async function event(name) {
    let timer;
    await Promise.race([
      new Promise((resolve) => {
        waiter = () => {
          if (messages.some((item) => item.event === name)) resolve();
        };
        waiter();
      }),
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`control reader did not report ${name}`)),
          name === "before-read" ? 10000 : 1500,
        );
      }),
    ]).finally(() => {
      clearTimeout(timer);
      waiter = null;
    });
    return messages.find((item) => item.event === name);
  }
  try {
    await event("before-read");
    await run({ child, root, event });
    child.stdin.end();
    const [code] = await exit;
    assert.equal(code, 0, "PowerShell control fixture failed");
  } finally {
    child.stdin.end();
    let timer;
    const ended = await Promise.race([
      exit.then(() => true),
      new Promise((resolve) => {
        timer = setTimeout(() => resolve(false), 1000);
      }),
    ]);
    clearTimeout(timer);
    if (!ended) {
      child.kill();
      await exit;
    }
    reader.close();
    await rm(root, { recursive: true, force: true });
  }
}

test(
  "control scheduling returns before stdin arrives and preserves coalesced STOP/CLEAN/EOF",
  { skip: !pwsh },
  async () => {
    await controller("receive", async ({ child, event }) => {
      assert.equal((await event("scheduled")).pending, true);
      child.stdin.end("STOP\nCLEAN\n");
      await event("stopped");
      await event("cleaned");
      await event("eof");
    });
  },
);

for (const mode of ["cancel", "deadline"]) {
  test(
    `control loop observes ${mode} while stdin remains open and silent`,
    { skip: !pwsh },
    async () => {
      await controller(mode, async ({ root, event }) => {
        assert.equal((await event("scheduled")).pending, true);
        if (mode === "cancel") await writeFile(path.join(root, "abort"), "");
        const result = await event(
          mode === "cancel" ? "cancelled" : "deadline",
        );
        assert.equal(result.inputPending, true);
      });
    },
  );
}

test("both diagnostic controllers use the same persistent control reader", async () => {
  for (const name of ["cdb", "etw"]) {
    const source = await readFile(
      new URL(`../../scripts/document-engine-${name}.ps1`, import.meta.url),
      "utf8",
    );
    assert.match(source, /document-engine-stdio\.ps1/);
    assert.match(source, /\$controlReader = New-DocumentControlReader/);
    assert.ok(!source.includes("[Console]::In.ReadLineAsync()"));
  }
});
