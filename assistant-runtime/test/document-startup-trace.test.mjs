import test from "node:test";
import assert from "node:assert/strict";
import {
  decodeDocumentEngineTrace,
  DOCUMENT_STARTUP_TRACE_LIMITS,
} from "../../scripts/document-startup-trace.mjs";

// Synthetic named MOF fields, not a claim of native tracerpt compatibility.
// Microsoft publishes these GUID/opcode/field contracts; the XML reader is
// independently owned by the native capture helper.
const THREAD = "3d6fa8d1-fe05-11d0-9dda-00c04fd7ba7c";
const FILE = "90cbdc39-4a3e-11d1-84f4-0000f80464e3";
const IMAGE = "2cb15d1d-5fc1-11d2-abe1-00a0c911f518";
const REGISTRY = "ae53722e-c863-11d2-8659-00c04fa321a1";
const event = (providerGuid, opcode, at, data, extra = {}) => ({
  providerGuid,
  opcode,
  version: 2,
  timestamp100ns: (133000000000000000n + BigInt(at)).toString(),
  headerPid: 4, // Deliberately System: payload / issuing thread controls ownership.
  headerTid: 999,
  data,
  ...extra,
});
const start = (at = 100, pid = 42, tid = 43) =>
  event(THREAD, 1, at, { ProcessId: String(pid), TThreadId: String(tid) });
const end = (at = 900, pid = 42, tid = 43) =>
  event(THREAD, 2, at, { ProcessId: String(pid), TThreadId: String(tid) });
const create = (at, irp, path, tid = 43) =>
  event(FILE, 64, at, { IrpPtr: irp, TTID: String(tid), OpenPath: path });
const complete = (at, irp, status = "0xc0000022") =>
  event(FILE, 76, at, { IrpPtr: irp, NtStatus: status });
const counters = {
  ownedStopSucceeded: true,
  eventsLost: 0,
  logBuffersLost: 0,
  realTimeBuffersLost: 0,
  buffersWritten: 3,
};
const decode = (events, extra = {}) =>
  decodeDocumentEngineTrace({
    events,
    lossCounters: counters,
    targets: [{ childPid: 42, primaryTid: 43 }],
    roots: { windows: "C:\\Windows", runtime: "D:\\runtime" },
    ...extra,
  });

test("associates payload thread/IRP statuses and never infers cause from the last DLL", () => {
  const report = decode([
    start(),
    event(IMAGE, 10, 150, {
      ProcessId: "42",
      FileName: "C:\\Windows\\System32\\KERNELBASE.dll",
    }),
    create(200, "0xabcdef1234567890", "C:\\Windows\\System32\\ucrtbase.dll"),
    complete(250, "0xabcdef1234567890"),
    event(
      REGISTRY,
      11,
      300,
      {
        Status: "3221225506",
        KeyName: "\\REGISTRY\\USER\\S-1-5-21-12345\\secret",
      },
      { headerTid: 43 },
    ),
    event(
      REGISTRY,
      11,
      350,
      { Status: "0xc0000022", KeyName: "private-key" },
      { headerPid: 42, headerTid: 999 },
    ),
    end(),
  ]);
  assert.equal(report.status, "observed-failures");
  assert.equal(report.cause, "undetermined");
  assert.equal(report.negativeConclusionAllowed, false);
  assert.deepEqual(
    report.events.map((x) => x.kind),
    ["image-map", "file-operation", "registry-operation"],
  );
  assert.equal(report.events[1].status, "0xc0000022");
  assert.equal(report.events[1].module, "ucrtbase.dll");
  assert.equal(report.events[2].rootAlias, "registry-user");
  const encoded = JSON.stringify(report);
  for (const secret of [
    "abcdef1234567890",
    "S-1-5-21",
    "12345",
    "secret",
    "private-key",
    "C:\\\\Windows",
  ])
    assert.ok(!encoded.includes(secret));
});

test("IRP reuse pairs completed operations and blocks overlaps or intervening operations", () => {
  const report = decode([
    start(),
    create(120, "0x1", "D:\\runtime\\node.exe"),
    complete(130, "0x1", "0"),
    create(140, "0x1", "C:\\Windows\\System32\\ntdll.dll"),
    complete(150, "0x1"),
    create(160, "0x2", "D:\\runtime\\node.exe"),
    create(170, "0x2", "C:\\Users\\synthetic-person\\private.dll", 999),
    complete(180, "0x2"),
    create(190, "0x3", "D:\\runtime\\node.exe"),
    event(FILE, 67, 195, { IrpPtr: "0x3", TTID: "999" }),
    complete(200, "0x3"),
    end(),
  ]);
  assert.equal(
    report.events.filter((x) => x.kind === "file-operation").length,
    2,
  );
  assert.deepEqual(
    report.events.map((x) => x.status),
    ["0x00000000", "0xc0000022"],
  );
  assert.ok(report.reasons.includes("operation-correlation-incomplete"));
  assert.ok(!JSON.stringify(report).includes("synthetic-person"));
});

test("opaque intervening records cannot transfer a foreign IRP completion to the target", () => {
  for (const opaque of [
    {
      ...create(170, "0x1", "C:\\private\\other.dll", 999),
      data: {
        IrpPtr: "0x1",
        TTID: "999",
        OpenPath: "C:\\private\\other.dll",
        ExtraInfo: 7,
      },
    },
    {
      ...create(170, "0x1", "C:\\private\\other.dll", 999),
      timestamp100ns: null,
    },
    {
      ...create(170, "0x1", "C:\\private\\other.dll", 999),
      providerGuid: null,
    },
  ]) {
    const report = decode([
      start(),
      create(160, "0x1", "D:\\runtime\\node.exe"),
      opaque,
      complete(180, "0x1"),
      end(),
    ]);
    assert.equal(report.events.length, 0);
    assert.equal(report.status, "inconclusive");
    assert.ok(report.reasons.includes("operation-correlation-incomplete"));
  }
});

test("foreign primary-TID lifetimes cannot overlap or ambiguously surround a target window", () => {
  for (const edges of [
    [start(50, 99, 43), end(950, 99, 43)],
    [start(50, 99, 43), end(250, 99, 43)],
    [start(800, 99, 43), end(950, 99, 43)],
    [start(50, 99, 43)],
    [end(950, 99, 43)],
  ]) {
    const rows = [
      ...edges,
      start(),
      event(IMAGE, 10, 150, {
        ProcessId: "42",
        FileName: "C:\\Windows\\System32\\kernelbase.dll",
      }),
      create(200, "0x1", "D:\\runtime\\node.exe"),
      complete(220, "0x1"),
      end(),
    ].sort((a, b) =>
      Number(BigInt(a.timestamp100ns) - BigInt(b.timestamp100ns)),
    );
    const report = decode(rows);
    assert.notEqual(report.targets[0].window, "matched");
    assert.equal(report.events.length, 0);
    assert.equal(report.status, "inconclusive");
    assert.ok(report.reasons.includes("thread-identity-conflict"));
  }
  const valid = decode([
    start(10, 99, 43),
    end(50, 99, 43),
    start(),
    create(200, "0x1", "D:\\runtime\\node.exe"),
    complete(220, "0x1"),
    end(),
    start(950, 99, 43),
    end(980, 99, 43),
  ]);
  assert.equal(valid.targets[0].window, "matched");
  assert.equal(valid.events.length, 1);
});

test("opaque rows still participate in time-order validation before new IRP associations", () => {
  const report = decode([
    start(),
    event(null, 64, 300, { IrpPtr: "0x1" }),
    create(200, "0x1", "D:\\runtime\\node.exe"),
    complete(350, "0x1"),
    end(),
  ]);
  assert.ok(report.reasons.includes("event-order-unknown"));
  assert.equal(report.events.length, 0);
  assert.equal(report.status, "inconclusive");
});

test("missing, duplicated or out-of-window primary identities remain inconclusive", () => {
  for (const events of [
    [
      start(),
      create(200, "0x1", "D:\\runtime\\node.exe"),
      complete(210, "0x1"),
    ],
    [start(), start(110), end()],
    [start(), end(90)],
    [
      start(),
      create(950, "0x1", "D:\\runtime\\node.exe"),
      complete(960, "0x1"),
      end(),
    ],
  ]) {
    const report = decode(events);
    assert.equal(report.status, "inconclusive");
    assert.equal(report.events.length, 0);
    assert.equal(report.negativeConclusionAllowed, false);
  }
});

test("loss unknown, nonzero or malformed never becomes a complete negative conclusion", () => {
  for (const lossCounters of [
    undefined,
    {},
    { ...counters, eventsLost: 1 },
    { ...counters, ownedStopSucceeded: false },
    { ...counters, logBuffersLost: -1 },
  ]) {
    const report = decode([start(), end()], { lossCounters });
    assert.notEqual(report.lossStatus, "zero-reported");
    assert.equal(report.status, "inconclusive");
    assert.equal(report.negativeConclusionAllowed, false);
  }
});

test("unknown schemas, missing metadata and arbitrary strings expose only safe fields/counts", () => {
  const report = decode([
    start(),
    event("private-provider-S-1-5-21-secret", 11, 200, {
      OpenPath: "C:\\Users\\private-account\\secret",
      "credential-name": "api-key",
    }),
    event(
      FILE,
      64,
      300,
      { IrpPtr: "0x1", TTID: "43", OpenPath: "private-data" },
      { version: null },
    ),
    event(
      FILE,
      76,
      350,
      { IrpPtr: "0x1", NtStatus: "0xc0000022" },
      { timestamp100ns: null },
    ),
    end(),
  ]);
  assert.equal(report.status, "inconclusive");
  assert.ok(
    report.schemaOverview.some((x) => x.knownFields.includes("OpenPath")),
  );
  const encoded = JSON.stringify(report);
  for (const secret of [
    "private-provider",
    "S-1-5-21",
    "private-account",
    "secret",
    "credential-name",
    "api-key",
    "private-data",
  ])
    assert.ok(!encoded.includes(secret));
});

test("replayable iterable is bounded and stream faults do not expose exception messages", () => {
  let calls = 0;
  const events = [start(), end()];
  const report = decode(() => {
    calls++;
    return events.values();
  });
  assert.equal(calls, 2);
  assert.equal(report.targets[0].window, "matched");
  const fault = decode(function* () {
    yield start();
    throw new Error("C:\\Users\\private-account\\secret-token");
  });
  assert.equal(fault.status, "inconclusive");
  assert.ok(fault.reasons.includes("input-read-failed"));
  assert.ok(!JSON.stringify(fault).includes("private-account"));
});

test("bounded output retains observed failures and never emits arbitrary path basenames", () => {
  const rows = [start()];
  for (let n = 0; n < 80; n++)
    rows.push(
      event(IMAGE, 10, 110 + n, {
        ProcessId: "42",
        FileName: "C:\\Users\\synthetic-person\\secret-token.dll",
      }),
    );
  rows.push(
    create(300, "0x8", "C:\\Windows\\System32\\kernelbase.dll"),
    complete(310, "0x8"),
    end(),
  );
  const report = decode(rows);
  assert.ok(report.events.length <= 64);
  assert.ok(report.events.some((x) => x.status === "0xc0000022"));
  assert.ok(!JSON.stringify(report).includes("secret-token"));
});

test("NT device paths need the exact mapped Windows root and direct system modules", () => {
  const rows = [
    start(),
    create(
      200,
      "0x1",
      "\\Device\\HarddiskVolume9\\Windows\\System32\\kernelbase.dll",
    ),
    complete(210, "0x1"),
    end(),
  ];
  const unknown = decode(rows);
  assert.equal(unknown.events[0].rootAlias, "unknown");
  assert.equal(unknown.events[0].module, undefined);
  assert.ok(unknown.reasons.includes("unmapped-resource-root"));
  const mapped = decode(rows, {
    roots: { windows: ["C:\\Windows", "\\Device\\HarddiskVolume9\\Windows"] },
  });
  assert.equal(mapped.events[0].rootAlias, "windows");
  assert.equal(mapped.events[0].module, "kernelbase.dll");
  const privateDll = decode([
    start(),
    create(200, "0x2", "C:\\Windows\\Temp\\private-user-token.dll"),
    complete(210, "0x2"),
    end(),
  ]);
  assert.equal(privateDll.events[0].module, undefined);
  assert.ok(!JSON.stringify(privateDll).includes("private-user-token"));
});

test("a changed replay source cannot associate first-pass identities with other events", () => {
  let pass = 0;
  const report = decode(() =>
    (++pass === 1
      ? [start(), end()]
      : [
          start(),
          create(200, "0x1", "D:\\runtime\\node.exe"),
          complete(210, "0x1"),
          end(),
        ]
    ).values(),
  );
  assert.equal(report.events.length, 0);
  assert.ok(report.reasons.includes("input-changed-between-passes"));
});

test("unknown or excessive data stays bounded without raw exceptions or an empty negative", () => {
  const report = decode([
    start(),
    event(FILE, 64, 200, {
      IrpPtr: "0x1",
      TTID: "43",
      OpenPath: "credential".repeat(1000),
    }),
    complete(210, "0x1"),
    end(),
  ]);
  assert.equal(report.events.length, 0);
  assert.ok(report.reasons.includes("malformed-event"));
  assert.ok(!JSON.stringify(report).includes("credential"));
  let yielded = 0;
  const capped = decode(function* () {
    yield start();
    yield end();
    while (true) {
      yielded++;
      yield null;
    }
  });
  assert.ok(yielded <= 2 * DOCUMENT_STARTUP_TRACE_LIMITS.eventsPerPass);
  assert.ok(capped.reasons.includes("input-event-limit"));
  assert.equal(capped.negativeConclusionAllowed, false);
});
