import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  assistantPreviewApi,
  assistantPresentationRevision,
  isAssistantHtmlPreviewUrl,
} from "@/lib/api/assistantPreview";
const call = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke: call }));
const request = {
  sessionId: "s",
  attachmentId: "a",
  expectedOwner: "o",
  expectedGeneration: 2,
};
const version = "a".repeat(64);
beforeEach(() => call.mockReset());
describe("authorized real file preview API", () => {
  it("uses revision metadata only on generated PPTX and keeps the immutable parent hash and changed pages", () => {
    const file = {
      id: "new-deck",
      origin: "generated" as const,
      name: "Deck.pptx",
      mimeType:
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
      sizeBytes: 100,
      status: "ready" as const,
      reason: null,
      details: {
        presentationRevision: {
          parentAttachmentId: "parent",
          parentVersion: version,
          pageNumbers: [4, 2],
          revisionId: "revision-1",
        },
      },
    };
    expect(assistantPresentationRevision(file)).toEqual({
      parentAttachmentId: "parent",
      parentVersion: version,
      pageNumbers: [4, 2],
      revisionId: "revision-1",
    });
    expect(
      assistantPresentationRevision({ ...file, origin: undefined }),
    ).toBeNull();
    for (const extra of [
      { parentAttachmentId: file.id },
      { parentVersion: "short" },
      { pageNumbers: [0] },
      { pageNumbers: [2, 2] },
      { revisionId: "../path" },
    ])
      expect(
        assistantPresentationRevision({
          ...file,
          details: {
            presentationRevision: {
              ...file.details.presentationRevision,
              ...extra,
            },
          },
        }),
      ).toBeNull();
  });
  it("does not confuse ready thumbnails with the real page count", async () => {
    const manifest = {
      attachmentId: "a",
      name: "Deck",
      version,
      kind: "pptx",
      totalPages: 70,
      width: 1280,
      height: 720,
      readyPages: [1],
      limits: { maxPagesPerRequest: 1, maxPage: null },
      warnings: [],
    };
    call.mockResolvedValue(manifest);
    expect(await assistantPreviewApi.manifest(request)).toEqual(manifest);
    expect(call).toHaveBeenCalledExactlyOnceWith("assistant_file_preview", {
      request,
    });
  });
  it("rejects a stale version or non-JPEG page instead of displaying its bytes", async () => {
    call.mockResolvedValue({
      attachmentId: "a",
      version: "b".repeat(64),
      page: 1,
      mimeType: "image/jpeg",
      dataBase64: "/9j/2Q==",
      width: 1280,
      height: 720,
    });
    await expect(
      assistantPreviewApi.page({ ...request, version, page: 1, scale: 1 }),
    ).rejects.toThrow("assistant_preview_unavailable");
  });
  it.each([
    "https://example.com/index.html",
    "http://localhost:43763/token/index.html",
    "http://127.0.0.1:80/" + "b".repeat(32) + "/index.html",
    "assistant-media://localhost/a/b",
    "http://user@127.0.0.1:43763/" + "b".repeat(32) + "/index.html",
    "http://127.0.0.1:43763/short/index.html",
  ])("rejects active HTML outside the dedicated preview origin: %s", (url) =>
    expect(isAssistantHtmlPreviewUrl(url)).toBe(false),
  );
  it("accepts only the exact versioned token URL from the dedicated server", async () => {
    const value = {
      attachmentId: "a",
      version,
      url: `http://127.0.0.1:43763/${"b".repeat(32)}/index.html`,
      missingResources: [],
    };
    call.mockResolvedValue(value);
    expect(await assistantPreviewApi.html({ ...request, version })).toEqual(
      value,
    );
    expect(call).toHaveBeenLastCalledWith("assistant_file_preview_html", {
      request: { ...request, version },
    });
  });
});
