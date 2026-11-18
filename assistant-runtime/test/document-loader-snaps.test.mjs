import test from "node:test";
import assert from "node:assert/strict";
import { projectDocumentLoaderSnaps } from "../../scripts/document-loader-snaps.mjs";

// Synthetic grammar fixtures; native SDK output compatibility is unverified.
const before = "OFOX_CDB_V2 BEFORE pid=2a tid=2b flags=70 END";
const after = "OFOX_CDB_V2 AFTER pid=2a tid=2b flags=72 END";
const ready = "OFOX_CDB_V2 READY END";
const preHold =
  "OFOX_CDB_V2 PRE_ATTACH_HOLD pid=2a tid=2b suspend=1 paired=2 END";
const continuing = "OFOX_CDB_V2 CONTINUE_REQUESTED END";
const host =
  "OFOX_CDB_V2 HOST_RESUMED pid=2a tid=2b hold=1 paired=2 previous=1 END";
const initFailure = (
  module = "C:\\Windows\\System32\\KERNELBASE.dll",
  identity = "2a:2b",
) =>
  `${identity} @ 100 - LdrpInitializeNode - ERROR: Init routine 00007FFF12345678 for DLL "${module}" failed during DLL_PROCESS_ATTACH`;
const project = (lines, extra = {}) =>
  projectDocumentLoaderSnaps({
    text: [before, after, ready, preHold, continuing, host, ...lines].join(
      "\n",
    ),
    target: { childPid: 42, primaryTid: 43 },
    roots: { windows: ["C:\\Windows", "\\Device\\HarddiskVolume3\\Windows"] },
    ...extra,
  });

test("flags READY alone cannot attribute a loader failure without actual Host Resume proof", () => {
  const report = project([], {
    text: [before, after, ready, initFailure()].join("\n"),
  });
  assert.equal(report.handshake.status, "unknown");
  assert.deepEqual(report.observations, []);
});

test("matches PID/TID privately and publishes only the before-or-bit2 readback", () => {
  const report = project([]);
  assert.equal(report.handshake.status, "matched");
  assert.deepEqual(report.handshake.before, {
    flags: 0x70,
  });
  assert.deepEqual(report.handshake.after, {
    flags: 0x72,
  });
  assert.equal(report.negativeConclusionAllowed, false);
  assert.equal(report.releaseGate, false);
  assert.equal(report.handshake.hostResumePrevious, 1);
  assert.deepEqual(report.handshake.preAttachHold, {
    suspendPrevious: 1,
    pairedResumePrevious: 2,
  });
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
    initFailure("C:\\Windows\\System32\\private-user.dll"),
    initFailure("C:\\Windows\\System32\\foreign.dll", "63:2b"),
    initFailure("C:\\Windows\\System32\\foreign-thread.dll", "2a:63"),
    "token=secret-token SID=S-1-5-21-999 Computer=private-host",
  ]);
  assert.deepEqual(report.observations, [
    {
      kind: "dll-init-failure",
      rootAlias: "unknown",
      module: null,
      status: null,
    },
    {
      kind: "dll-init-failure",
      rootAlias: "windows",
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
      text: [...markers, preHold, continuing, host, initFailure()].join("\n"),
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
      preHold,
      continuing,
      host,
      "future loader ERROR wording",
    ].join("\n"),
  });
  assert.deepEqual(report.observations, []);
  assert.equal(report.status, "inconclusive");
  assert.equal(report.negativeConclusionAllowed, false);
});

test("complete global Host proof retains an immediate failure before the ACK is observed", () => {
  const report = project([], {
    text: [before, after, ready, preHold, continuing, initFailure(), host].join(
      "\n",
    ),
  });
  assert.equal(report.handshake.status, "matched");
  assert.equal(report.observations.length, 1);
  assert.equal(report.observations[0].module, "kernelbase.dll");
});

test("missing, repeated, mismatched or early hold/continue/Host proof never attributes output", () => {
  for (const records of [
    [before, after, ready, preHold, continuing, initFailure()],
    [before, after, ready, continuing, host, initFailure()],
    [before, after, ready, preHold, host, initFailure()],
    [before, after, ready, preHold, host, continuing, initFailure()],
    [before, after, ready, preHold, continuing, host, host, initFailure()],
    [
      before,
      after,
      ready,
      preHold.replace("pid=2a", "pid=2c"),
      continuing,
      host,
      initFailure(),
    ],
    [
      before,
      after,
      ready,
      preHold.replace("paired=2", "paired=3"),
      continuing,
      host,
      initFailure(),
    ],
    [
      before,
      after,
      ready,
      preHold,
      continuing,
      host.replace("previous=1", "previous=2"),
      initFailure(),
    ],
    [
      before,
      after,
      ready,
      preHold,
      continuing,
      host.replace("hold=1", "hold=2"),
      initFailure(),
    ],
    [
      before,
      after,
      ready,
      preHold,
      continuing,
      host.replace("tid=2b", "tid=2c"),
      initFailure(),
    ],
  ]) {
    const report = project([], { text: records.join("\n") });
    assert.equal(report.handshake.status, "unknown");
    assert.deepEqual(report.observations, []);
  }
});

test("a short-line flood within the byte bound keeps the handshake", () => {
  // Run 44228529933: 66114 lines in 4 MiB exceeded a 50000-line bound and
  // discarded the whole projection, including the original Host proof.
  const flood = Array.from(
    { length: 60000 },
    () => "(2a.2b): Break instruction exception - code 80000003 (first chance)",
  );
  assert.equal(project(flood).handshake.status, "matched");
  // The line bound still follows the byte bound (16-byte lines in 4 MiB).
  const tiny = project(Array.from({ length: 300000 }, () => "x"));
  assert.ok(tiny.reasons.includes("input-limit"));
});

test("oversized inputs and excessive observations are bounded", () => {
  const overlong = project([], { text: "x".repeat(4 * 1024 * 1024 + 1) });
  assert.equal(overlong.status, "inconclusive");
  assert.deepEqual(overlong.observations, []);
  const report = project(Array.from({ length: 80 }, () => initFailure()));
  assert.equal(report.observations.length, 32);
  assert.ok(report.reasons.includes("observation-limit"));
});
