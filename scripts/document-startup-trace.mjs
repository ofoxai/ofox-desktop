// Diagnostic observations only. Never an isolation or release acceptance gate.
// Input is the capture helper's private tracerpt XML -> named-data projection.
// GUID/opcodes/fields: Microsoft FileIo_Create, FileIo_OpEnd, Registry_TypeGroup1,
// Thread_V2_TypeGroup1 and Image_Load MOF documentation. Thread v3 keeps the
// identity fields (Microsoft perfview/KernelTraceEventParser.cs ThreadTraceData).
import { createHash } from "node:crypto";
const GUIDS = {
  "3d6fa8d1-fe05-11d0-9dda-00c04fd7ba7c": "thread",
  "90cbdc39-4a3e-11d1-84f4-0000f80464e3": "file",
  "ae53722e-c863-11d2-8659-00c04fa321a1": "registry",
  "2cb15d1d-5fc1-11d2-abe1-00a0c911f518": "image",
};
const SAFE_FIELDS = new Set([
  "ProcessId",
  "TThreadId",
  "TTID",
  "IrpPtr",
  "OpenPath",
  "NtStatus",
  "Status",
  "KeyName",
  "FileName",
  "FileObject",
  "CreateOptions",
  "FileAttributes",
  "ShareAccess",
  "ExtraInfo",
  "InitialTime",
  "Index",
  "KeyHandle",
  "ImageBase",
  "ImageSize",
]);
export const DOCUMENT_STARTUP_TRACE_LIMITS = Object.freeze({
  eventsPerPass: 2_000_000,
  targets: 8,
  threadsPerTarget: 128,
  pendingIrps: 4096,
  outputEvents: 64,
  schemaShapes: 32,
  dataFields: 32,
  fieldCharacters: 4096,
});
const L = DOCUMENT_STARTUP_TRACE_LIMITS;

function unsigned(value, bits = 32) {
  if (typeof value === "number")
    return Number.isSafeInteger(value) && value >= 0 && value < 2 ** bits
      ? BigInt(value)
      : null;
  if (
    typeof value !== "string" ||
    !/^(?:0x[\da-f]{1,16}|\d{1,20})$/i.test(value)
  )
    return null;
  const n = BigInt(value);
  return n < 1n << BigInt(bits) ? n : null;
}
function id(value) {
  const n = unsigned(value);
  return n !== null && n > 0 ? Number(n) : null;
}
function pointer(value) {
  const n = unsigned(value, 64);
  return n !== null && n > 0 ? n.toString(16) : null;
}
function normalized(raw) {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const d = raw.data;
  if (!d || typeof d !== "object" || Array.isArray(d)) return null;
  const fields = Object.keys(d);
  if (
    fields.length > L.dataFields ||
    fields.some(
      (k) =>
        k.length > 128 ||
        typeof d[k] !== "string" ||
        d[k].length > L.fieldCharacters,
    )
  )
    return null;
  const guid =
    typeof raw.providerGuid === "string"
      ? raw.providerGuid.toLowerCase().replace(/^\{([\da-f-]{36})\}$/, "$1")
      : "";
  const provider = Object.hasOwn(GUIDS, guid) ? GUIDS[guid] : "unknown";
  const opcode =
    Number.isInteger(raw.opcode) && raw.opcode >= 0 && raw.opcode <= 255
      ? raw.opcode
      : null;
  const version =
    Number.isInteger(raw.version) && raw.version >= 0 && raw.version <= 255
      ? raw.version
      : null;
  const time =
    typeof raw.timestamp100ns === "string" &&
    /^\d{1,20}$/.test(raw.timestamp100ns)
      ? unsigned(raw.timestamp100ns, 64)
      : null;
  let kind = "unknown";
  if (time !== null && opcode !== null && version !== null) {
    if (
      provider === "thread" &&
      version <= 3 &&
      [1, 2].includes(opcode) &&
      id(d.ProcessId) &&
      id(d.TThreadId)
    )
      kind = "thread";
    if (provider === "file" && [2, 3].includes(version)) {
      if (
        opcode === 64 &&
        pointer(d.IrpPtr) &&
        id(d.TTID) &&
        typeof d.OpenPath === "string"
      )
        kind = "create";
      else if (
        opcode === 76 &&
        pointer(d.IrpPtr) &&
        unsigned(d.NtStatus) !== null
      )
        kind = "complete";
      // Another operation on this IRP must invalidate a pending create. It is
      // not interpreted as a supported file observation or a successful deny.
      else if (pointer(d.IrpPtr)) kind = "barrier";
    }
    if (
      provider === "registry" &&
      version === 2 &&
      opcode >= 10 &&
      opcode <= 27 &&
      id(raw.headerTid) &&
      unsigned(d.Status) !== null &&
      typeof d.KeyName === "string"
    )
      kind = "registry";
    if (
      provider === "image" &&
      [2, 3].includes(version) &&
      opcode === 10 &&
      id(d.ProcessId) &&
      typeof d.FileName === "string"
    )
      kind = "image";
  }
  return {
    provider,
    opcode,
    version,
    time,
    kind,
    data: d,
    tid: id(raw.headerTid),
    fields,
  };
}

function lexicalPath(value) {
  if (
    typeof value !== "string" ||
    value.length > L.fieldCharacters ||
    /[\x00-\x1f]/.test(value)
  )
    return null;
  const p = value
    .replaceAll("/", "\\")
    .replace(/^\\\\\?\\/, "")
    .replace(/^\\\?\?\\/, "");
  if (p.split("\\").some((part) => part === "." || part === "..")) return null;
  return p.replace(/\\+$/, "").toLowerCase();
}
function resource(value, roots, registry = false) {
  if (registry) {
    const p = lexicalPath(value);
    return {
      rootAlias: p?.startsWith("\\registry\\machine\\")
        ? "registry-machine"
        : p?.startsWith("\\registry\\user\\")
          ? "registry-user"
          : "registry-other",
    };
  }
  const p = lexicalPath(value);
  let rootAlias = "unknown";
  let relative = null;
  for (const alias of ["windows", "runtime", "task"]) {
    const paths =
      typeof roots?.[alias] === "string"
        ? [roots[alias]]
        : Array.isArray(roots?.[alias])
          ? roots[alias].slice(0, 8)
          : [];
    const base =
      p &&
      paths
        .map((r) => {
          const base = lexicalPath(r);
          return base &&
            (/^[a-z]:\\[^\\]+/.test(base) ||
              /^\\device\\[^\\]+\\[^\\]+/.test(base)) &&
            (p === base || p.startsWith(`${base}\\`))
            ? base
            : null;
        })
        .find(Boolean);
    if (base) {
      rootAlias = alias;
      relative = p.slice(base.length + 1);
      break;
    }
  }
  const leaf = p?.split("\\").at(-1) ?? "";
  // Unknown device/volume prefixes and arbitrary user paths never acquire a
  // system-module identity merely because their last component ends in .dll.
  const module =
    rootAlias === "windows" &&
    /^(?:system32|syswow64)\\[^\\]+$/.test(relative ?? "") &&
    /^[a-z\d][a-z\d_.-]{0,95}\.(?:dll|exe)$/.test(leaf)
      ? leaf
      : rootAlias === "runtime" && ["node.exe", "officecli.exe"].includes(leaf)
        ? leaf
        : undefined;
  return module ? { rootAlias, module } : { rootAlias };
}

export function decodeDocumentEngineTrace({
  events,
  lossCounters,
  targets,
  roots = {},
} = {}) {
  const reasons = new Set();
  const report = {
    schemaVersion: 1,
    scope: "document-startup-trace",
    status: "inconclusive",
    cause: "undetermined",
    negativeConclusionAllowed: false,
    lossStatus: "unknown",
    schemaOverview: [],
    targets: [],
    events: [],
    counts: {
      inputEvents: 0,
      unknownSchemaEvents: 0,
      malformedEvents: 0,
      omittedSchemaShapes: 0,
      omittedEvents: 0,
    },
    reasons: [],
  };
  const finish = () => {
    report.reasons = [...reasons];
    return report;
  };
  const counterKeys = [
    "eventsLost",
    "logBuffersLost",
    "realTimeBuffersLost",
    "buffersWritten",
  ];
  if (
    lossCounters?.ownedStopSucceeded === true &&
    counterKeys.every((k) => unsigned(lossCounters[k]) !== null)
  ) {
    report.lossStatus = counterKeys
      .slice(0, 3)
      .some((k) => Number(lossCounters[k]) !== 0)
      ? "lost-events"
      : "zero-reported";
  }
  if (report.lossStatus !== "zero-reported")
    reasons.add(
      report.lossStatus === "unknown"
        ? "capture-loss-unknown"
        : "capture-events-lost",
    );
  if (
    !Array.isArray(targets) ||
    targets.length === 0 ||
    targets.length > L.targets ||
    targets.some((t) => !id(t?.childPid) || !id(t?.primaryTid)) ||
    new Set(targets.map((t) => t.childPid)).size !== targets.length
  ) {
    reasons.add("invalid-targets");
    return finish();
  }
  if (!Array.isArray(events) && typeof events !== "function") {
    reasons.add("input-not-replayable");
    return finish();
  }
  const groups = targets.map((t) => ({
    pid: id(t.childPid),
    tid: id(t.primaryTid),
    edges: new Map(),
    intervals: new Map(),
    foreignEdges: new Map(),
    invalid: new Set(),
  }));
  const schemas = new Map();
  function read(visit, first) {
    let count = 0;
    const digest = createHash("sha256");
    try {
      const input = typeof events === "function" ? events() : events;
      for (const raw of input) {
        if (++count > L.eventsPerPass) {
          reasons.add("input-event-limit");
          break;
        }
        const e = normalized(raw);
        digest.update(
          e
            ? JSON.stringify({
                provider: e.provider,
                opcode: e.opcode,
                version: e.version,
                time: e.time?.toString(),
                kind: e.kind,
                tid: e.tid,
                data: e.data,
              })
            : "malformed",
        );
        digest.update("\n");
        if (first) {
          report.counts.inputEvents++;
          if (!e) {
            report.counts.malformedEvents++;
            reasons.add("malformed-event");
          } else {
            if (e.kind === "unknown" || e.kind === "barrier") {
              report.counts.unknownSchemaEvents++;
              reasons.add("unknown-schema");
            }
            const knownFields = e.fields
              .filter((k) => SAFE_FIELDS.has(k))
              .sort();
            const shape = {
              provider: e.provider,
              opcode: e.opcode,
              version: e.version,
              knownFields,
              unknownFieldCount: e.fields.length - knownFields.length,
            };
            const key = JSON.stringify(shape);
            if (schemas.has(key)) schemas.get(key).count++;
            else if (schemas.size < L.schemaShapes)
              schemas.set(key, { ...shape, count: 1 });
            else report.counts.omittedSchemaShapes++;
          }
        }
        if (e || !first) visit(e, count);
      }
      return digest.digest("hex");
    } catch {
      reasons.add("input-read-failed");
      return null;
    }
  }
  const firstRead = read((e) => {
    if (e.kind !== "thread") return;
    for (const g of groups)
      if (id(e.data.ProcessId) === g.pid) {
        const tid = id(e.data.TThreadId);
        if (!g.edges.has(tid) && g.edges.size >= L.threadsPerTarget) {
          reasons.add("target-thread-limit");
          continue;
        }
        const edges = g.edges.get(tid) ?? [];
        if (edges.length < 4) edges.push(e);
        else g.invalid.add(tid);
        g.edges.set(tid, edges);
      }
  }, true);
  report.schemaOverview = [...schemas.values()];
  if (!firstRead) return finish();
  for (const g of groups) {
    const primary = g.edges.get(g.tid) ?? [];
    const starts = primary.filter((e) => e.opcode === 1),
      ends = primary.filter((e) => e.opcode === 2);
    if (
      starts.length !== 1 ||
      ends.length !== 1 ||
      starts[0].time >= ends[0].time ||
      ends[0].time - starts[0].time > 3_000_000_000n ||
      g.invalid.has(g.tid)
    ) {
      report.targets.push({
        childPid: g.pid,
        primaryTid: g.tid,
        window: "missing-or-ambiguous",
      });
      reasons.add("target-window-missing-or-ambiguous");
      continue;
    }
    g.start = starts[0].time;
    g.end = ends[0].time;
    for (const [tid, edges] of g.edges) {
      const a = edges.filter(
        (e) => e.opcode === 1 && e.time >= g.start && e.time <= g.end,
      );
      const b = edges.filter(
        (e) => e.opcode === 2 && e.time >= g.start && e.time <= g.end,
      );
      if (
        a.length === 1 &&
        b.length === 1 &&
        a[0].time < b[0].time &&
        !g.invalid.has(tid)
      )
        g.intervals.set(tid, [a[0].time, b[0].time]);
      else g.invalid.add(tid);
    }
    report.targets.push({
      childPid: g.pid,
      primaryTid: g.tid,
      window: "matched",
      duration100ns: (g.end - g.start).toString(),
    });
  }
  function owner(tid, time) {
    const matches = groups
      .map((g, index) => ({ g, index, span: g.intervals.get(tid) }))
      .filter(
        ({ g, span }) =>
          span && !g.invalid.has(tid) && time >= span[0] && time <= span[1],
      );
    return matches.length === 1 ? matches[0] : null;
  }
  const observations = [];
  function record(kind, e, own, details, ordinal, tid = null) {
    if (details.rootAlias === "unknown") reasons.add("unmapped-resource-root");
    observations.push({
      kind,
      targetIndex: own.index,
      offset100ns: (e.time - own.g.start).toString(),
      tid,
      ...details,
      ordinal,
    });
    if (observations.length > L.outputEvents * 2) {
      observations.sort(
        (a, b) =>
          Number(Boolean(b.status && BigInt(b.status) >= 0x80000000n)) -
            Number(Boolean(a.status && BigInt(a.status) >= 0x80000000n)) ||
          a.ordinal - b.ordinal,
      );
      report.counts.omittedEvents += observations.length - L.outputEvents;
      observations.length = L.outputEvents;
      reasons.add("event-output-limit");
    }
  }
  const pending = new Map();
  let previousTime = null,
    ordered = true,
    correlate = true;
  const secondRead = read((e, ordinal) => {
    // Even an opaque provider has a usable trace timestamp. Ignoring it would
    // hide a backwards step before a later create/completion association.
    if (e?.time !== null && e?.time !== undefined) {
      if (previousTime !== null && e.time < previousTime) {
        ordered = false;
        pending.clear();
        reasons.add("event-order-unknown");
      }
      previousTime = e.time;
    }
    if (!e || e.time === null || e.kind === "unknown") {
      // An opaque row may be a reused IRP's next operation. No pending create
      // may cross it and acquire a later, unrelated operation's completion.
      if ([...pending.values()].some(Boolean))
        reasons.add("operation-correlation-incomplete");
      pending.clear();
      return;
    }
    if (e.kind === "thread") {
      for (const g of groups)
        if (
          g.intervals.has(id(e.data.TThreadId)) &&
          id(e.data.ProcessId) !== g.pid
        ) {
          const tid = id(e.data.TThreadId),
            edges = g.foreignEdges.get(tid) ?? [];
          if (edges.length < 32)
            edges.push({
              time: e.time,
              opcode: e.opcode,
              pid: id(e.data.ProcessId),
            });
          else {
            g.invalid.add(tid);
            reasons.add("thread-identity-conflict");
          }
          g.foreignEdges.set(tid, edges);
        }
    }
    if (e.kind === "create" && correlate && ordered) {
      const key = pointer(e.data.IrpPtr),
        own = owner(id(e.data.TTID), e.time);
      if (pending.has(key)) {
        if (pending.get(key) || own)
          reasons.add("operation-correlation-incomplete");
        pending.set(key, null);
      } else if (pending.size >= L.pendingIrps) {
        pending.clear();
        correlate = false;
        reasons.add("irp-capacity-limit");
      } else
        pending.set(
          key,
          own
            ? {
                own,
                time: e.time,
                tid: id(e.data.TTID),
                location: resource(e.data.OpenPath, roots),
              }
            : null,
        );
    } else if (e.kind === "complete" && correlate && ordered) {
      const key = pointer(e.data.IrpPtr),
        begin = pending.get(key);
      pending.delete(key);
      if (begin && e.time >= begin.time && e.time <= begin.own.g.end)
        record(
          "file-operation",
          e,
          begin.own,
          {
            status: `0x${unsigned(e.data.NtStatus).toString(16).padStart(8, "0")}`,
            ...begin.location,
          },
          ordinal,
          begin.tid,
        );
    } else if (e.provider === "file" && pointer(e.data.IrpPtr)) {
      const key = pointer(e.data.IrpPtr);
      if (pending.get(key)) reasons.add("operation-correlation-incomplete");
      pending.delete(key);
    } else if (e.kind === "registry") {
      const own = owner(e.tid, e.time);
      if (own)
        record(
          "registry-operation",
          e,
          own,
          {
            opcode: e.opcode,
            status: `0x${unsigned(e.data.Status).toString(16).padStart(8, "0")}`,
            ...resource(e.data.KeyName, roots, true),
          },
          ordinal,
          e.tid,
        );
    } else if (e.kind === "image") {
      const index = groups.findIndex(
        (g) =>
          g.start !== undefined &&
          id(e.data.ProcessId) === g.pid &&
          e.time >= g.start &&
          e.time <= g.end,
      );
      if (index >= 0)
        record(
          "image-map",
          e,
          { index, g: groups[index] },
          resource(e.data.FileName, roots),
          ordinal,
        );
    }
  }, false);
  if (!secondRead) return finish();
  if (secondRead !== firstRead) {
    reasons.add("input-changed-between-passes");
    return finish();
  }
  // Foreign lifetimes must be considered even when both edges lie outside the
  // target window. A containing interval cannot leave that identity trusted.
  for (const [index, g] of groups.entries()) {
    for (const [tid, edges] of g.foreignEdges) {
      const span = g.intervals.get(tid);
      let active = null,
        ambiguous = false;
      edges.sort((a, b) => (a.time < b.time ? -1 : a.time > b.time ? 1 : 0));
      for (const edge of edges) {
        if (edge.opcode === 1) {
          if (active) ambiguous = true;
          active = edge;
        } else {
          if (!active || active.pid !== edge.pid || active.time >= edge.time)
            ambiguous = true;
          else if (active.time <= span[1] && edge.time >= span[0])
            ambiguous = true;
          active = null;
        }
      }
      if (active || ambiguous) {
        g.invalid.add(tid);
        reasons.add("thread-identity-conflict");
      }
    }
    if (g.invalid.has(g.tid)) {
      report.targets[index] = {
        childPid: g.pid,
        primaryTid: g.tid,
        window: "missing-or-ambiguous",
      };
      reasons.add("target-window-missing-or-ambiguous");
    }
  }
  if ([...pending.values()].some(Boolean))
    reasons.add("operation-correlation-incomplete");
  const eligible = observations
    .filter(
      (e) =>
        !groups[e.targetIndex].invalid.has(groups[e.targetIndex].tid) &&
        (!e.tid || !groups[e.targetIndex].invalid.has(e.tid)) &&
        (ordered || e.kind !== "file-operation"),
    )
    .sort(
      (a, b) =>
        Number(Boolean(b.status && BigInt(b.status) >= 0x80000000n)) -
          Number(Boolean(a.status && BigInt(a.status) >= 0x80000000n)) ||
        a.ordinal - b.ordinal,
    );
  if (eligible.length > L.outputEvents) {
    report.counts.omittedEvents += eligible.length - L.outputEvents;
    reasons.add("event-output-limit");
  }
  report.events = eligible
    .slice(0, L.outputEvents)
    .sort((a, b) => a.ordinal - b.ordinal)
    .map(({ ordinal, ...e }) => e);
  if (
    report.events.some(
      (e) => e.status && (BigInt(e.status) & 0x80000000n) !== 0n,
    )
  )
    report.status = "observed-failures";
  else reasons.add("no-observed-failure");
  return finish();
}
