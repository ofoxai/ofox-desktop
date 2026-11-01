import { describe, expect, it } from "vitest";
import type {
  AssistantAttachment,
  AssistantAttachmentStatus,
} from "@/lib/api/assistant";
import { asDraftFile, isSendable } from "@/utils/assistantAttachments";

const file = (
  over: Partial<AssistantAttachment> = {},
): AssistantAttachment => ({
  id: "f1",
  name: "report.pdf",
  mimeType: "application/pdf",
  sizeBytes: 2048,
  status: "ready",
  reason: null,
  previewUrl: null,
  details: null,
  ...over,
});

describe("sending a file again", () => {
  it.each<[AssistantAttachmentStatus, boolean]>([
    ["ready", true],
    ["partial", true],
    ["model_unsupported", true],
    ["unreadable", false],
    ["unsupported", false],
    ["too_large", false],
    ["processing" as AssistantAttachmentStatus, false],
  ])("can send a %s file again: %s", (status, sendable) => {
    expect(isSendable({ status })).toBe(sendable);
  });
});

describe("a file in a new message", () => {
  it.each<[string, Partial<AssistantAttachment>]>([
    [
      "an image a text-only model could not read",
      {
        status: "model_unsupported",
        reason: "assistant_vision_model_required",
        details: { format: "image" },
      },
    ],
    [
      "a PDF read in part by that turn's model",
      {
        status: "partial",
        reason: "assistant_attachment_partial",
        details: { pagesWithoutText: [2], pagesAttachedAsImages: [] },
      },
    ],
    [
      "an image one too many for that message",
      {
        status: "too_large",
        reason: "assistant_attachment_too_large",
        details: { limit: "message_images", maxImages: 10 },
      },
    ],
    [
      "a page too large to attach as an image then",
      {
        status: "too_large",
        reason: "assistant_attachment_too_large",
        details: { limit: "rendered_page", page: 3 },
      },
    ],
  ])("is not judged by %s", (_, outcome) => {
    expect(asDraftFile(file(outcome))).toEqual(file());
  });

  it.each<[string, Partial<AssistantAttachment>]>([
    [
      "a password",
      {
        status: "unreadable",
        reason: "assistant_attachment_unreadable",
        details: { reason: "password_protected" },
      },
    ],
    [
      "its format",
      { status: "unsupported", reason: "assistant_attachment_unsupported" },
    ],
    [
      "its size",
      {
        status: "too_large",
        reason: "assistant_attachment_too_large",
        details: { limit: "file_size", maxBytes: 26_214_400 },
      },
    ],
    [
      "its page count",
      {
        status: "too_large",
        reason: "assistant_attachment_too_large",
        details: { limit: "pdf_pages", maxPages: 100, pageCount: 240 },
      },
    ],
  ])("still can't be sent when %s keeps it from being read", (_, outcome) => {
    const blocked = file(outcome);
    expect(asDraftFile(blocked)).toBe(blocked);
  });
});
