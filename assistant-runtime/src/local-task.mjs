// Engine-independent task state. Effects and verification come only from host
// results, never assistant prose. Intent does not affect operation identity.
const stable = (value) =>
  Array.isArray(value)
    ? value.map(stable)
    : value && typeof value === "object"
      ? Object.fromEntries(
          Object.keys(value)
            .filter((k) => k !== "intent")
            .sort()
            .map((k) => [k, stable(value[k])]),
        )
      : value;
export function validatePlan(steps) {
  if (
    !Array.isArray(steps) ||
    steps.length > 12 ||
    steps.some(
      (s) =>
        !s ||
        typeof s.step !== "string" ||
        !s.step.trim() ||
        [...s.step].length > 40 ||
        !["pending", "in_progress", "completed"].includes(s.status) ||
        Object.keys(s).some((k) => !["step", "status"].includes(k)),
    ) ||
    steps.filter((s) => s.status === "in_progress").length > 1
  )
    throw new Error("assistant_invalid_plan");
  return steps.map((s) => ({ ...s }));
}
const targetKey = (tool, folder) => JSON.stringify([tool, folder]);
// Match BindableTool's compatibility spellings. Successful observations still
// use the Host's canonical result.tool rather than trusting model arguments.
const canonicalTool = (tool) => {
  if (typeof tool !== "string") return tool;
  const name = tool.trim().toLowerCase();
  return name === "chatgpt" ? "codex" : name;
};
const MAX_TARGETS = 32;
const validPart = (value, bytes) =>
  typeof value === "string" &&
  value.trim().length > 0 &&
  Buffer.byteLength(value, "utf8") <= bytes;
const validTarget = (tool, folder) =>
  validPart(tool, 64) && validPart(folder, 4096);
const MUTATIONS = new Set([
  "repair_launcher",
  "rebind_tool",
  "install_tool",
  "update_tool",
  "uninstall_tool",
  "set_tool_model",
]);
// Host-observed tool activity scopes uncertain command effects to tool repairs.
const TOOL_OPERATIONS = new Set([
  "tool_status",
  "diagnose_in_folder",
  "verify_in_folder",
  "repair_launcher",
  "rebind_tool",
  "install_tool",
  "update_tool",
  "uninstall_tool",
  "set_tool_model",
]);
export class LocalTask {
  // `state` is the host-kept snapshot of an earlier run in this conversation:
  // a repair whose verification is still missing stays missing. `home` is
  // where a change is verified when the user named no problem folder.
  constructor(
    limits,
    clock = Date.now,
    state = null,
    home = null,
    reserveFinalStep = true,
  ) {
    this.reserveFinalStep = reserveFinalStep;
    this.home = home;
    this.limits = limits;
    this.clock = clock;
    this.started = clock();
    this.cost = 0;
    this.calls = 0;
    this.repeats = new Map();
    this.targets = new Map();
    this.observedTools = new Set();
    this.scopeIncomplete = false;
    this.changed = state?.changed === true;
    // An arbitrary command is not proof of a configuration change. Only a
    // tool repair needs its uncertain effects verified; e.g. a currency lookup
    // with curl must finish without inspecting unrelated tools.
    this.uncertain = false;
    this.inspected = false;
    if (this.changed) {
      const targets = Array.isArray(state.targets) ? state.targets : [];
      this.scopeIncomplete =
        state.scopeIncomplete === true ||
        (state.scopeIncomplete !== undefined &&
          state.scopeIncomplete !== false) ||
        !Array.isArray(state.targets) ||
        targets.length === 0 ||
        targets.length > MAX_TARGETS;
      for (const t of targets.slice(0, MAX_TARGETS)) {
        if (
          !validTarget(t?.tool, t?.folder) ||
          (t.verified !== undefined && typeof t.verified !== "boolean")
        ) {
          this.scopeIncomplete = true;
          continue;
        }
        const tool = canonicalTool(t.tool);
        const key = targetKey(tool, t.folder);
        const previous = this.targets.get(key);
        if (previous) this.scopeIncomplete = true;
        this.targets.set(key, {
          tool,
          folder: t.folder,
          verified: t.verified === true && (!previous || previous.verified),
        });
        this.observedTools.add(tool);
      }
    }
    this.retries = 0;
    this.stopped = null;
  }
  snapshot() {
    return this.needsVerification
      ? {
          changed: true,
          targets: [...this.targets.values()],
          ...(this.scopeIncomplete ? { scopeIncomplete: true } : {}),
        }
      : null;
  }
  hasTool(tool) {
    return [...this.targets.values()].some((target) => target.tool === tool);
  }
  addTarget(tool, folder) {
    const key = targetKey(tool, folder);
    if (!validTarget(tool, folder)) {
      this.scopeIncomplete = true;
    } else if (!this.targets.has(key)) {
      if (this.targets.size >= MAX_TARGETS) this.scopeIncomplete = true;
      else this.targets.set(key, { tool, folder, verified: false });
    }
  }
  scopeTool(tool, folder = this.home) {
    if (!this.hasTool(tool)) this.addTarget(tool, folder);
  }
  stop(code) {
    this.stopped = code;
    throw new Error(code);
  }
  beforeRequest() {
    if (this.stopped) throw new Error(this.stopped);
    if (this.clock() - this.started >= this.limits.maxDurationSeconds * 1000)
      this.stop("assistant_duration_limit");
    if (this.cost >= this.limits.maxCostUsd) this.stop("assistant_cost_limit");
  }
  get finalStep() {
    return (
      this.calls >= this.limits.maxToolCalls - Number(this.reserveFinalStep)
    );
  }
  beforeTool(name, args) {
    this.beforeRequest();
    if (this.finalStep) this.stop("assistant_tool_limit");
    // SDK folder strings are not canonical paths. The Host checks diagnosis
    // capacity after resolving them, before approvals or probes. A new named
    // mutation's home target can already be refused here without guessing.
    if (
      MUTATIONS.has(name) &&
      validPart(args.tool, 64) &&
      !this.hasTool(canonicalTool(args.tool)) &&
      this.targets.size >= MAX_TARGETS
    )
      throw new Error("assistant_verification_target_limit");
    this.calls++;
    const key = JSON.stringify([name, stable(args)]);
    const count = (this.repeats.get(key) ?? 0) + 1;
    this.repeats.set(key, count);
    if (count >= this.limits.repeatStop) this.stop("assistant_repeat_limit");
    return count === this.limits.repeatWarning ? "repeat_warning" : null;
  }
  addUsage(usage) {
    const cost = usage?.cost?.total;
    if (Number.isFinite(cost) && cost > 0) this.cost += cost;
  }
  addSearchUsage(usage, price) {
    if (!price || usage?.model !== price.model)
      this.stop("assistant_cost_unavailable");
    const parts = [
      [usage.searchCalls, price.perSearch],
      [usage.inputTokens, price.perInputToken],
      [usage.outputTokens, price.perOutputToken],
    ];
    if (
      parts.some(
        ([count, rate]) =>
          !Number.isFinite(count) ||
          count < 0 ||
          !Number.isFinite(Number(rate)) ||
          Number(rate) < 0,
      )
    )
      this.stop("assistant_cost_unavailable");
    this.addUsage({
      cost: {
        total: parts.reduce(
          (total, [count, rate]) => total + count * Number(rate),
          0,
        ),
      },
    });
  }
  observe(name, args, result) {
    if (!result || typeof result !== "object") return;
    const tool = canonicalTool(result.tool ?? args.tool);
    const folder = result.folder ?? args.folder;
    const key = targetKey(tool, folder);
    if (name === "diagnose_in_folder") this.addTarget(tool, folder);
    if (TOOL_OPERATIONS.has(name)) {
      this.inspected = true;
      if (validPart(tool, 64)) this.observedTools.add(tool);
    }
    if (result.changed === true) this.changed = true;
    if (result.requiresVerification === true) this.uncertain = true;
    if (result.changed === true && MUTATIONS.has(name)) {
      this.scopeTool(tool, result.verificationFolder ?? this.home);
    } else if (result.changed === true || (this.uncertain && this.inspected)) {
      for (const tool of this.observedTools) this.scopeTool(tool);
      if (this.targets.size === 0) this.scopeIncomplete = true;
    }
    if (result.changed === true || result.requiresVerification === true) {
      for (const target of this.targets.values()) target.verified = false;
    }
    if (name === "verify_in_folder" && this.targets.has(key))
      this.targets.get(key).verified = result.verified === true;
  }
  get needsVerification() {
    return (
      (this.changed || (this.uncertain && this.inspected)) &&
      (this.scopeIncomplete ||
        this.targets.size === 0 ||
        [...this.targets.values()].some((t) => !t.verified))
    );
  }
  continuation() {
    if (
      !this.needsVerification ||
      this.stopped ||
      this.finalStep ||
      this.retries >= this.limits.completionRetries
    )
      return null;
    this.retries++;
    if (this.scopeIncomplete)
      return "The original repair verification scope is incomplete. Do not guess another tool or folder, or claim a successful repair. Explain that the repair remains unverified.";
    return `Host verification is still missing after configuration changes. Do not claim a successful repair. ${this.targets.size ? `Run verify_in_folder for these original problem folders: ${JSON.stringify([...this.targets.values()].filter((t) => !t.verified))}` : "Run verify_in_folder for the changed tool in the user's home folder (leave folder out), or in the folder the user said fails."}`;
  }
}
