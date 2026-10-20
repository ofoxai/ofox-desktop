export const agentIds: readonly string[];
export const microsoftLearnEndpoint: string;
export const microsoftLearnQuery: string;
export interface NativeProfilePaths {
  root: string;
  home: string;
  workspace: string;
  receipt: string;
  proofPath: string;
  instructionPath: string;
  skillPath: string;
  mcpConfig: string;
  mcpNativePath: string;
}
export interface RemoteMcpEvidenceV1 {
  status: string;
  endpoint: string;
  tool: string;
  reason?: string;
  evidence?: string;
  callIdHash?: string;
  responseSha256?: string;
}
export function profilePaths(root: string, agentId: string): NativeProfilePaths;
export function acceptancePrompt(): string;
export function prepareProfile(input: {
  root: string;
  agentId: string;
  runId: string;
  sourceSha: string;
  nonce: string;
  instructionChallenge: string;
  skillChallenge: string;
  model?: string;
  apex?: string;
  credential?: string;
  resourcesMode?: "fixture" | "writer";
}): Promise<
  NativeProfilePaths & { prompt: string; model: string; key: string }
>;
export function nativeInvocation(
  agentId: string,
  paths: {
    workspace: string;
    prompt: string;
    model: string;
    mcpConfig?: string;
    home?: string;
    instructionPath?: string;
  },
): { args: string[]; cwd: string; shell: false };
export function nativeEvents(
  agentId: string,
  output: string,
): {
  completed: boolean;
  mcpCalled: boolean;
  structuredEvents: number;
  toolCallCount: number;
  remoteMcp: RemoteMcpEvidenceV1;
};
