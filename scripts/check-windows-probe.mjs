import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

// Query the named workflow, then independently validate the returned evidence.
export function successfulProbe(runs, sha, repository) {
  return runs.find(
    (run) =>
      // PR run.head_sha names the source branch, but checkout tests a merge ref.
      (run.event === "push" || run.event === "workflow_dispatch") &&
      run.head_sha === sha &&
      run.status === "completed" &&
      run.conclusion === "success" &&
      run.path === ".github/workflows/windows-probe.yml" &&
      run.head_repository?.full_name === repository,
  );
}

export function verifyWindowsProbe({
  EXPECTED_SHA: sha,
  EXPECTED_REPOSITORY: repository,
}) {
  if (
    !/^[a-f0-9]{40}$/.test(sha ?? "") ||
    !/^[\w.-]+\/[\w.-]+$/.test(repository ?? "")
  ) {
    throw new Error("Expected a full commit SHA and owner/repository");
  }
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
  const run = successfulProbe(
    pages.flatMap((page) => page.workflow_runs ?? []),
    sha,
    repository,
  );
  if (!run)
    throw new Error(
      `No successful Windows Probe for ${sha}. Run Windows Probe on this exact commit before retrying release.`,
    );
  console.log(`Windows Probe verified for ${sha}: ${run.html_url}`);
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  verifyWindowsProbe(process.env);
}
