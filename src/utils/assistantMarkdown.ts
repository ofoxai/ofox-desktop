import type { TFunction } from "i18next";
import {
  assistantErrorKey,
  type AssistantAttachment,
  type AssistantSessionDetail,
} from "@/lib/api/assistant";
import { fmtBinaryBytes } from "@/lib/format";
import { asDraftFile } from "@/utils/assistantAttachments";
import { describeAttachmentOutcome } from "@/utils/assistantAttachmentOutcome";

/** Metadata is plain text; message bodies keep their original Markdown. */
const inline = (text: string) =>
  text.replace(/[\r\n]+/g, " ").replace(/[\\`*_{}\[\]()<>#!|]/g, "\\$&");
function sourceUrl(text: string) {
  try {
    const url = new URL(text);
    if (
      !["https:", "http:"].includes(url.protocol) ||
      url.username ||
      url.password
    )
      return null;
    return url.href.replace(/</g, "%3C").replace(/>/g, "%3E");
  } catch {
    return null;
  }
}

/** Current readable turns and unsent draft; the full backup keeps old branches and file bytes. */
export function assistantMarkdown(
  detail: AssistantSessionDetail,
  t: TFunction,
  language: string,
): string {
  const parts = [
    `# ${inline(detail.session.title || t("assistant.untitled"))}`,
    t("assistant.markdownNotice"),
    `${t("assistant.markdownModel")}: ${inline(detail.session.modelId)}`,
  ];
  const files = (items: AssistantAttachment[] = []) => {
    if (!items.length) return;
    parts.push(`### ${t("assistant.attachedFiles")}`);
    parts.push(
      items
        .map((file) => {
          const notes = describeAttachmentOutcome(file, t, language).map(
            (note) => note.text,
          );
          return `- ${inline(file.name)} (${fmtBinaryBytes(file.sizeBytes)}, ${inline(file.mimeType)})${notes.length ? ` — ${notes.join(" ")}` : ""}`;
        })
        .join("\n"),
    );
  };
  for (const message of detail.messages) {
    if (
      message.superseded ||
      !["user", "assistant", "media"].includes(message.role)
    )
      continue;
    parts.push(
      `## ${t(`assistant.roles.${message.role === "media" ? "assistant" : message.role}`)}`,
    );
    if (message.media) {
      parts.push(
        [
          inline(message.media.model),
          ...(message.media.kind === "image"
            ? [message.media.params.size, message.media.params.quality]
            : [
                t("assistant.video.seconds", {
                  count: message.media.params.duration,
                }),
                message.media.params.resolution,
                message.media.params.aspectRatio,
              ]),
        ]
          .filter(Boolean)
          .join(" · "),
      );
      if (message.media.cost !== null)
        parts.push(t("assistant.media.cost", { cost: message.media.cost }));
    }
    if (message.content) parts.push(message.content.trimEnd());
    if (message.status === "error")
      parts.push(
        `> ${t("assistant.failed")}${message.errorCode ? `: ${t(assistantErrorKey(message.errorCode))}` : ""}`,
      );
    else if (message.status === "stopped")
      parts.push(`> ${t("assistant.stopped")}`);
    files(message.attachments);
    const sources = (message.sources ?? []).flatMap((source) => {
      const url = sourceUrl(source.url);
      return url
        ? [
            `- ${inline(source.id)}: [${inline(source.title || source.url)}](<${url}>)`,
          ]
        : [];
    });
    if (sources.length)
      parts.push(`### ${t("assistant.sources")}`, sources.join("\n"));
  }
  const draftFiles = detail.session.draftAttachmentIds.flatMap((id) => {
    const file = detail.attachments.find((item) => item.id === id);
    return file ? [asDraftFile(file)] : [];
  });
  if (detail.session.draft || draftFiles.length) {
    parts.push(`## ${t("assistant.markdownDraft")}`);
    if (detail.session.draft) parts.push(detail.session.draft.trimEnd());
    files(draftFiles);
  }
  const usage = detail.usage;
  if (Object.keys(usage).length) {
    parts.push(`## ${t("assistant.markdownUsage")}`);
    parts.push(
      t("assistant.usageTokens", {
        input: usage.inputTokens ?? "—",
        output: usage.outputTokens ?? "—",
      }),
    );
    if (
      usage.cacheReadTokens !== undefined ||
      usage.cacheWriteTokens !== undefined
    )
      parts.push(
        t("assistant.markdownCache", {
          read: usage.cacheReadTokens ?? "—",
          write: usage.cacheWriteTokens ?? "—",
        }),
      );
    parts.push(
      usage.usageCoverage !== "incomplete" &&
        typeof usage.estimatedCostUsd === "string"
        ? t("assistant.markdownCost", { cost: usage.estimatedCostUsd })
        : t("assistant.costUnavailable"),
    );
    if (
      usage.usageCoverage !== "incomplete" &&
      typeof usage.estimatedCostUsd === "string" &&
      usage.searchCount !== undefined
    )
      parts.push(
        t("assistant.markdownSearchCharges", { count: usage.searchCount }),
      );
  }
  return `${parts.join("\n\n")}\n`;
}
