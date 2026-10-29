import type { AssistantMessage } from "@/lib/api/assistant";

/** Where a turn stands among its versions, for the switcher that shows it. */
export interface TurnVersions {
  /**
   * The same for every version of the turn (what they all follow), so the
   * message that carries the switcher can keep its place on the screen, and
   * the keyboard focus on the switcher, when another version is shown.
   */
  group: string;
  /** The version shown, counted from 1, and how many there are. */
  index: number;
  count: number;
  /** The question to show for `‹` and `›`; none at the ends. */
  previousId?: string;
  nextId?: string;
}

/**
 * The turns that have more than one version, as the switcher shows them:
 * each keyed by the message that carries the switcher, which is the shown
 * version's last reply (or its question while it has no reply).
 *
 * Versions are the questions the model's own history holds that follow the
 * same message (`branchFrom`; null for the first turn), in the order they
 * were made. A question whose parent is unknown (`branchFrom` left out) or
 * that the model never kept has no versions. `messages` holds the shown ones
 * and the superseded ones alike.
 */
export function turnVersions(
  messages: ReadonlyArray<AssistantMessage>,
): Map<string, TurnVersions> {
  const groups = new Map<string, AssistantMessage[]>();
  for (const message of messages) {
    if (
      message.role !== "user" ||
      message.branchFrom === undefined ||
      !message.nativeEntryId
    )
      continue;
    const group = message.branchFrom ?? "start";
    groups.set(group, [...(groups.get(group) ?? []), message]);
  }
  const found = new Map<string, TurnVersions>();
  for (const [group, siblings] of groups) {
    if (siblings.length < 2) continue;
    // A stable sort: versions made in the same moment keep their order.
    const ordered = [...siblings].sort((a, b) => a.createdAt - b.createdAt);
    const at = ordered.findIndex((message) => !message.superseded);
    if (at < 0) continue;
    const shown = ordered[at];
    // The reply row that stays in view is the newest message's, so the
    // switcher goes on the run's last reply.
    const carrier =
      messages
        .filter(
          (message) =>
            message.role === "assistant" &&
            !message.superseded &&
            Boolean(shown.runId) &&
            message.runId === shown.runId,
        )
        .pop() ?? shown;
    found.set(carrier.id, {
      group,
      index: at + 1,
      count: ordered.length,
      previousId: ordered[at - 1]?.id,
      nextId: ordered[at + 1]?.id,
    });
  }
  return found;
}
