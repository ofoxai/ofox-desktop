import type { SkillDefinition } from "@/lib/api/library";

/** Repository names are case insensitive; repository paths are not. */
const normalized = (key: string) => {
  const colon = key.indexOf(":");
  return colon < 0 ? key : key.slice(0, colon).toLowerCase() + key.slice(colon);
};
export function collectedRepositorySkill(
  row: SkillDefinition,
  key: string,
): boolean {
  const sameId = normalized(row.id) === normalized(key);
  const colon = key.indexOf(":");
  if (
    colon < 0 ||
    row.sourceKind !== "repository" ||
    row.repository?.toLowerCase() !== key.slice(0, colon).toLowerCase()
  )
    return false;
  try {
    // A tracking branch may contain slashes. Only a pinned commit establishes
    // where the full repository path begins for older resource IDs.
    const url = new URL(row.sourceUrl ?? "");
    if (url.protocol !== "https:" || url.hostname !== "github.com")
      return false;
    const match = url.pathname.match(
      /^\/[^/]+\/[^/]+\/blob\/[a-f0-9]{40}\/(.*)$/i,
    );
    if (!match) return sameId;
    const document = decodeURIComponent(match[1]);
    const path =
      document === "SKILL.md" ? "" : document?.replace(/\/SKILL\.md$/, "");
    return path === key.slice(colon + 1);
  } catch {
    return sameId && !row.sourceUrl;
  }
}
