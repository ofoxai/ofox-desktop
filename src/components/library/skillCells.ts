import type {
  ResourceTarget,
  SkillDefinition,
  SkillTarget,
} from "@/lib/api/library";
import { RESOURCE_TARGET_IDS } from "@/config/resourceTargets";

export type SkillCellState =
  /** Ofox gave it to the agent and it is in place. */
  | "on"
  /** Given to the agent, not written yet. */
  | "pending"
  /** The agent reads it through another Ofox-owned folder (~/.agents/skills). */
  | "shared"
  /** The agent already has the very same skill, put there by someone else. */
  | "adoptable"
  /** A different skill by this name, or Ofox's copy drifted or can't be read. */
  | "attention"
  /** The agent was not found. */
  | "unavailable"
  /** The agent can't take this skill. */
  | "unsupported"
  | "off";

export interface SkillCell {
  state: SkillCellState;
  /** Shown lit: the agent has the skill. */
  on: boolean;
  /** Clicking writes; otherwise the chip only explains itself. */
  toggleable: boolean;
  reason?: string | null;
}

const ATTENTION = ["conflict", "needsReview", "unreadable", "failed"];

/**
 * Show detected agents (and existing assignments) only within OFox's bindable
 * tool catalogue and the backend's Skills adapters. Detection alone does not
 * make a tool a supported binding target. Keep saved assignments intact when
 * hiding a column so toggling another agent cannot withdraw legacy targets.
 */
export function skillColumns(
  targets: SkillTarget[] | undefined,
  skills: SkillDefinition[],
): string[] {
  const ids = new Set(
    (targets ?? [])
      .filter((target) => target.detected)
      .map((target) => target.id),
  );
  for (const skill of skills)
    for (const target of skill.targets)
      if (
        target.toolId !== "shared" &&
        (target.desired ||
          target.status === "applied" ||
          (!targets && target.available))
      )
        ids.add(target.toolId);
  const supported = targets && new Set(targets.map((target) => target.id));
  return RESOURCE_TARGET_IDS.filter(
    (id) => ids.has(id) && (!supported || supported.has(id)),
  );
}

export function skillCell(skill: SkillDefinition, toolId: string): SkillCell {
  const target: ResourceTarget | undefined = skill.targets.find(
    (target) => target.toolId === toolId,
  );
  if (!target) return { state: "unsupported", on: false, toggleable: false };
  if (ATTENTION.includes(target.status))
    return {
      state: "attention",
      on: !!target.desired,
      toggleable: false,
      reason: target.disabledReason,
    };
  if (target.desired)
    return {
      state: target.status === "applied" ? "on" : "pending",
      on: true,
      toggleable: true,
    };
  if (target.status === "shared")
    return { state: "shared", on: true, toggleable: false };
  if (target.adoptable)
    return { state: "adoptable", on: true, toggleable: true };
  if (!target.available)
    return { state: "unavailable", on: false, toggleable: false };
  if (target.disabledReason)
    return {
      state: "unsupported",
      on: false,
      toggleable: false,
      reason: target.disabledReason,
    };
  return { state: "off", on: false, toggleable: true };
}

export const desiredSkillTools = (skill: SkillDefinition) =>
  skill.targets
    .filter((target) => target.desired)
    .map((target) => target.toolId);

/** What clicking a chip asks for: the agents to keep, and those to take over first. */
export function skillToggle(
  skill: SkillDefinition,
  toolId: string,
  cell: SkillCell,
): { tools: string[]; adopt: string[] } {
  const current = desiredSkillTools(skill);
  if (cell.state === "off") return { tools: [...current, toolId], adopt: [] };
  const tools = current.filter((tool) => tool !== toolId);
  return { tools, adopt: cell.state === "adoptable" ? [toolId] : [] };
}

/**
 * The All chip covers every column that can take the skill. When each of them
 * has it, it takes the skill from all of them (and the shared folder);
 * otherwise it gives it to the rest.
 */
export function skillAll(
  skill: SkillDefinition,
  columns: string[],
): { count: number; allOn: boolean; tools: string[]; adopt: string[] } {
  const cells = columns.map((tool) => ({ tool, cell: skillCell(skill, tool) }));
  const takeable = cells.filter(({ cell }) =>
    ["on", "pending", "adoptable", "off"].includes(cell.state),
  );
  const allOn = takeable.length > 0 && takeable.every(({ cell }) => cell.on);
  const adopt = takeable
    .filter(({ cell }) => cell.state === "adoptable")
    .map(({ tool }) => tool);
  const current = desiredSkillTools(skill);
  // Switching all off also lets go of the shared folder, which the reader
  // agents' chips can't do themselves.
  const tools = allOn
    ? current.filter((tool) => !columns.includes(tool) && tool !== "shared")
    : [
        ...new Set([
          ...current,
          ...takeable
            .filter(({ cell }) => cell.state === "off")
            .map(({ tool }) => tool),
          ...adopt,
        ]),
      ];
  return { count: takeable.length, allOn, tools, adopt };
}
