import type { AssistantAttachment } from "@/lib/api/assistant";

/**
 * Outcomes a file may be sent with again: read in full or in part, or only
 * waiting for a model that can read images. The others can't be read at all.
 */
const SENDABLE = new Set<string>(["ready", "partial", "model_unsupported"]);

export function isSendable(file: Pick<AssistantAttachment, "status">) {
  return SENDABLE.has(file.status);
}

/** Limits that depended on the turn: its other images, its model's pages. */
const TURN_LIMITS = new Set<unknown>([
  "message_images",
  "rendered_page",
  "pdf_image",
]);

/**
 * A file as a new message would send it. A conversation keeps each file's
 * last outcome, from whichever turn read it last; one that depended on that
 * turn (its model, the other images it carried) says nothing about the next,
 * so the file is shown as freshly imported. A file that can't be read at all
 * (damaged, locked, unsupported, too large) stays as it is.
 */
export function asDraftFile(file: AssistantAttachment): AssistantAttachment {
  const turnOutcome =
    file.status === "partial" ||
    file.status === "model_unsupported" ||
    (file.status === "too_large" && TURN_LIMITS.has(file.details?.limit));
  return turnOutcome
    ? { ...file, status: "ready", reason: null, details: null }
    : file;
}
