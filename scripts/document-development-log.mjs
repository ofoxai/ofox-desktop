// The runner may emit arbitrary compiler, panic and child output. Only these
// Rust receipt fields can enter the uploaded step log, including on failure.
import { z } from "zod";

const flag = z.boolean();
const count = z.uint32();
const fourFlags = z.array(flag).length(4);
const registry = z.object({
  keys: z.tuple([
    z.literal("machine_root"),
    z.literal("winsock_catalog"),
    z.literal("user_environment"),
    z.literal("user_software"),
  ]),
  production: fourFlags,
  runtimeOnly: fourFlags,
  gates: fourFlags,
  productionVerified: flag,
  runtimeOnlyVerified: flag,
  scan: z.object({
    visited: count,
    vanished: count,
    hostUnopenable: count,
    enumerationStopped: count,
    checkErrors: count,
    readableByRuntimeOnly: count,
    readableByProduction: count,
    addedByRegistryRead: count,
    writableByRuntimeOnly: count,
    writableByProduction: count,
    writeAddedByRegistryRead: count,
  }),
});

// Same closed vocabulary as the fixed Rust startup comparison. A future call
// remains unknown until it is explicitly admitted here too.
const fatalCall = z.enum([
  "?",
  "WSAStartup",
  "uv__malloc",
  "QueryPerformanceCounter",
  "GetEnvironmentVariableW",
  "UnregisterWaitEx",
  "GetModuleHandleA",
  "GetModuleHandleW",
  "GetProcAddress",
  "LoadLibraryExA",
  "CreateIoCompletionPort",
  "GetQueuedCompletionStatus",
  "GetQueuedCompletionStatusEx",
  "PostQueuedCompletionStatus",
  "CreateEvent",
  "CreateEventA",
  "CreateEventW",
  "CreateJobObjectW",
  "SetInformationJobObject",
  "AssignProcessToJobObject",
  "QueryPerformanceFrequency",
  "SetConsoleCtrlHandler",
  "DuplicateHandle",
  "RegisterWaitForSingleObject",
  "UnregisterWait",
  "CreateFileW",
  "ReadFile",
  "WriteFile",
  "CreateThread",
  "QueueUserWorkItem",
  "TlsAlloc",
  "GetFileType",
  "NtQueryInformationFile",
  "NtSetInformationFile",
  "NtQueryVolumeInformationFile",
  "NtDeviceIoControlFile",
  "socket",
  "WSAIoctl",
  "closesocket",
  "CreateNamedPipeW",
  "ConnectNamedPipe",
  "SetNamedPipeHandleState",
  "GetConsoleMode",
  "SetConsoleMode",
  "GetConsoleScreenBufferInfo",
  "CancelIoEx",
  "GetSystemInfo",
  "OpenProcessToken",
  "GetTokenInformation",
]);
const stderrShape = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("libuv-fatal"),
    call: fatalCall.catch("?"),
    code: z.int32().nullable(),
  }),
  z.object({
    kind: z.enum([
      "empty",
      "node-assertion",
      "node-fatal",
      "v8-fatal",
      "icu",
      "openssl",
      "unknown",
    ]),
  }),
]);
const comparison = z.object({
  schemaVersion: z.literal(1),
  scope: z.literal("fixed-node-startup-launch-comparison"),
  diagnosticOnly: z.literal(true),
  releaseGate: z.literal(false),
  runtimePolicyChanged: z.literal(false),
  admissionGranted: z.literal(false),
  candidate: z.enum([
    "registryRead",
    "detachedConsole",
    "detachedRegistryRead",
    "detachedWinsockGrant",
  ]),
  winsockGrantApplied: flag.nullable(),
  winsockGrantFailureRestored: flag.nullable(),
  sameProfileAndInputs: flag,
  runtimeStable: flag,
  debuggerUsed: z.literal(false),
  status: z.enum(["supports-candidate", "inconclusive"]),
  taskCleanup: flag,
  profileCleanup: flag,
  rows: z
    .array(
      z.object({
        extraRegistryRead: flag,
        consoleMode: z.enum(["hidden", "detached"]),
        creationFlags: count,
        winsockGrant: flag,
        grantRestored: flag.nullable(),
        completed: flag,
        stage: z.enum(["create", "token", "resume", "wait", "complete"]),
        tokenVerified: flag,
        capabilityCount: count.nullable(),
        capabilityAttributes: z.array(count).max(2),
        lpacAccessMatched: flag.nullable(),
        childPolicy: count.nullable(),
        imagePolicy: count.nullable(),
        integrityRid: count.nullable(),
        resumePrevious: count.nullable(),
        exitStatus: count.nullable(),
        timedOut: flag,
        markerMatched: flag,
        stdoutBytes: count.nullable(),
        stderrBytes: count.nullable(),
        stderrShape: stderrShape.nullable(),
        cleanupConfirmed: flag,
      }),
    )
    .max(3),
});

// The trace test reports its machine ACL restore independently of the CDB
// controller's private-directory cleanup. Only the former proves restoration.
const trace = z
  .object({
    worker: z.object({
      evidence: z.object({
        loaderDiagnostic: z.object({
          diagnosticOnly: flag,
          configConfirmed: flag.nullable(),
          beforeFlags: count.nullable(),
          afterFlags: count.nullable(),
          preAttachSuspendPrevious: count.nullable(),
          preAttachPairedResumePrevious: count.nullable(),
          releaseProbeCount: count,
          releaseSuspendPrevious: count.nullable(),
          releasePairedResumePrevious: count.nullable(),
          suspendPrevious: count.nullable(),
          pairedResumePrevious: count.nullable(),
          hostResumePrevious: count.nullable(),
          debuggerClosed: flag.nullable(),
          launchProfile: z
            .literal("detached_runtime_only_winsock_grant")
            .nullable(),
          creationFlags: count.nullable(),
          winsockGrantApplied: flag.nullable(),
          winsockGrantRestored: flag.nullable(),
        }),
      }),
    }),
  })
  .transform((detail) => detail.worker.evidence.loaderDiagnostic);
const receipts = new Map([
  ["startup-trace", ["OFOX_STARTUP_TRACE_V1", trace]],
  ["registry-scope-prerequisite", ["OFOX_REGISTRY_ISOLATION_V1", registry]],
  ["launch-comparison", ["OFOX_STARTUP_COMPARISON_V1", comparison]],
]);

// Rust eprintln! writes each receipt on its own stderr line. Match that pipe
// alone; stdout contains the separate libtest selection/count proof.
export function developmentReceipt(step, stderr) {
  const spec = receipts.get(step);
  if (!spec) return null;
  const [marker, schema] = spec;
  const prefix = marker + " ";
  const start = stderr.startsWith(prefix)
    ? 0
    : stderr.indexOf("\n" + prefix) + 1;
  if (!stderr.startsWith(prefix, start)) return { status: "missing" };
  const end = stderr.indexOf("\n", start);
  if (
    end < 0 ||
    end - start > 65536 ||
    stderr.indexOf("\n" + prefix, end) !== -1
  )
    return { status: "invalid" };
  try {
    const body = stderr.slice(start + prefix.length, end);
    const result = schema.safeParse(
      schema === trace ? probeDetail(body) : JSON.parse(body),
    );
    if (result.success)
      return { status: "recorded", marker, data: result.data };
  } catch {}
  return { status: "invalid" };
}

const sourceAliases = new Map([
  ["documents/windows_sandbox", "sandbox"],
  ["documents/windows_sandbox/tests", "sandbox-tests"],
  ["documents/windows_sandbox/probe", "sandbox-probe"],
  ["documents/windows_sandbox/capability_set", "capability-set"],
  ["documents/windows_sandbox/cdb", "debugger"],
  ["documents/windows_sandbox/cdb_protocol", "debugger-protocol"],
  ["documents/windows_sandbox/startup_comparison", "comparison"],
  [
    "documents/windows_sandbox/startup_comparison_protocol",
    "comparison-protocol",
  ],
  ["documents/windows_sandbox/trace_launch", "trace-launch"],
  ["documents/windows_sandbox/winsock_grant", "winsock-grant"],
  ["attachments", "attachments"],
  ["host", "host"],
  ["host/local_interruption_tests", "host-interruption-tests"],
]);
function sourceLocation(text) {
  if (!text) return null;
  const match =
    /(?:^|\/)src\/services\/assistant\/([a-z_/]+)\.rs:(\d{1,7}):\d+:?$/.exec(
      text.replaceAll("\\", "/"),
    );
  const source = sourceAliases.get(match?.[1]);
  return source ? { source, line: Number(match[2]) } : null;
}
export function nativePanicLocation(stderr) {
  const match =
    /^thread '[^'\r\n]*'(?: \(\d+\))? panicked at ([^\r\n]{1,4096}):\r?$/m.exec(
      stderr,
    );
  return sourceLocation(match?.[1]);
}
export function cargoErrors(stderr) {
  const errors = [];
  for (const match of stderr.matchAll(
    /^error\[(E\d{4})\]:[^\r\n]*(?:\r?\n *--> ([^\r\n]{1,4096}))?/gm,
  )) {
    errors.push({ code: match[1], location: sourceLocation(match[2]) });
    if (errors.length === 8) break;
  }
  return errors;
}

const workerStage = z
  .enum([
    "worker_start",
    "os_version",
    "current_user",
    "runtime_capability",
    "cache_source",
    "cache_source_digest",
    "cache_descriptor",
    "cache_lock",
    "cache_owner_open",
    "cache_owner_query",
    "cache_owner_match",
    "cache_acl",
    "cache_digest",
    "cache_parent",
    "cache_source_read",
    "cache_staging_create",
    "cache_staging_write",
    "cache_staging_digest",
    "cache_publish",
    "admission",
    "input_paths",
    "input_snapshot",
    "profile_create",
    "profile_folder",
    "profile_folder_decode",
    "profile_folder_validate",
    "profile_pin",
    "profile_label_open",
    "profile_label_descriptor",
    "profile_label_set",
    "task_create",
    "task_copy",
    "task_home",
    "command",
    "environment",
    "pipes",
    "attributes",
    "job_create",
    "job_limits",
    "process_create",
    "job_assign",
    "token_open",
    "token_container",
    "token_package",
    "token_capabilities",
    "token_integrity",
    "token_mitigations",
    "token_lpac_duplicate",
    "token_lpac_descriptors",
    "token_lpac_world",
    "token_lpac_all_packages",
    "token_lpac_capability",
    "token_lpac_foreign",
    "token_registry_isolation",
    "cdb_handshake",
    "cdb_initial_hold",
    "cdb_continue",
    "cdb_hold",
    "cdb_stop",
    "resume",
    "process_wait",
    "process_status",
    "job_stop",
    "job_process_wait",
    "job_accounting",
    "stdout_receive",
    "stderr_receive",
    "copyback",
    "task_cleanup",
    "profile_cleanup",
    "complete",
  ])
  .catch("unknown");
const hexStatus = z.string().regex(/^[0-9a-fA-F]{8}$/);
const controlBits = z
  .string()
  .regex(/^[0-9a-fA-F]{4}$/)
  .nullish();
const functionalFailure = z.object({
  native: z
    .object({
      code: hexStatus.optional(),
      sourceLine: count.optional(),
      status: hexStatus.optional(),
      completion: hexStatus.optional(),
      actualControl: controlBits,
      expectedControl: controlBits,
      actualRevision: count.nullish(),
      expectedRevision: count.nullish(),
    })
    .nullable(),
  stage: z.enum([
    "runtime_before",
    "setup",
    "node_isolation",
    "node_postconditions",
    "python_create",
    "python_artifact",
    "office_validate",
    "office_svg",
    "python_source_edit",
    "source_preservation",
    "runtime_after",
  ]),
  reason: z.enum([
    "unavailable",
    "unsafe_file_or_io",
    "cancelled",
    "runtime_changed",
    "cleanup_failed",
    "unproven_runtime",
    "other",
    "process_timeout",
    "process_exit",
    "win32",
    "rename_status",
    "cache_acl_query",
    "cache_acl_mismatch",
  ]),
  engine: z
    .object({
      status: z.int32().nullable(),
      timedOut: flag,
      stdoutBytes: count,
      stderrBytes: count,
      nodeCheckpointBeforeStep: z.int().min(1).max(14).nullable(),
      childSpawnResult: z
        .enum([
          "EPERM",
          "EACCES",
          "ETIMEDOUT",
          "ENOENT",
          "UNKNOWN",
          "child-started",
          "marker-present",
          "other-error",
        ])
        .nullish(),
    })
    .nullable(),
  worker: z
    .object({
      firstFailure: workerStage.nullable(),
      evidence: z.object({
        processStatus: count.nullable(),
        registryIsolation: z
          .array(
            z.object({
              key: z.enum([
                "machine_root",
                "winsock_catalog",
                "user_environment",
                "user_software",
              ]),
              opened: flag,
              descriptorRead: flag,
              accessChecked: flag,
              readGranted: flag.nullable(),
            }),
          )
          .max(4),
      }),
    })
    .nullable(),
});

function probeDetail(error) {
  if (typeof error !== "string") return null;
  const marker = ":probe ";
  const index = error.indexOf(marker);
  return index < 0 ? null : JSON.parse(error.slice(index + marker.length));
}

export function functionalProbeFailure(stderr) {
  // Result::expect quotes the String returned by the trusted functional probe.
  // Rebuild only typed evidence; the original panic includes process IDs.
  const prefix =
    "native Python + OfficeCLI and all LPAC negative probes must pass: ";
  const start = stderr.indexOf(prefix);
  if (start < 0) return null;
  const end = stderr.indexOf("\n", start);
  if (end < 0 || end - start > 65536) return null;
  try {
    const error = JSON.parse(stderr.slice(start + prefix.length, end));
    const result = functionalFailure.safeParse(probeDetail(error));
    return result.success ? result.data : null;
  } catch {
    return null;
  }
}
