import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import { ImageParams } from "@/components/assistant/ImageParams";
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
it("offers only advertised image parameters and estimates count times price", async () => {
  const change = vi.fn();
  render(
    <ImageParams
      model={model}
      value={{ n: 2 }}
      onChange={change}
      disabled={false}
    />,
  );
  expect(screen.getByText(/0.04/)).toBeVisible();
  await userEvent.click(screen.getByRole("combobox", { name: "Quality" }));
  expect(screen.queryByRole("option", { name: "medium" })).toBeNull();
  await userEvent.click(screen.getByRole("option", { name: "low" }));
  expect(change).toHaveBeenCalledWith({ n: 2, quality: "low" });
});
it("does not invent controls or a token-based price estimate", () => {
  render(
    <ImageParams
      model={{
        ...model,
        image: {
          ...model.image!,
          n: false,
          sizes: [],
          qualities: [],
          perImagePrice: null,
        },
      }}
      value={{}}
      onChange={vi.fn()}
      disabled={false}
    />,
  );
  expect(screen.queryByRole("combobox")).toBeNull();
  expect(screen.getByText("Cost depends on actual token usage")).toBeVisible();
});
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
