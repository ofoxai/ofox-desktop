import type { TFunction } from "i18next";
import { sessionModelNames } from "@/lib/api/sessions";
import type { ManagedSession, SessionSubagent } from "@/lib/api/sessions";
import { fmtAgo, fmtBytes } from "@/lib/format";
export const activity = (s: ManagedSession) =>
  s.lastActiveAt ?? s.createdAt ?? 0;
export function groupByFolder(sessions: ManagedSession[]) {
  const folders = new Map<string, ManagedSession[]>();
  for (const s of sessions) {
    const path = s.projectDir || "";
    const group = folders.get(path) ?? [];
    group.push(s);
    folders.set(path, group);
  }
  return [...folders]
    .map(([path, items]) => ({
      path,
      sessions: items.sort((a, b) => activity(b) - activity(a)),
    }))
    .sort((a, b) => activity(b.sessions[0]) - activity(a.sessions[0]));
}
/** How deep subagent threads are drawn under one another. */
export const MAX_DEPTH = 8;
const threadOf = (providerId: string, sessionId: string) =>
  `${providerId}:${sessionId}`;
/** The threads drawn under each session, by its session key. */
export type SessionKids = Map<string, ManagedSession[]>;
/**
 * The sessions listed under another one listed: a thread whose parent is,
 * by a chain with no loop in it (Magpie sessions client.js `nest`, MIT). A
 * loop, a chain deeper than is drawn, or a parent not listed leaves it on top.
 */
export function nestSessions(list: ManagedSession[]) {
  const byThread = new Map<string, ManagedSession>();
  for (const s of list) {
    const key = threadOf(s.providerId, s.sessionId);
    if (!byThread.has(key)) byThread.set(key, s);
  }
  const parentOf = (s: ManagedSession) =>
    s.parentSessionId
      ? byThread.get(threadOf(s.providerId, s.parentSessionId))
      : undefined;
  const kids: SessionKids = new Map();
  const parents = new Map<string, ManagedSession>();
  for (const s of list) {
    let p = parentOf(s);
    let root: ManagedSession | undefined;
    const seen = new Set([threadOf(s.providerId, s.sessionId)]);
    while (
      p &&
      !seen.has(threadOf(p.providerId, p.sessionId)) &&
      seen.size <= MAX_DEPTH
    ) {
      seen.add(threadOf(p.providerId, p.sessionId));
      root = p;
      p = parentOf(p);
    }
    if (!root || p) continue;
    const parent = parentOf(s)!;
    parents.set(s.sessionKey, parent);
    kids.set(parent.sessionKey, [...(kids.get(parent.sessionKey) ?? []), s]);
  }
  const started = (s: ManagedSession) => s.createdAt ?? s.lastActiveAt ?? 0;
  for (const list of kids.values())
    list.sort((a, b) => started(a) - started(b));
  return {
    tops: list.filter((s) => !parents.has(s.sessionKey)),
    kids,
    /** The session each nested one is drawn under, by session key. */
    parents,
  };
}
/** A session and every thread drawn under it. */
export function descendants(
  s: ManagedSession,
  kids: SessionKids,
  depth = 0,
): ManagedSession[] {
  const under =
    depth < MAX_DEPTH
      ? (kids.get(s.sessionKey) ?? []).flatMap((k) =>
          descendants(k, kids, depth + 1),
        )
      : [];
  return [s, ...under];
}
/**
 * A session's subagents as they started one another, each with how deep it
 * is under the session (Magpie sessions client.js `subTree`, MIT). One in a
 * loop of parents is still shown, at the top.
 */
export function subagentTree(
  subagents: SessionSubagent[],
): [SessionSubagent, number][] {
  const ids = new Set(subagents.map((s) => s.id));
  const under = new Map<string, SessionSubagent[]>();
  for (const s of subagents) {
    const parent =
      s.parent && ids.has(s.parent) && s.parent !== s.id ? s.parent : "";
    under.set(parent, [...(under.get(parent) ?? []), s]);
  }
  const out: [SessionSubagent, number][] = [];
  const seen = new Set<string>();
  const walk = (parent: string, depth: number) => {
    for (const s of under.get(parent) ?? []) {
      if (seen.has(s.id)) continue;
      seen.add(s.id);
      out.push([s, Math.min(depth, MAX_DEPTH)]);
      walk(s.id, depth + 1);
    }
  };
  walk("", 0);
  for (const s of subagents) {
    if (seen.has(s.id)) continue;
    seen.add(s.id);
    out.push([s, 0]);
    walk(s.id, 1);
  }
  return out;
}
export function agentCounts(sessions: ManagedSession[]): [string, number][] {
  const counts = new Map<string, number>();
  for (const s of sessions)
    counts.set(s.providerId, (counts.get(s.providerId) ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}
export function sessionSubline(
  s: ManagedSession,
  language: string,
  t: TFunction,
) {
  const models = sessionModelNames(s);
  return [
    fmtAgo(s.lastActiveAt ?? s.createdAt, language),
    models.slice(0, 2).join(", ") +
      (models.length > 2 ? ` +${models.length - 2}` : ""),
    s.messageCount
      ? t("sessionCenter.messageCount", { count: s.messageCount })
      : "",
    s.sizeBytes ? fmtBytes(s.sizeBytes) : "",
    s.title === s.sessionId ? "" : s.sessionId.slice(0, 8),
  ]
    .filter(Boolean)
    .join(" · ");
}
export const canSelect = (s: ManagedSession) =>
  !!(s.capabilities.canTrash && s.revision);
