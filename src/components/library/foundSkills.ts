import type { LibraryInventory, SkillDefinition } from "@/lib/api/library";
import { RESOURCE_TARGET_IDS } from "@/config/resourceTargets";

type Entry = LibraryInventory["entries"][number];
export type FoundSkill = { entry: Entry; others: string[] };
export function foundSkillAgents(entry: Entry): string[] {
  return [
    ...new Set([
      ...(entry.locations ?? []).map((location) => location.targetId),
      ...entry.sources.map((source) => source.toolId),
    ]),
  ].filter((tool) => tool && tool !== "shared");
}

/** One displayed version per name, like Magpie: shared first, then agent order.
 * The inventory entry itself stays intact: its ticket authorizes only that version.
 */
export function foundSkills(
  entries: Entry[],
  library: SkillDefinition[],
): FoundSkill[] {
  const groups = new Map<string, Entry[]>();
  for (const entry of entries) {
    const name = (entry.directory ?? entry.name).toLowerCase();
    groups.set(name, [...(groups.get(name) ?? []), entry]);
  }
  const rank = (entry: Entry) => {
    if (entry.locations?.some((location) => location.targetId === "shared"))
      return -1;
    const ranks = foundSkillAgents(entry).map((tool) => {
      const index = (RESOURCE_TARGET_IDS as readonly string[]).indexOf(tool);
      return index < 0 ? RESOURCE_TARGET_IDS.length : index;
    });
    return Math.min(...ranks, RESOURCE_TARGET_IDS.length);
  };
  return [...groups.values()]
    .flatMap((group) => {
      if (
        group.some((entry) => entry.matchedResourceIds.length) ||
        library.some(
          (row) =>
            row.directory.toLowerCase() ===
            (group[0].directory ?? group[0].name).toLowerCase(),
        )
      )
        return [];
      const [entry, ...other] = [...group].sort(
        (a, b) => rank(a) - rank(b) || a.entryId.localeCompare(b.entryId),
      );
      return [{ entry, others: [...new Set(other.flatMap(foundSkillAgents))] }];
    })
    .sort((a, b) => a.entry.name.localeCompare(b.entry.name));
}

export function canResolveSkill(row: SkillDefinition, tool: string): boolean {
  const target = row.targets.find((target) => target.toolId === tool);
  return (
    tool !== "shared" &&
    target?.status === "conflict" &&
    /library\.(skillNameConflict|skillEntryChanged)/.test(
      target.disabledReason ?? "",
    )
  );
}

/** Different versions outside the direct writable target still remain visible. */
export function nativeSkillDifferences(
  entries: Entry[],
  row: SkillDefinition,
): string[] {
  return [
    ...new Set(
      entries
        .filter(
          (entry) =>
            (entry.directory ?? entry.name).toLowerCase() ===
              row.directory.toLowerCase() &&
            !entry.matchedResourceIds.includes(row.id),
        )
        .flatMap(foundSkillAgents),
    ),
  ];
}
