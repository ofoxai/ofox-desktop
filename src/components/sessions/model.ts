import type { TFunction } from "i18next";
import { sessionModelNames } from "@/lib/api/sessions";
import type { ManagedSession } from "@/lib/api/sessions";
import { fmtAgo } from "@/lib/format";
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
    s.messageCount == null
      ? ""
      : t("sessionCenter.messageCount", { count: s.messageCount }),
  ]
    .filter(Boolean)
    .join(" · ");
}
export const canSelect = (s: ManagedSession) =>
  !!(s.capabilities.canTrash && s.revision);
