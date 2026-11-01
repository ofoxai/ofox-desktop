import { beforeEach, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { describeAttachmentOutcome } from "@/utils/assistantAttachmentOutcome";
import type {
  AssistantAttachment,
  AssistantAttachmentStatus,
} from "@/lib/api/assistant";

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

const file = (
  status: AssistantAttachmentStatus,
  details: Record<string, unknown> | null = null,
  over: Partial<AssistantAttachment> = {},
): AssistantAttachment => ({
  id: "f1",
  name: "report.pdf",
  mimeType: "application/pdf",
  sizeBytes: 2048,
  status,
  reason: null,
  previewUrl: null,
  details,
  ...over,
});
const notes = (attachment: AssistantAttachment) =>
  describeAttachmentOutcome(attachment, i18n.t, "en");
const tones = (attachment: AssistantAttachment) =>
  notes(attachment).map((note) => note.tone);
const messages = (key: string) =>
  (en.assistant.attachmentOutcome as Record<string, string>)[key];

describe("the tone of an attachment's notes", () => {
  it("has none for a file that was read in full", () => {
    expect(notes(file("ready"))).toEqual([]);
  });

  it.each([
    ["officeTextOnly", { format: "xlsx", textOnly: true }],
    ["officeWordScope", { format: "docx", textOnly: true }],
    ["officeStructure", { format: "pptx", structureCoverage: "partial" }],
    ["officeCachedValues", { format: "xlsx", textOnly: true }],
  ])("says %s as information", (key, details) => {
    const found = notes(file("partial", details));
    expect(found).toContainEqual({ text: messages(key), tone: "info" });
  });

  it("gives an Office file read as text only information notes", () => {
    expect(tones(file("partial", { format: "docx", textOnly: true }))).toEqual([
      "info",
      "info",
    ]);
  });

  it.each([
    ["pages without text", { format: "pdf", pagesWithoutText: [3] }],
    [
      "pages a vision model did not get",
      { format: "pdf", pagesWithoutText: [3], pagesAttachedAsImages: [1] },
    ],
    ["a text cut at the limit", { truncated: true, extractionLimit: 1000 }],
    ["a partial read without details", null],
  ])("warns about %s of a file that can still be sent", (_, details) => {
    const found = tones(file("partial", details));
    expect(found.length).toBeGreaterThan(0);
    expect(found.every((tone) => tone === "warn")).toBe(true);
  });

  it.each([
    ["an image", { format: "image" }],
    ["a PDF", { format: "pdf" }],
  ])("warns that the model can't read %s", (_, details) => {
    expect(tones(file("model_unsupported", details))).toEqual(["warn"]);
  });

  it("keeps each note of a file with several apart by its own tone", () => {
    const found = notes(
      file("partial", {
        format: "docx",
        textOnly: true,
        pagesWithoutText: [2],
        truncated: true,
        extractionLimit: 1000,
      }),
    );
    expect(found.map((note) => note.tone)).toEqual([
      "info",
      "info",
      "warn",
      "warn",
    ]);
  });

  it.each<[AssistantAttachmentStatus, Record<string, unknown> | null]>([
    ["too_large", { limit: "file_size", maxBytes: 26_214_400 }],
    ["too_large", { limit: "office_expansion" }],
    ["too_large", null],
    ["unreadable", { reason: "password_protected" }],
    ["unreadable", { reason: "encrypted_office" }],
    ["unreadable", { reason: "missing" }],
    ["unsupported", { extension: ".doc" }],
    ["unsupported", null],
    ["processing" as AssistantAttachmentStatus, null],
  ])("calls a %s file (%o) an error: it can't be sent", (status, details) => {
    const found = notes(file(status, details));
    expect(found).toHaveLength(1);
    expect(found[0].tone).toBe("error");
  });

  it("still words every note, whatever its tone", () => {
    const found = notes(
      file("partial", { format: "pdf", pagesWithoutText: [3] }),
    );
    expect(found[0].text).toBe(
      "Page 3 has no text and wasn't read. Choose a model that can read images to read it.",
    );
  });
});
