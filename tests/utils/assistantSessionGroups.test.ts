import { describe, expect, it } from "vitest";
import {
  groupSessions,
  startOfLocalDay,
  type SessionGroup,
} from "@/utils/assistantSessionGroups";

// Local time throughout: the groups follow the reader's own calendar day.
const at = (day: number, hour = 12, minute = 0, second = 0, ms = 0) =>
  new Date(2026, 9, day, hour, minute, second, ms).getTime();
const today = startOfLocalDay(at(29));

const row = (
  id: string,
  updatedAt: number,
  pinnedAt: number | null = null,
) => ({
  id,
  updatedAt,
  pinnedAt,
});
const ids = (groups: SessionGroup<ReturnType<typeof row>>[]) =>
  groups.map((group) => [group.id, group.sessions.map((item) => item.id)]);

describe("startOfLocalDay", () => {
  it("is local midnight, and steps by calendar days", () => {
    expect(startOfLocalDay(at(29, 23, 59, 59, 999))).toBe(at(29, 0));
    expect(startOfLocalDay(at(29, 0))).toBe(at(29, 0));
    expect(startOfLocalDay(at(29, 15), 1)).toBe(at(30, 0));
    expect(startOfLocalDay(at(29, 15), -7)).toBe(at(22, 0));
  });
});

describe("groupSessions", () => {
  it("puts each conversation in the group of its last change's local day", () => {
    const groups = groupSessions(
      [
        row("now", at(29, 9)),
        row("last-night", at(28, 23, 59, 59, 999)),
        row("monday", at(26, 8)),
        row("week-edge", at(22, 0)),
        row("old", at(21, 23, 59, 59, 999)),
      ],
      today,
    );
    expect(ids(groups)).toEqual([
      ["today", ["now"]],
      ["yesterday", ["last-night"]],
      ["week", ["monday", "week-edge"]],
      ["earlier", ["old"]],
    ]);
  });

  it("starts today at local midnight, to the millisecond", () => {
    const groups = groupSessions(
      [row("first", at(29, 0)), row("last", at(28, 23, 59, 59, 999))],
      today,
    );
    expect(ids(groups)).toEqual([
      ["today", ["first"]],
      ["yesterday", ["last"]],
    ]);
  });

  it("moves a conversation down a group once midnight has passed", () => {
    const items = [row("a", at(29, 23, 59, 30))];
    expect(
      ids(groupSessions(items, startOfLocalDay(at(29, 23, 59, 31)))),
    ).toEqual([["today", ["a"]]]);
    expect(ids(groupSessions(items, startOfLocalDay(at(30, 0, 0, 1))))).toEqual(
      [["yesterday", ["a"]]],
    );
  });

  it("lists pinned conversations first, whatever their date, and keeps their order", () => {
    const groups = groupSessions(
      [
        row("pinned-old", at(1), at(28, 10)),
        row("pinned-new", at(29, 8), at(28, 9)),
        row("today", at(29, 7)),
      ],
      today,
    );
    expect(ids(groups)).toEqual([
      ["pinned", ["pinned-old", "pinned-new"]],
      ["today", ["today"]],
    ]);
  });

  it("keeps the order it was given within a group, and leaves empty groups out", () => {
    const groups = groupSessions(
      [row("b", at(29, 11)), row("a", at(29, 10)), row("c", at(1))],
      today,
    );
    expect(ids(groups)).toEqual([
      ["today", ["b", "a"]],
      ["earlier", ["c"]],
    ]);
    expect(groupSessions([], today)).toEqual([]);
  });

  it("counts a time ahead of the clock as today", () => {
    expect(ids(groupSessions([row("skewed", at(31, 12))], today))).toEqual([
      ["today", ["skewed"]],
    ]);
  });
});
