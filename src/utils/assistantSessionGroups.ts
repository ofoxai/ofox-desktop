/** The groups of the conversation list, in the order they are listed. */
export const SESSION_GROUPS = [
  "pinned",
  "today",
  "yesterday",
  "week",
  "earlier",
] as const;

export type SessionGroupId = (typeof SESSION_GROUPS)[number];

export interface SessionGroup<T> {
  id: SessionGroupId;
  sessions: T[];
}

/**
 * Local midnight of `at`'s day, moved by whole calendar days: the day, not 24
 * hours, so a day with a clock change still starts at midnight.
 */
export function startOfLocalDay(at: number, days = 0): number {
  const date = new Date(at);
  return new Date(
    date.getFullYear(),
    date.getMonth(),
    date.getDate() + days,
  ).getTime();
}

/**
 * The group a conversation is listed in. `today` is local midnight of the
 * current day: yesterday starts a day before it, and "the past 7 days" a week
 * before it. A time ahead of the clock counts as today.
 */
function groupOf(
  session: { pinnedAt: number | null; updatedAt: number },
  today: number,
): SessionGroupId {
  if (session.pinnedAt !== null) return "pinned";
  if (session.updatedAt >= today) return "today";
  if (session.updatedAt >= startOfLocalDay(today, -1)) return "yesterday";
  if (session.updatedAt >= startOfLocalDay(today, -7)) return "week";
  return "earlier";
}

/**
 * Splits the list into its groups, each keeping the order it was given in
 * (the backend sorts: pins first, then the latest change). Empty groups are
 * left out.
 */
export function groupSessions<
  T extends { pinnedAt: number | null; updatedAt: number },
>(sessions: T[], today: number): SessionGroup<T>[] {
  const byGroup = new Map<SessionGroupId, T[]>();
  for (const session of sessions) {
    const id = groupOf(session, today);
    byGroup.set(id, [...(byGroup.get(id) ?? []), session]);
  }
  return SESSION_GROUPS.flatMap((id) => {
    const members = byGroup.get(id);
    return members ? [{ id, sessions: members }] : [];
  });
}
