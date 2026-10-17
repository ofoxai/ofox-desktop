// @vitest-environment node
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import * as childProcess from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { runAcceptance } from "../../scripts/native-acceptance.mjs";
import * as nativeEvidence from "../../scripts/native-acceptance-evidence.mjs";
import { prepareProfile } from "../../scripts/native-acceptance-drivers.mjs";
import { openSupervisedAcceptance } from "../../scripts/native-acceptance-supervised.mjs";
import type {
  OperatorContextV1,
  OperatorObservationV1,
} from "../../scripts/native-acceptance-supervised.mjs";

vi.mock("node:child_process", async (original) => ({
  ...(await original<typeof import("node:child_process")>()),
  spawn: vi.fn(),
}));

const roots: string[] = [];
const hash = (value: string | Buffer) =>
  createHash("sha256").update(value).digest("hex");
const sha = "a".repeat(40);
async function temp() {
  const root = await mkdtemp(join(tmpdir(), "ofox-operator-test-"));
  roots.push(root);
  return root;
}
async function setup() {
  const root = await temp();
  const screenshot = Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    Buffer.from("synthetic unit-test screenshot"),
  ]);
  const path = join(root, "synthetic.png");
  await writeFile(path, screenshot, { mode: 0o600 });
  const state = {
    used: 0,
    actual: 0 as number | null,
    patch: {} as Record<string, unknown>,
  };
  const observation = (request: OperatorContextV1): OperatorObservationV1 => ({
    ...request,
    observedAt: Date.now() - 1,
    expiresAt: Date.now() + 60_000,
    period: "total",
    enabled: true,
    mode: "notify_and_block",
    limitNanoUsd: 10_000_000_000,
    usedNanoUsd: state.used,
    caseActualNanoUsd: state.actual,
    requestIdsSha256: [hash(`synthetic-${request.runId}-${request.callId}`)],
    evidence: {
      path,
      sha256: hash(screenshot),
      origin: "https://app.ofox.ai/manage/api-keys",
    },
    ...state.patch,
  });
  const operator = {
    ledgerDirectory: join(root, "ledger"),
    keyIdSha256: hash("synthetic-key-id"),
    credentialSha256: hash("synthetic-test-credential"),
    observe: vi.fn(async (request: OperatorContextV1) => observation(request)),
    credentialFor: vi.fn(async () => "synthetic-test-credential"),
    close: vi.fn(async (request: OperatorContextV1) => ({
      ...observation(request),
      credentialDisabled: true,
    })),
  };
  const options = { sourceSha: sha, runId: "run-one", apex: "ofox.ai" };
  const reserve = {
    runId: options.runId,
    callId: "codex-0",
    agentId: "codex",
    clientSha256: hash("synthetic-client"),
  };
  return { root, state, operator, options, reserve, observation };
}
afterEach(async () => {
  vi.restoreAllMocks();
  vi.mocked(childProcess.spawn).mockReset();
  await Promise.all(
    roots.splice(0).map((root) => rm(root, { recursive: true, force: true })),
  );
});
describe("code-only supervised acceptance", () => {
  it("uses an explicit operator only after source checks; a manifest quota never unlocks native execution", async () => {
    const root = await temp();
    const operator = {
      ledgerDirectory: join(root, "ledger"),
      keyIdSha256: hash("synthetic-key-id"),
      credentialSha256: hash("synthetic-test-credential"),
      observe: vi.fn(),
      credentialFor: vi.fn(),
      close: vi.fn(),
    };
    const report = await runAcceptance(
      { schemaVersion: 1, mode: "native", sourceSha: sha, agents: ["codex"] },
      { outputDirectory: join(root, "report"), operator },
    );
    expect(report.reason).toBe("sourceArtifactIdentityUnverified");
    expect(operator.observe).not.toHaveBeenCalled();
    expect(operator.credentialFor).not.toHaveBeenCalled();
    expect(report.nativeAccepted).toBe(false);
  });
  it("binds fresh operator evidence to nonce, source, client, key and exact case identity", async () => {
    for (const patch of [
      { nonce: "replayed" },
      { sourceSha: "b".repeat(40) },
      { clientSha256: hash("other-client") },
      { keyIdSha256: hash("other-key") },
      { credentialSha256: hash("other-credential") },
      { runId: "other-run" },
      { callId: "other-case" },
      { ledgerSha256: hash("old-ledger") },
      { observedAt: Date.now() - 180_000 },
      { expiresAt: Date.now() - 1 },
      { enabled: false },
      { mode: "notify_only" },
      { limitNanoUsd: 11_000_000_000 },
      {
        evidence: {
          path: "/not/read",
          sha256: hash("x"),
          origin: "https://example.invalid",
        },
      },
    ]) {
      const { operator, options, reserve, state } = await setup();
      state.patch = patch;
      const session = await openSupervisedAcceptance(operator, options);
      await expect(session.reserve(reserve)).rejects.toThrow();
      expect(operator.credentialFor).not.toHaveBeenCalled();
      expect(session.snapshot().unresolvedNanoUsd).toBe(0);
      await session.close();
    }
  });
  it("records exact known NanoDollar charges while keeping console evidence separate from server finality", async () => {
    const { operator, options, reserve, state } = await setup();
    const session = await openSupervisedAcceptance(operator, options);
    await session.reserve(reserve);
    expect(await session.credentialFor("codex")).toBe(
      "synthetic-test-credential",
    );
    state.used = state.actual = 123_456;
    expect(await session.settle(reserve)).toBe(1); // conservative local cent hold
    expect(session.snapshot()).toMatchObject({
      mode: "supervisedOfficialConsole",
      actualNanoUsdForRun: 123_456,
      unresolvedNanoUsd: 0,
    });
    await session.close();
    expect(operator.close).toHaveBeenCalledOnce();
    const ledger = await readFile(
      join(operator.ledgerDirectory, operator.keyIdSha256, "ledger.json"),
      "utf8",
    );
    expect(ledger).not.toContain("synthetic-test-credential");
    expect(ledger).not.toContain('"serverFinalized"');
  });
  it("retains cross-run cumulative spend and unknown holds until explicit reconciliation", async () => {
    const { operator, options, reserve, state } = await setup();
    let session = await openSupervisedAcceptance(operator, options);
    await session.reserve(reserve);
    state.used = state.actual = 20_000_000;
    expect(await session.settle(reserve)).toBe(2);
    await session.close();
    const next = { ...options, runId: "run-two" };
    const nextCase = { ...reserve, runId: next.runId };
    session = await openSupervisedAcceptance(operator, next);
    await session.reserve(nextCase);
    state.used = 30_000_000;
    state.actual = null;
    expect(await session.settle(nextCase)).toBeNull();
    await expect(
      session.reserve({ ...nextCase, callId: "codex-1" }),
    ).rejects.toThrow("supervisedSettlementRequired");
    expect(session.snapshot().unresolvedNanoUsd).toBe(9_980_000_000);
    await session.close();
    const third = { ...options, runId: "run-three" };
    session = await openSupervisedAcceptance(operator, third);
    const observations = operator.observe.mock.calls.length;
    await expect(
      session.reserve({ ...reserve, runId: third.runId }),
    ).rejects.toThrow("supervisedSettlementRequired");
    expect(operator.observe).toHaveBeenCalledTimes(observations);
    state.actual = 10_000_000;
    await session.reconcilePending();
    expect(session.snapshot()).toMatchObject({
      lastObservedUsedNanoUsd: 30_000_000,
      unresolvedNanoUsd: 0,
    });
    await session.reserve({ ...reserve, runId: third.runId });
    await session.markNotStarted({ callId: reserve.callId });
    await session.close();
  });
  it("rejects reentry and a second process lease; interrupted work remains held on reopen", async () => {
    const { operator, options, reserve, observation } = await setup();
    let release!: (value: OperatorObservationV1) => void;
    let request!: OperatorContextV1;
    operator.observe.mockImplementationOnce(async (context) => {
      request = context;
      return await new Promise<OperatorObservationV1>((resolve) => {
        release = resolve;
      });
    });
    const session = await openSupervisedAcceptance(operator, options);
    const pending = session.reserve(reserve);
    await vi.waitFor(() => expect(release).toBeTypeOf("function"));
    await expect(
      session.reserve({ ...reserve, callId: "codex-1" }),
    ).rejects.toThrow("supervisedReentryRejected");
    await expect(
      openSupervisedAcceptance(operator, {
        ...options,
        runId: "other-process",
      }),
    ).rejects.toThrow("supervisedSessionLocked");
    release(observation(request));
    await pending;
    await session.close();
    const ledgerPath = join(
      operator.ledgerDirectory,
      operator.keyIdSha256,
      "ledger.json",
    );
    // Synthetic crash fixture: a durable running entry survived without a live lease.
    const ledger = JSON.parse(await readFile(ledgerPath, "utf8"));
    ledger.entries[0].state = "running";
    await writeFile(ledgerPath, JSON.stringify(ledger));
    const reopened = await openSupervisedAcceptance(operator, {
      ...options,
      runId: "after-crash",
    });
    expect(reopened.snapshot().entries[0]).toMatchObject({
      state: "pending",
      reason: "interruptedCase",
    });
    await expect(
      reopened.reserve({ ...reserve, runId: "after-crash" }),
    ).rejects.toThrow("supervisedSettlementRequired");
    await reopened.close();
  });
  it("rejects a second authorized case while the first is still running", async () => {
    const { operator, options, reserve } = await setup();
    const session = await openSupervisedAcceptance(operator, options);
    await session.reserve(reserve);
    await expect(
      session.reserve({ ...reserve, callId: "codex-1" }),
    ).rejects.toThrow("supervisedSettlementRequired");
    expect(session.snapshot().unresolvedNanoUsd).toBe(10_000_000_000);
    expect(operator.observe).toHaveBeenCalledOnce();
    await session.close();
  });
  it("rechecks closure for historical pending work and exposes an unconfirmed disable", async () => {
    const { operator, options, reserve, observation } = await setup();
    let session = await openSupervisedAcceptance(operator, options);
    await session.reserve(reserve);
    await session.close();
    session = await openSupervisedAcceptance(operator, {
      ...options,
      runId: "resume-pending",
    });
    operator.close.mockImplementationOnce(async (request) => ({
      ...observation(request),
      credentialDisabled: false,
    }));
    await expect(session.close()).rejects.toThrow(
      "supervisedClosureUnverified",
    );
    expect(operator.close).toHaveBeenCalledTimes(2);
    expect(session.snapshot()).toMatchObject({
      unresolvedNanoUsd: 10_000_000_000,
      closure: { status: "unverified", credentialDisabled: false },
    });
  });
  it("checks disable after a crash between settled billing and the first closure", async () => {
    const { operator, options, reserve, state } = await setup();
    const session = await openSupervisedAcceptance(operator, options);
    await session.reserve(reserve);
    state.used = state.actual = 10_000_000;
    await session.settle(reserve);
    // Synthetic dead-process fixture: release only the stale lease, never the ledger.
    await rm(
      join(operator.ledgerDirectory, operator.keyIdSha256, "session.lock"),
    );
    const reopened = await openSupervisedAcceptance(operator, {
      ...options,
      runId: "after-settle-crash",
    });
    await reopened.close();
    expect(operator.close).toHaveBeenCalledOnce();
    expect(reopened.snapshot().closure).toMatchObject({
      status: "verified",
      credentialDisabled: true,
    });
  });
  it("cannot reuse an older verified closure after another authorization settles and crashes", async () => {
    const { operator, options, reserve, state } = await setup();
    let session = await openSupervisedAcceptance(operator, options);
    await session.reserve(reserve);
    state.used = state.actual = 10_000_000;
    await session.settle(reserve);
    await session.close();
    const oldClosure = session.snapshot().closure;
    const secondOptions = { ...options, runId: "second-authorized-run" };
    const secondCase = { ...reserve, runId: secondOptions.runId };
    session = await openSupervisedAcceptance(operator, secondOptions);
    await session.reserve(secondCase);
    state.used = 20_000_000;
    state.actual = 10_000_000;
    await session.settle(secondCase);
    await rm(
      join(operator.ledgerDirectory, operator.keyIdSha256, "session.lock"),
    );
    const reopened = await openSupervisedAcceptance(operator, {
      ...options,
      runId: "after-second-crash",
    });
    await reopened.close();
    expect(operator.close).toHaveBeenCalledTimes(2);
    expect(reopened.snapshot().closure).not.toEqual(oldClosure);
    expect(reopened.snapshot().closure?.authorizationId).not.toBe(
      oldClosure?.authorizationId,
    );
  });
  it("refuses replayed or regressed settlement and a mismatching credential without exposing it", async () => {
    const { operator, options, reserve, state } = await setup();
    const session = await openSupervisedAcceptance(operator, options);
    await session.reserve(reserve);
    operator.credentialFor.mockResolvedValueOnce("wrong-synthetic-secret");
    await expect(session.credentialFor("codex")).rejects.toThrow(
      "supervisedCredentialMismatch",
    );
    state.used = 10_000_000;
    state.actual = 20_000_000;
    expect(await session.settle(reserve)).toBeNull();
    expect(session.snapshot().unresolvedNanoUsd).toBe(10_000_000_000);
    await session.close();
    const reopened = await openSupervisedAcceptance(operator, {
      ...options,
      runId: "new-run",
    });
    state.used = 0;
    state.actual = 0;
    await reopened.reconcilePending();
    expect(reopened.snapshot().unresolvedNanoUsd).toBe(10_000_000_000);
    await reopened.close();
  });
  it("never resets a corrupt persistent ledger or accepts a manifest operator", async () => {
    const { operator, options, root } = await setup();
    const session = await openSupervisedAcceptance(operator, options);
    await session.close();
    const path = join(
      operator.ledgerDirectory,
      operator.keyIdSha256,
      "ledger.json",
    );
    await writeFile(path, "{corrupt");
    await expect(openSupervisedAcceptance(operator, options)).rejects.toThrow(
      "supervisedLedgerInvalid",
    );
    expect(await readFile(path, "utf8")).toBe("{corrupt");
    const report = await runAcceptance(
      {
        schemaVersion: 1,
        sourceSha: sha,
        mode: "native",
        operator: { verified: true },
        quota: { total: 10 },
      },
      { outputDirectory: join(root, "manifest-only") },
    );
    expect(report).toMatchObject({
      status: "blocked",
      reason: "serverCapVerifierUnavailable",
      nativeAccepted: false,
    });
  });
  it("scrubs callback errors before they leave the supervisor boundary", async () => {
    const first = await setup();
    first.operator.observe.mockRejectedValueOnce(
      new Error("synthetic-sensitive-observation"),
    );
    const denied = await openSupervisedAcceptance(
      first.operator,
      first.options,
    );
    await expect(denied.reserve(first.reserve)).rejects.toThrow(
      "supervisedOperatorObservationFailed",
    );
    await denied.close();
    const second = await setup();
    const session = await openSupervisedAcceptance(
      second.operator,
      second.options,
    );
    await session.reserve(second.reserve);
    second.operator.credentialFor.mockRejectedValueOnce(
      new Error("synthetic-sensitive-credential"),
    );
    await expect(session.credentialFor("codex")).rejects.toThrow(
      "supervisedCredentialUnavailable",
    );
    second.operator.close.mockRejectedValueOnce(
      new Error("synthetic-sensitive-closure"),
    );
    await expect(session.close()).rejects.toThrow(
      "supervisedOperatorClosureFailed",
    );
    const reopened = await openSupervisedAcceptance(second.operator, {
      ...second.options,
      runId: "reopen",
    });
    expect(reopened.snapshot().unresolvedNanoUsd).toBe(10_000_000_000);
    await reopened.close();
  });
  it("stops the native runner after one simulated case with unknown charges and never claims finalization", async () => {
    const { root, operator, state } = await setup();
    state.actual = null;
    const profileRoot = join(root, "synthetic-profile");
    const profile = await prepareProfile({
      root: profileRoot,
      agentId: "codex",
      runId: "prepared-run",
      sourceSha: sha,
      nonce: "fixture-nonce",
      instructionChallenge: "fixture-instruction",
      skillChallenge: "fixture-skill",
    });
    const proof = JSON.parse(await readFile(profile.proofPath, "utf8"));
    // Only this test replaces identity and writer verification with synthetic inputs.
    vi.spyOn(nativeEvidence, "verifyAcceptanceIdentity").mockResolvedValue({
      status: "verified",
      requestedSourceSha: sha,
      sourceSha: sha,
      checkout: { status: "verified" },
      artifact: { status: "verified" },
    });
    vi.spyOn(nativeEvidence, "verifyWriterProfile").mockResolvedValue({
      root: profileRoot,
      receipt: {} as never,
      proof,
      receiptSha256: hash("synthetic-receipt"),
      writer: "ofoxSandboxWriterV1",
      verifiedFileCount: 4,
    });
    const executable = join(root, "synthetic-client");
    await writeFile(executable, "synthetic executable bytes");
    const spawn = vi.mocked(childProcess.spawn).mockImplementation(((
      _command: string,
      args: string[],
    ) => {
      const process_ = Object.assign(new EventEmitter(), {
        stdout: new EventEmitter(),
        stderr: new EventEmitter(),
      });
      setImmediate(async () => {
        if (args.includes("--version"))
          process_.stdout.emit("data", Buffer.from("1.2.3\n"));
        else {
          await writeFile(
            proof.receipt,
            JSON.stringify({
              schemaVersion: 1,
              runId: proof.runId,
              sourceSha: sha,
              agentId: "codex",
              toolName: "attest",
              nonceHash: hash(proof.nonce),
              instructionHash: hash(proof.instructionChallenge),
              skillHash: hash(proof.skillChallenge),
              receiptId: hash(`${proof.runId}:${proof.nonce}:codex`),
            }),
          );
          process_.stdout.emit(
            "data",
            Buffer.from(
              [
                {
                  type: "item.completed",
                  item: {
                    type: "mcp_tool_call",
                    server: "ofox_acceptance",
                    tool: "attest",
                    status: "completed",
                    id: "fixture-call",
                  },
                },
                { type: "turn.completed" },
              ]
                .map((event) => JSON.stringify(event))
                .join("\n"),
            ),
          );
        }
        process_.emit("close", 0);
      });
      return process_;
    }) as never);
    const report = await runAcceptance(
      {
        schemaVersion: 1,
        mode: "native",
        sourceSha: sha,
        agents: ["codex", "gemini"],
        models: { codex: "fixture-model" },
        versions: { codex: "1.2.3" },
        executables: {
          codex: {
            path: executable,
            sha256: hash("synthetic executable bytes"),
          },
        },
        writerProfiles: {
          codex: { root: profileRoot },
          gemini: { root: "not opened" },
        },
      },
      { outputDirectory: join(root, "runner-report"), operator },
    );
    expect(spawn).toHaveBeenCalledTimes(2); // fixed version + one mocked native case
    expect(report.results).toHaveLength(1);
    expect(report).toMatchObject({
      status: "pendingSettlement",
      nativeAccepted: false,
      billing: {
        actualCents: null,
        actualNanoUsd: null,
        status: "settlementPending",
      },
      supervisedBudget: {
        mode: "supervisedOfficialConsole",
        unresolvedNanoUsd: 10_000_000_000,
      },
    });
    expect(
      operator.observe.mock.calls.map(([request]) => request.phase),
    ).toEqual(["beforeCase", "afterCase"]);
    expect(JSON.stringify(report)).not.toContain("synthetic-test-credential");
    expect(JSON.stringify(report)).not.toContain("serverFinalized");
  });
  it("cancels after a fresh operator authorization without retrieving a credential or starting a client", async () => {
    const { root, operator, observation } = await setup();
    const controller = new AbortController();
    vi.spyOn(nativeEvidence, "verifyAcceptanceIdentity").mockResolvedValue({
      status: "verified",
      requestedSourceSha: sha,
      sourceSha: sha,
      checkout: { status: "verified" },
      artifact: { status: "verified" },
    });
    const path = join(root, "synthetic-client");
    await writeFile(path, "synthetic executable bytes");
    operator.observe.mockImplementationOnce(async (request) => {
      controller.abort();
      return observation(request);
    });
    const report = await runAcceptance(
      {
        schemaVersion: 1,
        sourceSha: sha,
        mode: "native",
        agents: ["codex"],
        executables: {
          codex: { path, sha256: hash("synthetic executable bytes") },
        },
        writerProfiles: { codex: { root: "never opened" } },
      },
      {
        outputDirectory: join(root, "cancelled-report"),
        operator,
        signal: controller.signal,
      },
    );
    expect(childProcess.spawn).not.toHaveBeenCalled();
    expect(operator.credentialFor).not.toHaveBeenCalled();
    expect(report.supervisedBudget?.unresolvedNanoUsd).toBe(0);
    expect(report.results[0]?.reason).toBe("cancelled");
    expect(report.nativeAccepted).toBe(false);
    expect(report.billing.actualCents).toBeNull();
    expect(report.billing.actualNanoUsd).toBeNull();
  });
});
