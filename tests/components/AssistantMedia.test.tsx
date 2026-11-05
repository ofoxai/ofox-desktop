import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import { MediaBlock } from "@/components/assistant/MediaBlock";
import type { AssistantModel, AssistantMessage } from "@/lib/api/assistant";
import en from "@/i18n/locales/en.json";
beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.io", isLoading: false }),
}));
const model = {
  id: "image/test",
  kind: "image",
  image: {
    generations: true,
    edits: true,
    n: true,
    sizes: ["1024x1024"],
    qualities: ["low", "high"],
    perImagePrice: "0.02",
    tokenPricing: null,
  },
} as AssistantModel;
const file = {
  id: "f".repeat(32),
  name: "result.png",
  mimeType: "image/png",
  sizeBytes: 100,
  status: "ready",
  reason: null,
  details: null,
  origin: "generated",
};
const message = {
  id: "m",
  role: "media",
  content: "",
  status: "completed",
  createdAt: 1,
  attachments: [file],
  media: {
    kind: "image",
    model: model.id,
    params: {},
    inputIds: [],
    status: "completed",
    outputs: [file.id],
    cost: "0.02",
  },
} as AssistantMessage;
it("renders private lazy images and supports preview, save, input and a new generation", async () => {
  const save = vi.fn().mockResolvedValue(true),
    use = vi.fn(),
    regen = vi.fn();
  render(
    <MediaBlock
      message={message}
      sessionId={"s".repeat(32)}
      owner={"ab".repeat(32)}
      generation={7}
      onSave={save}
      onUseInput={use}
      onRegenerate={regen}
    />,
  );
  const image = screen.getByRole("img");
  expect(image).toHaveAttribute("loading", "lazy");
  expect(image.getAttribute("src")).toContain("assistant-media");
  expect(image.getAttribute("src")).toContain("generation=7");
  await userEvent.click(screen.getByRole("button", { name: "View image" }));
  expect(screen.getByRole("dialog")).toBeVisible();
  await userEvent.keyboard("{Escape}");
  await userEvent.click(screen.getByRole("button", { name: "Save…" }));
  expect(save).toHaveBeenCalledWith(file);
  await userEvent.click(screen.getByRole("button", { name: "Use as input" }));
  expect(use).toHaveBeenCalledWith(file);
  await userEvent.click(screen.getByRole("button", { name: "Generate again" }));
  expect(regen).toHaveBeenCalledOnce();
});
it("unknown outcomes never offer an automatic retry", () => {
  render(
    <MediaBlock
      message={{
        ...message,
        status: "error",
        errorCode: "assistant_media_outcome_unknown",
        attachments: [],
        media: {
          ...message.media!,
          status: "unknown",
          outputs: [],
          cost: null,
        },
      }}
      sessionId="s"
      owner="o"
      generation={7}
      onSave={vi.fn()}
      onUseInput={vi.fn()}
      onRegenerate={vi.fn()}
    />,
  );
  expect(screen.getByText(/may have been charged/i)).toBeVisible();
  expect(screen.getByRole("link", { name: "View activity" })).toHaveAttribute(
    "href",
    "https://app.ofox.io/analytics/activity",
  );
  expect(screen.queryByRole("button", { name: "Generate again" })).toBeNull();
});
