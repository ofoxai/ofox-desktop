export interface OperatorContextV1 {
  phase: "beforeCase" | "afterCase" | "reconcilePending" | "close";
  nonce: string;
  runId: string;
  callId: string;
  sourceSha: string;
  agentId: string | null;
  clientSha256: string | null;
  keyIdSha256: string;
  credentialSha256: string;
  ledgerSha256: string;
}
export interface OperatorEvidenceV1 {
  path: string;
  sha256: string;
  origin: string;
}
export interface OperatorObservationV1 extends OperatorContextV1 {
  observedAt: number;
  expiresAt: number;
  evidence: OperatorEvidenceV1;
  period: "total";
  enabled: boolean;
  mode: "notify_and_block" | "notify_only";
  limitNanoUsd: number;
  usedNanoUsd: number;
  caseActualNanoUsd?: number | null;
  requestIdsSha256?: string[];
}
export interface SupervisedOperatorV1 {
  ledgerDirectory: string;
  keyIdSha256: string;
  credentialSha256: string;
  observe(context: OperatorContextV1): Promise<OperatorObservationV1>;
  credentialFor(identity: {
    keyIdSha256: string;
    credentialSha256: string;
  }): Promise<string>;
  close(context: OperatorContextV1): Promise<
    OperatorContextV1 & {
      observedAt: number;
      expiresAt: number;
      evidence: OperatorEvidenceV1;
      credentialDisabled: boolean;
    }
  >;
}
export interface SupervisedEntryV1 {
  runId: string;
  callId: string;
  sourceSha: string;
  agentId: string;
  clientSha256: string;
  state: string;
  heldNanoUsd: number;
  actualNanoUsd: number | null;
  reason?: string;
}
export interface SupervisedSnapshotV1 {
  mode: "supervisedOfficialConsole";
  keyIdSha256: string;
  credentialSha256: string;
  lastObservedUsedNanoUsd: number;
  actualNanoUsdForRun: number;
  unresolvedNanoUsd: number;
  entries: SupervisedEntryV1[];
  closure: {
    status: string;
    runId: string;
    authorizationId: string | null;
    credentialDisabled: boolean;
    sha256?: string;
    origin?: string;
    observedAt?: number;
  } | null;
}
export function openSupervisedAcceptance(
  operator: SupervisedOperatorV1,
  options: {
    sourceSha: string;
    runId: string;
    apex: string;
    limitCents?: number;
    now?: () => number;
  },
): Promise<{
  mode: "supervisedOfficialConsole";
  reserve(request: {
    runId: string;
    callId: string;
    agentId: string;
    clientSha256: string;
    maxCents?: number;
  }): Promise<void>;
  credentialFor(agentId: string): Promise<string>;
  settle(request: { runId: string; callId: string }): Promise<number | null>;
  markUnsettled(request: { callId: string }): Promise<void>;
  markNotStarted(request: { callId: string }): Promise<void>;
  reconcilePending(): Promise<void>;
  snapshot(): SupervisedSnapshotV1;
  close(): Promise<void>;
}>;
