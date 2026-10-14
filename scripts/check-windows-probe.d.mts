export interface ProbeRun {
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
}): void;
