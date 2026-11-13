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
  constructor(limits, clock = Date.now, state = null, home = null) {
    this.home = home;
    this.homeVerified = false;
    this.limits = limits;
    this.clock = clock;
    this.started = clock();
    this.cost = 0;
    this.calls = 0;
    this.repeats = new Map();
    this.targets = new Map();
    this.changed = state?.changed === true;
    // An arbitrary command is not proof of a configuration change. Only a
    // tool repair needs its uncertain effects verified; e.g. a currency lookup
    // with curl must finish without inspecting unrelated tools.
    this.uncertain = false;
    this.inspected = false;
    for (const t of Array.isArray(state?.targets) ? state.targets : [])
      if (typeof t?.tool === "string" && typeof t?.folder === "string")
        this.targets.set(targetKey(t.tool, t.folder), {
          tool: t.tool,
          folder: t.folder,
          verified: t.verified === true,
        });
    this.retries = 0;
    this.stopped = null;
  }
  snapshot() {
    return this.needsVerification
      ? { changed: true, targets: [...this.targets.values()] }
      : null;
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
    return this.calls >= this.limits.maxToolCalls - 1;
  }
  beforeTool(name, args) {
    this.beforeRequest();
    if (this.finalStep) this.stop("assistant_tool_limit");
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
    const folder = result.folder ?? args.folder;
    const key = targetKey(args.tool, folder);
    if (name === "diagnose_in_folder" && !this.targets.has(key))
      this.targets.set(key, { tool: args.tool, folder, verified: false });
    if (TOOL_OPERATIONS.has(name)) this.inspected = true;
    if (result.changed === true) this.changed = true;
    if (result.requiresVerification === true) this.uncertain = true;
    if (result.changed === true || result.requiresVerification === true) {
      for (const target of this.targets.values()) target.verified = false;
      this.homeVerified = false;
    }
    if (
      name === "verify_in_folder" &&
      this.home &&
      folder === this.home &&
      result.verified === true
    )
      this.homeVerified = true;
    if (name === "verify_in_folder" && this.targets.has(key))
      this.targets.get(key).verified = result.verified === true;
  }
  get needsVerification() {
    return (
      (this.changed || (this.uncertain && this.inspected)) &&
      (this.targets.size
        ? [...this.targets.values()].some((t) => !t.verified)
        : !this.homeVerified)
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
    return `Host verification is still missing after configuration changes. Do not claim a successful repair. ${this.targets.size ? `Run verify_in_folder for these original problem folders: ${JSON.stringify([...this.targets.values()].filter((t) => !t.verified))}` : "Run verify_in_folder for the changed tool in the user's home folder (leave folder out), or in the folder the user said fails."}`;
  }
}
