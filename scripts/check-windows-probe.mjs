import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

// Query the named workflow, then independently validate the returned evidence.
function matchingProbe(run, sha, repository) {
  return (
    // PR run.head_sha names the source branch, but checkout tests a merge ref.
    (run.event === "push" || run.event === "workflow_dispatch") &&
    run.head_sha === sha &&
    run.path === ".github/workflows/windows-probe.yml" &&
    run.head_repository?.full_name === repository
  );
}
export function successfulProbe(runs, sha, repository) {
  return runs.find(
    (run) =>
      matchingProbe(run, sha, repository) &&
      run.status === "completed" &&
      run.conclusion === "success",
  );
}

function validateIdentity({
  EXPECTED_SHA: sha,
  EXPECTED_REPOSITORY: repository,
}) {
  if (
    !/^[a-f0-9]{40}$/.test(sha ?? "") ||
    !/^[\w.-]+\/[\w.-]+$/.test(repository ?? "")
  ) {
    throw new Error("Expected a full commit SHA and owner/repository");
  }
  return { sha, repository };
}
function fetchProbeRuns({ sha, repository }) {
  const pages = JSON.parse(
    execFileSync(
      "gh",
      [
        "api",
        "--paginate",
        "--slurp",
        `repos/${repository}/actions/workflows/windows-probe.yml/runs?head_sha=${sha}&per_page=100`,
      ],
      { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 },
    ),
  );
  return pages.flatMap((page) => page.workflow_runs ?? []);
}

export function verifyWindowsProbe(identity) {
  const { sha, repository } = validateIdentity(identity);
  const run = successfulProbe(
    fetchProbeRuns({ sha, repository }),
    sha,
    repository,
  );
  if (!run)
    throw new Error(
      `No successful Windows Probe for ${sha}. Run Windows Probe on this exact commit before retrying release.`,
    );
  console.log(`Windows Probe verified for ${sha}: ${run.html_url}`);
  return run;
}

// Branch pushes launch this workflow and the independent Windows Probe together.
// A bounded wait handles that race without allowing pending or foreign evidence.
export async function waitForWindowsProbe(
  identity,
  {
    listRuns = fetchProbeRuns,
    sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    maxAttempts = 180,
  } = {},
) {
  const { sha, repository } = validateIdentity(identity);
  if (!Number.isInteger(maxAttempts) || maxAttempts < 1 || maxAttempts > 180)
    throw new Error("Invalid Windows Probe wait limit");
  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    const runs = await listRuns({ sha, repository });
    const success = successfulProbe(runs, sha, repository);
    if (success) return success;
    const latest = runs
      .filter((run) => matchingProbe(run, sha, repository))
      .sort((a, b) => b.id - a.id)[0];
    if (latest?.status === "completed")
      throw new Error(`Windows Probe failed for ${sha}: ${latest.html_url}`);
    if (attempt + 1 < maxAttempts) await sleep(30_000);
  }
  throw new Error(`Timed out waiting for Windows Probe for ${sha}`);
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  if (process.argv.includes("--wait")) {
    const run = await waitForWindowsProbe(process.env);
    console.log(
      `Windows Probe verified for ${process.env.EXPECTED_SHA}: ${run.html_url}`,
    );
  } else {
    verifyWindowsProbe(process.env);
  }
}
