import { describe, expect, it } from "vitest";
import type {
  ResourceTarget,
  SkillDefinition,
  SkillTarget,
} from "@/lib/api/library";
import {
  skillAll,
  skillCell,
  skillColumns,
  skillToggle,
} from "@/components/library/skillCells";

const target = (
  toolId: string,
  extra: Partial<ResourceTarget> = {},
): ResourceTarget => ({
  toolId,
  path: `~/.${toolId}/skills/pdf`,
  targetGroupId: toolId,
  available: true,
  discoveryStatus: "ready",
  status: "unassigned",
  desired: false,
  ...extra,
});
const skill = (targets: ResourceTarget[]): SkillDefinition => ({
  id: "pdf",
  name: "pdf",
  directory: "pdf",
  revision: 3,
  sourceState: "ready",
  targets,
});
const agent = (id: string, detected = true): SkillTarget => ({
  id,
  label: id,
  detected,
  readsShared: false,
  scopeKnown: true,
});

describe("skillColumns", () => {
  it("does not resurrect non-bindable or unsupported agents from saved assignments", () => {
    const saved = skill([
      target("cursor", { desired: true, status: "applied" }),
      target("claude-desktop", { desired: true, status: "applied" }),
      target("codebuddy", { desired: true, status: "applied" }),
      target("gemini", { desired: true, status: "applied" }),
      target("codex", { desired: true, status: "applied" }),
    ]);
    expect(skillColumns([agent("codex")], [saved])).toEqual(["codex"]);
  });
  it("also filters non-bindable agents in the older-backend fallback", () => {
    expect(
      skillColumns(undefined, [
        skill([
          target("cursor"),
          target("claude-desktop"),
          target("codebuddy"),
          target("openclaw"),
          target("codex"),
        ]),
      ]),
    ).toEqual(["codex", "openclaw"]);
  });
  it("All changes only visible agents and preserves existing hidden assignments", () => {
    const saved = skill([
      target("cursor", { desired: true, status: "applied" }),
      target("codex"),
      target("claude-desktop"),
    ]);
    const columns = skillColumns(
      [agent("cursor"), agent("codex"), agent("claude-desktop")],
      [saved],
    );
    expect(skillAll(saved, columns).tools).toEqual(["cursor", "codex"]);
    const enabled = {
      ...saved,
      targets: saved.targets.map((t) =>
        t.toolId === "codex" ? { ...t, desired: true, status: "applied" } : t,
      ),
    };
    expect(skillAll(enabled, columns).tools).toEqual(["cursor"]);
  });

  it("shows only detected bindable agents with a Skills adapter, in tool order", () => {
    expect(
      skillColumns(
        [
          agent("claude-desktop"),
          agent("cursor"),
          agent("pi"),
          agent("openclaw"),
          agent("unregistered"),
          agent("codebuddy"),
          agent("codex"),
          agent("claude"),
          agent("gemini", false),
        ],
        [],
      ),
    ).toEqual(["claude", "codex", "openclaw", "pi"]);
  });
  it("keeps an undetected agent a skill is already given to, but never the shared folder", () => {
    expect(
      skillColumns(
        [agent("claude"), agent("gemini", false)],
        [
          skill([
            target("gemini", { desired: true, status: "applied" }),
            target("shared", { desired: true, status: "applied" }),
          ]),
        ],
      ),
    ).toEqual(["claude", "gemini"]);
  });
  it("falls back to the available skill targets when the backend lists no agents", () => {
    expect(
      skillColumns(undefined, [
        skill([
          target("codex"),
          target("pi", { available: false }),
          target("shared", { status: "shared" }),
          target("claude"),
        ]),
      ]),
    ).toEqual(["claude", "codex"]);
  });
});

describe("skillCell", () => {
  it("is lit and toggleable once given, with pending until written", () => {
    expect(
      skillCell(
        skill([target("codex", { desired: true, status: "applied" })]),
        "codex",
      ),
    ).toMatchObject({ state: "on", on: true, toggleable: true });
    expect(
      skillCell(
        skill([target("codex", { desired: true, status: "pending" })]),
        "codex",
      ),
    ).toMatchObject({ state: "pending", on: true, toggleable: true });
  });
  it("lights an agent that reads the shared folder without letting it be clicked", () => {
    expect(
      skillCell(skill([target("gemini", { status: "shared" })]), "gemini"),
    ).toMatchObject({ state: "shared", on: true, toggleable: false });
  });
  it("lights an identical entry someone else put there and lets it be taken away", () => {
    expect(
      skillCell(skill([target("claude", { adoptable: true })]), "claude"),
    ).toMatchObject({ state: "adoptable", on: true, toggleable: true });
  });
  it("explains conflicts, missing agents and unsupported targets without writing", () => {
    for (const status of ["conflict", "needsReview", "unreadable", "failed"])
      expect(
        skillCell(
          skill([
            target("claude", {
              status,
              disabledReason: "library.skillNameConflict",
            }),
          ]),
          "claude",
        ),
      ).toMatchObject({
        state: "attention",
        toggleable: false,
        reason: "library.skillNameConflict",
      });
    expect(
      skillCell(skill([target("pi", { available: false })]), "pi").state,
    ).toBe("unavailable");
    expect(
      skillCell(
        skill([target("pi", { disabledReason: "library.scopeUnknown" })]),
        "pi",
      ),
    ).toMatchObject({ state: "unsupported", reason: "library.scopeUnknown" });
    expect(skillCell(skill([]), "pi").state).toBe("unsupported");
    expect(skillCell(skill([target("pi")]), "pi")).toMatchObject({
      state: "off",
      on: false,
      toggleable: true,
    });
  });
});

describe("skillToggle and skillAll", () => {
  const row = skill([
    target("codex", { desired: true, status: "applied" }),
    target("claude", { adoptable: true }),
    target("gemini", { status: "shared" }),
    target("pi"),
    target("hermes", { available: false }),
  ]);
  it("adds an agent, removes one, and takes over an adoptable entry before removing it", () => {
    expect(skillToggle(row, "pi", skillCell(row, "pi"))).toEqual({
      tools: ["codex", "pi"],
      adopt: [],
    });
    expect(skillToggle(row, "codex", skillCell(row, "codex"))).toEqual({
      tools: [],
      adopt: [],
    });
    expect(skillToggle(row, "claude", skillCell(row, "claude"))).toEqual({
      tools: ["codex"],
      adopt: ["claude"],
    });
  });
  it("gives the skill to every agent that can take it, adopting what is already there", () => {
    const columns = ["claude", "codex", "gemini", "pi", "hermes"];
    expect(skillAll(row, columns)).toEqual({
      count: 3,
      allOn: false,
      tools: ["codex", "pi", "claude"],
      adopt: ["claude"],
    });
    const full = skill([
      target("codex", { desired: true, status: "applied" }),
      target("claude", { adoptable: true }),
      target("pi", { desired: true, status: "applied" }),
      target("shared", { desired: true, status: "applied" }),
    ]);
    // All off lets go of the shared folder too, which no reader chip can do.
    expect(skillAll(full, ["claude", "codex", "pi"])).toEqual({
      count: 3,
      allOn: true,
      tools: [],
      adopt: ["claude"],
    });
  });
});
