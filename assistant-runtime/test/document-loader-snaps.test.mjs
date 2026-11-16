import test from "node:test";
import assert from "node:assert/strict";
import { projectDocumentLoaderSnaps } from "../../scripts/document-loader-snaps.mjs";

// Synthetic grammar fixtures; native SDK output compatibility is unverified.
const before = "OFOX_CDB_V1 BEFORE pid=2a tid=2b flags=70 END";
const after = "OFOX_CDB_V1 AFTER pid=2a tid=2b flags=72 END";
const ready = "OFOX_CDB_V1 READY END";
const initFailure = (
  module = "C:\\Windows\\System32\\KERNELBASE.dll",
  identity = "2a:2b",
) =>
  `${identity} @ 100 - LdrpInitializeNode - ERROR: Init routine 00007FFF12345678 for DLL "${module}" failed during DLL_PROCESS_ATTACH`;
const project = (lines, extra = {}) =>
  projectDocumentLoaderSnaps({
    text: [before, after, ready, ...lines].join("\n"),
    target: { childPid: 42, primaryTid: 43 },
    roots: { windows: ["C:\\Windows", "\\Device\\HarddiskVolume3\\Windows"] },
    ...extra,
  });

test("keeps actual PID/TID and the exact before-or-bit2 readback in a closed handshake", () => {
  const report = project([]);
  assert.equal(report.handshake.status, "matched");
  assert.deepEqual(report.handshake.before, {
    childPid: 42,
    primaryTid: 43,
    flags: 0x70,
  });
  assert.deepEqual(report.handshake.after, {
    childPid: 42,
    primaryTid: 43,
    flags: 0x72,
  });
  assert.equal(report.negativeConclusionAllowed, false);
  assert.equal(report.releaseGate, false);
});

test("projects an explicit target DLL initialization failure without borrowing a later status", () => {
  const report = project([
    initFailure(),
    "2a:2b @ 101 - LdrpLoadDllInternal - RETURN: Status: 0xc0000142",
  ]);
  assert.equal(report.status, "observed-failures");
  assert.deepEqual(report.observations, [
    {
      kind: "dll-init-failure",
      childPid: 42,
      threadId: 43,
      rootAlias: "windows",
      module: "kernelbase.dll",
      status: null,
    },
  ]);
  assert.equal(report.cause, "undetermined");
  assert.ok(!JSON.stringify(report).includes("12345678"));
});

test("a same-line module and status are projected without inferring the last DLL is the cause", () => {
  const report = project([
    "ModLoad: 00007FFF00000000 00007FFF00010000 C:\\Windows\\System32\\unrelated.dll",
    '2a:2b @ 102 - LdrpInitializeProcess - ERROR: Loading Windows subsystem DLL "KERNEL32.DLL" failed with status 0xc0000142',
  ]);
  assert.deepEqual(report.observations, [
    {
      kind: "subsystem-load-failure",
      childPid: 42,
      threadId: 43,
      rootAlias: "windows-subsystem",
      module: "kernel32.dll",
      status: "0xc0000142",
    },
  ]);
  assert.ok(!JSON.stringify(report).includes("unrelated"));
  assert.deepEqual(
    project([
      "ModLoad: 00007FFF00000000 00007FFF00010000 C:\\Windows\\System32\\last.dll",
      "2a:2b @ 101 - LdrpLoadDllInternal - RETURN: Status: 0xc0000142",
    ]).observations,
    [],
  );
});

test("unknown roots, foreign identities, raw addresses and arbitrary output remain private", () => {
  const report = project([
    initFailure("D:\\private-user\\secret-name.dll"),
    initFailure("C:\\Windows\\System32\\foreign.dll", "63:2b"),
    initFailure("C:\\Windows\\System32\\foreign-thread.dll", "2a:63"),
    "token=secret-token SID=S-1-5-21-999 Computer=private-host",
  ]);
  assert.deepEqual(report.observations, [
    {
      kind: "dll-init-failure",
      childPid: 42,
      threadId: 43,
      rootAlias: "unknown",
      module: null,
      status: null,
    },
  ]);
  const encoded = JSON.stringify(report);
  for (const secret of [
    "private-user",
    "secret-name",
    "foreign",
    "secret-token",
    "S-1-5",
    "private-host",
    "12345678",
  ])
    assert.ok(!encoded.includes(secret));
});

test("device aliases require an exact root and only direct system directories expose basenames", () => {
  const report = project([
    initFailure("\\Device\\HarddiskVolume3\\Windows\\System32\\ntdll.dll"),
    initFailure("\\Device\\HarddiskVolume9\\Windows\\System32\\unknown.dll"),
    initFailure("C:\\Windows-old\\System32\\neighbor.dll"),
    initFailure("C:\\Windows\\Temp\\private.dll"),
  ]);
  assert.deepEqual(
    report.observations.map((item) => item.module),
    ["ntdll.dll", null, null, null],
  );
  assert.deepEqual(
    report.observations.map((item) => item.rootAlias),
    ["windows", "unknown", "unknown", "windows"],
  );
});

test("incomplete, repeated, reordered or mismatched handshake never attributes loader output", () => {
  for (const markers of [
    [before, ready],
    [before, after],
    [after, before, ready],
    [before, after, ready, ready],
    [before, after.replace("flags=72", "flags=73"), ready],
    [before, after.replace("pid=2a", "pid=2c"), ready],
    [before, after.replace("tid=2b", "tid=2c"), ready],
    [`0:000> .echo ${before}`, after, ready],
  ]) {
    const report = project([], {
      text: [...markers, initFailure()].join("\n"),
    });
    assert.equal(report.handshake.status, "unknown");
    assert.deepEqual(report.observations, []);
    assert.equal(report.status, "inconclusive");
  }
});

test("pre-READY failures and unknown native grammar cannot become target observations", () => {
  const report = project([], {
    text: [
      before,
      initFailure(),
      after,
      ready,
      "future loader ERROR wording",
    ].join("\n"),
  });
  assert.deepEqual(report.observations, []);
  assert.equal(report.status, "inconclusive");
  assert.equal(report.negativeConclusionAllowed, false);
});

test("oversized inputs and excessive observations are bounded", () => {
  const overlong = project([], { text: "x".repeat(4 * 1024 * 1024 + 1) });
  assert.equal(overlong.status, "inconclusive");
  assert.deepEqual(overlong.observations, []);
  const report = project(Array.from({ length: 80 }, () => initFailure()));
  assert.equal(report.observations.length, 32);
  assert.ok(report.reasons.includes("observation-limit"));
});
