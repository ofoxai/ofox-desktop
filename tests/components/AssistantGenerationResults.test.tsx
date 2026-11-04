import { render, screen, within } from "@testing-library/react";
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

beforeEach(async () => {
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
