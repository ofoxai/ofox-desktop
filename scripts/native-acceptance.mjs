import {
  resourceClientIds,
  acceptanceResourceKinds,
} from "./resource-targets.mjs";
import { createHash, randomBytes, randomUUID } from "node:crypto";
import { spawn } from "node:child_process";
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { release, arch, platform } from "node:os";
import { dirname, isAbsolute, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  agentIds,
  nativeEvents,
  nativeInvocation,
  prepareProfile,
  profilePaths,
  acceptancePrompt,
} from "./native-acceptance-drivers.mjs";
import {
  verifyAcceptanceIdentity,
  profileResourceEvidence,
  verifyWriterProfile,
} from "./native-acceptance-evidence.mjs";
import { openSupervisedAcceptance } from "./native-acceptance-supervised.mjs";

const fixtureDriver = fileURLToPath(
  new URL("./native-acceptance-fixture.mjs", import.meta.url),
);
const hash = (value) => createHash("sha256").update(value).digest("hex");
const identifier = /^[a-z0-9-]{1,64}$/;

export class BudgetLedger {
  constructor(limitCents = 1000) {
    if (
      !Number.isSafeInteger(limitCents) ||
      limitCents < 1 ||
      limitCents > 1000
    )
      throw new Error("invalidBudgetLimit");
    this.limitCents = limitCents;
    this.calls = [];
  }
  reserve(callId, maxCents) {
    if (
      !identifier.test(callId) ||
      this.calls.some((row) => row.callId === callId) ||
      !Number.isSafeInteger(maxCents) ||
      maxCents < 0
    )
      throw new Error("invalidReservation");
    const used = this.calls.reduce(
      (sum, row) => sum + (row.settledCents ?? row.reservedCents),
      0,
    );
    if (used + maxCents > this.limitCents)
      throw new Error("localBudgetExceeded");
    this.calls.push({
      callId,
      reservedCents: maxCents,
      settledCents: null,
      state: "reserved",
    });
  }
  settle(callId, cents) {
    const row = this.calls.find((item) => item.callId === callId);
    if (!row || row.state !== "reserved") throw new Error("invalidSettlement");
    if (
      cents !== null &&
      (!Number.isSafeInteger(cents) || cents < 0 || cents > row.reservedCents)
    )
      throw new Error("serverBudgetEvidenceInvalid");
    row.settledCents = cents;
    row.state = cents === null ? "pending" : "settled";
  }
  snapshot() {
    return {
      limitCents: this.limitCents,
      settledCents: this.calls.reduce(
        (sum, row) => sum + (row.settledCents ?? 0),
        0,
      ),
      unresolvedCents: this.calls
        .filter((row) => row.settledCents === null)
        .reduce((sum, row) => sum + row.reservedCents, 0),
      requestCount: this.calls.length,
      calls: this.calls.map((row) => ({ ...row })),
    };
  }
}

export function isolatedEnvironment(root, inherited = process.env) {
  const home = join(root, "home");
  const result = {
    HOME: home,
    USERPROFILE: home,
    CC_SWITCH_TEST_HOME: home,
    APPDATA: join(home, "AppData/Roaming"),
    LOCALAPPDATA: join(home, "AppData/Local"),
    XDG_CONFIG_HOME: join(home, ".config"),
    XDG_DATA_HOME: join(home, ".local/share"),
    XDG_CACHE_HOME: join(home, ".cache"),
    XDG_STATE_HOME: join(home, ".local/state"),
    CODEX_HOME: join(home, ".codex"),
    CLAUDE_CONFIG_DIR: join(home, ".claude"),
    GEMINI_CLI_HOME: home,
    HERMES_HOME: join(home, ".hermes"),
    OPENCODE_CONFIG_DIR: join(home, ".config/opencode"),
    OPENCLAW_HOME: home,
    OPENCLAW_STATE_DIR: join(home, ".openclaw"),
    OPENCLAW_CONFIG_PATH: join(home, ".openclaw/openclaw.json"),
    WORKBUDDY_CONFIG_DIR: join(home, ".workbuddy"),
    PI_CODING_AGENT_DIR: join(home, ".pi/agent"),
    PI_OFFLINE: "1",
    PI_TELEMETRY: "0",
    DSH_HOME: join(home, ".dsh"),
    TMPDIR: join(root, "tmp"),
    TMP: join(root, "tmp"),
    TEMP: join(root, "tmp"),
    PATH: inherited.PATH ?? "",
    TERM: "dumb",
    NO_COLOR: "1",
    CI: "1",
    DISABLE_AUTOUPDATER: "1",
    DISABLE_TELEMETRY: "1",
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
    OPENCODE_DISABLE_AUTOUPDATE: "1",
    OPENCODE_DISABLE_MODELS_FETCH: "1",
  };
  if (process.platform === "win32")
    for (const name of ["SystemRoot", "WINDIR", "ComSpec", "PATHEXT"])
      if (inherited[name]) result[name] = inherited[name];
  return result;
}

async function save(path, value) {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, {
    flag: "wx",
    mode: 0o600,
  });
  try {
    await rename(temporary, path);
  } finally {
    await rm(temporary, { force: true });
  }
}
async function killTree(child) {
  if (!child.pid) return;
  if (process.platform === "win32") {
    await new Promise((resolve) => {
      const killer = spawn(
        "taskkill.exe",
        ["/PID", String(child.pid), "/T", "/F"],
        { stdio: "ignore", shell: false },
      );
      killer.on("error", resolve);
      killer.on("close", resolve);
    });
  } else {
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch (error) {
      if (error.code !== "ESRCH") throw new Error("processCleanupUnverified");
    }
  }
}
async function launch(command, args, { cwd, env, signal, timeoutMs, onSpawn }) {
  if (signal?.aborted) throw new Error("cancelled");
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
      env,
      shell: false,
      detached: process.platform !== "win32",
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    let stdout = "";
    let bytes = 0;
    let reason;
    let cleanup = Promise.resolve();
    const stop = (nextReason) => {
      if (reason) return;
      reason = nextReason;
      cleanup = killTree(child);
    };
    const abort = () => stop("cancelled");
    signal?.addEventListener("abort", abort, { once: true });
    const timer = setTimeout(() => stop("timeout"), timeoutMs);
    child.stdout.on("data", (chunk) => {
      bytes += chunk.length;
      if (bytes > 2 * 1024 * 1024) stop("outputBudgetExceeded");
      else stdout += chunk.toString("utf8");
    });
    // Never save or echo raw stderr, which can contain configuration credentials.
    child.stderr.on("data", (chunk) => {
      bytes += chunk.length;
      if (bytes > 2 * 1024 * 1024) stop("outputBudgetExceeded");
    });
    child.on("error", () => {
      reason = "clientLaunchFailed";
    });
    child.on("close", async (code) => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
      // Also reap MCP descendants that retained this process group after normal exit.
      await killTree(child).catch(() => {
        reason = "processCleanupUnverified";
      });
      await cleanup.catch(() => {
        reason = "processCleanupUnverified";
      });
      if (reason) reject(new Error(reason));
      else if (code !== 0) reject(new Error("clientRunFailed"));
      else resolve(stdout);
    });
    onSpawn?.({ cwd });
    if (signal?.aborted) abort();
  });
}

function validateManifest(manifest) {
  if (
    manifest.schemaVersion !== 1 ||
    !/^[a-f0-9]{40}$/.test(manifest.sourceSha ?? "") ||
    !["fixture", "native"].includes(manifest.mode)
  )
    throw new Error("invalidManifest");
  if (
    ["shell", "command", "args", "argv", "prompt", "credentials"].some(
      (name) => name in manifest,
    )
  )
    throw new Error("unsafeManifestField");
  const kinds = acceptanceResourceKinds(manifest.resourceKinds);
  const agents = manifest.agents ?? [
    ...(kinds.includes("instructions") ? agentIds : resourceClientIds),
  ];
  if (
    !Array.isArray(agents) ||
    agents.length < 1 ||
    agents.some((id) => !resourceClientIds.includes(id)) ||
    new Set(agents).size !== agents.length
  )
    throw new Error("invalidAgentSelection");
  if (
    manifest.maxRetries !== undefined &&
    ![0, 1].includes(manifest.maxRetries)
  )
    throw new Error("invalidRetryLimit");
  if (
    manifest.timeoutMs !== undefined &&
    (!Number.isSafeInteger(manifest.timeoutMs) ||
      manifest.timeoutMs < 100 ||
      manifest.timeoutMs > 120_000)
  )
    throw new Error("invalidTimeout");
  if (
    manifest.fixtureScenario !== undefined &&
    (manifest.mode !== "fixture" ||
      ![
        "success",
        "retryOnce",
        "staleReceipt",
        "timeout",
        "failAlways",
      ].includes(manifest.fixtureScenario))
  )
    throw new Error("invalidFixtureScenario");
  return agents;
}

// There is no known OFox API contract proving an aggregate server-enforced cap.
// A manifest's quota JSON, account balance, token estimate, or self-signed claim
// is deliberately not accepted. Add a reviewed provider adapter here only when
// its real enforcement/reservation/settlement contract is available.
async function trustedBudgetProvider() {
  throw new Error("serverCapVerifierUnavailable");
}

async function verifiedExecutable(manifest, agentId) {
  const spec = manifest.executables?.[agentId];
  if (
    !spec ||
    !isAbsolute(spec.path ?? "") ||
    !/^[a-f0-9]{64}$/.test(spec.sha256 ?? "")
  )
    throw new Error("clientIdentityUnavailable");
  const path = await realpath(spec.path);
  const stat = await lstat(path);
  if (!stat.isFile() || hash(await readFile(path)) !== spec.sha256)
    throw new Error("clientIdentityChanged");
  return { path, sha256: spec.sha256 };
}
async function validateReceipt(profile, proof) {
  let receipt;
  try {
    receipt = JSON.parse(await readFile(profile.receipt, "utf8"));
  } catch {
    throw new Error("mcpReceiptMissing");
  }
  if (
    receipt.schemaVersion !== 1 ||
    receipt.runId !== proof.runId ||
    receipt.sourceSha !== proof.sourceSha ||
    receipt.agentId !== proof.agentId ||
    receipt.toolName !== "attest" ||
    receipt.nonceHash !== hash(proof.nonce) ||
    receipt.instructionHash !== hash(proof.instructionChallenge) ||
    receipt.skillHash !== hash(proof.skillChallenge) ||
    receipt.receiptId !== hash(`${proof.runId}:${proof.nonce}:${proof.agentId}`)
  )
    throw new Error("receiptMismatch");
  return {
    receiptId: receipt.receiptId,
    toolName: "attest",
    instructionContentConsumed: true,
    skillContentConsumed: true,
  };
}

export async function runAcceptance(
  manifest,
  { outputDirectory, signal, onSpawn, checkoutRoot, operator } = {},
) {
  const agents = validateManifest(manifest);
  if (!outputDirectory || !isAbsolute(outputDirectory))
    throw new Error("absoluteOutputDirectoryRequired");
  const budget = new BudgetLedger(manifest.budgetLimitCents ?? 1000);
  const runId = randomUUID();
  const report = {
    schemaVersion: 1,
    runId,
    requestedSourceSha: manifest.sourceSha,
    sourceSha: null,
    identity: await verifyAcceptanceIdentity(manifest, { checkoutRoot }),
    environment: {
      platform: platform(),
      release: release(),
      arch: arch(),
      nodeVersion: process.version,
    },
    mode: manifest.mode,
    status: "running",
    nativeAccepted: false,
    configurationWriterVerified: false,
    profileOrigin: "runnerGeneratedFixtures",
    results: [],
    selectedAgentIds: agents,
    budget: budget.snapshot(),
    billing: {
      currency: "USD",
      limitCents: budget.limitCents,
      actualCents: null,
      status: "notMeasured",
    },
  };
  report.sourceSha = report.identity.sourceSha;
  await mkdir(outputDirectory, { recursive: true, mode: 0o700 });
  const lock = join(outputDirectory, ".acceptance.lock");
  await writeFile(lock, runId, { flag: "wx", mode: 0o600 });
  let sandbox;
  let provider;
  try {
    if (manifest.mode === "native") {
      if (operator && report.identity.status !== "verified") {
        report.status = "blocked";
        report.reason = "sourceArtifactIdentityUnverified";
        return report;
      }
      try {
        provider = operator
          ? await openSupervisedAcceptance(operator, {
              sourceSha: manifest.sourceSha,
              runId,
              apex: manifest.ofoxApex ?? "ofox.ai",
              limitCents: budget.limitCents,
            })
          : await trustedBudgetProvider(manifest);
      } catch {
        report.status = "blocked";
        report.reason = operator
          ? "supervisedOperatorUnavailable"
          : "serverCapVerifierUnavailable";
        report.results = agents.map((agentId) => ({
          agentId,
          status: "blocked",
          evidenceLevel: "none",
          reason: report.reason,
          attempts: 0,
        }));
        return report;
      }
      if (
        report.identity.status !== "verified" ||
        agents.some((agentId) => !manifest.writerProfiles?.[agentId])
      ) {
        report.status = "blocked";
        report.reason =
          report.identity.status !== "verified"
            ? "sourceArtifactIdentityUnverified"
            : "sandboxWriterProfileRequired";
        return report;
      }
    }
    sandbox = await mkdtemp(join(tmpdir(), "ofox-native-acceptance-"));
    for (const agentId of agents) {
      if (signal?.aborted) {
        report.status = "cancelled";
        break;
      }
      const row = {
        agentId,
        status: "failed",
        evidenceLevel:
          manifest.mode === "fixture" ? "fixture" : "nativeClientAndLocalMcp",
        attempts: 0,
        instructionDelivery: !acceptanceResourceKinds(
          manifest.resourceKinds,
        ).includes("instructions")
          ? "notInScope"
          : agentId === "claude"
            ? "explicitCliFileNoKeychain"
            : "nativeDefaultPath",
        defaultInstructionDiscovery: !acceptanceResourceKinds(
          manifest.resourceKinds,
        ).includes("instructions")
          ? "notInScope"
          : agentId === "claude"
            ? "unverified"
            : "pending",
        clientVersion: {
          status: manifest.mode === "fixture" ? "fixtureOnly" : "notExecuted",
          expected: manifest.versions?.[agentId] ?? null,
          measured: null,
        },
        remoteMcp: {
          status: "notExecuted",
          endpoint: "https://learn.microsoft.com/api/mcp",
          tool: "microsoft_docs_search",
          reason: manifest.mode === "fixture" ? "fixtureOnly" : "pending",
        },
      };
      report.results.push(row);
      for (
        let attempt = 0;
        attempt <= (manifest.maxRetries ?? 1);
        attempt += 1
      ) {
        row.attempts += 1;
        const callId = `${agentId}-${attempt}`;
        const supervised = provider?.mode === "supervisedOfficialConsole";
        const reservedCents =
          manifest.mode === "fixture"
            ? 0
            : supervised
              ? budget.limitCents - budget.snapshot().settledCents
              : 100;
        budget.reserve(callId, reservedCents);
        await save(
          join(outputDirectory, "budget-ledger.json"),
          budget.snapshot(),
        );
        const root = join(sandbox, `${agentId}-${attempt}`);
        const proof = {
          root,
          agentId,
          sourceSha: manifest.sourceSha,
          runId,
          nonce: randomBytes(32).toString("hex"),
          instructionChallenge: randomBytes(32).toString("hex"),
          skillChallenge: randomBytes(32).toString("hex"),
          model: manifest.models?.[agentId] ?? "fixture-model",
          apex: manifest.ofoxApex ?? "ofox.ai",
          resourceKinds: acceptanceResourceKinds(manifest.resourceKinds),
        };
        let profile;
        let paidLaunchStarted = false;
        try {
          const binary = provider
            ? await verifiedExecutable(manifest, agentId)
            : null;
          if (signal?.aborted) throw new Error("cancelled");
          if (provider)
            await provider.reserve({
              runId,
              callId,
              maxCents: reservedCents,
              agentId,
              clientSha256: binary.sha256,
            });
          if (signal?.aborted) throw new Error("cancelled");
          const credential = provider
            ? await provider.credentialFor(agentId)
            : undefined;
          if (
            manifest.mode === "native" &&
            manifest.writerProfiles?.[agentId]
          ) {
            const verified = await verifyWriterProfile(
              manifest.writerProfiles[agentId],
              {
                sourceSha: manifest.sourceSha,
                agentId,
                resourceKinds: manifest.resourceKinds,
              },
            );
            profile = {
              ...profilePaths(verified.root, agentId),
              model: proof.model,
              prompt: acceptancePrompt(manifest.resourceKinds),
              resourceKinds: acceptanceResourceKinds(manifest.resourceKinds),
              external: true,
            };
            Object.assign(proof, verified.proof);
            row.writerProfile = {
              status: "verified",
              receiptSha256: verified.receiptSha256,
              verifiedFileCount: verified.verifiedFileCount,
            };
          } else profile = await prepareProfile({ ...proof, credential });
          row.resources = await profileResourceEvidence(profile);
          await mkdir(join(profile.root, "tmp"), {
            recursive: true,
            mode: 0o700,
          });
          const env = isolatedEnvironment(profile.root);
          if (profile.external)
            await writeFile(
              join(profile.root, ".ofox-acceptance-consumed.json"),
              JSON.stringify({ runId, sourceSha: manifest.sourceSha }),
              { flag: "wx", mode: 0o600 },
            );
          await writeFile(
            `${profile.proofPath}.paths`,
            JSON.stringify({
              instructionPath: profile.instructionPath,
              skillPath: profile.skillPath,
            }),
            { mode: 0o600 },
          );
          let command = process.execPath;
          let args = [
            fixtureDriver,
            "offline-driver",
            profile.proofPath,
            manifest.fixtureScenario ?? "success",
            String(attempt),
          ];
          if (provider) {
            command = binary.path;
            const versionOutput = await launch(
              command,
              agentId === "claude" ? ["--bare", "--version"] : ["--version"],
              { cwd: profile.workspace, env, signal, timeoutMs: 10_000 },
            );
            const measured = versionOutput.match(
              /\d+\.\d+\.\d+(?:[-+][A-Za-z0-9._-]+)?/,
            )?.[0];
            if (!measured || measured !== manifest.versions?.[agentId])
              throw new Error("clientVersionMismatch");
            row.clientVersion = {
              status: "verified",
              expected: manifest.versions[agentId],
              measured,
              executableSha256: binary.sha256,
            };
            const invocation = nativeInvocation(agentId, profile);
            args = invocation.args;
            // Only this provider's explicitly supplied dedicated credential is exposed.
            env.OFOX_ACCEPTANCE_TEST_KEY = credential;
            if (agentId === "gemini") {
              env.GEMINI_API_KEY = credential;
              env.GOOGLE_GEMINI_BASE_URL = `https://api.${proof.apex}/gemini`;
            }
            if (agentId === "claude") {
              env.ANTHROPIC_API_KEY = credential;
              env.ANTHROPIC_BASE_URL = `https://api.${proof.apex}/anthropic`;
            }
            if (agentId === "hermes") env.OFOX_API_KEY = credential;
          }
          paidLaunchStarted = !!provider;
          const stdout = await launch(command, args, {
            cwd: profile.workspace,
            env,
            signal,
            timeoutMs: manifest.timeoutMs ?? 60_000,
            onSpawn,
          });
          const evidence =
            manifest.mode === "fixture"
              ? {
                  completed: stdout.split(/\r?\n/).some((line) => {
                    try {
                      const value = JSON.parse(line);
                      return (
                        value.type === "fixture.completed" &&
                        value.runId === runId &&
                        value.agentId === agentId &&
                        value.sourceSha === manifest.sourceSha
                      );
                    } catch {
                      return false;
                    }
                  }),
                  mcpCalled: true,
                }
              : nativeEvents(agentId, stdout);
          row.events = {
            completed: evidence.completed,
            mcpCalled: evidence.mcpCalled,
            structuredEvents: evidence.structuredEvents ?? 1,
            toolCallCount: evidence.toolCallCount ?? 1,
          };
          if (manifest.mode === "native") row.remoteMcp = evidence.remoteMcp;
          // Keep independently observed facts even if another required stage
          // fails. Missing local attest must not erase a real remote MCP result.
          if (!evidence.completed || !evidence.mcpCalled)
            throw new Error("nativeEvidenceIncomplete");
          row.receipt = await validateReceipt(profile, proof);
          if (profile.external)
            await verifyWriterProfile(manifest.writerProfiles[agentId], {
              sourceSha: manifest.sourceSha,
              agentId,
              resourceKinds: manifest.resourceKinds,
            });
          if (
            manifest.mode === "native" &&
            agentId !== "claude" &&
            acceptanceResourceKinds(manifest.resourceKinds).includes(
              "instructions",
            )
          )
            row.defaultInstructionDiscovery = "observedContentConsumption";
          // No token-price estimate is a settlement. Unknown server charges remain held.
          const cents = provider ? await provider.settle({ runId, callId }) : 0;
          budget.settle(callId, cents);
          row.status =
            manifest.mode === "native" &&
            acceptanceResourceKinds(manifest.resourceKinds).includes(
              "instructions",
            ) &&
            (row.remoteMcp.status !== "verified" ||
              row.defaultInstructionDiscovery === "unverified")
              ? "partial"
              : "passed";
          break;
        } catch (error) {
          row.reason = [
            "cancelled",
            "timeout",
            "outputBudgetExceeded",
            "processCleanupUnverified",
            "clientLaunchFailed",
            "clientRunFailed",
            "desktopEntryRequired",
            "receiptMismatch",
            "mcpReceiptMissing",
            "nativeEvidenceIncomplete",
            "clientIdentityUnavailable",
            "clientIdentityChanged",
            "clientVersionMismatch",
            "serverBudgetEvidenceInvalid",
            "supervisedSettlementRequired",
            "supervisedObservationMismatch",
            "supervisedBudgetObservationInvalid",
            "supervisedBudgetExhausted",
            "supervisedEvidenceInvalid",
            "supervisedCaseAlreadyRecorded",
            "supervisedCredentialMismatch",
            "supervisedCredentialUnavailable",
            "supervisedOperatorObservationFailed",
          ].includes(error.message)
            ? error.message
            : "acceptanceFailed";
          if (supervised) {
            if (paidLaunchStarted) await provider.markUnsettled({ callId });
            else await provider.markNotStarted({ callId });
          }
          budget.settle(
            callId,
            provider && (!supervised || paidLaunchStarted) ? null : 0,
          );
          // Never retry a paid, ambiguous or non-idempotent attempt without server reconciliation.
          if (
            provider ||
            signal?.aborted ||
            ["receiptMismatch", "processCleanupUnverified"].includes(row.reason)
          )
            break;
        } finally {
          try {
            await save(
              join(outputDirectory, "budget-ledger.json"),
              budget.snapshot(),
            );
          } finally {
            if (profile && !profile.external)
              await rm(root, { recursive: true, force: true });
          }
        }
      }
      if (
        provider?.mode === "supervisedOfficialConsole" &&
        (row.status === "failed" || provider.snapshot().unresolvedNanoUsd > 0)
      ) {
        report.status =
          provider.snapshot().unresolvedNanoUsd > 0
            ? "pendingSettlement"
            : "failed";
        break;
      }
      if (signal?.aborted) {
        report.status = "cancelled";
        break;
      }
    }
    if (report.status === "running")
      report.status = report.results.every((row) => row.status === "passed")
        ? manifest.mode === "fixture"
          ? "fixturePassed"
          : "nativePassed"
        : report.results.some((row) => row.status === "failed")
          ? "failed"
          : "nativePartial";
    if (
      report.status === "nativePassed" &&
      budget.snapshot().unresolvedCents > 0
    )
      report.status = "pendingSettlement";
    report.configurationWriterVerified =
      manifest.mode === "native" &&
      report.results.length === agents.length &&
      report.results.every((row) => row.writerProfile?.status === "verified");
    if (report.configurationWriterVerified)
      report.profileOrigin = "verifiedSandboxWriterProfiles";
    if (
      manifest.mode === "native" &&
      provider?.mode !== "supervisedOfficialConsole" &&
      budget.snapshot().requestCount > 0 &&
      budget.snapshot().unresolvedCents === 0
    ) {
      report.billing = {
        currency: "USD",
        limitCents: budget.limitCents,
        actualCents: budget.snapshot().settledCents,
        status: "serverSettledForThisRun",
      };
      try {
        const final = await provider.finalize({ runId });
        if (
          !Number.isSafeInteger(final.totalAccountCents) ||
          final.totalAccountCents < report.billing.actualCents ||
          final.totalAccountCents > budget.limitCents ||
          final.pendingCents !== 0 ||
          final.credentialRevoked !== true
        )
          throw new Error("serverBudgetFinalizationUnavailable");
        report.billing.accountTotalCents = final.totalAccountCents;
        report.billing.status = "serverFinalized";
      } catch {
        report.status = "pendingSettlement";
        report.reason = "serverBudgetFinalizationUnavailable";
      }
    }
    report.nativeAccepted =
      provider?.mode !== "supervisedOfficialConsole" &&
      report.status === "nativePassed" &&
      report.identity.status === "verified" &&
      report.configurationWriterVerified &&
      agents.length ===
        (acceptanceResourceKinds(manifest.resourceKinds).includes(
          "instructions",
        )
          ? agentIds
          : resourceClientIds
        ).length &&
      (acceptanceResourceKinds(manifest.resourceKinds).includes("instructions")
        ? agentIds
        : resourceClientIds
      ).every((agent) => agents.includes(agent)) &&
      report.results.every(
        (row) =>
          (!acceptanceResourceKinds(manifest.resourceKinds).includes(
            "instructions",
          ) ||
            row.remoteMcp.status === "verified") &&
          row.defaultInstructionDiscovery !== "unverified",
      );
    if (report.status === "nativePassed" && !report.nativeAccepted)
      report.status = "nativePartial";
    if (provider?.mode === "supervisedOfficialConsole") {
      const snapshot = provider.snapshot();
      const measured =
        snapshot.unresolvedNanoUsd === 0 &&
        snapshot.entries.some(
          (entry) => entry.runId === runId && entry.state === "settled",
        );
      report.supervisedBudget = snapshot;
      report.billing = {
        currency: "USD",
        limitCents: budget.limitCents,
        actualCents: measured
          ? snapshot.actualNanoUsdForRun / 10_000_000
          : null,
        actualNanoUsd: measured ? snapshot.actualNanoUsdForRun : null,
        knownActualNanoUsd: snapshot.actualNanoUsdForRun,
        status: "settlementPending",
      };
      report.status = "pendingSettlement";
      report.nativeAccepted = false;
    }
    return report;
  } finally {
    if (provider) {
      try {
        await provider.close({ runId });
      } catch {
        report.nativeAccepted = false;
        report.status = "pendingSettlement";
        report.reason =
          provider.mode === "supervisedOfficialConsole"
            ? "supervisedClosureUnverified"
            : "serverBudgetClosureUnverified";
      }
      if (provider.mode === "supervisedOfficialConsole")
        report.supervisedBudget = provider.snapshot();
    }
    report.budget = budget.snapshot();
    try {
      await save(join(outputDirectory, "budget-ledger.json"), report.budget);
      await save(join(outputDirectory, "summary.json"), report);
    } finally {
      try {
        if (sandbox) await rm(sandbox, { recursive: true, force: true });
      } finally {
        await rm(lock, { force: true }).catch((error) => {
          if (!["ENOENT", "ENOTDIR"].includes(error.code)) throw error;
        });
      }
    }
  }
}

export async function prepareAcceptanceProfiles(manifest, { outputDirectory }) {
  const agents = validateManifest(manifest);
  if (!isAbsolute(outputDirectory ?? ""))
    throw new Error("absoluteOutputDirectoryRequired");
  const preparedRunId = randomUUID();
  const stagingRoot = await mkdtemp(
    join(tmpdir(), "ofox-acceptance-prepared-"),
  );
  const profiles = [];
  for (const agentId of agents) {
    const root = join(stagingRoot, agentId);
    const profile = await prepareProfile({
      root,
      agentId,
      sourceSha: manifest.sourceSha,
      runId: preparedRunId,
      nonce: randomBytes(32).toString("hex"),
      instructionChallenge: randomBytes(32).toString("hex"),
      skillChallenge: randomBytes(32).toString("hex"),
      model: manifest.models?.[agentId] ?? "fixture-model",
      apex: manifest.ofoxApex ?? "ofox.ai",
      resourcesMode: "writer",
      resourceKinds: manifest.resourceKinds,
    });
    profiles.push({
      agentId,
      root,
      home: profile.home,
      input: join(root, "writer-input.json"),
      receiptPath: join(root, "writer-receipt.json"),
    });
  }
  const result = {
    schemaVersion: 1,
    requestedSourceSha: manifest.sourceSha,
    preparedRunId,
    status: "awaitingSandboxWriter",
    profiles,
  };
  await save(join(outputDirectory, "prepared-profiles.json"), result);
  return result;
}

async function main() {
  const arguments_ = process.argv.slice(2);
  const preparing = arguments_[0] === "prepare";
  const [manifestPath, outputDirectory] = preparing
    ? arguments_.slice(1)
    : arguments_;
  if (
    !manifestPath ||
    !outputDirectory ||
    !isAbsolute(manifestPath) ||
    !isAbsolute(outputDirectory)
  )
    throw new Error(
      "usage: node scripts/native-acceptance.mjs /absolute/manifest.json /absolute/report-directory",
    );
  const controller = new AbortController();
  for (const name of ["SIGINT", "SIGTERM"])
    process.once(name, () => controller.abort());
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  if (preparing) {
    const result = await prepareAcceptanceProfiles(manifest, {
      outputDirectory,
    });
    console.log(
      JSON.stringify({
        preparedRunId: result.preparedRunId,
        status: result.status,
        profiles: result.profiles.length,
      }),
    );
    return;
  }
  const report = await runAcceptance(manifest, {
    outputDirectory,
    signal: controller.signal,
    checkoutRoot: fileURLToPath(new URL("..", import.meta.url)),
  });
  console.log(
    JSON.stringify({
      runId: report.runId,
      status: report.status,
      sourceSha: report.sourceSha,
      nativeAccepted: report.nativeAccepted,
    }),
  );
  if (!["fixturePassed", "nativePassed"].includes(report.status))
    process.exitCode = 2;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  main().catch(() => {
    console.error(
      "nativeAcceptanceFailed: inspect the sanitized report; no raw process output is published",
    );
    process.exitCode = 1;
  });
