import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { GenerationCards } from "@/components/assistant/GenerationCards";
import type {
  AssistantAttachment,
  AssistantGeneration,
  AssistantMessage,
  AssistantModel,
} from "@/lib/api/assistant";

// A generation's result in a reply: the result alone, its actions and
// details only when asked for, and a failure said where the result would be.

vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.ai", isLoading: false }),
}));
vi.mock("@/hooks/useOfoxAuth", () => ({
  useOfoxAuth: () => ({
    status: { user: { role: "owner" } },
    isLoading: false,
  }),
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: vi.fn() },
}));
const toasts = vi.hoisted(() => ({ error: vi.fn() }));
vi.mock("sonner", () => ({ toast: toasts }));

beforeEach(async () => {
  toasts.error.mockReset();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

const models = [
  { id: "image/test", name: "Image Test", shortName: "Image Test" },
  {
    id: "video/test",
    name: "Video Test",
    shortName: "Video Test",
    kind: "video",
    video: {
      modes: ["i2v"],
      resolutions: ["480p", "720p"],
      defaultResolution: "720p",
      minDuration: 4,
      maxDuration: 6,
      supportsAudio: true,
      aspectRatios: ["16:9"],
      fromPricePerSecond: "0.04",
    },
  },
] as unknown as AssistantModel[];
const file: AssistantAttachment = {
  id: "file-1",
  name: "generated-1.png",
  mimeType: "image/png",
  sizeBytes: 10,
  status: "ready",
  reason: null,
  details: null,
  origin: "generated",
  sourceMessageId: "reply",
};
const frame: AssistantAttachment = {
  ...file,
  id: "frame-1",
  name: "girl.png",
  origin: undefined,
  sourceMessageId: "q",
};
const image = (extra: Partial<AssistantGeneration> = {}) =>
  ({
    toolCallId: "call-1",
    kind: "image",
    model: "image/test",
    params: { size: "1024x1024", quality: "low" },
    prompt: "a red fox in snow",
    inputIds: [],
    status: "completed",
    outputs: ["file-1"],
    cost: "0.02",
    errorCode: null,
    ...extra,
  }) as unknown as AssistantGeneration;

const cards = (
  item: AssistantGeneration,
  onRegenerate = vi.fn(),
  sourceFile?: (id: string) => AssistantAttachment | undefined,
) =>
  render(
    <GenerationCards
      message={
        {
          id: "reply",
          role: "assistant",
          content: "",
          createdAt: 0,
          attachments: [file],
          generations: [item],
        } as unknown as AssistantMessage
      }
      sessionId="s1"
      owner="o"
      generation={1}
      models={models}
      onSave={vi.fn()}
      onSaveAndOpen={vi.fn()}
      onCancelVideo={vi.fn()}
      onResumeVideo={vi.fn()}
      onRegenerate={onRegenerate}
      sourceFile={sourceFile}
    />,
  );

describe("a generation's result", () => {
  it("shows the result alone, with its model, cost and prompt behind Details", async () => {
    cards(image());
    const result = screen.getByRole("region", { name: "Generate image" });
    expect(within(result).getByRole("img")).toHaveAttribute(
      "alt",
      "generated-1.png",
    );
    for (const hidden of [
      "Image Test",
      "1024x1024",
      "a red fox in snow",
      "$0.02",
    ])
      expect(result).not.toHaveTextContent(hidden);
    expect(
      within(result).getByRole("button", { name: "Save…" }),
    ).toBeInTheDocument();
    expect(
      within(result).getByRole("button", {
        name: "Regenerate with other parameters",
      }),
    ).toBeInTheDocument();
    await userEvent.click(
      within(result).getByRole("button", { name: "Details" }),
    );
    const details = await screen.findByRole("dialog", { name: "Details" });
    expect(details).toHaveTextContent("Image Test");
    expect(details).toHaveTextContent("1024x1024");
    expect(details).toHaveTextContent("Cost $0.02");
    expect(details).toHaveTextContent("a red fox in snow");
  });

  it("says why it failed where the result would be, with regenerate at hand", async () => {
    const onRegenerate = vi.fn();
    cards(
      image({
        status: "error",
        outputs: [],
        errorCode: "assistant_insufficient_balance",
      }),
      onRegenerate,
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      en.assistant.errors.insufficientBalance,
    );
    expect(screen.getByRole("button", { name: "Top up" })).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Regenerate with other parameters" }),
    );
    expect(
      await screen.findByRole("dialog", {
        name: "Regenerate with other parameters",
      }),
    ).toBeInTheDocument();
  });

  it("holds its place while it is being made", () => {
    cards(image({ status: "submitting", outputs: [] }));
    expect(screen.getByRole("status")).toHaveTextContent("Generating image…");
  });
});

describe("a generated document", () => {
  const PPTX =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation";
  const deck: AssistantAttachment = {
    id: "deck-1",
    name: "q3-review.pptx",
    mimeType: PPTX,
    sizeBytes: 482_000,
    status: "ready",
    reason: null,
    details: null,
    origin: "generated",
    sourceMessageId: "reply",
  };
  const document = (
    params: Record<string, unknown> = {},
    extra: Partial<AssistantGeneration> = {},
  ) =>
    ({
      toolCallId: "call-doc",
      kind: "document",
      model: "",
      params: { name: "q3-review", format: "pptx", slides: 8, ...params },
      inputIds: [],
      status: "completed",
      outputs: ["deck-1"],
      cost: "0",
      errorCode: null,
      ...extra,
    }) as unknown as AssistantGeneration;
  const issues = (count: number) =>
    Array.from({ length: count }, (_, index) => ({
      severity: index % 2 ? "error" : "warning",
      message: `Problem ${index + 1}`,
      path: `/slide[${index + 1}]`,
    }));
  const documentCards = (
    item: AssistantGeneration,
    handlers: {
      onSave?: (file: AssistantAttachment) => Promise<unknown>;
      onSaveAndOpen?: (file: AssistantAttachment) => Promise<unknown>;
    } = {},
  ) =>
    render(
      <GenerationCards
        message={
          {
            id: "reply",
            role: "assistant",
            content: "",
            createdAt: 0,
            attachments: [deck],
            generations: [item],
          } as unknown as AssistantMessage
        }
        sessionId="s1"
        owner="o"
        generation={1}
        models={models}
        onSave={handlers.onSave ?? vi.fn().mockResolvedValue(true)}
        onSaveAndOpen={
          handlers.onSaveAndOpen ?? vi.fn().mockResolvedValue(true)
        }
        onCancelVideo={vi.fn()}
        onResumeVideo={vi.fn()}
        onRegenerate={vi.fn()}
      />,
    );

  it("renders a document card with counts and both buttons", () => {
    documentCards(document());
    const card = screen.getByRole("region", { name: "Document" });
    expect(card).toHaveTextContent("q3-review.pptx");
    expect(card).toHaveTextContent("8 slides");
    expect(card).toHaveTextContent("471 KB");
    expect(
      within(card).getByRole("button", { name: "Save and open" }),
    ).toBeEnabled();
    expect(within(card).getByRole("button", { name: "Save…" })).toBeEnabled();
    // Nothing to regenerate with other parameters, and no preview to play.
    expect(
      within(card).queryByRole("button", { name: /Regenerate/ }),
    ).toBeNull();
    expect(card.querySelector("img, audio, video")).toBeNull();
    expect(within(card).queryByRole("button", { name: /issue/ })).toBeNull();
  });

  it.each([
    [{ format: "pdf", slides: undefined, pages: 3 }, "3 pages"],
    [{ format: "xlsx", slides: undefined, sheets: 1 }, "1 sheet"],
    [{ format: "pptx", slides: 1 }, "1 slide"],
  ])("counts %o as %s", (params, text) => {
    documentCards(document(params));
    expect(screen.getByRole("region", { name: "Document" })).toHaveTextContent(
      text,
    );
  });

  it("is still a card without a count, for a format that has none", () => {
    documentCards(document({ format: "md", slides: undefined }));
    const card = screen.getByRole("region", { name: "Document" });
    expect(card).toHaveTextContent("MD");
    expect(card).not.toHaveTextContent(/slide|page|sheet/);
  });

  it("save and open passes open:true, and the plain save does not", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    const onSaveAndOpen = vi.fn().mockResolvedValue(true);
    documentCards(document(), { onSave, onSaveAndOpen });
    await userEvent.click(
      screen.getByRole("button", { name: "Save and open" }),
    );
    expect(onSaveAndOpen).toHaveBeenCalledExactlyOnceWith(deck);
    expect(onSave).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Save…" }));
    expect(onSave).toHaveBeenCalledExactlyOnceWith(deck);
    expect(onSaveAndOpen).toHaveBeenCalledOnce();
  });

  it("keeps both buttons off while a save is open", async () => {
    let finish!: (saved: boolean) => void;
    const onSaveAndOpen = vi.fn(
      () => new Promise<boolean>((resolve) => (finish = resolve)),
    );
    documentCards(document(), { onSaveAndOpen });
    await userEvent.click(
      screen.getByRole("button", { name: "Save and open" }),
    );
    expect(
      screen.getByRole("button", { name: "Save and open" }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "Save…" })).toBeDisabled();
    finish(true);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save…" })).toBeEnabled(),
    );
  });

  it("shows an issues badge that lists them", async () => {
    documentCards(document({ issues: issues(2) }));
    const badge = screen.getByRole("button", { name: "2 issues" });
    expect(badge).toHaveTextContent("2");
    await userEvent.click(badge);
    const list = await screen.findByRole("list", { name: "2 issues" });
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Problem 1");
    expect(items[0]).toHaveTextContent("/slide[1]");
    expect(items[0]).toHaveTextContent("Warning");
    expect(items[1]).toHaveTextContent("Error");
  });

  it("lists at most 20 issues", async () => {
    documentCards(document({ issues: issues(25) }));
    await userEvent.click(screen.getByRole("button", { name: "20 issues" }));
    expect(
      within(await screen.findByRole("list")).getAllByRole("listitem"),
    ).toHaveLength(20);
  });

  it("holds its place while it is being made", () => {
    documentCards(document({}, { status: "submitting", outputs: [] }));
    expect(screen.getByRole("status")).toHaveTextContent(
      "Generating document…",
    );
    expect(screen.queryByRole("button", { name: "Save and open" })).toBeNull();
  });

  it("reports a failed save and open in a notice, and lets the user try again", async () => {
    const onSaveAndOpen = vi
      .fn()
      .mockRejectedValueOnce("assistant_open_failed")
      .mockResolvedValue(true);
    documentCards(document(), { onSaveAndOpen });
    await userEvent.click(
      screen.getByRole("button", { name: "Save and open" }),
    );
    await waitFor(() =>
      expect(toasts.error).toHaveBeenCalledExactlyOnceWith(
        en.assistant.errors.openFailed,
      ),
    );
    // The card stays: the copy may be saved, and both buttons work again.
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Save and open" }),
      ).toBeEnabled(),
    );
    expect(screen.getByRole("button", { name: "Save…" })).toBeEnabled();
    await userEvent.click(
      screen.getByRole("button", { name: "Save and open" }),
    );
    expect(onSaveAndOpen).toHaveBeenCalledTimes(2);
    expect(toasts.error).toHaveBeenCalledOnce();
  });

  it("reports a failed plain save the same way", async () => {
    const onSave = vi.fn().mockRejectedValue("assistant_export_failed");
    documentCards(document(), { onSave });
    await userEvent.click(screen.getByRole("button", { name: "Save…" }));
    await waitFor(() =>
      expect(toasts.error).toHaveBeenCalledExactlyOnceWith(
        en.assistant.errors.exportFailed,
      ),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save…" })).toBeEnabled(),
    );
  });

  it("says why it failed without offering to regenerate it", () => {
    documentCards(
      document(
        {},
        {
          status: "error",
          outputs: [],
          errorCode: "assistant_model_request_failed",
        },
      ),
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      en.assistant.errors.modelRequestFailed,
    );
    expect(screen.queryByRole("button", { name: /Regenerate/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Details" })).toBeNull();
  });
});

describe("regenerating with other parameters", () => {
  const video = image({
    kind: "video",
    model: "video/test",
    params: {
      duration: 5,
      resolution: "720p",
      generateAudio: true,
      firstFrameId: "frame-1",
    },
    prompt: "the girl raises her hand",
    outputs: [],
  } as never);

  it("shows the frame it starts from, a counted prompt and named parameters", async () => {
    cards(video, vi.fn(), (id) => (id === "frame-1" ? frame : undefined));
    await userEvent.click(
      screen.getByRole("button", { name: "Regenerate with other parameters" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Regenerate with other parameters",
    });
    expect(within(dialog).getByRole("img", { name: "First frame" })).toHaveAttribute(
      "src",
      expect.stringContaining("frame-1"),
    );
    expect(
      within(dialog).getByRole("textbox", { name: "Prompt sent" }),
    ).toHaveValue("the girl raises her hand");
    expect(dialog).toHaveTextContent("24 characters");
    for (const name of ["Model", "Duration", "Resolution", "Aspect ratio"])
      expect(within(dialog).getByText(name)).toBeInTheDocument();
    expect(
      within(dialog).getByRole("combobox", { name: "Duration" }),
    ).toHaveTextContent("5 s");
  });

  it("shows no frame when the generation used none", async () => {
    cards(image());
    await userEvent.click(
      screen.getByRole("button", { name: "Regenerate with other parameters" }),
    );
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).queryByRole("img")).toBeNull();
  });
});

describe("a finished video", () => {
  it("shows the frame it started from, not a black box, before it plays", () => {
    const movie: AssistantAttachment = {
      ...file,
      id: "movie-1",
      name: "movie.mp4",
      mimeType: "video/mp4",
    };
    const { container } = render(
      <GenerationCards
        message={
          {
            id: "reply",
            role: "assistant",
            content: "",
            createdAt: 0,
            attachments: [movie],
            generations: [
              image({
                kind: "video",
                model: "video/test",
                params: { duration: 5, resolution: "720p", firstFrameId: "frame-1" },
                outputs: ["movie-1"],
              } as never),
            ],
          } as unknown as AssistantMessage
        }
        sessionId="s1"
        owner="o"
        generation={1}
        models={models}
        onSave={vi.fn()}
        onSaveAndOpen={vi.fn()}
        onCancelVideo={vi.fn()}
        onResumeVideo={vi.fn()}
        sourceFile={(id) => (id === "frame-1" ? frame : undefined)}
      />,
    );
    const video = container.querySelector("video")!;
    expect(video.getAttribute("poster")).toContain("frame-1");
    // Without a first frame, the player is asked for the video's own.
    expect(video.getAttribute("src")).toMatch(/movie-1.*#t=0\.1$/);
  });
});
