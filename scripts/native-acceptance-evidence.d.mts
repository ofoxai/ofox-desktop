export interface AcceptanceIdentityV1 {
  status: string;
  requestedSourceSha: string;
  sourceSha: string | null;
  checkout: { status: string; commitSha?: string; clean?: boolean };
  artifact: {
    status: string;
    sha256?: string;
    bytes?: number;
    provenanceSha256?: string;
    sourceSha?: string;
    evidence?: string;
  };
}
export interface ResourceEvidenceV1 {
  instructions: { sha256: string; bytes: number; version: string };
  mcp: { sha256: string; bytes: number; version: string };
  skills: { sha256: string; bytes: number; version: string };
}
export interface WriterProfileReceiptV1 {
  schemaVersion: number;
  writer: string;
  sourceSha: string;
  compiledSourceSha: string;
  compiledSourceClean: boolean;
  preparedRunId: string;
  root: string;
  agentId: string;
  operations: Array<{
    kind: string;
    operationId: string;
    result: string;
    receiptSha256: string;
  }>;
  files: Array<{ relativePath: string; kind: string; sha256: string }>;
}
export function verifyAcceptanceIdentity(
  manifest: Record<string, unknown>,
  options?: { checkoutRoot?: string },
): Promise<AcceptanceIdentityV1>;
export function profileResourceEvidence(profile: {
  instructionPath: string;
  mcpConfig: string;
  mcpNativePath?: string;
  skillPath: string;
}): Promise<ResourceEvidenceV1>;
export function verifyWriterProfile(
  spec: { root: string; receiptPath: string; receiptSha256: string },
  expected: { sourceSha: string; agentId: string; now?: number },
): Promise<{
  root: string;
  receipt: WriterProfileReceiptV1;
  proof: Record<string, unknown>;
  receiptSha256: string;
  writer: string;
  verifiedFileCount: number;
}>;
