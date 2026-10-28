import { useState } from "react";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zh from "@/i18n/locales/zh.json";
import { AttachmentList } from "@/components/assistant/AssistantAttachments";
import type {
  AssistantAttachment,
  AssistantAttachmentStatus,
} from "@/lib/api/assistant";

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

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

const list = () => screen.getByRole("list", { name: "Attached files" });

describe("attachment chips", () => {
  it("shows each file's name and size, and no outcome while it is ready", () => {
    render(
      <AttachmentList
        files={[
          file(),
          file({
            id: "f2",
            name: "photo.png",
            mimeType: "image/png",
            sizeBytes: 3_400_000,
          }),
        ]}
      />,
    );
    const items = within(list()).getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("report.pdf");
    expect(items[0]).toHaveTextContent("2.0 KB");
    expect(items[1]).toHaveTextContent("photo.png");
    expect(items[1]).toHaveTextContent("3.2 MB");
    expect(items[0].textContent).toBe("report.pdf2.0 KB");
  });

  it.each([
    [
      "partial",
      { textOnly: true, structureCoverage: "partial", format: "xlsx" },
      "Only text was read",
      "Sheet names or slide order",
    ],
    [
      "unreadable",
      { reason: "encrypted_office" },
      "password-protected",
      "Office",
    ],
    ["unreadable", { reason: "invalid_office" }, "damaged", "Office"],
    ["unreadable", { reason: "empty_office" }, "No readable text", "PDF"],
    [
      "too_large",
      { limit: "office_expansion" },
      "safe extraction limits",
      "smaller",
    ],
  ] as const)(
    "explains Office %s outcomes",
    (status, details, first, second) => {
      render(
        <AttachmentList
          files={[file({ name: "文档.xlsx", status, details })]}
        />,
      );
      const item = within(list()).getByRole("listitem");
      expect(item).toHaveTextContent(first);
      expect(item).toHaveTextContent(second);
    },
  );

  it("measures a file as its limits do, so a file at the limit reads as it", () => {
    render(<AttachmentList files={[file({ sizeBytes: 26_214_400 })]} />);
    expect(within(list()).getByRole("listitem")).toHaveTextContent("25 MB");
  });

  it.each<
    [
      string,
      AssistantAttachmentStatus,
      string | null,
      Record<string, unknown> | null,
      string,
    ]
  >([
    [
      "PDF pages without text, for a model without image input",
      "partial",
      "assistant_attachment_partial",
      {
        format: "pdf",
        pageCount: 9,
        textCoverage: "partial",
        pagesWithoutText: [2, 3, 4, 7],
        truncated: false,
      },
      "Pages 2–4, 7 have no text and weren't read. Choose a model that can read images to read them.",
    ],
    [
      "PDF pages without text a vision model did not get",
      "partial",
      "assistant_attachment_partial",
      {
        format: "pdf",
        pagesWithoutText: [2, 3, 4, 7],
        pagesAttachedAsImages: [2, 3],
      },
      "Pages 4, 7 have no text and weren't attached as images. Ask about those pages by number to have them read.",
    ],
    [
      "PDF page without text a vision model did not get",
      "partial",
      "assistant_attachment_partial",
      {
        format: "pdf",
        pagesWithoutText: [1, 2, 3],
        pagesAttachedAsImages: [1, 2],
      },
      "Page 3 has no text and wasn't attached as an image. Ask about that page by number to have it read.",
    ],
    [
      "text cut at the extraction limit",
      "partial",
      "assistant_attachment_partial",
      { format: "text", truncated: true, extractionLimit: 16_777_216 },
      "The text is too long: only the first 16,777,216 characters were read.",
    ],
    [
      "partial read without details",
      "partial",
      "assistant_attachment_partial",
      null,
      "Part of this file wasn't read.",
    ],
    [
      "image for a text-only model",
      "model_unsupported",
      "assistant_vision_model_required",
      { format: "image", mimeType: "image/png" },
      "This model can't read images. Choose a model that can read images, then try again.",
    ],
    [
      "PDF without text for a text-only model",
      "model_unsupported",
      "assistant_vision_model_required",
      { format: "pdf", pageCount: 2, pagesWithoutText: [1, 2] },
      "This PDF has no text, and this model can't read images. Choose a model that can read images, then try again.",
    ],
    [
      "file over the size limit",
      "too_large",
      "assistant_attachment_too_large",
      { format: "pdf", limit: "file_size", maxBytes: 26_214_400 },
      "This file is too large. Files can be up to 25 MB.",
    ],
    [
      "image over the image limit",
      "too_large",
      "assistant_attachment_too_large",
      { format: "image", limit: "image_size", maxBytes: 10_485_760 },
      "This image is too large. Images can be up to 10 MB.",
    ],
    [
      "PDF with too many pages",
      "too_large",
      "assistant_attachment_too_large",
      { format: "pdf", limit: "pdf_pages", maxPages: 100, pageCount: 240 },
      "This PDF has 240 pages. PDFs can have up to 100 pages.",
    ],
    [
      "one image too many for a message",
      "too_large",
      "assistant_attachment_too_large",
      { format: "image", limit: "message_images", maxImages: 10 },
      "A message can carry up to 10 images. Send this one in another message.",
    ],
    [
      "page too large to render",
      "too_large",
      "assistant_attachment_too_large",
      { format: "pdf", limit: "rendered_page", maxBytes: 5_242_880, page: 3 },
      "Page 3 is too large to read as an image.",
    ],
    [
      "too large without details",
      "too_large",
      "assistant_attachment_too_large",
      null,
      "This file is too large to read. Attach a smaller file instead.",
    ],
    [
      "password-protected PDF",
      "unreadable",
      "assistant_attachment_unreadable",
      { format: "pdf", reason: "password_protected" },
      "This PDF is password-protected. Remove the password, then attach it again.",
    ],
    [
      "damaged PDF",
      "unreadable",
      "assistant_attachment_unreadable",
      { format: "pdf", reason: "invalid_pdf" },
      "This PDF is damaged or isn't a valid PDF. Save or export it as a PDF again, then attach it again.",
    ],
    [
      "text that is not UTF-8",
      "unreadable",
      "assistant_attachment_unreadable",
      { format: "text", reason: "invalid_utf8" },
      "This text file isn't UTF-8. Save it as UTF-8, then attach it again.",
    ],
    [
      "damaged image",
      "unreadable",
      "assistant_attachment_unreadable",
      { format: "image", reason: "invalid_image" },
      "This image is damaged or isn't a valid image. Save it again as a PNG or JPEG, then attach it again.",
    ],
    [
      "file that is gone",
      "unreadable",
      "assistant_attachment_not_found",
      { reason: "missing" },
      "This file is no longer available. Attach it again.",
    ],
    [
      "file that could not be read",
      "unreadable",
      "assistant_attachment_unreadable",
      { reason: "read_failed" },
      "This file couldn't be read. Check it and attach it again.",
    ],
    [
      "format the runtime cannot read",
      "unsupported",
      "assistant_attachment_unsupported",
      { extension: "heic", mimeType: "image/heic" },
      "This file type can't be read. Attach images, PDFs, docx, xlsx, pptx or UTF-8 text. For .doc, save it as docx or PDF.",
    ],
  ])(
    "explains a %s in words, with what to do next",
    (_, status, reason, details, expected) => {
      render(<AttachmentList files={[file({ status, reason, details })]} />);
      const item = within(list()).getByRole("listitem");
      expect(item).toHaveTextContent(expected);
      expect(item).not.toHaveTextContent(/assistant_|_/);
    },
  );

  it("never shows an unknown status or reason as it came", () => {
    render(
      <AttachmentList
        files={[
          file({
            status: "processing" as AssistantAttachmentStatus,
            reason: "assistant_brand_new",
            details: { reason: "brand_new_reason" },
          }),
        ]}
      />,
    );
    const item = within(list()).getByRole("listitem");
    expect(item).toHaveTextContent(
      "This file can't be sent. Remove it and attach it again.",
    );
    expect(item).not.toHaveTextContent(/processing|brand|_/);
  });

  it("explains an outcome in the app's language", async () => {
    i18n.addResourceBundle("ja", "translation", ja, true, true);
    i18n.addResourceBundle("zh", "translation", zh, true, true);
    const blocked = file({
      status: "model_unsupported",
      reason: "assistant_vision_model_required",
      details: { format: "image" },
    });
    const view = render(<AttachmentList files={[blocked]} />);
    await act(async () => {
      await i18n.changeLanguage("zh");
    });
    view.rerender(<AttachmentList files={[blocked]} />);
    expect(
      screen.getByRole("list", { name: zh.assistant.attachedFiles }),
    ).toHaveTextContent(zh.assistant.attachmentOutcome.imageNeedsVision);
    await act(async () => {
      await i18n.changeLanguage("ja");
    });
    view.rerender(<AttachmentList files={[blocked]} />);
    expect(
      screen.getByRole("list", { name: ja.assistant.attachedFiles }),
    ).toHaveTextContent(ja.assistant.attachmentOutcome.imageNeedsVision);
  });

  it("has every outcome message in every language", () => {
    const keys = Object.keys(en.assistant.attachmentOutcome).sort();
    expect(Object.keys(ja.assistant.attachmentOutcome).sort()).toEqual(keys);
    expect(Object.keys(zh.assistant.attachmentOutcome).sort()).toEqual(keys);
  });

  it("keeps its list role however it is styled, and gives remove a 24 px target", () => {
    render(<AttachmentList files={[file()]} onRemove={() => undefined} />);
    expect(list()).toHaveAttribute("role", "list");
    expect(
      screen.getByRole("button", { name: "Remove report.pdf" }),
    ).toHaveClass("h-6", "w-6");
  });

  it("moves focus to the next file's remove button, and past the last one when none is left", async () => {
    const emptied = vi.fn();
    function Draft() {
      const [files, setFiles] = useState([
        file(),
        file({ id: "f2", name: "notes.txt", mimeType: "text/plain" }),
      ]);
      return (
        <AttachmentList
          files={files}
          onRemove={(id) =>
            setFiles((old) => old.filter((item) => item.id !== id))
          }
          onAllRemoved={emptied}
        />
      );
    }
    render(<Draft />);
    await userEvent.click(
      screen.getByRole("button", { name: "Remove report.pdf" }),
    );
    expect(
      screen.getByRole("button", { name: "Remove notes.txt" }),
    ).toHaveFocus();
    expect(emptied).not.toHaveBeenCalled();
    await userEvent.keyboard("{Enter}");
    expect(emptied).toHaveBeenCalledOnce();
  });

  it("removes a file by name when the list allows it", async () => {
    const onRemove = vi.fn();
    const rendered = render(
      <AttachmentList files={[file()]} onRemove={onRemove} removeDisabled />,
    );
    const remove = screen.getByRole("button", { name: "Remove report.pdf" });
    expect(remove).toBeDisabled();
    rendered.rerender(<AttachmentList files={[file()]} onRemove={onRemove} />);
    await userEvent.click(remove);
    expect(onRemove).toHaveBeenCalledWith("f1");
    rendered.rerender(<AttachmentList files={[file()]} />);
    expect(screen.queryByRole("button", { name: /Remove/ })).toBeNull();
  });
});
