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
export class LocalTask {
  constructor(limits, clock = Date.now) {
    this.limits = limits;
    this.clock = clock;
    this.started = clock();
    this.cost = 0;
    this.calls = 0;
    this.repeats = new Map();
    this.targets = new Map();
    this.changed = false;
    this.retries = 0;
    this.stopped = null;
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
    const key = JSON.stringify([args.tool, folder]);
    if (name === "diagnose_in_folder" && !this.targets.has(key))
      this.targets.set(key, { tool: args.tool, folder, verified: false });
    if (result.changed === true || result.requiresVerification === true) {
      this.changed = true;
      for (const target of this.targets.values()) target.verified = false;
    }
    if (name === "verify_in_folder" && this.targets.has(key))
      this.targets.get(key).verified = result.verified === true;
  }
  get needsVerification() {
    return (
      this.changed &&
      (!this.targets.size ||
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
    return `Host verification is still missing after configuration changes. Do not claim a successful repair. ${this.targets.size ? `Run verify_in_folder for these original problem folders: ${JSON.stringify([...this.targets.values()].filter((t) => !t.verified))}` : "Ask the user for the original problem folder with request_folder, diagnose there, then verify there."}`;
  }
}
