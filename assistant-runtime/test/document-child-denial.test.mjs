import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";
import os from "node:os";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { fileURLToPath } from "node:url";

const rust = await readFile(
  new URL(
    "../../src-tauri/src/services/assistant/documents/windows_sandbox.rs",
    import.meta.url,
  ),
  "utf8",
);

// Exercise the same string compiled into the signed Host and source witness.
const probe = /^const NODE_PROBE: &str = r#"([\s\S]*?)"#;/m.exec(rust)[1];

// Run the actual embedded Node probe up to the first native-module check.
// All filesystem denials before the spawn are simulated; the unrestricted
// positive control writes its fixed marker only inside a fresh temp directory.
function runProbe(spawn, markerExists = () => false) {
  const next = new Error("reached the next check");
  const denied = () => {
    throw new Error("fixture denied");
  };
  const events = [];
  const modules = {
    "node:assert/strict": assert,
    "node:fs": {
      readFileSync: denied,
      writeFileSync: denied,
      existsSync: markerExists,
    },
    "node:net": {},
    "node:child_process": { spawnSync: spawn },
  };
  try {
    runInNewContext(probe, {
      require: (name) => modules[name],
      process: {
        execPath: process.execPath,
        versions: { node: "22.23.4" },
        env: {},
        argv: [],
      },
      console: {
        error: (line) => {
          events.push(line);
          if (line === "OFOX_LPAC_PROBE_STAGE:8") throw next;
        },
      },
      OUTSIDE_PATH: "outside",
      OUTSIDE_AAP_PATH: "outside-aap",
    });
    assert.fail("probe terminated before the next check");
  } catch (error) {
    if (error !== next) throw error;
  }
  return events;
}
const failure = (code) => ({
  error: { code },
  pid: 0,
  status: null,
  signal: null,
});
test("the child denial check reaches a policy refusal without allocating another stdio pipe", () => {
  for (const code of ["EPERM", "EACCES"]) {
    const events = runProbe((_exe, _args, opts) => {
      assert.equal(
        opts?.stdio,
        "inherit",
        "LPAC cannot allocate the default libuv stdio pipes",
      );
      return failure(code);
    });
    assert.ok(events.includes("OFOX_LPAC_CHILD_RESULT:" + code));
    assert.ok(events.includes("OFOX_LPAC_PROBE_STAGE:8"));
  }
});
test("timeouts, unrelated spawn errors and any evidence of a created child cannot prove denial", () => {
  for (const result of [
    failure("ETIMEDOUT"),
    failure("ENOENT"),
    failure("UNKNOWN"),
    { ...failure("EPERM"), pid: 123 },
    { ...failure("EPERM"), status: 1 },
    { ...failure("EPERM"), signal: "SIGTERM" },
  ]) {
    assert.throws(() => runProbe(() => result));
  }
  assert.throws(() =>
    runProbe(
      () => failure("EPERM"),
      () => true,
    ),
  );
});
test("the same request really creates the marker when child processes are allowed", async (t) => {
  const cwd = await mkdtemp(path.join(os.tmpdir(), "document-child-denial-"));
  t.after(() => rm(cwd, { recursive: true, force: true }));
  const markerExists = () => existsSync(path.join(cwd, "child-marker"));
  assert.throws(() =>
    runProbe(
      (exe, args, opts) =>
        spawnSync(exe, args, {
          ...opts,
          cwd,
          env: { SystemRoot: process.env.SystemRoot || "" },
        }),
      markerExists,
    ),
  );
  assert.equal(await readFile(path.join(cwd, "child-marker"), "utf8"), "bad");
});

const eventTestPwsh =
  process.env.OFOX_DOCUMENT_TEST_PWSH ??
  (process.platform === "win32" ? "pwsh" : null);
test(
  "the kernel child-block witness binds the exact native event, process lifetime, thread and request",
  { skip: !eventTestPwsh },
  async (t) => {
    const dir = await mkdtemp(path.join(os.tmpdir(), "document-child-event-"));
    t.after(() => rm(dir, { recursive: true, force: true }));
    const result = spawnSync(
      eventTestPwsh,
      [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-File",
        fileURLToPath(
          new URL("./fixtures/document-child-block-event.ps1", import.meta.url),
        ),
      ],
      {
        encoding: "utf8",
        timeout: 10000,
        maxBuffer: 4096,
        env: {
          ...process.env,
          HOME: dir,
          USERPROFILE: dir,
          APPDATA: dir,
          LOCALAPPDATA: dir,
          POWERSHELL_TELEMETRY_OPTOUT: "1",
        },
      },
    );
    assert.equal(result.error, undefined);
    assert.equal(result.status, 0);
    assert.equal(result.stdout.trim(), "passed");
  },
);
