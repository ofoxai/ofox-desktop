import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { projectDocumentNativeOperations } from "../../scripts/document-native-operations.mjs";
import { safeDebuggerState } from "../../scripts/document-engine-development-check.mjs";

const setup = [
  "OFOX_NT_V1 INSTALLED pid=2a tid=2b thread=0 count=14 END",
  "OFOX_CDB_V2 BEFORE pid=2a tid=2b flags=0 END",
  "OFOX_CDB_V2 AFTER pid=2a tid=2b flags=2 END",
  "OFOX_CDB_V2 READY END",
  "OFOX_CDB_V2 PRE_ATTACH_HOLD pid=2a tid=2b suspend=1 paired=2 END",
  "OFOX_CDB_V2 CONTINUE_REQUESTED END",
];
const host =
  "OFOX_CDB_V2 HOST_RESUMED pid=2a tid=2b hold=1 paired=2 previous=1 END";
const pwsh =
  process.env.OFOX_DOCUMENT_TEST_PWSH ??
  (process.platform === "win32" ? "pwsh" : null);

test(
  "actual PowerShell requires matched x64 layout and enabled resolved thread-specific breakpoint rows",
  { skip: !pwsh },
  () => {
    const root = mkdtempSync(path.join(os.tmpdir(), "ofox-operation-setup-"));
    try {
      for (const mode of [
        "valid",
        "alias",
        "layout",
        "foreign",
        "wrongthread",
        "unbound",
        "disabled",
        "unresolved",
        "duplicate",
        "symbol",
        "missing",
        "extra",
      ]) {
        const result = spawnSync(
          pwsh,
          [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-File",
            fileURLToPath(
              new URL(
                "./fixtures/document-operation-setup.ps1",
                import.meta.url,
              ),
            ),
            "-Mode",
            mode,
          ],
          {
            encoding: "utf8",
            env: {
              ...process.env,
              HOME: root,
              USERPROFILE: root,
              APPDATA: root,
              LOCALAPPDATA: root,
            },
            timeout: 10000,
            maxBuffer: 65536,
          },
        );
        assert.equal(result.error, undefined);
        assert.equal(result.status, 0, result.stderr);
        const report = JSON.parse(result.stdout);
        assert.equal(report.verified, ["valid", "alias"].includes(mode), mode);
        if (mode !== "valid") continue;
        assert.equal(report.threadIndex, 1);
        assert.equal(Object.keys(report.program.files).length, 15);
        const entry = report.program.files["operation-1.cmd"],
          port = report.program.files["operation-10.cmd"],
          returned = report.program.files["operation-return.cmd"];
        assert.ok(entry.includes(", @edx, @$t6"));
        assert.ok(entry.includes("wo(@$t5) <= 0x400"));
        assert.ok(entry.includes("%msu"));
        assert.ok(!entry.includes("%mu"));
        assert.ok(port.includes("r @$t5 = @rdx"));
        assert.ok(port.includes("access=none root=1"));
        assert.ok(!port.includes("@edx"));
        assert.ok(
          entry.includes('~~[0n43]bp30 @$t3 "$><operation-return.cmd"'),
        );
        assert.ok(returned.includes("@rsp == (@$t2 + 8)"));
        assert.ok(returned.includes("@rip == @$t3"));
        assert.ok(returned.includes("@eax"));
        assert.ok(returned.includes("bc 30"));
        assert.ok(!JSON.stringify(report.program).includes("/1"));
        assert.ok(!JSON.stringify(report.program).includes("gu;"));
        assert.ok(
          report.program.commands.includes(
            '~~[0n43]bp10 ntdll!NtOpenKey "$><operation-1.cmd"',
          ),
        );
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  },
);
function call({
  seq = 1,
  api = 10,
  access = "none",
  root = "1",
  name = "\\Windows\\ApiPort",
  status = "c0000022",
  thread = "000",
} = {}) {
  return [
    `OFOX_NT_RAW_V1 ENTER seq=${seq} api=${api} pid=2a tid=2b sp=1000 ret=2000 access=${access} root=${root} END`,
    `OFOX_NT_RAW_V1 NAME seq=${seq} bytes=${Buffer.byteLength(name, "utf16le")} value="${name}" END`,
    `OFOX_NT_RAW_V1 RETURN_BP_BEGIN seq=${seq} END`,
    `30 e 00000000\`00002000 0001 (0001) 0:${thread} KERNELBASE!privateCaller "private command"`,
    `OFOX_NT_RAW_V1 RETURN_BP_END seq=${seq} END`,
    `OFOX_NT_RAW_V1 RETURN seq=${seq} pid=2a tid=2b sp=1008 status=${status} END`,
  ];
}
function project(lines, replaceSetup = setup) {
  return projectDocumentNativeOperations({
    text: [...replaceSetup, ...lines, host].join("\n"),
    target: { childPid: 42, primaryTid: 43 },
  });
}

test("pairs an actual thread and stack return with its verified persistent breakpoint", () => {
  const report = project(call());
  assert.equal(report.status, "observed-failures");
  assert.equal(report.coverage.status, "complete-for-fixed-set");
  assert.equal(report.coverage.completedCalls, 1);
  assert.deepEqual(report.observations, [
    {
      sequence: 1,
      api: "NtAlpcConnectPort",
      status: "0xc0000022",
      desiredAccess: null,
      objectAlias: "csr-api-port",
    },
  ]);
  assert.equal(report.cause, "undetermined");
  assert.equal(report.releaseGate, false);
  assert.equal(report.negativeConclusionAllowed, false);
});

test("keeps low32 access masks and separates relative names and registry values from absolute resources", () => {
  const report = project([
    ...call({
      api: 1,
      access: "20019",
      root: "1",
      name: "\\Registry\\Machine\\Software\\Microsoft",
    }),
    ...call({
      seq: 2,
      api: 1,
      access: "20019",
      root: "0",
      name: "\\Registry\\Machine\\private-user",
    }),
    ...call({
      seq: 3,
      api: 4,
      access: "none",
      root: "none",
      name: "private-value",
    }),
    ...call({ seq: 4, api: 10, name: "\\Sessions\\2\\Windows\\ApiPort" }),
  ]);
  assert.equal(report.status, "observed-failures");
  assert.deepEqual(
    report.observations.map((row) => row.objectAlias),
    ["machine-registry", "unknown", "unknown", "csr-api-port"],
  );
  assert.deepEqual(
    report.observations.map((row) => row.desiredAccess),
    [0x20019, 0x20019, null, null],
  );
  const encoded = JSON.stringify(report);
  for (const secret of [
    "private-user",
    "private-value",
    "privateCaller",
    "private command",
    "2000",
    "1008",
    "\\Registry",
    "\\Sessions",
  ])
    assert.ok(!encoded.includes(secret), secret);
});

test("does not attribute without the complete original Host Resume and independently verified installation", () => {
  for (const records of [
    setup.slice(1),
    setup.slice(0, -1),
    setup.map((line) => line.replace("count=14", "count=11")),
    [...setup, setup[0]],
    setup.map((line) => line.replace("tid=2b thread", "tid=2c thread")),
  ]) {
    const report = project(call(), records);
    assert.equal(report.status, "inconclusive");
    assert.deepEqual(report.observations, []);
  }
  const report = projectDocumentNativeOperations({
    text: [...setup, ...call()].join("\n"),
    target: { childPid: 42, primaryTid: 43 },
  });
  assert.equal(report.status, "inconclusive");
  assert.deepEqual(report.observations, []);
});

test("wrong return TID, stack, breakpoint ID, enabled state, address or thread cannot pair", () => {
  for (const mutate of [
    (lines) =>
      lines.map((line) =>
        line.includes(" RETURN seq=") ? line.replace("tid=2b", "tid=2c") : line,
      ),
    (lines) => lines.map((line) => line.replace("sp=1008", "sp=1010")),
    (lines) => lines.map((line) => line.replace("30 e ", "31 e ")),
    (lines) => lines.map((line) => line.replace("30 e ", "30 d ")),
    (lines) => lines.map((line) => line.replace("30 e ", "30 eu ")),
    (lines) => lines.map((line) => line.replace("00002000", "00002008")),
    (lines) =>
      lines.map((line) => line.replace("00000000`00002000", "````````")),
    (lines) =>
      lines.map((line) =>
        line.replace("00000000`00002000", "0000`000000002000"),
      ),
    (lines) => lines.map((line) => line.replace("0:000", "0:***")),
    (lines) => lines.map((line) => line.replace("0:000", "0:001")),
    (lines) => lines.filter((line) => !line.includes("RETURN_BP_")),
  ]) {
    const report = project(mutate(call()));
    assert.equal(report.status, "inconclusive");
    assert.equal(report.coverage.status, "incomplete");
    assert.deepEqual(report.observations, []);
  }
});

test("missing returns, nesting, duplicate sequence, bad ABI and overflow remain inconclusive", () => {
  for (const lines of [
    call().slice(0, -1),
    [call()[0], ...call({ seq: 2 })],
    [...call(), ...call()],
    call({ access: "1234" }),
    call({ api: 1, access: "100020019" }),
    call().map((line) => line.replace("status=c0000022", "status=1c0000022")),
    [...call(), "OFOX_NT_RAW_V1 INVALID nested END"],
    [...call(), "OFOX_NT_RAW_V1 INVALID limit END"],
    call().map((line) => line.replace("bytes=32", "bytes=1000")),
  ]) {
    const report = project(lines);
    assert.equal(report.status, "inconclusive");
    assert.equal(report.coverage.status, "incomplete");
  }
});

test("unknown objects never leak names and a successful operation cannot become a cause", () => {
  const report = project(
    call({
      name: "\\Device\\private-user\\token-secret-S-1-5-21",
      status: "0",
    }),
  );
  assert.equal(report.status, "observed-operations");
  assert.equal(report.coverage.completedCalls, 1);
  assert.deepEqual(report.observations, []);
  assert.equal(report.cause, "undetermined");
  assert.ok(!JSON.stringify(report).includes("private-user"));
  assert.equal(
    project(["OFOX_NT_RAW_V1 UNKNOWN secret END"]).status,
    "inconclusive",
  );
  assert.equal(
    projectDocumentNativeOperations({
      text: "x".repeat(4 * 1024 * 1024 + 1),
      target: { childPid: 42, primaryTid: 43 },
    }).status,
    "inconclusive",
  );
});

test("status-only sibling port APIs never interpret an undocumented argument as a resource or access mask", () => {
  const calls = [13, 14].flatMap((api, index) =>
    call({ seq: index + 1, api, root: "none" }).map((line) =>
      line.includes(" NAME seq=")
        ? `OFOX_NT_RAW_V1 NAME_UNKNOWN seq=${index + 1} END`
        : line,
    ),
  );
  const report = project(calls);
  assert.equal(report.coverage.status, "complete-for-fixed-set");
  assert.deepEqual(
    report.observations.map(({ api, desiredAccess, objectAlias }) => ({
      api,
      desiredAccess,
      objectAlias,
    })),
    [
      {
        api: "NtAlpcConnectPortEx",
        desiredAccess: null,
        objectAlias: "unknown",
      },
      {
        api: "NtSecureConnectPort",
        desiredAccess: null,
        objectAlias: "unknown",
      },
    ],
  );
});

test("setup failure state retains only fixed booleans and a bounded breakpoint count", () => {
  const state = safeDebuggerState({
    operationSetup: {
      layoutVerified: true,
      threadMatched: false,
      installationVerified: "private-secret",
      breakpointRows: 14,
      address: "secret-pointer",
      raw: "secret-value",
    },
  });
  assert.deepEqual(state.operationSetup, {
    layoutVerified: true,
    threadMatched: false,
    installationVerified: null,
    breakpointRows: 14,
  });
  assert.ok(!JSON.stringify(state).includes("secret"));
  for (const count of [-1, 15, "14", null])
    assert.equal(
      safeDebuggerState({ operationSetup: { breakpointRows: count } })
        .operationSetup.breakpointRows,
      null,
    );
});
