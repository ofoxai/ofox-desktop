import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import ja from "@/i18n/locales/ja.json";
import { AssistantFileWorkspace } from "@/components/assistant/AssistantFileWorkspace";
import type { AssistantAttachment } from "@/lib/api/assistant";
import type {
  FilePreviewManifest,
  FilePreviewPage,
} from "@/lib/api/assistantPreview";

const api = vi.hoisted(() => ({
  manifest: vi.fn(),
  page: vi.fn(),
  html: vi.fn(),
}));
vi.mock("@/lib/api/assistantPreview", async (original) => ({
  ...(await original<typeof import("@/lib/api/assistantPreview")>()),
  assistantPreviewApi: api,
}));
const version = "a".repeat(64);
const authority = {
  sessionId: "s",
  expectedOwner: "owner",
  expectedGeneration: 2,
};
const file: AssistantAttachment = {
  id: "deck",
  name: "Long deck.pptx",
  origin: "generated",
  mimeType:
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  sizeBytes: 200,
  status: "ready",
  reason: null,
  details: null,
};
const manifest: FilePreviewManifest = {
  attachmentId: file.id,
  name: file.name,
  version,
  kind: "pptx",
  totalPages: 70,
  width: 1280,
  height: 720,
  readyPages: [1],
  limits: { maxPagesPerRequest: 1, maxPage: null },
  warnings: [],
};
const result = (page: number): FilePreviewPage => ({
  attachmentId: file.id,
  version,
  page,
  mimeType: "image/jpeg",
  dataBase64: "/9j/2Q==",
  width: 1280,
  height: 720,
});
const props = () => ({
  files: [file],
  authority,
  selectedId: file.id,
  onSelect: vi.fn(),
  onQuote: vi.fn(),
  children: <textarea aria-label="Message" defaultValue="Keep this draft" />,
});
const pane = () =>
  screen.getByRole("complementary", { name: "File workspace" });
const ready = () => within(pane()).findByRole("img");
const openActions = async () => {
  await userEvent.click(
    await screen.findByRole("button", { name: "More preview actions" }),
  );
};
const openDetails = async () => {
  await openActions();
  await userEvent.click(
    screen.getByRole("menuitem", { name: "Preview details" }),
  );
  await screen.findByRole("dialog", { name: "Preview details" });
};
beforeEach(async () => {
  vi.clearAllMocks();
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  api.manifest.mockResolvedValue(manifest);
  api.page.mockImplementation(async ({ page }) => result(page));
});
afterEach(() => vi.restoreAllMocks());

describe("real generated file workspace", () => {
  it("shows the loading fox while the preview has not arrived, not a bare line of text", async () => {
    api.manifest.mockReturnValue(new Promise(() => {}));
    render(<AssistantFileWorkspace {...props()} />);
    const loading = await within(pane()).findByRole("status", {
      name: "Loading preview…",
    });
    // The fox is drawn, with what it is loading written under it.
    expect(loading.querySelector("svg")).not.toBeNull();
    expect(loading).toHaveTextContent("Loading preview…");
  });

  it("switches files from the single heading without replacing the chat draft", async () => {
    const other = { ...file, id: "other", name: "Other deck.pptx" };
    const p = { ...props(), files: [file, other] };
    const { rerender } = render(<AssistantFileWorkspace {...p} />);
    p.onSelect.mockImplementation((id) =>
      rerender(<AssistantFileWorkspace {...p} selectedId={id} />),
    );
    const draft = screen.getByRole("textbox", { name: "Message" });
    await ready();
    expect(screen.getAllByText(file.name)).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "Switch file" }));
    expect(screen.getByRole("menuitem", { name: file.name })).toHaveAttribute(
      "aria-current",
      "true",
    );
    await userEvent.click(screen.getByRole("menuitem", { name: other.name }));
    expect(p.onSelect).toHaveBeenCalledExactlyOnceWith(other.id);
    expect(screen.getByRole("textbox", { name: "Message" })).toBe(draft);
    expect(draft).toHaveValue("Keep this draft");
    await ready();
    expect(api.page).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("heading", { name: other.name })).toHaveFocus();
    expect(screen.getByRole("textbox", { name: "Message" })).toBe(draft);
  });

  it("closes details with a visible button and restores keyboard focus without hiding the file", async () => {
    const p = props();
    render(<AssistantFileWorkspace {...p} />);
    await openDetails();
    const dialog = screen.getByRole("dialog", { name: "Preview details" });
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Close" }),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(
      screen.getByRole("button", { name: "More preview actions" }),
    ).toHaveFocus();
    expect(p.onSelect).not.toHaveBeenCalled();
    expect(api.page).toHaveBeenCalledTimes(1);
  });

  it("keeps deliberate composer focus after quoting from the menu", async () => {
    const p = props();
    render(<AssistantFileWorkspace {...p} />);
    const draft = screen.getByRole("textbox", { name: "Message" });
    p.onQuote.mockImplementation(() => draft.focus());
    await ready();
    await openActions();
    await userEvent.click(
      screen.getByRole("menuitem", { name: "Quote current page" }),
    );
    expect(screen.getByRole("textbox", { name: "Message" })).toHaveFocus();
    expect(api.page).toHaveBeenCalledTimes(1);
  });

  it("keeps a narrow-window chat toggle available when file tools are collapsed", async () => {
    const bounds = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockReturnValue({ width: 650, right: 650 } as DOMRect);
    const p = props();
    render(<AssistantFileWorkspace {...p} />);
    await ready();
    const views = screen.getByRole("group", { name: "Workspace view" });
    await userEvent.click(
      within(views).getByRole("button", { name: en.assistant.chat }),
    );
    expect(pane()).toHaveClass("hidden");
    expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue(
      "Keep this draft",
    );
    await userEvent.click(within(views).getByRole("button", { name: "Files" }));
    expect(pane()).not.toHaveClass("hidden");
    expect(api.page).toHaveBeenCalledTimes(1);
    bounds.mockRestore();
  });

  it.each([
    ["en", en],
    ["zh", zh],
    ["ja", ja],
  ] as const)(
    "keeps the compact preview controls translated in %s",
    async (language, messages) => {
      i18n.addResourceBundle(language, "translation", messages, true, true);
      await i18n.changeLanguage(language);
      render(<AssistantFileWorkspace {...props()} />);
      await within(screen.getByRole("complementary")).findByRole("img");
      const words = messages.assistant.workspace;
      expect(screen.getByRole("button", { name: words.expand })).toBeVisible();
      await userEvent.click(
        screen.getByRole("button", { name: words.moreActions }),
      );
      expect(
        screen.getByRole("menuitemcheckbox", { name: words.showThumbnails }),
      ).toBeVisible();
    },
  );

  it("keeps the default preview to one file heading and basic viewing controls", async () => {
    render(<AssistantFileWorkspace {...props()} />);
    await screen.findByRole("figure", { name: "Page 1" });
    expect(screen.getAllByText(file.name)).toHaveLength(1);
    expect(screen.queryByText(`Version ${version.slice(0, 12)}`)).toBeNull();
    expect(screen.queryByRole("combobox", { name: "Switch file" })).toBeNull();
    expect(screen.queryByRole("combobox", { name: "Preview zoom" })).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Quote current page" }),
    ).toBeNull();
    expect(
      screen.queryByRole("navigation", { name: "Document pages" }),
    ).toBeNull();
    expect(screen.queryByRole("textbox", { name: "Go to page" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Expand preview" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Close file workspace" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "More preview actions" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Page 1 of 70" }),
    ).toHaveTextContent("1 / 70");
  });

  it("opens page navigation and secondary preview controls only when requested", async () => {
    const p = props();
    render(<AssistantFileWorkspace {...p} />);
    await within(pane()).findByRole("img", { name: "Page 1" });
    await userEvent.click(screen.getByRole("button", { name: "Next page" }));
    await within(pane()).findByRole("img", { name: "Page 2" });
    await userEvent.click(
      screen.getByRole("button", { name: "More preview actions" }),
    );
    expect(
      screen.getByRole("menuitem", { name: "Quote current page" }),
    ).toBeVisible();
    await userEvent.click(screen.getByRole("menuitemradio", { name: "200%" }));
    await waitFor(() =>
      expect(api.page).toHaveBeenLastCalledWith(
        expect.objectContaining({ page: 2, scale: 2 }),
      ),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "More preview actions" }),
    );
    await userEvent.click(
      screen.getByRole("menuitemcheckbox", { name: "Show thumbnails" }),
    );
    expect(
      screen.getByRole("navigation", { name: "Document pages" }),
    ).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Page 2 of 70" }));
    await userEvent.type(
      screen.getByRole("textbox", { name: "Go to page" }),
      "{Control>}a{/Control}70",
    );
    await userEvent.click(screen.getByRole("button", { name: "Go" }));
    await within(pane()).findByRole("img", { name: "Page 70" });
    expect(screen.queryByRole("textbox", { name: "Go to page" })).toBeNull();
    expect(p.onQuote).not.toHaveBeenCalled();
    expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue(
      "Keep this draft",
    );
  });

  it("keeps secondary draft actions in a keyboard menu without executing or changing the quoted identity", async () => {
    const p = { ...props(), onPrepare: vi.fn() };
    render(<AssistantFileWorkspace {...p} />);
    await screen.findByRole("figure", { name: "Page 1" });
    await userEvent.click(screen.getByRole("button", { name: "Next page" }));
    await ready();
    expect(
      screen.queryByRole("button", { name: "Optimize page visuals" }),
    ).toBeNull();
    expect(
      screen.queryByText(
        "These actions prepare a draft. Send it to run; nothing has been changed yet.",
      ),
    ).toBeNull();
    const more = screen.getByRole("button", { name: "More preview actions" });
    more.focus();
    await userEvent.keyboard("{Enter}");
    const menu = screen.getByRole("menu");
    expect(
      within(menu).getByText(/These actions prepare a draft/),
    ).toBeVisible();
    expect(
      within(menu).getByRole("menuitem", { name: "Optimize page visuals" }),
    ).toHaveFocus();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(more).toHaveFocus();
    expect(p.onSelect).not.toHaveBeenCalled();
    await userEvent.keyboard("{Enter}{Enter}");
    expect(p.onPrepare).toHaveBeenCalledExactlyOnceWith(
      { attachmentId: file.id, version, name: file.name, page: 2 },
      "optimize",
    );
    expect(p.onQuote).not.toHaveBeenCalled();
    expect(more).toHaveFocus();
    expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue(
      "Keep this draft",
    );
    expect(api.page.mock.calls.map(([request]) => request.page)).toEqual([
      1, 2,
    ]);
    await userEvent.click(more);
    await userEvent.click(
      screen.getByRole("menuitem", { name: "Replace page image" }),
    );
    expect(p.onPrepare).toHaveBeenLastCalledWith(
      { attachmentId: file.id, version, name: file.name, page: 2 },
      "replace-image",
    );
  });

  it("keeps secondary actions disabled until the actual current page is ready", async () => {
    api.page.mockImplementation(() => new Promise(() => {}));
    const p = { ...props(), onPrepare: vi.fn() };
    render(<AssistantFileWorkspace {...p} />);
    await screen.findByRole("figure", { name: "Page 1" });
    await userEvent.click(
      screen.getByRole("button", { name: "More preview actions" }),
    );
    expect(
      screen.getByRole("menuitem", { name: "Optimize page visuals" }),
    ).toHaveAttribute("data-disabled");
    expect(
      screen.getByRole("menuitem", { name: "Replace page image" }),
    ).toHaveAttribute("data-disabled");
    fireEvent.click(
      screen.getByRole("menuitem", { name: "Optimize page visuals" }),
    );
    fireEvent.click(
      screen.getByRole("menuitem", { name: "Replace page image" }),
    );
    await userEvent.keyboard("{Escape}");
    expect(p.onPrepare).not.toHaveBeenCalled();
    expect(p.onQuote).not.toHaveBeenCalled();
    expect(p.onSelect).not.toHaveBeenCalled();
    expect(api.page).toHaveBeenCalledTimes(1);
  });

  it("keeps long preview notes off the canvas until requested and closes secondary menus on authority change", async () => {
    api.page.mockImplementation(async ({ page }) => ({
      ...result(page),
      mathFallback: true,
    }));
    const p = { ...props(), onPrepare: vi.fn() };
    const view = render(<AssistantFileWorkspace {...p} />);
    await screen.findByRole("figure", { name: "Page 1" });
    expect(
      screen.queryByText(
        "Compatibility preview: formula editing remains unverified.",
      ),
    ).toBeNull();
    await openDetails();
    expect(
      screen.getByText(
        "Compatibility preview: formula editing remains unverified.",
      ),
    ).toBeVisible();
    await userEvent.keyboard("{Escape}");
    expect(p.onSelect).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole("button", { name: "More preview actions" }),
    );
    expect(screen.getByRole("menu")).toBeVisible();
    view.rerender(
      <AssistantFileWorkspace
        {...p}
        authority={{ ...authority, expectedGeneration: 3 }}
      />,
    );
    expect(screen.queryByRole("menu")).toBeNull();
    expect(p.onPrepare).not.toHaveBeenCalled();
    expect(p.onQuote).not.toHaveBeenCalled();
  });

  it("opens a revision directly on modified page 70, requests no cover, and preserves a later user-selected page", async () => {
    const revision = {
      ...file,
      details: {
        presentationRevision: {
          parentAttachmentId: "parent",
          parentVersion: "b".repeat(64),
          pageNumbers: [70],
          revisionId: "revision-70",
        },
      },
    };
    const p = { ...props(), files: [revision] };
    const view = render(<AssistantFileWorkspace {...p} />);
    await waitFor(() => expect(api.page).toHaveBeenCalledTimes(1));
    expect(api.page).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 70 }),
    );
    expect(screen.getByRole("figure", { name: "Page 70" })).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Previous page" }),
    );
    await screen.findByRole("figure", { name: "Page 69" });
    view.rerender(
      <AssistantFileWorkspace
        {...p}
        files={[{ ...revision, details: { ...revision.details } }]}
      />,
    );
    expect(screen.getByRole("figure", { name: "Page 69" })).toBeVisible();
    expect(api.page.mock.calls.map(([request]) => request.page)).toEqual([
      70, 69,
    ]);
  });
  it("returns to an immutable parent only after verifying its exact version, keeping both files", async () => {
    const parent = { ...file, id: "parent", name: "Before edit.pptx" };
    const revision = {
      ...file,
      details: {
        presentationRevision: {
          parentAttachmentId: parent.id,
          parentVersion: "b".repeat(64),
          pageNumbers: [2, 3, 4, 7],
          revisionId: "revision-1",
        },
      },
    };
    api.manifest.mockImplementation(async ({ attachmentId }) =>
      attachmentId === parent.id
        ? {
            ...manifest,
            attachmentId,
            name: parent.name,
            version: "b".repeat(64),
          }
        : manifest,
    );
    const p = { ...props(), files: [revision, parent] };
    const { rerender } = render(<AssistantFileWorkspace {...p} />);
    p.onSelect.mockImplementation((id, expectedVersion) =>
      rerender(
        <AssistantFileWorkspace
          {...p}
          selectedId={id}
          expectedVersion={expectedVersion}
        />,
      ),
    );
    await screen.findByRole("figure", { name: "Page 2" });
    await openDetails();
    expect(screen.getByText(/Changed pages: 2–4, 7/)).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Return to previous version" }),
    );
    await waitFor(() =>
      expect(p.onSelect).toHaveBeenCalledExactlyOnceWith(
        parent.id,
        "b".repeat(64),
      ),
    );
    await ready();
    expect(screen.getByRole("heading", { name: parent.name })).toHaveFocus();
    expect(p.files).toHaveLength(2);
    expect(p.onQuote).not.toHaveBeenCalled();
  });

  it("keeps the current file when the parent changed and never silently restores different bytes", async () => {
    const parent = { ...file, id: "parent", name: "Before edit.pptx" };
    const revision = {
      ...file,
      details: {
        presentationRevision: {
          parentAttachmentId: parent.id,
          parentVersion: "b".repeat(64),
          pageNumbers: [2],
          revisionId: "revision-1",
        },
      },
    };
    api.manifest.mockImplementation(async ({ attachmentId }) =>
      attachmentId === parent.id
        ? { ...manifest, attachmentId, version: "c".repeat(64) }
        : manifest,
    );
    const p = { ...props(), files: [revision, parent] };
    render(<AssistantFileWorkspace {...p} />);
    await openDetails();
    await userEvent.click(
      screen.getByRole("button", { name: "Return to previous version" }),
    );
    await screen.findByText(
      "The file changed. Reload its preview before viewing or quoting another page.",
    );
    expect(p.onSelect).not.toHaveBeenCalled();
  });
  it("discards a late parent verification after switching conversation authority", async () => {
    const parent = { ...file, id: "parent" };
    const revision = {
      ...file,
      details: {
        presentationRevision: {
          parentAttachmentId: parent.id,
          parentVersion: "b".repeat(64),
          pageNumbers: [2],
          revisionId: "revision-1",
        },
      },
    };
    let finish!: (value: FilePreviewManifest) => void;
    api.manifest.mockImplementation(async ({ attachmentId }) =>
      attachmentId === parent.id
        ? new Promise((resolve) => {
            finish = resolve;
          })
        : manifest,
    );
    const p = { ...props(), files: [revision, parent] };
    const view = render(<AssistantFileWorkspace {...p} />);
    await openDetails();
    await userEvent.click(
      screen.getByRole("button", { name: "Return to previous version" }),
    );
    view.rerender(
      <AssistantFileWorkspace
        {...p}
        authority={{ ...authority, sessionId: "other", expectedGeneration: 3 }}
      />,
    );
    await act(async () =>
      finish({ ...manifest, attachmentId: parent.id, version: "b".repeat(64) }),
    );
    expect(p.onSelect).not.toHaveBeenCalled();
  });
  it("checks the expected parent hash again before displaying the switched file", async () => {
    render(
      <AssistantFileWorkspace {...props()} expectedVersion={"b".repeat(64)} />,
    );
    await screen.findByText(
      "The file changed. Reload its preview before viewing or quoting another page.",
    );
    expect(api.page).not.toHaveBeenCalled();
    expect(within(pane()).queryByRole("img")).toBeNull();
  });
  it("does not request an invalid modified page outside the real manifest count", async () => {
    const revision = {
      ...file,
      details: {
        presentationRevision: {
          parentAttachmentId: "parent",
          parentVersion: "b".repeat(64),
          pageNumbers: [71],
          revisionId: "revision-71",
        },
      },
    };
    render(<AssistantFileWorkspace {...props()} files={[revision]} />);
    await waitFor(() => expect(api.page).toHaveBeenCalledTimes(1));
    expect(api.page).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 1 }),
    );
    await openDetails();
    expect(
      screen.getByText(
        "The recorded modified page is outside the available preview range. Showing the first page.",
      ),
    ).toBeVisible();
  });
  it("uses the actual 70-page count and requests only the selected page, including page 70", async () => {
    render(<AssistantFileWorkspace {...props()} />);
    await screen.findByRole("figure", { name: "Page 1" });
    await waitFor(() => expect(api.page).toHaveBeenCalledTimes(1));
    expect(api.page).toHaveBeenLastCalledWith({
      ...authority,
      attachmentId: file.id,
      version,
      page: 1,
      scale: 1,
    });
    await userEvent.click(screen.getByRole("button", { name: "Page 1 of 70" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Go to page" }), {
      target: { value: "70" },
    });
    await userEvent.click(screen.getByRole("button", { name: "Go" }));
    await waitFor(() => expect(api.page).toHaveBeenCalledTimes(2));
    expect(api.page).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 70 }),
    );
    expect(
      within(pane()).getByRole("button", { name: "Page 70 of 70" }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Next page" })).toBeDisabled();
    expect(screen.getByRole("figure", { name: "Page 70" })).toHaveFocus();
  });

  it("does not show a late page or manifest after authority/session changes", async () => {
    let resolve!: (value: FilePreviewPage) => void;
    api.page.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const p = props();
    const view = render(<AssistantFileWorkspace {...p} />);
    await waitFor(() => expect(api.page).toHaveBeenCalledTimes(1));
    api.manifest.mockImplementationOnce(() => new Promise(() => {}));
    view.rerender(
      <AssistantFileWorkspace
        {...p}
        authority={{ ...authority, sessionId: "other", expectedGeneration: 3 }}
      />,
    );
    await act(async () => resolve(result(1)));
    expect(within(pane()).queryByRole("img")).toBeNull();
    expect(
      within(pane()).queryByRole("button", { name: "Quote current page" }),
    ).toBeNull();
    expect(p.onQuote).not.toHaveBeenCalled();
  });

  it("quotes the frozen current page without replacing or remounting the chat draft", async () => {
    const p = props();
    const view = render(<AssistantFileWorkspace {...p} />);
    const input = screen.getByRole("textbox", { name: "Message" });
    await ready();
    await openActions();
    await userEvent.click(
      screen.getByRole("menuitem", { name: "Quote current page" }),
    );
    expect(p.onQuote).toHaveBeenCalledExactlyOnceWith({
      attachmentId: file.id,
      version,
      name: file.name,
      page: 1,
    });
    await userEvent.click(
      screen.getByRole("button", { name: "Expand preview" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Restore split view" }),
    );
    view.rerender(<AssistantFileWorkspace {...p} selectedId={null} />);
    expect(screen.getByRole("textbox", { name: "Message" })).toBe(input);
    expect(input).toHaveValue("Keep this draft");
  });

  it("fits a 900px split and provides a keyboard-adjustable bounded divider", async () => {
    const bounds = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockReturnValue({ width: 900, right: 900 } as DOMRect);
    render(<AssistantFileWorkspace {...props()} />);
    const divider = screen.getByRole("separator", {
      name: "Resize chat and file workspace",
    });
    divider.focus();
    await userEvent.keyboard("{End}");
    expect(divider).toHaveAttribute("aria-valuenow", "65");
    await userEvent.keyboard("{Home}");
    expect(divider).toHaveAttribute("aria-valuenow", "35");
    expect(
      screen.getByRole("textbox", { name: "Message" }).parentElement,
    ).toHaveClass("min-w-0");
    expect(pane()).toHaveClass("min-w-0");
    bounds.mockRestore();
  });

  it("closes with Escape inside the preview without invoking the chat's stop shortcut", async () => {
    const p = props();
    const parentKey = vi.fn();
    render(
      <div onKeyDown={parentKey}>
        <AssistantFileWorkspace {...p} />
      </div>,
    );
    const figure = await screen.findByRole("figure", { name: "Page 1" });
    figure.focus();
    await userEvent.keyboard("{Escape}");
    expect(p.onSelect).toHaveBeenCalledExactlyOnceWith(null);
    expect(parentKey).not.toHaveBeenCalled();
  });

  it("keeps failed pages navigable and retries only the current page", async () => {
    api.page.mockRejectedValueOnce(new Error("no preview"));
    render(<AssistantFileWorkspace {...props()} />);
    await screen.findByRole("button", { name: "Retry preview" });
    await userEvent.click(screen.getByRole("button", { name: "Next page" }));
    await ready();
    expect(api.page).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 2 }),
    );
  });

  it("opens an uploaded deck and does not request a nonexistent page in an empty deck", async () => {
    api.manifest.mockResolvedValue({
      ...manifest,
      totalPages: 0,
      readyPages: [],
    });
    render(
      <AssistantFileWorkspace
        {...props()}
        files={[{ ...file, origin: undefined }]}
      />,
    );
    await screen.findByText("This presentation has no slides.");
    expect(api.page).not.toHaveBeenCalled();
  });

  it("keeps formula compatibility and unverified notices attached to the actual rendered page", async () => {
    api.page.mockImplementationOnce(async () => ({
      ...result(1),
      mathFallback: true,
      mathUnverified: true,
    }));
    render(<AssistantFileWorkspace {...props()} />);
    await within(pane()).findByRole("img", { name: "Page 1" });
    await openDetails();
    await screen.findByText(
      "Compatibility preview: formula editing remains unverified.",
    );
    expect(
      screen.getByText("Some formula rendering could not be verified."),
    ).toBeVisible();
    await userEvent.keyboard("{Escape}");
    await userEvent.click(screen.getByRole("button", { name: "Next page" }));
    await ready();
    await openDetails();
    expect(
      screen.queryByText(
        "Compatibility preview: formula editing remains unverified.",
      ),
    ).toBeNull();
  });

  it("requests a higher-resolution current page for zoom without fetching other slides", async () => {
    render(<AssistantFileWorkspace {...props()} />);
    await ready();
    await openActions();
    await userEvent.click(screen.getByRole("menuitemradio", { name: "200%" }));
    await waitFor(() => expect(api.page).toHaveBeenCalledTimes(2));
    expect(api.page).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 1, scale: 2 }),
    );
    expect(screen.getByRole("figure", { name: "Page 1" })).toBeVisible();
  });

  it("ignores a late image error from a page that has already been left", async () => {
    render(<AssistantFileWorkspace {...props()} />);
    const image = await within(pane()).findByRole("img", { name: "Page 1" });
    await userEvent.click(screen.getByRole("button", { name: "Next page" }));
    await within(pane()).findByRole("img", { name: "Page 2" });
    fireEvent.error(image);
    expect(within(pane()).getByRole("img", { name: "Page 2" })).toBeVisible();
    await openActions();
    expect(
      screen.getByRole("menuitem", { name: "Quote current page" }),
    ).toBeEnabled();
  });

  it("mounts real HTML only in an opaque sandbox and removes it when inactive", async () => {
    const html = {
      ...file,
      id: "html",
      name: "Demo.html",
      mimeType: "text/html",
    };
    api.manifest.mockResolvedValue({
      ...manifest,
      attachmentId: html.id,
      name: html.name,
      kind: "html",
      totalPages: 1,
    });
    const url = `http://127.0.0.1:43763/${"b".repeat(32)}/index.html`;
    api.html.mockResolvedValue({
      attachmentId: html.id,
      version,
      url,
      missingResources: ["chart.js"],
    });
    const p = { ...props(), files: [html], selectedId: html.id };
    const view = render(<AssistantFileWorkspace {...p} />);
    await waitFor(() =>
      expect(document.querySelector("iframe")).not.toBeNull(),
    );
    const frame = document.querySelector("iframe")!;
    expect(frame).toHaveAttribute("src", url);
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame).not.toHaveAttribute("srcdoc");
    expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
    expect(screen.getByRole("alert")).toHaveTextContent("chart.js");
    expect(api.page).not.toHaveBeenCalled();
    const report = (
      source: MessageEventSource | null,
      token: string,
      message: string,
    ) =>
      fireEvent(
        window,
        new MessageEvent("message", {
          source,
          data: { type: "ofox-html-preview-error", token, message },
        }),
      );
    report(window, "b".repeat(32), "Wrong frame");
    report(frame.contentWindow, "wrong-token", "Wrong token");
    expect(screen.queryByText("Wrong frame")).toBeNull();
    expect(screen.queryByText("Wrong token")).toBeNull();
    report(frame.contentWindow, "b".repeat(32), "Chart initialization failed");
    expect(screen.getByText("Chart initialization failed")).toBeVisible();
    report(frame.contentWindow, "b".repeat(32), "x".repeat(501));
    expect(screen.queryByText("x".repeat(501))).toBeNull();
    view.rerender(<AssistantFileWorkspace {...p} active={false} />);
    expect(document.querySelector("iframe")).toBeNull();
    report(frame.contentWindow, "b".repeat(32), "Stale report");
    expect(screen.queryByText("Stale report")).toBeNull();
  });
});
