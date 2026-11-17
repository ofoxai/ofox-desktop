// Fixed development-only observations; never an admission or causal verdict.
import { projectDocumentLoaderSnaps } from "./document-loader-snaps.mjs";

const APIS = [
  ["NtOpenKey", "attributes"],
  ["NtOpenKeyEx", "attributes"],
  ["NtCreateKey", "attributes"],
  ["NtQueryValueKey", "value"],
  ["NtOpenSection", "attributes"],
  ["NtOpenFile", "attributes"],
  ["NtCreateFile", "attributes"],
  ["NtOpenDirectoryObject", "attributes"],
  ["NtOpenEvent", "attributes"],
  ["NtAlpcConnectPort", "port"],
  ["NtConnectPort", "none"],
  ["NtMapViewOfSection", "none"],
  ["NtAlpcConnectPortEx", "none"],
  ["NtSecureConnectPort", "none"],
];
const INSTALL =
  /^OFOX_NT_V1 INSTALLED pid=([a-f\d]{1,8}) tid=([a-f\d]{1,8}) thread=(\d{1,3}) count=14 END$/i;
const ENTER =
  /^OFOX_NT_RAW_V1 ENTER seq=(\d{1,3}) api=(\d{1,2}) pid=([a-f\d]{1,8}) tid=([a-f\d]{1,8}) sp=([a-f\d]{1,16}) ret=([a-f\d]{1,16}) access=(none|[a-f\d]{1,8}) root=(none|0|1) END$/i;
const NAME =
  /^OFOX_NT_RAW_V1 NAME seq=(\d{1,3}) bytes=(\d{1,4}) value="([^\r\n\0]*)" END$/;
const RETURN =
  /^OFOX_NT_RAW_V1 RETURN seq=(\d{1,3}) pid=([a-f\d]{1,8}) tid=([a-f\d]{1,8}) sp=([a-f\d]{1,16}) status=([a-f\d]{1,8}) END$/i;
const BP =
  /^\s*30 e ([a-f\d]{8}`[a-f\d]{8}|[a-f\d]{8,16})\s+0*1\s+\(0*1\)\s+0:(\d{3})\s+[^\r\n]*$/i;
const hex = (value) => Number.parseInt(value, 16);
const pointer = (value) => BigInt(`0x${value.replaceAll("`", "")}`);
function objectAlias(name, root) {
  if (root !== "1" || typeof name !== "string") return "unknown";
  if (/^\\(?:Sessions\\\d{1,10}\\)?Windows\\ApiPort$/i.test(name))
    return "csr-api-port";
  if (/^\\Registry\\Machine(?:\\|$)/i.test(name)) return "machine-registry";
  const dll = /^\\KnownDlls\\(ntdll|kernelbase|kernel32)\.dll$/i.exec(name);
  return dll ? `known-dll-${dll[1].toLowerCase()}` : "unknown";
}

export function projectDocumentNativeOperations({ text, target } = {}) {
  const report = {
    schemaVersion: 1,
    scope: "fixed-windows-startup-operations",
    releaseGate: false,
    debuggerAssisted: true,
    status: "inconclusive",
    cause: "undetermined",
    negativeConclusionAllowed: false,
    coverage: {
      status: "incomplete",
      completedCalls: 0,
      fixedApis: APIS.length,
    },
    observations: [],
    reasons: [],
  };
  // Reuse the complete original V2 proof; operation markers cannot replace it.
  const loader = projectDocumentLoaderSnaps({ text, target });
  if (loader.handshake.status !== "matched") {
    report.reasons.push("original-host-proof-unverified");
    return report;
  }
  const lines = text.split(/\r?\n/);
  const installs = lines.flatMap((line, index) =>
    line.startsWith("OFOX_NT_V1 ")
      ? [{ match: INSTALL.exec(line), index }]
      : [],
  );
  const installed = installs[0]?.match;
  if (
    installs.length !== 1 ||
    !installed ||
    hex(installed[1]) !== target.childPid ||
    hex(installed[2]) !== target.primaryTid ||
    installs[0].index >= lines.indexOf("OFOX_CDB_V2 READY END")
  ) {
    report.reasons.push("installation-unverified");
    return report;
  }
  const threadIndex = Number(installed[3]);
  let active = null,
    next = 1,
    listing = null,
    incomplete = false;
  const reject = () => {
    incomplete = true;
  };
  const same = (pid, tid) =>
    hex(pid) === target.childPid && hex(tid) === target.primaryTid;
  for (const line of lines.slice(
    lines.indexOf("OFOX_CDB_V2 CONTINUE_REQUESTED END") + 1,
  )) {
    const endList = /^OFOX_NT_RAW_V1 RETURN_BP_END seq=(\d{1,3}) END$/.exec(
      line,
    );
    if (listing) {
      if (endList) {
        if (
          !active ||
          Number(endList[1]) !== active.sequence ||
          listing.length !== 1
        )
          reject();
        else {
          const row = BP.exec(listing[0]);
          if (
            !row ||
            pointer(row[1]) !== active.returnAddress ||
            Number(row[2]) !== threadIndex
          )
            reject();
          else active.breakpointVerified = true;
        }
        listing = null;
      } else if (line.trim()) listing.push(line);
      continue;
    }
    const entry = ENTER.exec(line);
    if (entry) {
      const sequence = Number(entry[1]),
        api = APIS[Number(entry[2]) - 1];
      if (
        active ||
        sequence !== next ||
        sequence > 256 ||
        !api ||
        !same(entry[3], entry[4])
      ) {
        reject();
        continue;
      }
      const mode = api[1],
        access = entry[7].toLowerCase(),
        root = entry[8];
      if (
        (mode === "attributes" && (access === "none" || root === "none")) ||
        (mode !== "attributes" && access !== "none") ||
        (mode === "port" && root !== "1") ||
        (["none", "value"].includes(mode) && root !== "none")
      ) {
        reject();
        continue;
      }
      active = {
        sequence,
        api: api[0],
        mode,
        stack: pointer(entry[5]),
        returnAddress: pointer(entry[6]),
        access: access === "none" ? null : hex(access),
        root,
        nameSeen: false,
        name: null,
        breakpointVerified: false,
      };
      next++;
      continue;
    }
    const name = NAME.exec(line),
      unknown = /^OFOX_NT_RAW_V1 NAME_UNKNOWN seq=(\d{1,3}) END$/.exec(line);
    if (name || unknown) {
      const match = name ?? unknown;
      if (!active || Number(match[1]) !== active.sequence || active.nameSeen) {
        reject();
        continue;
      }
      active.nameSeen = true;
      if (name) {
        const bytes = Number(name[2]);
        if (
          active.mode === "none" ||
          bytes > 1024 ||
          bytes % 2 ||
          bytes !== Buffer.byteLength(name[3], "utf16le") ||
          /[\x00-\x1f]/.test(name[3])
        )
          reject();
        else active.name = name[3];
      }
      continue;
    }
    const begin = /^OFOX_NT_RAW_V1 RETURN_BP_BEGIN seq=(\d{1,3}) END$/.exec(
      line,
    );
    if (begin) {
      if (
        !active ||
        Number(begin[1]) !== active.sequence ||
        !active.nameSeen ||
        active.breakpointVerified
      )
        reject();
      listing = [];
      continue;
    }
    const returned = RETURN.exec(line);
    if (returned) {
      if (
        !active ||
        Number(returned[1]) !== active.sequence ||
        !same(returned[2], returned[3]) ||
        pointer(returned[4]) !== active.stack + 8n ||
        !active.nameSeen ||
        !active.breakpointVerified
      ) {
        reject();
        continue;
      }
      const status = hex(returned[5]);
      report.coverage.completedCalls++;
      if ((status & 0x80000000) !== 0) {
        if (report.observations.length === 32) reject();
        else
          report.observations.push({
            sequence: active.sequence,
            api: active.api,
            status: `0x${status.toString(16).padStart(8, "0")}`,
            desiredAccess: active.access,
            objectAlias: objectAlias(active.name, active.root),
          });
      }
      active = null;
      continue;
    }
    if (line.startsWith("OFOX_NT_RAW_V")) reject();
  }
  if (active || listing || !report.coverage.completedCalls) reject();
  if (incomplete) report.reasons.push("operation-coverage-incomplete");
  else {
    report.coverage.status = "complete-for-fixed-set";
    report.status = report.observations.length
      ? "observed-failures"
      : "observed-operations";
  }
  return report;
}
