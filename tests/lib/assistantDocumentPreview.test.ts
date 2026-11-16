import { describe, expect, it } from "vitest";
import {
  assistantDocumentPreview,
  assistantDocumentPreviewUrl,
  type AssistantAttachment,
} from "@/lib/api/assistant";

const file: AssistantAttachment = {
  id: "attachment",
  name: "deck.pptx",
  mimeType:
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  origin: "generated",
  sizeBytes: 123,
  status: "ready",
  reason: null,
  details: { documentPreview: { mimeType: "image/jpeg", pages: [4, 1] } },
};
describe("private document preview contract", () => {
  it("uses only persisted raster metadata and never legacy preview URLs", () => {
    expect(assistantDocumentPreview(file)).toEqual({
      mimeType: "image/jpeg",
      pages: [1, 4],
    });
    expect(
      assistantDocumentPreview({
        ...file,
        details: null,
        previewUrl: "https://example.test/private",
      }),
    ).toBeNull();
    expect(assistantDocumentPreview({ ...file, origin: undefined })).toBeNull();
    expect(assistantDocumentPreview({ ...file, status: "missing" })).toBeNull();
    expect(
      assistantDocumentPreview({ ...file, mimeType: "text/html" }),
    ).toBeNull();
  });
  it.each(
    [
      [],
      [0],
      [1001],
      [1, 1],
      ["1"],
      [1.5],
      Array.from({ length: 61 }, (_, i) => i + 1),
    ].map((pages) => ({ pages })),
  )("rejects invalid published pages $pages", ({ pages }) => {
    expect(
      assistantDocumentPreview({
        ...file,
        details: { documentPreview: { mimeType: "image/jpeg", pages } },
      }),
    ).toBeNull();
  });
  it("accepts sparse persisted pages through 1000 without raising the batch limit", () => {
    expect(
      assistantDocumentPreview({
        ...file,
        details: {
          documentPreview: { mimeType: "image/jpeg", pages: [1000, 70] },
        },
      }),
    ).toEqual({ mimeType: "image/jpeg", pages: [70, 1000] });
    for (const page of [70, 1000]) {
      expect(assistantDocumentPreviewUrl("s", "a", "o", 7, page)).toContain(
        `/s/a/preview/${page}?owner=o&generation=7`,
      );
    }
  });
  it("rejects active formats and scopes every URL to the supplied current authority", () => {
    expect(
      assistantDocumentPreview({
        ...file,
        details: { documentPreview: { mimeType: "image/svg+xml", pages: [1] } },
      }),
    ).toBeNull();
    const url = assistantDocumentPreviewUrl(
      "session",
      "attachment",
      "owner?a=b",
      7,
      4,
    );
    expect(url).toContain(
      "/session/attachment/preview/4?owner=owner%3Fa%3Db&generation=7",
    );
    for (const page of [0, 1001, 1.5, Number.NaN])
      expect(() =>
        assistantDocumentPreviewUrl("s", "a", "o", 7, page),
      ).toThrow();
  });
});
