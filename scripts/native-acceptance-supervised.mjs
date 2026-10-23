import { createHash, randomUUID } from "node:crypto";
import {
  lstat,
  mkdir,
  open,
  readFile,
  realpath,
  rename,
  rm,
} from "node:fs/promises";
import { dirname, isAbsolute, join } from "node:path";
import { resourceClientIds } from "./resource-targets.mjs";

const digest = (value) => createHash("sha256").update(value).digest("hex");
const sha256 = /^[a-f0-9]{64}$/;
const identifier = /^[a-z0-9-]{1,64}$/;
const nanoPerCent = 10_000_000;
const knownAgents = new Set(resourceClientIds);
const pngHeader = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
async function readPrivate(path) {
  const metadata = await lstat(path);
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size > 4 * 1024 * 1024
  )
    throw new Error("supervisedEvidenceInvalid");
  return await readFile(path);
}
async function atomic(path, value) {
  const temporary = `${path}.${randomUUID()}.tmp`;
  const handle = await open(temporary, "wx", 0o600);
  try {
    await handle.writeFile(`${JSON.stringify(value, null, 2)}\n`);
    await handle.sync();
  } finally {
    await handle.close();
  }
  try {
    await rename(temporary, path);
    const directory = await open(dirname(path), "r");
    try {
      await directory.sync();
    } catch (error) {
      if (
        process.platform !== "win32" ||
        !["EINVAL", "EPERM", "EISDIR"].includes(error.code)
      )
        throw error;
    } finally {
      await directory.close();
    }
  } finally {
    await rm(temporary, { force: true });
  }
}
const validAmount = (value) => Number.isSafeInteger(value) && value >= 0;

// This is an in-process, trusted operator boundary, never a manifest plugin.
// The callback observes the official UI; a screenshot is supporting evidence,
// not a provider signature, an atomic reservation, or final settlement proof.
export async function openSupervisedAcceptance(
  operator,
  { sourceSha, runId, apex, limitCents = 1000, now = Date.now },
) {
  if (
    !operator ||
    !isAbsolute(operator.ledgerDirectory ?? "") ||
    !sha256.test(operator.keyIdSha256 ?? "") ||
    !sha256.test(operator.credentialSha256 ?? "") ||
    typeof operator.observe !== "function" ||
    typeof operator.credentialFor !== "function" ||
    typeof operator.close !== "function" ||
    !/^[a-f0-9]{40}$/.test(sourceSha) ||
    !identifier.test(runId) ||
    !["ofox.ai", "ofox.io"].includes(apex) ||
    !Number.isSafeInteger(limitCents) ||
    limitCents < 1 ||
    limitCents > 1000
  )
    throw new Error("supervisedOperatorInvalid");
  await mkdir(operator.ledgerDirectory, { recursive: true, mode: 0o700 });
  const directoryMetadata = await lstat(operator.ledgerDirectory);
  if (
    !directoryMetadata.isDirectory() ||
    directoryMetadata.isSymbolicLink() ||
    (process.platform !== "win32" && (directoryMetadata.mode & 0o077) !== 0)
  )
    throw new Error("supervisedLedgerNotPrivate");
  const directory = join(
    await realpath(operator.ledgerDirectory),
    operator.keyIdSha256,
  );
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const keyDirectoryMetadata = await lstat(directory);
  if (
    !keyDirectoryMetadata.isDirectory() ||
    keyDirectoryMetadata.isSymbolicLink() ||
    (process.platform !== "win32" && (keyDirectoryMetadata.mode & 0o077) !== 0)
  )
    throw new Error("supervisedLedgerInvalid");
  const lockPath = join(directory, "session.lock");
  const leaseId = randomUUID();
  const lock = await open(lockPath, "wx", 0o600).catch(() => {
    throw new Error("supervisedSessionLocked");
  });
  await lock.writeFile(JSON.stringify({ runId, leaseId, pid: process.pid }));
  await lock.sync();
  await lock.close();
  const ledgerPath = join(directory, "ledger.json");
  let ledger;
  try {
    try {
      ledger = JSON.parse((await readPrivate(ledgerPath)).toString("utf8"));
    } catch (error) {
      if (error.code !== "ENOENT") throw new Error("supervisedLedgerInvalid");
      ledger = {
        schemaVersion: 1,
        keyIdSha256: operator.keyIdSha256,
        credentialSha256: operator.credentialSha256,
        lastObservedUsedNanoUsd: 0,
        entries: [],
      };
    }
    if (
      ledger.schemaVersion !== 1 ||
      ledger.keyIdSha256 !== operator.keyIdSha256 ||
      ledger.credentialSha256 !== operator.credentialSha256 ||
      !validAmount(ledger.lastObservedUsedNanoUsd) ||
      !Array.isArray(ledger.entries) ||
      ledger.entries.length > 1000 ||
      ledger.entries.some(
        (entry) =>
          !identifier.test(entry.runId ?? "") ||
          !identifier.test(entry.callId ?? "") ||
          ![
            "authorizing",
            "running",
            "pending",
            "settled",
            "notStarted",
          ].includes(entry.state) ||
          !validAmount(entry.heldNanoUsd) ||
          entry.heldNanoUsd > 10_000_000_000 ||
          !validAmount(entry.beforeUsedNanoUsd) ||
          !/^[a-f0-9]{40}$/.test(entry.sourceSha ?? "") ||
          !knownAgents.has(entry.agentId) ||
          !sha256.test(entry.clientSha256 ?? "") ||
          (entry.state === "settled" && entry.actualNanoUsd === null) ||
          (entry.state !== "settled" && entry.actualNanoUsd !== null) ||
          (entry.actualNanoUsd !== null && !validAmount(entry.actualNanoUsd)),
      ) ||
      new Set(ledger.entries.map((entry) => `${entry.runId}:${entry.callId}`))
        .size !== ledger.entries.length
    )
      throw new Error("supervisedLedgerInvalid");
    for (const entry of ledger.entries)
      if (["authorizing", "running"].includes(entry.state)) {
        entry.state = "pending";
        entry.reason = "interruptedCase";
      }
    await atomic(ledgerPath, ledger);
  } catch (error) {
    await rm(lockPath, { force: true });
    throw error;
  }
  let busy = false;
  let closed = false;
  let started = false;
  let credential;
  const exclusive = async (body) => {
    if (closed || busy) throw new Error("supervisedReentryRejected");
    busy = true;
    try {
      return await body();
    } finally {
      busy = false;
    }
  };
  const persist = () => atomic(ledgerPath, ledger);
  const latestAuthorizationId = () => {
    const entry = ledger.entries.findLast(
      (row) => row.beforeEvidence || row.state === "settled",
    );
    return entry
      ? digest(
          `${entry.runId}:${entry.callId}:${entry.beforeEvidence?.nonceSha256 ?? "legacy"}`,
        )
      : null;
  };
  function context(phase, entry = {}) {
    return {
      phase,
      nonce: randomUUID(),
      runId: entry.runId ?? runId,
      callId: entry.callId ?? "closure",
      sourceSha: entry.sourceSha ?? sourceSha,
      agentId: entry.agentId ?? null,
      clientSha256: entry.clientSha256 ?? null,
      keyIdSha256: operator.keyIdSha256,
      credentialSha256: operator.credentialSha256,
      ledgerSha256: digest(JSON.stringify(ledger)),
    };
  }
  async function evidence(observation, request) {
    if (
      !observation ||
      Object.entries(request).some(
        ([key, value]) => observation[key] !== value,
      ) ||
      !Number.isSafeInteger(observation.observedAt) ||
      !Number.isSafeInteger(observation.expiresAt) ||
      observation.observedAt > now() ||
      observation.observedAt < now() - 120_000 ||
      observation.expiresAt <= now() ||
      observation.expiresAt > observation.observedAt + 120_000
    )
      throw new Error("supervisedObservationMismatch");
    const item = observation.evidence;
    if (
      !item ||
      !isAbsolute(item.path ?? "") ||
      !sha256.test(item.sha256 ?? "")
    )
      throw new Error("supervisedEvidenceInvalid");
    let url;
    try {
      url = new URL(item.origin);
    } catch {
      throw new Error("supervisedEvidenceInvalid");
    }
    if (
      url.origin !== `https://app.${apex}` ||
      url.username ||
      url.password ||
      url.search ||
      url.hash ||
      !["/manage/api-keys", "/analytics", "/analytics/activity"].includes(
        url.pathname,
      )
    )
      throw new Error("supervisedEvidenceInvalid");
    const bytes = await readPrivate(item.path);
    if (
      !bytes.subarray(0, pngHeader.length).equals(pngHeader) ||
      digest(bytes) !== item.sha256
    )
      throw new Error("supervisedEvidenceInvalid");
    return {
      sha256: item.sha256,
      origin: url.href,
      observedAt: observation.observedAt,
      nonceSha256: digest(request.nonce),
      ledgerSha256: request.ledgerSha256,
    };
  }
  async function observe(request) {
    let result;
    try {
      result = await operator.observe(request);
    } catch {
      throw new Error("supervisedOperatorObservationFailed");
    }
    const proof = await evidence(result, request);
    if (
      result.period !== "total" ||
      result.enabled !== true ||
      result.mode !== "notify_and_block" ||
      !validAmount(result.limitNanoUsd) ||
      result.limitNanoUsd < 1 ||
      result.limitNanoUsd > limitCents * nanoPerCent ||
      !validAmount(result.usedNanoUsd) ||
      result.usedNanoUsd > result.limitNanoUsd ||
      result.usedNanoUsd < ledger.lastObservedUsedNanoUsd
    )
      throw new Error("supervisedBudgetObservationInvalid");
    return { result, proof };
  }
  async function reconcile(entry, phase) {
    try {
      const { result, proof } = await observe(context(phase, entry));
      ledger.lastObservedUsedNanoUsd = result.usedNanoUsd;
      entry.afterEvidence = proof;
      const actual = result.caseActualNanoUsd;
      const ids = result.requestIdsSha256;
      if (
        actual === null ||
        !validAmount(actual) ||
        actual > result.limitNanoUsd ||
        actual !== result.usedNanoUsd - entry.beforeUsedNanoUsd ||
        !Array.isArray(ids) ||
        ids.length < 1 ||
        ids.length > 200 ||
        new Set(ids).size !== ids.length ||
        ids.some((id) => !sha256.test(id)) ||
        ledger.entries.some(
          (other) =>
            other !== entry &&
            other.requestIdsSha256?.some((id) => ids.includes(id)),
        )
      ) {
        entry.state = "pending";
        entry.reason = "operatorSettlementUnknown";
        await persist();
        return null;
      }
      entry.state = "settled";
      entry.actualNanoUsd = actual;
      entry.requestIdsSha256 = ids;
      delete entry.reason;
      await persist();
      return Math.ceil(actual / nanoPerCent);
    } catch {
      entry.state = "pending";
      entry.reason = "operatorSettlementUnknown";
      await persist();
      return null;
    }
  }
  return {
    mode: "supervisedOfficialConsole",
    reserve: (request) =>
      exclusive(async () => {
        if (
          request.runId !== runId ||
          !identifier.test(request.callId ?? "") ||
          !knownAgents.has(request.agentId) ||
          !sha256.test(request.clientSha256 ?? "")
        )
          throw new Error("supervisedCaseIdentityInvalid");
        if (
          ledger.entries.some((entry) =>
            ["authorizing", "running", "pending"].includes(entry.state),
          )
        )
          throw new Error("supervisedSettlementRequired");
        if (
          ledger.entries.some(
            (entry) => entry.runId === runId && entry.callId === request.callId,
          )
        )
          throw new Error("supervisedCaseAlreadyRecorded");
        if (ledger.entries.length >= 1000)
          throw new Error("supervisedLedgerFull");
        const entry = {
          runId,
          callId: request.callId,
          sourceSha,
          agentId: request.agentId,
          clientSha256: request.clientSha256,
          state: "authorizing",
          heldNanoUsd: limitCents * nanoPerCent,
          actualNanoUsd: null,
          beforeUsedNanoUsd: ledger.lastObservedUsedNanoUsd,
        };
        ledger.entries.push(entry);
        await persist();
        try {
          const { result, proof } = await observe(context("beforeCase", entry));
          if (result.usedNanoUsd >= result.limitNanoUsd)
            throw new Error("supervisedBudgetExhausted");
          entry.beforeUsedNanoUsd = result.usedNanoUsd;
          entry.heldNanoUsd = result.limitNanoUsd - result.usedNanoUsd;
          entry.beforeEvidence = proof;
          entry.state = "running";
          // Persist new authorization and closure invalidation together, before
          // any client can launch. A previous run's disable proof cannot cover it.
          ledger.lastClosure = {
            status: "unverified",
            runId,
            authorizationId: latestAuthorizationId(),
            credentialDisabled: false,
          };
          ledger.lastObservedUsedNanoUsd = result.usedNanoUsd;
          await persist();
          started = true;
        } catch (error) {
          entry.state = "notStarted";
          entry.heldNanoUsd = 0;
          entry.reason = "operatorAuthorizationRejected";
          await persist();
          throw error;
        }
      }),
    credentialFor: (agentId) =>
      exclusive(async () => {
        const entry = ledger.entries.find(
          (row) =>
            row.runId === runId &&
            row.agentId === agentId &&
            row.state === "running",
        );
        if (!entry) throw new Error("supervisedCaseNotAuthorized");
        if (credential === undefined) {
          try {
            credential = await operator.credentialFor({
              keyIdSha256: operator.keyIdSha256,
              credentialSha256: operator.credentialSha256,
            });
          } catch {
            throw new Error("supervisedCredentialUnavailable");
          }
        }
        if (
          typeof credential !== "string" ||
          credential.length === 0 ||
          digest(credential) !== operator.credentialSha256
        )
          throw new Error("supervisedCredentialMismatch");
        return credential;
      }),
    settle: ({ runId: requestedRun, callId }) =>
      exclusive(async () => {
        const entry = ledger.entries.find(
          (row) =>
            row.runId === requestedRun &&
            row.callId === callId &&
            row.state === "running",
        );
        if (!entry || requestedRun !== runId)
          throw new Error("supervisedCaseNotAuthorized");
        return await reconcile(entry, "afterCase");
      }),
    markUnsettled: ({ callId }) =>
      exclusive(async () => {
        const entry = ledger.entries.find(
          (row) => row.runId === runId && row.callId === callId,
        );
        if (entry && ["running", "authorizing"].includes(entry.state)) {
          entry.state = "pending";
          entry.reason = "caseCompletionUnknown";
          await persist();
        }
      }),
    markNotStarted: ({ callId }) =>
      exclusive(async () => {
        const entry = ledger.entries.find(
          (row) => row.runId === runId && row.callId === callId,
        );
        if (entry && ["running", "authorizing"].includes(entry.state)) {
          entry.state = "notStarted";
          entry.heldNanoUsd = 0;
          entry.reason = "clientNotStarted";
          await persist();
        }
      }),
    reconcilePending: () =>
      exclusive(async () => {
        for (const entry of ledger.entries.filter(
          (row) => row.state === "pending",
        ))
          if ((await reconcile(entry, "reconcilePending")) === null) break;
      }),
    snapshot: () => ({
      mode: "supervisedOfficialConsole",
      keyIdSha256: operator.keyIdSha256,
      credentialSha256: operator.credentialSha256,
      lastObservedUsedNanoUsd: ledger.lastObservedUsedNanoUsd,
      actualNanoUsdForRun: ledger.entries
        .filter((entry) => entry.runId === runId)
        .reduce((sum, entry) => sum + (entry.actualNanoUsd ?? 0), 0),
      unresolvedNanoUsd: ledger.entries
        .filter((entry) =>
          ["running", "pending", "authorizing"].includes(entry.state),
        )
        .reduce((sum, entry) => sum + entry.heldNanoUsd, 0),
      entries: structuredClone(ledger.entries),
      closure: ledger.lastClosure ? structuredClone(ledger.lastClosure) : null,
    }),
    close: () =>
      exclusive(async () => {
        try {
          const authorizationId = latestAuthorizationId();
          const needsClosure =
            started ||
            ledger.entries.some((entry) =>
              ["authorizing", "running", "pending"].includes(entry.state),
            ) ||
            ledger.lastClosure?.credentialDisabled === false ||
            (authorizationId !== null &&
              (ledger.lastClosure?.status !== "verified" ||
                ledger.lastClosure?.credentialDisabled !== true ||
                ledger.lastClosure?.authorizationId !== authorizationId));
          for (const entry of ledger.entries)
            if (["running", "authorizing"].includes(entry.state)) {
              entry.state = "pending";
              entry.reason = "interruptedCase";
            }
          await persist();
          if (needsClosure) {
            ledger.lastClosure = {
              status: "unverified",
              runId,
              authorizationId,
              credentialDisabled: false,
            };
            await persist();
            const request = context("close");
            let closure;
            try {
              closure = await operator.close(request);
            } catch {
              throw new Error("supervisedOperatorClosureFailed");
            }
            ledger.lastClosure = {
              status:
                closure.credentialDisabled === true ? "verified" : "unverified",
              runId,
              authorizationId,
              ...(await evidence(closure, request)),
              credentialDisabled: closure.credentialDisabled === true,
            };
            await persist();
            if (closure.credentialDisabled !== true)
              throw new Error("supervisedClosureUnverified");
          }
          await persist();
        } finally {
          credential = undefined;
          closed = true;
          const current = JSON.parse(
            (await readPrivate(lockPath)).toString("utf8"),
          );
          if (current.leaseId !== leaseId)
            throw new Error("supervisedLockChanged");
          await rm(lockPath);
        }
      }),
  };
}
