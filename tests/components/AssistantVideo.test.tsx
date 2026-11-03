import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import { VideoBlock } from "@/components/assistant/VideoBlock";
import type {
  AssistantMessage,
  AssistantAttachment,
} from "@/lib/api/assistant";
import en from "@/i18n/locales/en.json";
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.io", isLoading: false }),
}));
beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
const frame: AssistantAttachment = {
  id: "frame",
  name: "frame.webp",
  mimeType: "image/webp",
  sizeBytes: 100,
  status: "ready",
  reason: null,
  details: null,
};
const message = (status: string, apex = "ofox.io") =>
  ({
    id: "m",
    role: "media",
    content: "square",
    createdAt: Date.now() - 1000,
    status,
    attachments: [],
    media: {
      kind: "video",
      model: "video/test",
      params: { duration: 4, resolution: "480p" },
      inputIds: [],
      status,
      outputs: [],
      cost: null,
      jobId: "job",
      apex,
      submittedAt: Date.now() - 1000,
    },
  }) as unknown as AssistantMessage;
const block = (
  m: AssistantMessage,
  actions: {
    onCancel?: () => Promise<unknown>;
    onResume?: () => Promise<unknown>;
    onSave?: (file: AssistantAttachment) => Promise<unknown>;
  } = {},
) => (
  <VideoBlock
    message={m}
    sessionId="s"
    owner="ab"
    generation={7}
    onSave={vi.fn().mockResolvedValue(true)}
    onCancel={vi.fn().mockResolvedValue(undefined)}
    onResume={vi.fn().mockResolvedValue(undefined)}
    {...actions}
  />
);
it("shows queued progress and sends an explicit cancel without offering regeneration", async () => {
  const cancel = vi.fn().mockResolvedValue(undefined);
  render(block(message("queued"), { onCancel: cancel }));
  expect(screen.getByRole("status")).toHaveTextContent("Queued");
  await userEvent.click(screen.getByRole("button", { name: "Cancel video" }));
  expect(cancel).toHaveBeenCalledOnce();
  expect(screen.queryByRole("button", { name: "Generate again" })).toBeNull();
});
it.each([
  ["download_failed", "Download again"],
  ["query_expired", "Continue querying"],
])("offers only a free query for %s", async (status, label) => {
  const resume = vi.fn().mockResolvedValue(undefined);
  render(block(message(status), { onResume: resume }));
  await userEvent.click(screen.getByRole("button", { name: label }));
  expect(resume).toHaveBeenCalledOnce();
  expect(screen.queryByRole("button", { name: "Generate again" })).toBeNull();
});
it("pauses controls until the video task's region is selected", () => {
  render(block(message("queued", "ofox.ai")));
  expect(screen.getByText(/Switch back to ofox.ai/)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Cancel video" })).toBeNull();
});
it("plays a completed local video through the private protocol without autoplay", () => {
  const file: AssistantAttachment = {
    ...frame,
    id: "movie",
    name: "movie.mp4",
    mimeType: "video/mp4",
    origin: "generated",
  };
  const m = message("completed");
  m.attachments = [file];
  m.media!.outputs = [file.id];
  const { container } = render(block(m));
  const video = container.querySelector("video")!;
  expect(video).not.toBeNull();
  expect(video).toHaveAttribute("controls");
  expect(video).not.toHaveAttribute("autoplay");
  expect(video.getAttribute("src")).toContain("assistant-media");
  expect(video.getAttribute("src")).toContain("generation=7");
});
it("does not request a file excluded from the backup", () => {
  const file: AssistantAttachment = {
    ...frame,
    id: "movie",
    name: "movie.mp4",
    mimeType: "video/mp4",
    status: "missing",
    reason: "assistant_attachment_not_in_backup",
  };
  const m = message("completed");
  m.attachments = [file];
  m.media!.outputs = [file.id];
  const { container } = render(block(m));
  expect(container.querySelector("video")).toBeNull();
  expect(
    screen.getByText("This video was not included in the backup."),
  ).toBeVisible();
});

it("keeps a completed local video available after switching regions", () => {
  render(block(message("completed", "ofox.ai")));
  expect(screen.queryByText(/Switch back to ofox.ai/)).toBeNull();
  expect(screen.getByText("Video ready")).toBeVisible();
});
