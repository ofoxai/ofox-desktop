import { libraryApi } from "@/lib/api/library";
import { skillCell } from "./skillCells";

export interface SkillDistributionResult {
  tool: string;
  status: "applied" | "skipped" | "conflict" | "failed";
  reason?: string;
}
export interface CollectedSkillVersion {
  epoch: number;
  revision: number;
  artifactHash?: string;
}

/** A target failure must not prevent independent agents from receiving a skill. */
export async function distributeSkill(
  id: string,
  agents: string[],
  expected: CollectedSkillVersion,
): Promise<SkillDistributionResult[]> {
  const results: SkillDistributionResult[] = [];
  for (const tool of new Set(agents)) {
    try {
      // Refresh between writes: every preview is bound to the actual latest revision.
      const data = await libraryApi.list("skills");
      const row = data.skills?.find((skill) => skill.id === id);
      if (!row) throw new Error("library.resourceMissing");
      if (data.epoch !== expected.epoch)
        throw new Error("library.epochChanged");
      if (
        row.revision !== expected.revision ||
        row.artifactHash !== expected.artifactHash
      )
        throw new Error("library.revisionChanged");
      const cell = skillCell(row, tool);
      if (["on", "shared"].includes(cell.state)) {
        results.push({ tool, status: "skipped" });
        continue;
      }
      if (["attention", "unsupported", "unavailable"].includes(cell.state)) {
        results.push({
          tool,
          status: cell.state === "attention" ? "conflict" : "failed",
          reason: cell.reason ?? "library.nativeCliRequired",
        });
        continue;
      }
      const preview = await libraryApi.previewSkillDistribution(
        id,
        [tool],
        cell.state === "adoptable" ? [tool] : [],
        expected.revision,
        expected.epoch,
      );
      const blocked = preview.targets.find(
        (target) => target.status !== "pending",
      );
      if (blocked) {
        await libraryApi.cancel(preview.operationId);
        results.push({
          tool,
          status: blocked.status === "conflict" ? "conflict" : "failed",
          reason: blocked.reason ?? "library.failed",
        });
        continue;
      }
      const applied = await libraryApi.apply(preview.operationId);
      const failed = applied.targets.find(
        (target) => target.status !== "applied",
      );
      results.push(
        failed
          ? {
              tool,
              status: "failed",
              reason: failed.reason ?? "library.failed",
            }
          : { tool, status: "applied" },
      );
    } catch (reason) {
      // Only library error keys are exposed; raw filesystem/network errors may contain secrets.
      const key = String(reason).match(/\blibrary\.[a-zA-Z]+\b/)?.[0];
      results.push({ tool, status: "failed", reason: key ?? "library.failed" });
    }
  }
  return results;
}
