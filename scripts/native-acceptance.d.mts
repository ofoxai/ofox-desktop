import type {
  AcceptanceIdentityV1,
  ResourceEvidenceV1,
} from "./native-acceptance-evidence.mjs";
import type { RemoteMcpEvidenceV1 } from "./native-acceptance-drivers.mjs";
import type {
  SupervisedOperatorV1,
  SupervisedSnapshotV1,
} from "./native-acceptance-supervised.mjs";
export interface BudgetSnapshotV1 {
  limitCents: number;
  settledCents: number;
  unresolvedCents: number;
  requestCount: number;
  calls: unknown[];
}
export class BudgetLedger {
  constructor(limitCents?: number);
  reserve(callId: string, maxCents: number): void;
  settle(callId: string, cents: number | null): void;
  snapshot(): BudgetSnapshotV1;
}
export interface NativeAcceptanceReportV1 {
  schemaVersion: number;
  runId: string;
  sourceSha: string | null;
  requestedSourceSha: string;
  identity: AcceptanceIdentityV1;
  environment: {
    platform: string;
    release: string;
    arch: string;
    nodeVersion: string;
  };
  selectedAgentIds: string[];
  mode: string;
  status: string;
  reason?: string;
  nativeAccepted: boolean;
  configurationWriterVerified: boolean;
  profileOrigin: string;
  results: Array<{
    agentId: string;
    status: string;
    evidenceLevel: string;
    attempts: number;
    reason?: string;
    resources?: ResourceEvidenceV1;
    remoteMcp?: RemoteMcpEvidenceV1;
    clientVersion?: {
      status: string;
      expected: string | null;
      measured: string | null;
      executableSha256?: string;
    };
  }>;
  budget: BudgetSnapshotV1;
  supervisedBudget?: SupervisedSnapshotV1;
  billing: {
    currency: string;
    limitCents: number;
    actualCents: number | null;
    actualNanoUsd?: number | null;
    knownActualNanoUsd?: number;
    status: string;
    accountTotalCents?: number;
  };
}
export function isolatedEnvironment(
  root: string,
  inherited?: Record<string, string | undefined>,
): Record<string, string>;
export function runAcceptance(
  manifest: Record<string, unknown>,
  options: {
    outputDirectory: string;
    signal?: AbortSignal;
    onSpawn?: (observation: { cwd: string }) => void;
    checkoutRoot?: string;
    operator?: SupervisedOperatorV1;
  },
): Promise<NativeAcceptanceReportV1>;
export function prepareAcceptanceProfiles(
  manifest: Record<string, unknown>,
  options: { outputDirectory: string },
): Promise<{
  schemaVersion: number;
  requestedSourceSha: string;
  preparedRunId: string;
  status: string;
  profiles: Array<{
    agentId: string;
    root: string;
    home: string;
    input: string;
    receiptPath: string;
  }>;
}>;
