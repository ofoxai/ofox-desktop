// Development-only projection. Raw debugger text remains in the private capture.
// Loader wording is an explicitly supported synthetic grammar, not an SDK ABI.
const LIMITS = {
  bytes: 4 * 1024 * 1024,
  // Follows the byte bound (16-byte lines): run 44228529933's 66114 short
  // exception-notice lines fit in 4 MiB but exceeded a 50000-line bound.
  lines: 262_144,
  line: 16_384,
  observations: 32,
};
const MARKER =
  /^OFOX_CDB_V2 (BEFORE|AFTER) pid=([a-f\d]{1,8}) tid=([a-f\d]{1,8}) flags=([a-f\d]{1,8}) END$/i;
const PRE_HOLD =
  /^OFOX_CDB_V2 PRE_ATTACH_HOLD pid=([a-f\d]{1,8}) tid=([a-f\d]{1,8}) suspend=1 paired=2 END$/i;
const HOST_RESUMED =
  /^OFOX_CDB_V2 HOST_RESUMED pid=([a-f\d]{1,8}) tid=([a-f\d]{1,8}) hold=1 paired=2 previous=1 END$/i;
const INIT =
  /^([a-f\d]{1,8}):([a-f\d]{1,8}) @ \d{1,20} - LdrpInitializeNode - ERROR: Init routine [a-f\d]{8,16} for DLL "([^"\r\n]+)" failed during DLL_PROCESS_ATTACH$/i;
const SUBSYSTEM =
  /^([a-f\d]{1,8}):([a-f\d]{1,8}) @ \d{1,20} - LdrpInitializeProcess - ERROR: Loading Windows subsystem DLL "(ntdll\.dll|kernel32\.dll|kernelbase\.dll)" failed with status (0x[a-f\d]{8})$/i;

const processId = (value) =>
  Number.isInteger(value) && value > 0 && value <= 0xffffffff;
const hex = (value) => Number.parseInt(value, 16);
function lexical(value) {
  if (typeof value !== "string" || value.includes("\0")) return null;
  const normalized = value
    .replaceAll("/", "\\")
    .toLowerCase()
    .replace(/\\+$/, "");
  if (normalized.split("\\").some((part) => part === "." || part === ".."))
    return null;
  return normalized;
}
function moduleProjection(value, roots) {
  const normalized = lexical(value);
  for (const alias of ["windows", "runtime"]) {
    const values =
      typeof roots?.[alias] === "string" ? [roots[alias]] : roots?.[alias];
    if (!Array.isArray(values)) continue;
    for (const entry of values.slice(0, 8)) {
      const root = lexical(entry);
      if (!root || !normalized?.startsWith(`${root}\\`)) continue;
      const relative = normalized.slice(root.length + 1);
      const system =
        /^(?:system32|syswow64)\\((?:ntdll|kernel32|kernelbase)\.dll)$/.exec(
          relative,
        );
      const runtime = /^(node\.exe|officecli\.exe)$/.exec(relative);
      return {
        rootAlias: alias,
        module:
          alias === "windows" ? (system?.[1] ?? null) : (runtime?.[1] ?? null),
      };
    }
  }
  return { rootAlias: "unknown", module: null };
}

export function projectDocumentLoaderSnaps({ text, target, roots } = {}) {
  const report = {
    schemaVersion: 2,
    scope: "windows-node-loader-diagnostic",
    releaseGate: false,
    debuggerAssisted: true,
    status: "inconclusive",
    cause: "undetermined",
    negativeConclusionAllowed: false,
    handshake: { status: "unknown" },
    observations: [],
    unknownLines: 0,
    reasons: [],
  };
  if (!processId(target?.childPid) || !processId(target?.primaryTid)) {
    report.reasons.push("target-unknown");
    return report;
  }
  if (
    typeof text !== "string" ||
    Buffer.byteLength(text, "utf8") > LIMITS.bytes
  ) {
    report.reasons.push("input-unavailable-or-limit");
    return report;
  }
  const lines = text.split(/\r?\n/);
  if (
    lines.length > LIMITS.lines ||
    lines.some((line) => line.length > LIMITS.line)
  ) {
    report.reasons.push("input-limit");
    return report;
  }
  let stage = 0,
    invalid = false,
    before = null,
    after = null,
    continueAt = -1;
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index];
    const marker = MARKER.exec(line);
    if (marker) {
      if (
        marker[1].toUpperCase() !==
        (stage === 0 ? "BEFORE" : stage === 1 ? "AFTER" : "")
      )
        invalid = true;
      const values = {
        childPid: hex(marker[2]),
        primaryTid: hex(marker[3]),
        flags: hex(marker[4]),
      };
      if (
        values.childPid !== target.childPid ||
        values.primaryTid !== target.primaryTid
      )
        invalid = true;
      if (stage === 0) before = values;
      else if (stage === 1) after = values;
      stage++;
    } else if (line === "OFOX_CDB_V2 READY END") {
      if (stage !== 2) invalid = true;
      stage++;
    } else if (PRE_HOLD.test(line)) {
      const values = PRE_HOLD.exec(line);
      if (
        stage !== 3 ||
        hex(values[1]) !== target.childPid ||
        hex(values[2]) !== target.primaryTid
      )
        invalid = true;
      stage = 4;
    } else if (line === "OFOX_CDB_V2 CONTINUE_REQUESTED END") {
      if (stage !== 4) invalid = true;
      continueAt = index;
      stage = 5;
    } else if (HOST_RESUMED.test(line)) {
      const values = HOST_RESUMED.exec(line);
      if (
        stage !== 5 ||
        hex(values[1]) !== target.childPid ||
        hex(values[2]) !== target.primaryTid
      )
        invalid = true;
      stage = 6;
    } else if (line.startsWith("OFOX_CDB_V")) invalid = true;
  }
  if (
    invalid ||
    stage !== 6 ||
    continueAt < 0 ||
    !before ||
    !after ||
    BigInt(after.flags) !== (BigInt(before.flags) | 2n)
  ) {
    report.reasons.push("handshake-unverified");
    return report;
  }
  report.handshake = {
    status: "matched",
    before: { flags: before.flags },
    after: { flags: after.flags },
    ready: true,
    preAttachHold: { suspendPrevious: 1, pairedResumePrevious: 2 },
    finalHold: { suspendPrevious: 1, pairedResumePrevious: 2 },
    hostResumePrevious: 1,
    continueRequested: true,
  };
  // The actual target can fail before the Host ACK reaches the controller.
  // Require the complete global proof, then retain only post-continue output;
  // ACK placement is not an event timestamp and cannot discard that failure.
  for (const line of lines.slice(continueAt + 1)) {
    if (!line || line.startsWith("OFOX_CDB_V2 ")) continue;
    const init = INIT.exec(line),
      subsystem = SUBSYSTEM.exec(line);
    const match = init ?? subsystem;
    if (!match) {
      report.unknownLines++;
      continue;
    }
    if (
      hex(match[1]) !== target.childPid ||
      hex(match[2]) !== target.primaryTid
    )
      continue;
    if (report.observations.length === LIMITS.observations) {
      if (!report.reasons.includes("observation-limit"))
        report.reasons.push("observation-limit");
      continue;
    }
    report.observations.push({
      kind: init ? "dll-init-failure" : "subsystem-load-failure",
      ...(init
        ? moduleProjection(match[3], roots)
        : { rootAlias: "windows-subsystem", module: match[3].toLowerCase() }),
      status: init ? null : match[4].toLowerCase(),
    });
  }
  if (report.observations.length) report.status = "observed-failures";
  else report.reasons.push("no-supported-loader-failure");
  return report;
}
