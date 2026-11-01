import type {
  AssistantRequestRecord,
  AssistantSessionDetail,
} from "@/lib/api/assistant";

/** Prefer current media events to the last settled conversation snapshot. */
export function assistantRequests(
  detail: AssistantSessionDetail | null,
): AssistantRequestRecord[] {
  if (!detail) return [];
  const result = (detail.usage.requests ?? []).filter(
    (r) => !["image", "video", "speech", "transcription"].includes(r.kind),
  );
  for (const message of detail.messages) {
    const media = message.media;
    if (!media) continue;
    for (const [index, attempt] of (media.requestAttempts ?? []).entries()) {
      const last = index + 1 === media.requestAttempts!.length;
      result.push({
        ...attempt,
        kind: media.kind,
        modelId: media.model,
        apex: media.apex,
        status:
          attempt.httpStatus >= 400 && attempt.httpStatus < 500
            ? "rejected"
            : media.status,
        usage: last ? media.responseUsage : null,
        priceSnapshot: media.priceSnapshot,
        estimatedCostUsd: last && media.kind === "image" ? media.cost : null,
        reportedCostUsd: last && media.kind === "video" ? media.cost : null,
        billedCostUsd: null,
        jobId: media.jobId,
      });
    }
  }
  for (const media of detail.audioOperations ?? []) {
    result.push({
      kind: media.kind,
      modelId: media.model,
      apex: media.apex,
      status: media.status,
      requestId: media.audio.requestId,
      requestIdSource: media.audio.requestIdSource,
      usage: media.audio.usage,
      priceSnapshot: media.audio.pricing,
      estimatedCostUsd:
        media.status === "completed"
          ? (media.cost ?? media.audio.estimate)
          : null,
      billedCostUsd: null,
    });
  }
  return result;
}
