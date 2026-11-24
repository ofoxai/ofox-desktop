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
function runProbe(
  spawn,
  markerExists = () => false,
  nativeDenial,
  {
    events = [],
    addonError,
    skiaError,
    prefixes = [],
    addonArg = "fixed-native-probe",
    leafStat = "file",
    realpath = "resolved",
  } = {},
) {
  const next = new Error("reached the next check");
  const denied = () => {
    throw new Error("fixture denied");
  };
  const modules = {
    "node:assert/strict": assert,
    "node:fs": {
      readFileSync: denied,
      writeFileSync: denied,
      existsSync: markerExists,
      statSync: (filename) => {
        assert.equal(filename, addonArg);
        if (leafStat === "failed") throw new Error("private-path secret-token");
        return { isFile: () => leafStat === "file" };
      },
      realpathSync: (filename) => {
        assert.equal(filename, addonArg);
        if (realpath === "failed") throw new Error("private-path secret-token");
        return "private-path secret-token";
      },
    },
    "node:net": {},
    "node:path": path.win32,
    "node:child_process": { spawnSync: spawn },
    "fixed-native-probe": { probe: () => nativeDenial },
    "fixed-skia": {},
  };
  try {
    runInNewContext(probe, {
      require: (name) => {
        if (name === addonArg) {
          if (addonError) throw addonError;
          return modules["fixed-native-probe"];
        }
        if (name === "fixed-skia" && skiaError) throw skiaError;
        return modules[name];
      },
      process: {
        execPath: process.execPath,
        versions: { node: "22.23.4" },
        env: {},
        argv: [undefined, "fixed-skia", undefined, addonArg],
      },
      console: {
        error: (line) => {
          events.push(line);
          if (line === "OFOX_LPAC_PROBE_STAGE:8") throw next;
        },
      },
      OUTSIDE_PATH: "outside",
      OUTSIDE_AAP_PATH: "outside-aap",
      NATIVE_CHILD_LOADER_PREFIXES: prefixes,
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
const blocked = {
  created: false,
  error: 367,
  processId: 0,
  threadId: 0,
  processHandle: false,
  threadHandle: false,
  cleanupComplete: true,
};

test("the same Node process can prove the exact native denial when libuv reports UNKNOWN", () => {
  const events = runProbe(
    () => failure("UNKNOWN"),
    () => false,
    blocked,
  );
  assert.ok(events.includes("OFOX_LPAC_NATIVE_CHILD_DENIED:367"));
  assert.ok(events.includes("OFOX_LPAC_PROBE_STAGE:8"));
  assert.ok(events.every((event) => !event.includes("SKIA_LOAD")));
});

test("an addon-load failure reports only fixed loader and Skia categories", () => {
  const privateText = "private-path and secret-token";
  const privateArg = "\\\\?\\C:\\private-path\\native.node";
  for (const [
    code,
    message,
    loader,
    skiaError,
    skia,
    addonArg,
    leafStat = "file",
    realpath = "resolved",
  ] of [
    [
      "ERR_DLOPEN_FAILED",
      "Fixed system text\r\n" + privateText,
      "5",
      null,
      "loaded",
    ],
    [
      "ERR_DLOPEN_FAILED",
      `${privateArg} is not an example binary.\r\n${privateText}`,
      "193",
      null,
      "loaded",
      privateArg,
    ],
    [
      "ERR_DLOPEN_FAILED",
      privateText + "Fixed system text\r\n",
      "unknown",
      Error(privateText),
      "failed",
    ],
    ["MODULE_NOT_FOUND", privateText, null, null, "loaded"],
    [
      "EISDIR",
      privateText,
      null,
      Error(privateText),
      "failed",
      privateArg,
      "other",
      "failed",
    ],
    ["EACCES", privateText, null, null, "loaded", undefined, "file", "failed"],
    ["EPERM", privateText, null, null, "loaded", undefined, "failed", "failed"],
  ]) {
    const events = [];
    assert.throws(
      () =>
        runProbe(
          () => failure("UNKNOWN"),
          () => false,
          undefined,
          {
            events,
            addonError: Object.assign(new Error(message), { code }),
            skiaError,
            addonArg,
            leafStat,
            realpath,
            prefixes: [
              [5, "Fixed system text\r\n"],
              [193, "%1 is not an example binary.\r\n"],
            ],
          },
        ),
      { message: "the fixed native child probe could not load" },
    );
    assert.deepEqual(
      events.filter((line) => line.startsWith("OFOX_LPAC_NATIVE_CHILD_")),
      [
        `OFOX_LPAC_NATIVE_CHILD_RESULT:${code === "MODULE_NOT_FOUND" ? "module-missing" : "module-load-failed"}`,
        `OFOX_LPAC_NATIVE_CHILD_LOAD_CODE:${code}`,
        `OFOX_LPAC_NATIVE_CHILD_MODULE_ARG_NAMESPACED:${addonArg === privateArg}`,
        `OFOX_LPAC_NATIVE_CHILD_LEAF_STAT:${leafStat}`,
        `OFOX_LPAC_NATIVE_CHILD_REALPATH:${realpath}`,
        ...(loader === null
          ? []
          : [`OFOX_LPAC_NATIVE_CHILD_LOADER_WIN32:${loader}`]),
        `OFOX_LPAC_NATIVE_CHILD_SKIA_LOAD:${skia}`,
      ],
    );
    assert.doesNotMatch(events.join("\n"), /private-path|secret-token/);
  }
});

test("a libuv refusal cannot authorize a missing, unrelated or successful native result", () => {
  for (const native of [
    undefined,
    { ...blocked, error: 5 },
    { ...blocked, error: 2 },
    { ...blocked, error: 0 },
    { ...blocked, created: true },
    { ...blocked, processId: 123 },
    { ...blocked, threadId: 123 },
    { ...blocked, processHandle: true },
    { ...blocked, threadHandle: true },
    { ...blocked, cleanupComplete: false },
  ]) {
    assert.throws(() =>
      runProbe(
        () => failure("EPERM"),
        () => false,
        native,
      ),
    );
  }
});
test("the child denial check reaches a policy refusal without allocating another stdio pipe", () => {
  for (const code of ["EPERM", "EACCES"]) {
    const events = runProbe(
      (_exe, _args, opts) => {
        assert.equal(
          opts?.stdio,
          "inherit",
          "LPAC cannot allocate the default libuv stdio pipes",
        );
        return failure(code);
      },
      () => false,
      blocked,
    );
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
