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
  "OFOX_NT_V1 INSTALLED pid=2a tid=2b thread=0 count=19 arch=amd64 abi=windows-x64 END",
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
  "actual PowerShell requires a native x64 child receipt and enabled resolved thread-specific breakpoint rows",
  { skip: !pwsh },
  () => {
    const root = mkdtempSync(path.join(os.tmpdir(), "ofox-operation-setup-"));
    try {
      for (const mode of [
        "valid",
        "alias",
        "layout",
        "api-failed",
        "wow64",
        "arm64",
        "architecture-type",
        "architecture-missing",
        "architecture-float",
        "architecture-string",
        "foreign",
        "wrongthread",
        "unbound",
        "disabled",
        "unresolved",
        "duplicate",
        "symbol",
        "missing",
        "extra",
        "dml",
        "dml-disabled",
        "dml-wrongthread",
        "dml-unbound",
        "dml-action",
        "dml-echo",
        "dml-echo-row",
        "outside",
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
        assert.equal(
          report.verified,
          ["valid", "alias", "dml", "dml-echo"].includes(mode),
          mode,
        );
        if (mode !== "valid") continue;
        assert.equal(report.threadIndex, 1);
        assert.ok(report.program.commands.includes(".expr /s masm"));
        // Run 44221172203 froze after DLL mapping at the loader's debugger
        // break. Breaks and exceptions are now reported without stopping, each
        // with its default handling; only the invalid-handle exception, raised
        // solely because a debugger is attached, is set to handled.
        const sx = report.program.commands.filter((line) =>
          line.startsWith("sx"),
        );
        assert.deepEqual(sx, [
          "sxn ibp",
          "sxn bpe",
          "sxe -h ch",
          "sxn ch",
          "sxn av",
          "sxn dm",
          "sxn dz",
          "sxn c000008e",
          "sxn gp",
          "sxn iov",
          "sxn ip",
          "sxn isc",
          "sxn lsq",
          "sxn sbo",
          "sxn sov",
          "sxn asrt",
          "sxn wkd",
          "sxn aph",
          "sxn eh",
          "sxn ii",
          "sxn *",
        ]);
        // No exception is blanket-ignored or forced to handled.
        assert.ok(!sx.some((line) => /^sx[id]\b/.test(line)));
        assert.ok(
          report.program.commands.indexOf("sxn *") <
            report.program.commands.indexOf(
              ".echo OFOX_NT_RAW_V1 BREAKPOINTS_BEGIN",
            ),
        );
        assert.doesNotMatch(JSON.stringify(report.program), /&&|\|\|/);
        assert.doesNotMatch(JSON.stringify(report.program), /@@c\+\+|LAYOUT/);
        assert.ok(
          report.program.files["operation-1.cmd"].includes("dwo(@r8) == 0x30"),
        );
        // The projector's MAX_CALLS mirrors this debugger-side call limit.
        assert.ok(
          report.program.files["operation-1.cmd"].includes("@$t0 >= 0n1024"),
        );
        // 19 entry scripts, the Nt return pair and the WSAStartup scope pair.
        assert.equal(Object.keys(report.program.files).length, 22);
        assert.deepEqual(
          report.definitions.slice(15).map((row) => [row.Name, row.Kind]),
          [
            ["NtEnumerateKey", "none"],
            ["NtQueryKey", "none"],
            ["NtNotifyChangeKey", "none"],
            ["NtNotifyChangeMultipleKeys", "none"],
          ],
        );
        assert.ok(report.program.commands.includes("bl /L 10-28"));
        assert.ok(
          report.program.commands.includes(
            '~~[0n43]bp24 ntdll!NtCreateUserProcess "$><operation-15.cmd"',
          ),
        );
        assert.ok(
          report.program.files["operation-15.cmd"].includes("bd 10-28"),
        );
        // The WSAStartup scope is a deferred breakpoint set after the
        // verified listing, so an unresolved scope never blocks the trace.
        const scope = report.program.commands.indexOf(
          '~~[0n43]bu31 ws2_32!WSAStartup "$><operation-scope.cmd"',
        );
        assert.ok(
          scope >
            report.program.commands.indexOf(
              ".echo OFOX_NT_RAW_V1 BREAKPOINTS_END",
            ),
        );
        assert.ok(report.program.commands.includes("r @$t10 = 0"));
        const enter = report.program.files["operation-scope.cmd"],
          leave = report.program.files["operation-scope-return.cmd"];
        assert.ok(enter.startsWith(".if (@$tid != 0n43) { gc } .elsif"));
        assert.ok(enter.includes("SCOPE_ENTER seq=%d"));
        assert.ok(enter.includes("r @$t11 = @rsp"));
        assert.ok(enter.includes("r @$t12 = poi(@rsp)"));
        assert.ok(
          enter.includes('~~[0n43]bp32 @$t12 "$><operation-scope-return.cmd"'),
        );
        assert.ok(enter.includes("INVALID scope END"));
        assert.ok(leave.includes("@rsp == (@$t11 + 8)"));
        assert.ok(leave.includes("@rip == @$t12"));
        assert.ok(leave.includes("result=%x"));
        assert.ok(leave.includes("@eax"));
        assert.ok(leave.includes("bc 31; bc 32"));
        assert.ok(leave.includes("INVALID scope_pair END"));
        const entry = report.program.files["operation-1.cmd"],
          port = report.program.files["operation-10.cmd"],
          returned = report.program.files["operation-return.cmd"];
        assert.ok(entry.includes(", @edx, @$t6"));
        assert.ok(entry.includes("wo(@$t5) <= 0x400"));
        assert.ok(entry.includes("$vvalid(@r8, 0x30)"));
        assert.ok(entry.includes("$vvalid(@$t5, 0x10)"));
        assert.ok(entry.includes("$vvalid(poi(@$t5 + 8), wo(@$t5))"));
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

test("the first marker glued to the waiting CDB prompt still pairs", () => {
  // Native run 44049635186: after -cf, CDB prints "0:000> " without a newline
  // and the first output after the controller's g lands on that line.
  const glued = call();
  glued[0] = `0:000> ${glued[0]}`;
  const report = project(glued);
  assert.equal(report.status, "observed-failures");
  assert.equal(report.coverage.completedCalls, 1);
  assert.deepEqual(report.diagnostics, {
    firstRejection: null,
    promptJoined: 1,
  });
  // An echoed command after a prompt is never a marker.
  const echoed = [
    '0:000> .printf "OFOX_NT_RAW_V1 ENTER seq=%d END\\n", @$t0',
    ...call(),
  ];
  const plain = project(echoed);
  assert.equal(plain.status, "observed-failures");
  assert.equal(plain.diagnostics.promptJoined, 0);
  // A glued invalid marker is no longer silently ignored.
  const invalid = project([
    ...call(),
    "0:000> OFOX_NT_RAW_V1 INVALID nested END",
  ]);
  assert.equal(invalid.diagnostics.firstRejection, "invalid-nested");
  // Inside a bl listing a prompt line is skipped, never promoted to a marker.
  const listed = call().flatMap((line) =>
    line.startsWith("OFOX_NT_RAW_V1 RETURN_BP_END")
      ? [`0:000> ${line}`]
      : [line],
  );
  const inside = project(listed);
  assert.equal(inside.status, "inconclusive");
  assert.equal(inside.diagnostics.promptJoined, 0);
  // Double prompts and other-case markers stay visible as rejections.
  for (const extra of [
    "0:000> 0:000> OFOX_NT_RAW_V1 INVALID limit END",
    "0:000> ofox_nt_raw_v1 invalid nested end",
    "ofox_nt_raw_v1 invalid nested end",
  ])
    assert.equal(
      project([...call(), extra]).diagnostics.firstRejection,
      "marker-unknown",
      extra,
    );
});

test("an inconclusive projection names only its first fixed rejection", () => {
  const cases = [
    [call(), null],
    [[...call(), ...call()], "enter-order"],
    [
      call().map((l) => l.replace("tid=2b sp=1000", "tid=2c sp=1000")),
      "enter-identity",
    ],
    [call({ access: "1234" }), "enter-arguments"],
    [call({ api: 20, root: "none" }), "enter-arguments"],
    [call({ api: 0, root: "none" }), "enter-arguments"],
    [
      call().map((l) =>
        l.startsWith("OFOX_NT_RAW_V1 RETURN_BP_END")
          ? l.replace("seq=1", "seq=2")
          : l,
      ),
      "listing-row",
    ],
    [
      call().flatMap((l) => (l.startsWith("30 e ") ? [l, l] : [l])),
      "listing-row",
    ],
    [call().slice(1), "name-orphan"],
    [call().map((l) => l.replace("bytes=32", "bytes=1000")), "name-shape"],
    [call().filter((l) => !l.includes(" NAME seq=")), "listing-orphan"],
    [call().map((l) => l.replace("30 e ", "30 d ")), "listing-row"],
    [
      call()
        .filter((l) => !l.includes(" ENTER "))
        .slice(4),
      "return-orphan",
    ],
    [call().map((l) => l.replace("sp=1008", "sp=1010")), "return-mismatch"],
    [[...call(), "OFOX_NT_RAW_V1 INVALID nested END"], "invalid-nested"],
    [[...call(), "OFOX_NT_RAW_V1 INVALID limit END"], "invalid-limit"],
    [
      [...call(), "OFOX_NT_RAW_V1 INVALID return_pair END"],
      "invalid-return-pair",
    ],
    [
      [...call(), "OFOX_NT_RAW_V1 UNKNOWN private-secret END"],
      "marker-unknown",
    ],
    [call().slice(0, -1), "unterminated"],
    [[], "no-calls"],
  ];
  for (const [lines, expected] of cases) {
    const report = project(lines);
    assert.equal(report.diagnostics.firstRejection, expected, expected);
    assert.equal(report.status === "inconclusive", expected !== null);
    assert.ok(!JSON.stringify(report.diagnostics).includes("secret"));
  }
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
  ])
    assert.ok(!encoded.includes(secret), secret);
  assert.deepEqual(
    report.calls.map((row) => row.objectShape),
    [
      "\\Registry\\Machine\\SOFTWARE\\Microsoft",
      "\\Registry\\Machine\\?",
      "?",
      "\\Sessions\\?\\Windows\\ApiPort",
    ],
  );
});

test("lists every paired call in order with only vocabulary object shapes", () => {
  const unknownName = (lines) =>
    lines.map((line) =>
      line.includes(" NAME seq=")
        ? line.replace(/ NAME seq=(\d+) .*$/, " NAME_UNKNOWN seq=$1 END")
        : line,
    );
  const report = project([
    ...call({
      api: 1,
      access: "80000000",
      name: "\\Registry\\Machine\\software\\Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options\\node.exe",
      status: "c0000022",
    }),
    ...call({
      seq: 2,
      api: 1,
      access: "1",
      root: "0",
      name: "S-1-5-21-1-2-3-1001\\Session Manager",
      status: "0",
    }),
    ...call({
      seq: 3,
      api: 4,
      root: "none",
      name: "GlobalFlag",
      status: "c0000034",
    }),
    ...call({
      seq: 4,
      api: 5,
      access: "4",
      name: "\\KnownDlls\\KERNELBASE.dll",
      status: "0",
    }),
    ...call({ seq: 5, name: "\\Sessions\\2\\Windows\\ApiPort", status: "0" }),
    ...unknownName(
      call({ seq: 6, api: 1, access: "1", root: "2", status: "0" }),
    ),
    ...unknownName(call({ seq: 7, api: 12, root: "none", status: "0" })),
    ...call({
      seq: 8,
      api: 1,
      access: "1",
      name: "\\Registry".repeat(33),
      status: "0",
    }),
  ]);
  assert.equal(report.coverage.status, "complete-for-fixed-set");
  // No WSAStartup scope marker: every call is outside it.
  assert.ok(report.calls.every((row) => row.insideWsaStartup === false));
  assert.deepEqual(
    report.calls.map(({ insideWsaStartup, ...row }) => row),
    [
      {
        sequence: 1,
        api: "NtOpenKey",
        status: "0xc0000022",
        desiredAccess: 0x80000000,
        root: "absolute",
        objectShape:
          "\\Registry\\Machine\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options\\?",
      },
      {
        sequence: 2,
        api: "NtOpenKey",
        status: "0x00000000",
        desiredAccess: 1,
        root: "relative",
        // A user SID is a fixed token; the hive path after it stays visible.
        objectShape: "<user-sid>\\Session Manager",
      },
      {
        sequence: 3,
        api: "NtQueryValueKey",
        status: "0xc0000034",
        desiredAccess: null,
        root: null,
        objectShape: "GlobalFlag",
      },
      {
        sequence: 4,
        api: "NtOpenSection",
        status: "0x00000000",
        desiredAccess: 4,
        root: "absolute",
        objectShape: "\\KnownDlls\\kernelbase.dll",
      },
      {
        sequence: 5,
        api: "NtAlpcConnectPort",
        status: "0x00000000",
        desiredAccess: null,
        root: "absolute",
        objectShape: "\\Sessions\\?\\Windows\\ApiPort",
      },
      {
        sequence: 6,
        api: "NtOpenKey",
        status: "0x00000000",
        desiredAccess: 1,
        root: "unknown",
        objectShape: null,
      },
      {
        sequence: 7,
        api: "NtMapViewOfSection",
        status: "0x00000000",
        desiredAccess: null,
        root: null,
        objectShape: null,
      },
      {
        sequence: 8,
        api: "NtOpenKey",
        status: "0x00000000",
        desiredAccess: 1,
        root: "absolute",
        objectShape: "?overflow",
      },
    ],
  );
  const encoded = JSON.stringify(report);
  for (const secret of ["node.exe", "S-1-5", "1001", "software\\"])
    assert.ok(!encoded.includes(secret), secret);
});

test("object shapes never print text after an unknown component", () => {
  const shapes = (names) =>
    project(
      names.flatMap((name, index) =>
        call({ seq: index + 1, api: 7, access: "1", name, status: "0" }),
      ),
    ).calls.map((row) => row.objectShape);
  assert.deepEqual(
    shapes([
      "\\??\\C:\\Users\\Server\\AppData\\Local\\Temp\\x.docx",
      "\\Device\\Mup\\Server\\Default\\Config",
      "\\\u212AnownDlls\\ntdll.dll",
      "\\Sessions\\2\\BaseNamedObjects",
      "\\GLOBAL??\\?\\Windows",
      `${"\\Registry".repeat(31)}`,
    ]),
    [
      "\\??\\?\\?\\?\\?\\?\\?\\?",
      "\\Device\\?\\?\\?\\?",
      "\\?\\?",
      "\\Sessions\\?\\BaseNamedObjects",
      "\\GLOBAL??\\?\\?",
      "\\Registry".repeat(31),
    ],
  );
});

test("a later rejection keeps earlier paired calls but stays inconclusive", () => {
  const report = project([
    ...call({ status: "0" }),
    "OFOX_NT_RAW_V1 INVALID nested END",
  ]);
  assert.equal(report.status, "inconclusive");
  assert.equal(report.calls.length, 1);
  assert.equal(report.calls[0].status, "0x00000000");
});

test("does not attribute without the complete original Host Resume and independently verified installation", () => {
  for (const records of [
    setup.slice(1),
    setup.slice(0, -1),
    setup.map((line) => line.replace("count=19", "count=15")),
    setup.map((line) => line.replace("count=19", "count=18")),
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

test("pairs the documented DML breakpoint row with a thread-qualified column", () => {
  const dml = (lines) =>
    lines.map((line) =>
      line.startsWith("30 e ")
        ? `    ${line
            .replace("30 e ", "30 e Disable Clear  ")
            .replace(" 0001 (0001) 0:000 ", "     0001 (0001)  0:~000 ")}`
        : line,
    );
  const report = project(dml(call()));
  assert.equal(report.status, "observed-failures");
  assert.equal(report.coverage.completedCalls, 1);
  // A prompt-prefixed command echo is never a breakpoint row.
  const echoed = dml(call()).flatMap((line) =>
    line.startsWith("OFOX_NT_RAW_V1 RETURN_BP_BEGIN")
      ? [line, "0:000> bl /L 30"]
      : [line],
  );
  assert.equal(project(echoed).status, "observed-failures");
  const promptRow = dml(call()).map((line) =>
    line.includes(" Clear  00000000") ? `0:000> ${line.trimStart()}` : line,
  );
  assert.equal(project(promptRow).status, "inconclusive");
  for (const mutate of [
    (line) => line.replace(" e Disable Clear ", " d Enable Clear "),
    (line) => line.replace("Disable Clear", "Disable Private"),
    (line) => line.replace("0:~000", "0:~001"),
    (line) => line.replace("0:~000", "0:****"),
  ]) {
    const rejected = project(
      dml(call()).map((line) =>
        line.includes(" Clear  00000000") ? mutate(line) : line,
      ),
    );
    assert.equal(rejected.status, "inconclusive");
    assert.deepEqual(rejected.observations, []);
  }
});

test("ordinary startup misses do not hide the last failed operation", () => {
  const misses = Array.from({ length: 40 }, (_, index) =>
    call({
      seq: index + 1,
      api: 1,
      access: "20019",
      name: "\\Registry\\Machine\\private-missing",
      status: "c0000034",
    }),
  ).flat();
  const report = project([...misses, ...call({ seq: 41 })]);
  assert.equal(report.status, "observed-failures");
  assert.equal(report.coverage.status, "complete-for-fixed-set");
  assert.equal(report.coverage.completedCalls, 41);
  assert.equal(report.observations.length, 41);
  assert.deepEqual(report.observations.at(-1), {
    sequence: 41,
    api: "NtAlpcConnectPort",
    status: "0xc0000022",
    desiredAccess: null,
    objectAlias: "csr-api-port",
  });
  const calls = (length) =>
    Array.from({ length }, (_, index) =>
      call({ seq: index + 1, status: "0" }),
    ).flat();
  assert.equal(project(calls(1024)).status, "observed-operations");
  assert.equal(project(calls(1025)).status, "inconclusive");
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
    call({ root: "2" }),
    call({ api: 4, root: "2" }),
    call({ api: 13, root: "2" }),
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
  const calls = [13, 14, 15, 16, 17, 18, 19].flatMap((api, index) =>
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
      // Status-only: a blocked conhost launch shows here without its arguments.
      {
        api: "NtCreateUserProcess",
        desiredAccess: null,
        objectAlias: "unknown",
      },
      // Status-only key operations on an already open handle.
      ...[
        "NtEnumerateKey",
        "NtQueryKey",
        "NtNotifyChangeKey",
        "NtNotifyChangeMultipleKeys",
      ].map((api) => ({ api, desiredAccess: null, objectAlias: "unknown" })),
    ],
  );
});

test("an unreadable optional object name still pairs its real operation result", () => {
  const lines = call({ api: 1, access: "20019", root: "2" }).map((line) =>
    line.includes(" NAME seq=")
      ? "OFOX_NT_RAW_V1 NAME_UNKNOWN seq=1 END"
      : line,
  );
  const report = project(lines);
  assert.equal(report.status, "observed-failures");
  assert.equal(report.observations[0].objectAlias, "unknown");
  assert.equal(report.observations[0].desiredAccess, 0x20019);
});

test("every operation setup stage survives the debugger-state allowlist", () => {
  for (const stage of [
    "operation_architecture_unverified",
    "operation_layout_unverified",
    "operation_thread_unverified",
    "operation_install_unverified",
    "operation_target_invalid",
  ])
    assert.equal(safeDebuggerState({ stage }).stage, stage);
  assert.equal(safeDebuggerState({ stage: "private-stage" }).stage, "unknown");
});

test("setup failure state retains only fixed booleans and a bounded breakpoint count", () => {
  const state = safeDebuggerState({
    operationSetup: {
      abiVerified: true,
      threadMatched: false,
      installationVerified: "private-secret",
      breakpointRows: 15,
      address: "secret-pointer",
      raw: "secret-value",
    },
  });
  assert.deepEqual(state.operationSetup, {
    abiVerified: true,
    threadMatched: false,
    installationVerified: null,
    breakpointRows: 15,
  });
  assert.ok(!JSON.stringify(state).includes("secret"));
  for (const count of [-1, 20, "15", null])
    assert.equal(
      safeDebuggerState({ operationSetup: { breakpointRows: count } })
        .operationSetup.breakpointRows,
      null,
    );
});

const scopeEnter = ({ seq = 1, tid = "2b", sp = "3000" } = {}) =>
  `OFOX_NT_RAW_V1 SCOPE_ENTER seq=${seq} pid=2a tid=${tid} sp=${sp} ret=4000 END`;
const scopeReturn = ({
  seq = 3,
  tid = "2b",
  sp = "3008",
  result = "277b",
} = {}) =>
  `OFOX_NT_RAW_V1 SCOPE_RETURN seq=${seq} pid=2a tid=${tid} sp=${sp} result=${result} END`;
const catalogCalls = () => [
  ...call({
    seq: 2,
    api: 1,
    access: "20019",
    root: "0",
    name: "SYSTEM\\CurrentControlSet\\Services\\WinSock2\\Parameters",
    status: "0",
  }),
  ...call({
    seq: 3,
    api: 1,
    access: "20019",
    root: "0",
    name: "Protocol_Catalog9",
    status: "c0000022",
  }),
];

test("marks the calls made inside WSAStartup and keeps its result code", () => {
  const report = project([
    ...call({ status: "0" }),
    scopeEnter(),
    ...catalogCalls(),
    scopeReturn(),
    ...call({ seq: 4, status: "0" }),
  ]);
  assert.equal(report.status, "observed-failures");
  assert.equal(report.coverage.status, "complete-for-fixed-set");
  assert.deepEqual(report.winsockStartup, {
    status: "returned",
    callsBeforeEntry: 1,
    callsAtReturn: 3,
    result: 10107,
  });
  assert.deepEqual(
    report.calls.map((row) => row.insideWsaStartup),
    [false, true, true, false],
  );
  assert.deepEqual(
    report.calls.slice(1, 3).map((row) => row.objectShape),
    [
      "SYSTEM\\CurrentControlSet\\Services\\WinSock2\\Parameters",
      "Protocol_Catalog9",
    ],
  );
  // An entry without its paired return keeps every later call inside.
  const open = project([
    ...call({ status: "0" }),
    scopeEnter(),
    ...catalogCalls(),
  ]);
  assert.equal(open.coverage.status, "complete-for-fixed-set");
  assert.deepEqual(open.winsockStartup, {
    status: "entered",
    callsBeforeEntry: 1,
    callsAtReturn: null,
    result: null,
  });
  assert.deepEqual(
    open.calls.map((row) => row.insideWsaStartup),
    [false, true, true],
  );
  // The first marker after the waiting prompt may be a scope marker.
  const glued = project([
    ...call({ status: "0" }),
    `0:000> ${scopeEnter()}`,
    ...catalogCalls(),
    scopeReturn(),
  ]);
  assert.equal(glued.winsockStartup.status, "returned");
  assert.equal(glued.diagnostics.promptJoined, 1);
  // A scope that never resolved leaves the full trace intact.
  const unseen = project([...call({ status: "0" }), ...catalogCalls()]);
  assert.equal(unseen.coverage.status, "complete-for-fixed-set");
  assert.deepEqual(unseen.winsockStartup, {
    status: "not-seen",
    callsBeforeEntry: null,
    callsAtReturn: null,
    result: null,
  });
  assert.ok(unseen.calls.every((row) => row.insideWsaStartup === false));
});

test("an unpaired or misplaced WSAStartup scope stays inconclusive", () => {
  const first = call({ status: "0" });
  const cases = [
    [[...first, ...catalogCalls(), scopeReturn()], "scope-order"],
    [[...first, scopeEnter(), scopeEnter(), ...catalogCalls()], "scope-order"],
    [[...first, scopeEnter({ seq: 5 }), ...catalogCalls()], "scope-order"],
    [[...first, scopeEnter({ tid: "2c" }), ...catalogCalls()], "scope-order"],
    [[first[0], scopeEnter({ seq: 0 }), ...first.slice(1)], "scope-order"],
    [
      [...first, scopeEnter(), ...catalogCalls(), scopeReturn({ sp: "3010" })],
      "scope-order",
    ],
    [
      [...first, scopeEnter(), ...catalogCalls(), scopeReturn({ seq: 2 })],
      "scope-order",
    ],
    [
      [...first, scopeEnter(), ...catalogCalls(), scopeReturn(), scopeReturn()],
      "scope-order",
    ],
    // A scope marker can never stand in for a breakpoint listing row.
    [
      first.flatMap((line) =>
        line.startsWith("30 e ") ? [line, scopeEnter({ seq: 0 })] : [line],
      ),
      "listing-row",
    ],
    [[...first, "OFOX_NT_RAW_V1 INVALID scope END"], "invalid-scope"],
    [[...first, "OFOX_NT_RAW_V1 INVALID scope_pair END"], "invalid-scope-pair"],
  ];
  for (const [lines, expected] of cases) {
    const report = project(lines);
    assert.equal(report.status, "inconclusive", expected);
    assert.equal(report.diagnostics.firstRejection, expected, expected);
  }
});

test("hive, package and catalog shapes stay visible while SIDs and entry numbers are masked", () => {
  const names = [
    "\\REGISTRY\\USER\\S-1-5-21-1111111111-2222222222-3333333333-1001\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
    "\\REGISTRY\\USER\\S-1-5-21-1-2-3-1001_Classes\\Local Settings",
    "\\Sessions\\1\\AppContainerNamedObjects\\S-1-15-2-1-2-3-4-5-6-7",
    "\\REGISTRY\\MACHINE\\SYSTEM\\CurrentControlSet\\Services\\WinSock2\\Parameters\\Protocol_Catalog9\\Catalog_Entries64\\000000000001",
    "\\REGISTRY\\MACHINE\\SYSTEM\\CurrentControlSet\\Services\\WinSock2\\Parameters\\NameSpace_Catalog5\\Catalog_Entries64\\000000000007",
    "\\REGISTRY\\MACHINE\\SYSTEM\\CurrentControlSet\\Services\\WinSock2\\Parameters\\AppId_Catalog\\0F1D2A3B",
    "\\REGISTRY\\MACHINE\\SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\\Winsock",
    "\\REGISTRY\\USER\\S-1-5-21-1-2-3-1001-9\\Software",
    "\\REGISTRY\\USER\\S-1-5-21-1-2-3\\Software",
    "\\Sessions\\1\\AppContainerNamedObjects\\S-1-15-3-1024-1-2-3-4-5-6-7-8\\Windows",
    "\\REGISTRY\\MACHINE\\1234567890123\\SYSTEM",
  ];
  const report = project(
    names.flatMap((name, index) =>
      call({ seq: index + 1, api: 1, access: "20019", name, status: "0" }),
    ),
  );
  assert.deepEqual(
    report.calls.map((row) => row.objectShape),
    [
      "\\Registry\\User\\<user-sid>\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
      "\\Registry\\User\\<user-sid>_Classes\\Local Settings",
      "\\Sessions\\?\\AppContainerNamedObjects\\<package-sid>",
      "\\Registry\\Machine\\SYSTEM\\CurrentControlSet\\Services\\WinSock2\\Parameters\\Protocol_Catalog9\\Catalog_Entries64\\?",
      "\\Registry\\Machine\\SYSTEM\\CurrentControlSet\\Services\\WinSock2\\Parameters\\NameSpace_Catalog5\\Catalog_Entries64\\?",
      "\\Registry\\Machine\\SYSTEM\\CurrentControlSet\\Services\\WinSock2\\Parameters\\AppId_Catalog\\?",
      "\\Registry\\Machine\\SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\\Winsock",
      "\\Registry\\User\\?\\?",
      "\\Registry\\User\\?\\?",
      "\\Sessions\\?\\AppContainerNamedObjects\\?\\?",
      "\\Registry\\Machine\\?\\?",
    ],
  );
  const values = project(
    [
      "PackedCatalogItem",
      "Num_Catalog_Entries64",
      "Serial_Access_Num",
      "private-value",
    ].flatMap((name, index) =>
      call({ seq: index + 1, api: 4, root: "none", name, status: "0" }),
    ),
  );
  assert.deepEqual(
    values.calls.map((row) => row.objectShape),
    ["PackedCatalogItem", "Num_Catalog_Entries64", "Serial_Access_Num", "?"],
  );
  const encoded = JSON.stringify([report, values]);
  for (const secret of [
    "S-1-",
    "1001",
    "1111111111",
    "0F1D2A3B",
    "000000000001",
    "private-value",
  ])
    assert.ok(!encoded.includes(secret), secret);
});

test("debugger exception events and stops are reported as closed-set positions", () => {
  const report = project([
    ...call({ status: "0" }),
    "(2a.2b): Invalid handle - code c0000008 (first chance)",
    "First chance exceptions are reported before any exception handling.",
    "(2a.2c): Access violation - code c0000005 (first chance)",
    "(99.2b): Access violation - code c0000005 (first chance)",
    ...call({ seq: 2, status: "0" }),
    "(2a.2b): Private secret text - code e06d7363 (!!! second chance !!!)",
    "0:000> ",
  ]);
  assert.equal(report.coverage.status, "complete-for-fixed-set");
  assert.deepEqual(report.debuggerEvents, [
    {
      afterCall: 1,
      activeCall: null,
      code: "0xc0000008",
      chance: "first",
      primaryThread: true,
    },
    {
      afterCall: 1,
      activeCall: null,
      code: "0xc0000005",
      chance: "first",
      primaryThread: false,
    },
    {
      afterCall: 2,
      activeCall: null,
      code: "0xe06d7363",
      chance: "second",
      primaryThread: true,
    },
  ]);
  assert.deepEqual(report.debuggerStops, [
    { afterCall: 2, activeCall: null, primaryThread: true },
  ]);
  assert.equal(report.debuggerEventsTotal, 3);
  const encoded = JSON.stringify(report);
  for (const secret of ["Private", "secret", "Invalid handle", "99"])
    assert.ok(!encoded.includes(secret), secret);
  // The first output after the waiting prompt may be an exception line, and
  // a stop on another thread names only that it was not the primary one.
  const glued = project([
    "0:000> (2a.2b): Invalid handle - code c0000008 (first chance)",
    ...call({ status: "0" }),
    "0:003> ",
  ]);
  assert.equal(glued.debuggerEvents.length, 1);
  assert.equal(glued.debuggerEvents[0].afterCall, 0);
  assert.deepEqual(glued.debuggerStops, [
    { afterCall: 1, activeCall: null, primaryThread: false },
  ]);
  // Events are bounded; the total still counts every one.
  const many = project([
    ...call({ status: "0" }),
    ...Array.from(
      { length: 40 },
      () => "(2a.2b): Invalid handle - code c0000008 (first chance)",
    ),
  ]);
  assert.equal(many.debuggerEvents.length, 32);
  assert.equal(many.debuggerEventsTotal, 40);
  // Without any event the fields are empty, never missing.
  const quiet = project(call({ status: "0" }));
  assert.deepEqual(quiet.debuggerEvents, []);
  assert.deepEqual(quiet.debuggerStops, []);
});
