export interface BuildInput {
  root: string;
  output: string;
  platform: string;
  version: string;
  sourceSha: string;
}
export interface ReleaseInput {
  artifactsRoot: string;
  provenanceRoot: string;
  notaryRoot: string;
  sourceSha: string;
  version: string;
  expectedTeamId: string;
  auth: { appleId: string; password: string; teamId: string };
  run?: (command: string, args: string[]) => Promise<string>;
}
export interface ReleaseManifest {
  schemaVersion: number;
  sourceSha: string;
  version: string;
  platforms: Array<{
    platform: string;
    artifact: { file: string; sha256: string; bytes: number };
    notarization?: { status: string };
  }>;
}
export function recordBuild(input: BuildInput): Promise<unknown>;
export function verifyRelease(input: ReleaseInput): Promise<ReleaseManifest>;
export function checkRelease(input: {
  artifactsRoot: string;
  sourceSha: string;
  version: string;
}): Promise<ReleaseManifest>;
