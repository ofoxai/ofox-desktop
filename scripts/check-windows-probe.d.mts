export interface ProbeRun {
  id?: number;
  event: string;
  head_sha: string;
  status: string;
  conclusion: string | null;
  path: string;
  head_repository?: { full_name: string } | null;
  html_url: string;
}
export function successfulProbe(
  runs: ProbeRun[],
  sha: string,
  repository: string,
): ProbeRun | undefined;
export function verifyWindowsProbe(environment: {
  EXPECTED_SHA?: string;
  EXPECTED_REPOSITORY?: string;
}): ProbeRun;
export function waitForWindowsProbe(
  environment: { EXPECTED_SHA?: string; EXPECTED_REPOSITORY?: string },
  options?: {
    listRuns?: (identity: {
      sha: string;
      repository: string;
    }) => ProbeRun[] | Promise<ProbeRun[]>;
    sleep?: (milliseconds: number) => unknown | Promise<unknown>;
    maxAttempts?: number;
  },
): Promise<ProbeRun>;
