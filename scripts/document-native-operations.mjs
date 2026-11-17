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
// Matches the debugger script's 0n256 call limit. Each failure belongs to one
// unique sequence, so the call bound also bounds retained observations; ordinary
// startup misses must not evict the final failed operation.
const MAX_CALLS = 256;
const INSTALL =
  /^OFOX_NT_V1 INSTALLED pid=([a-f\d]{1,8}) tid=([a-f\d]{1,8}) thread=(\d{1,3}) count=14 arch=amd64 abi=windows-x64 END$/i;
const ENTER =
  /^OFOX_NT_RAW_V1 ENTER seq=(\d{1,3}) api=(\d{1,2}) pid=([a-f\d]{1,8}) tid=([a-f\d]{1,8}) sp=([a-f\d]{1,16}) ret=([a-f\d]{1,16}) access=(none|[a-f\d]{1,8}) root=(none|0|1|2) END$/i;
const NAME =
  /^OFOX_NT_RAW_V1 NAME seq=(\d{1,3}) bytes=(\d{1,4}) value="([^\r\n\0]*)" END$/;
const RETURN =
  /^OFOX_NT_RAW_V1 RETURN seq=(\d{1,3}) pid=([a-f\d]{1,8}) tid=([a-f\d]{1,8}) sp=([a-f\d]{1,16}) status=([a-f\d]{1,8}) END$/i;
// A debugger prompt echo (for example `0:000> bl /L 30`) is never a row.
const PROMPT = /^(?:\d{1,3}:)?\d{1,3}:[a-f\d]{3}>/i;
// After -cf, CDB prints "0:000> " without a newline while it waits for the
// controller's g; the first output after g lands on that line (native runs
// 44020847288 and 44049635186 each show exactly one extra prompt line). Only
// our own marker text directly after the prompt is recovered.
const GLUED = /^(?:\d{1,3}:)?\d{1,3}:[a-f\d]{3}> (?=OFOX_NT_RAW_V1 )/i;
const UNMATCHED = /^(?:(?:\d{1,3}:)?\d{1,3}:[a-f\d]{3}>\s*)*OFOX_NT_RAW_V/i;
const INVALID = {
  "OFOX_NT_RAW_V1 INVALID nested END": "invalid-nested",
  "OFOX_NT_RAW_V1 INVALID limit END": "invalid-limit",
  "OFOX_NT_RAW_V1 INVALID return_pair END": "invalid-return-pair",
};
// Same row grammar as the setup check: optional DML action text and ~thread.
const BP =
  /^\s*30 e (?:Disable Clear\s+)?([a-f\d]{8}`[a-f\d]{8}|[a-f\d]{8,16})\s+0*1\s+\(0*1\)\s+0:~?(\d{3})\s+[^\r\n]*$/i;
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
// Public Windows object-manager, registry and loader names seen at process
// startup. Only an exact (case-insensitive) component is emitted, in this
// spelling; user names, SIDs, numbers, image names and anything else become "?".
// UNC (Mup) is deliberately absent so a host name is always masked.
const VOCABULARY = new Map(
  [
    // Object manager, devices and known DLL sections.
    "Registry",
    "Machine",
    "User",
    "KnownDlls",
    "KnownDlls32",
    "Sessions",
    "BaseNamedObjects",
    "AppContainerNamedObjects",
    "Windows",
    "ApiPort",
    "SbApiPort",
    "WindowStations",
    "Device",
    "ConDrv",
    "Connect",
    "Server",
    "Reference",
    "Input",
    "Output",
    "CurrentIn",
    "CurrentOut",
    "KernelConnect",
    "RPC Control",
    "GLOBAL??",
    "??",
    "Global",
    "Local",
    "Session",
    "Callback",
    "DosDevices",

    "BNOLINKS",
    "NamedPipe",
    "Afd",
    "System32",
    "SysWOW64",
    "ntdll.dll",
    "kernel32.dll",
    "kernelbase.dll",
    "apphelp.dll",
    "user32.dll",
    "win32u.dll",
    "gdi32.dll",
    "gdi32full.dll",
    "advapi32.dll",
    "msvcrt.dll",
    "sechost.dll",
    "rpcrt4.dll",
    "bcrypt.dll",
    "ucrtbase.dll",
    "ws2_32.dll",
    "combase.dll",
    "ole32.dll",
    "oleaut32.dll",
    "shell32.dll",
    "shlwapi.dll",
    "crypt32.dll",
    "dbghelp.dll",
    "iphlpapi.dll",
    "psapi.dll",
    "userenv.dll",
    "winmm.dll",
    "powrprof.dll",
    "dnsapi.dll",
    "imm32.dll",
    "bcryptprimitives.dll",
    "kernel.appcore.dll",
    "msvcp_win.dll",
    "cfgmgr32.dll",
    "netapi32.dll",
    "nsi.dll",
    "mswsock.dll",
    "cryptbase.dll",
    "wldp.dll",
    "umpdc.dll",
    "profapi.dll",
    "ntmarta.dll",
    "wintrust.dll",
    // Registry keys.
    "SOFTWARE",
    "SYSTEM",
    "CurrentControlSet",
    "ControlSet001",
    "Control",
    "Session Manager",
    "Segment Heap",
    "Kernel",
    "Memory Management",
    "Executive",
    "Environment",

    "DOS Devices",
    "SubSystems",
    "ApiSetSchemaExtensions",
    "AppCompatCache",
    "Microsoft",
    "Windows NT",
    "CurrentVersion",
    "Image File Execution Options",
    "SideBySide",
    "Winners",
    "Policies",
    "Safer",
    "CodeIdentifiers",
    "Nls",
    "Sorting",
    "Versions",
    "Ids",
    "CustomLocale",
    "ExtendedLocale",
    "Locale",
    "Language",
    "Language Groups",
    "CodePage",
    "Normalization",
    "Terminal Server",
    "AppModel",
    "StateRepository",
    "PackageRepository",
    "AppContainer",
    "Mappings",
    "Storage",
    "Classes",
    "Local Settings",
    "StateSeparation",
    "RedirectionMap",
    "Keys",
    "Wow64",
    "WOW6432Node",
    "GRE_Initialize",
    "FontLink",
    "SystemLink",
    "FontSubstitutes",
    "LanguagePack",
    "MUI",
    "UILanguages",
    "Settings",
    "SafeBoot",
    "Option",
    "Lsa",
    "FipsAlgorithmPolicy",
    "Security",
    "WMI",
    "Power",
    "ProductOptions",
    "Services",
    "Winlogon",
    "Diagnostics",
    "AeDebug",
    "Windows Error Reporting",
    "Hardware Profiles",
    "Current",
    "Cryptography",
    "Configuration",
    "OLE",
    "Rpc",
    "COM3",
    "Internet Settings",
    "Explorer",
    "AppCompatFlags",
    "Layers",
    "Custom",
    "InstalledSDB",
    "CI",
    "Config",
    "DeviceGuard",
    "Scenarios",
    "WindowsRuntime",
    "ActivatableClassId",
    "Group Policy",
    "SrpV2",
    "AppLocker",
    "Instrumentation",

    "Autologger",
    "Console",
    "ConsoleHost",
    "Delegation",
    "Interactive Session",
    "Profiles",
    "Capabilities",
    "Appx",
    "PackageList",
    "MitigationOptions",
    // Registry value names.
    "GlobalFlag",
    "GlobalFlag2",
    "TransparentEnabled",
    "CWDIllegalInDllSearch",
    "SafeDllSearchMode",
    "SafeProcessSearchMode",
    "TSAppCompat",
    "TSUserEnabled",
    "ResourcePolicies",
    "MitigationAuditOptions",
    "UseFilter",
    "FilterFullPath",
    "DevOverrideEnable",
    "RaiseExceptionOnPossibleDeadlock",
    "CriticalSectionTimeout",
    "Debugger",
    "VerifierDlls",
    "VerifierFlags",
    "ShutdownFlags",
    "MaxLoaderThreads",
    "DisableUserModeCallbackFilter",
    "FrontEndHeapDebugOptions",
    "Enabled",
    "Default",
    "InstallLanguage",
    "PreferExternalManifest",
    "DisableLocalOverride",
    "ImageExecutionOptions",
    "DisableExceptionChainValidation",
    "PageHeapFlags",
    "LoaderSettings",
    "AuthenticodeEnabled",
    "PolicyScope",
    "DefaultLevel",
    "ExecutableTypes",
    "Levels",
    "LogFileName",
    "ForceDefault",
    "DelegationConsole",
    "DelegationTerminal",
  ].map((word) => [word.toLowerCase(), word]),
);
function objectShape(name) {
  if (typeof name !== "string") return null;
  const parts = name.split("\\");
  if (parts.length > 32) return "?overflow";
  // After the first non-vocabulary component everything is "?", so a user,
  // host or share name that happens to equal a vocabulary word never prints.
  // Plain numbers (session IDs) are masked without hiding what follows.
  let opaque = false;
  return parts
    .map((part) => {
      if (!part) return "";
      if (opaque) return "?";
      if (/^\d{1,10}$/.test(part)) return "?";
      // ASCII only: no Unicode case folding onto a vocabulary word.
      const word = /^[\x20-\x7e]+$/.test(part)
        ? VOCABULARY.get(part.toLowerCase())
        : undefined;
      if (word) return word;
      opaque = true;
      return "?";
    })
    .join("\\");
}
const ROOTS = { 0: "relative", 1: "absolute", 2: "unknown" };

export function projectDocumentNativeOperations({ text, target } = {}) {
  const report = {
    schemaVersion: 1,
    scope: "fixed-windows-startup-operations",
    releaseGate: false,
    debuggerAssisted: true,
    abiBasis: "documented-windows-x64",
    status: "inconclusive",
    cause: "undetermined",
    negativeConclusionAllowed: false,
    coverage: {
      status: "incomplete",
      completedCalls: 0,
      fixedApis: APIS.length,
    },
    observations: [],
    // Every paired call in order, including successes. Names appear only as
    // VOCABULARY shapes, so the sequence near the DLL failure stays visible.
    calls: [],
    reasons: [],
    // Closed-set shape only: a fixed code and a count, never raw text.
    diagnostics: { firstRejection: null, promptJoined: 0 },
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
  // firstRejection describes this pairing loop; earlier exits use reasons.
  const reject = (code) => {
    incomplete = true;
    report.diagnostics.firstRejection ??= code;
  };
  const same = (pid, tid) =>
    hex(pid) === target.childPid && hex(tid) === target.primaryTid;
  for (const output of lines.slice(
    lines.indexOf("OFOX_CDB_V2 CONTINUE_REQUESTED END") + 1,
  )) {
    // Inside a bl listing a prompt line is skipped below, never recovered.
    const line = listing
      ? output
      : output.replace(GLUED, () => {
          report.diagnostics.promptJoined++;
          return "";
        });
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
          reject("listing-row");
        else {
          const row = BP.exec(listing[0]);
          if (
            !row ||
            pointer(row[1]) !== active.returnAddress ||
            Number(row[2]) !== threadIndex
          )
            reject("listing-row");
          else active.breakpointVerified = true;
        }
        listing = null;
      } else if (line.trim() && !PROMPT.test(line)) listing.push(line);
      continue;
    }
    const entry = ENTER.exec(line);
    if (entry) {
      const sequence = Number(entry[1]),
        api = APIS[Number(entry[2]) - 1];
      if (active || sequence !== next || sequence > MAX_CALLS) {
        reject("enter-order");
        continue;
      }
      if (!same(entry[3], entry[4])) {
        reject("enter-identity");
        continue;
      }
      const mode = api?.[1],
        access = entry[7].toLowerCase(),
        root = entry[8];
      if (
        !api ||
        (mode === "attributes" && (access === "none" || root === "none")) ||
        (mode !== "attributes" && access !== "none") ||
        (mode === "port" && root !== "1") ||
        (["none", "value"].includes(mode) && root !== "none")
      ) {
        reject("enter-arguments");
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
        reject("name-orphan");
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
          reject("name-shape");
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
        reject("listing-orphan");
      listing = [];
      continue;
    }
    const returned = RETURN.exec(line);
    if (returned) {
      if (!active || Number(returned[1]) !== active.sequence) {
        reject("return-orphan");
        continue;
      }
      if (
        !same(returned[2], returned[3]) ||
        pointer(returned[4]) !== active.stack + 8n ||
        !active.nameSeen ||
        !active.breakpointVerified
      ) {
        reject("return-mismatch");
        continue;
      }
      const status = hex(returned[5]);
      report.coverage.completedCalls++;
      const statusText = `0x${status.toString(16).padStart(8, "0")}`;
      report.calls.push({
        sequence: active.sequence,
        api: active.api,
        status: statusText,
        desiredAccess: active.access,
        root: ROOTS[active.root] ?? null,
        objectShape: objectShape(active.name),
      });
      if ((status & 0x80000000) !== 0)
        report.observations.push({
          sequence: active.sequence,
          api: active.api,
          status: statusText,
          desiredAccess: active.access,
          objectAlias: objectAlias(active.name, active.root),
        });
      active = null;
      continue;
    }
    // Any other marker, in any case or behind further prompts, is visible.
    if (UNMATCHED.test(line)) reject(INVALID[line] ?? "marker-unknown");
  }
  if (active || listing) reject("unterminated");
  if (!report.coverage.completedCalls) reject("no-calls");
  if (incomplete) report.reasons.push("operation-coverage-incomplete");
  else {
    report.coverage.status = "complete-for-fixed-set";
    report.status = report.observations.length
      ? "observed-failures"
      : "observed-operations";
  }
  return report;
}
