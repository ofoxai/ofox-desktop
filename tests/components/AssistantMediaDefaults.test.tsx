import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import {
  MediaDefaultsFields,
  generationModel,
} from "@/components/assistant/MediaDefaultsFields";
import type { AssistantModel } from "@/lib/api/assistant";
import en from "@/i18n/locales/en.json";

beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

const image = {
  id: "image/test",
  name: "Image",
  shortName: "Image",
  kind: "image",
  image: {
    generations: true,
    edits: true,
    n: true,
    sizes: ["1024x1024"],
    qualities: [],
    perImagePrice: "0.02",
    tokenPricing: null,
  },
} as unknown as AssistantModel;
const video = {
  id: "video/test",
  name: "Video",
  shortName: "Video",
  kind: "video",
  video: {
    modes: ["t2v"],
    resolutions: ["480p", "720p"],
    defaultResolution: "480p",
    minDuration: 4,
    maxDuration: 6,
    supportsAudio: true,
    aspectRatios: ["16:9"],
    fromPricePerSecond: "0.02",
  },
} as unknown as AssistantModel;
const speech = {
  id: "speech/test",
  name: "Speech",
  shortName: "Speech",
  kind: "speech",
  audio: {
    suggestedVoices: ["Kore", "Puck"],
    responseFormats: ["wav", "pcm"],
    defaultFormat: "wav",
  },
} as unknown as AssistantModel;
const models = [image, video, speech];

it("offers only the parameters the image model lists", async () => {
  render(
    <MediaDefaultsFields kind="image" models={models} value={{}} onChange={() => {}} />,
  );
  expect(screen.getByRole("combobox", { name: "Size" })).toBeVisible();
  expect(screen.queryByRole("combobox", { name: "Quality" })).toBeNull();
  await userEvent.click(screen.getByRole("combobox", { name: "Images" }));
  expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
    "1",
    "2",
    "3",
    "4",
  ]);
});

it("limits video durations to the model's range and keeps sound on by default", async () => {
  const onChange = vi.fn();
  render(
    <MediaDefaultsFields
      kind="video"
      models={models}
      value={{ model: "video/test" }}
      onChange={onChange}
    />,
  );
  expect(screen.getByRole("switch", { name: "Sound" })).toBeChecked();
  await userEvent.click(screen.getByRole("combobox", { name: "Duration" }));
  expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
    "Default",
    "4 s",
    "5 s",
    "6 s",
  ]);
  await userEvent.click(screen.getByRole("option", { name: "5 s" }));
  expect(onChange).toHaveBeenCalledWith({ model: "video/test", duration: 5 });
});

it("never offers raw PCM for speech", async () => {
  render(
    <MediaDefaultsFields kind="speech" models={models} value={{}} onChange={() => {}} />,
  );
  await userEvent.click(screen.getByRole("combobox", { name: "Format" }));
  expect(screen.queryByRole("option", { name: "pcm" })).toBeNull();
  expect(screen.getByRole("option", { name: "wav" })).toBeVisible();
});

it("starts another model from its own parameters", async () => {
  const onChange = vi.fn();
  const second = { ...image, id: "image/second", name: "Second", shortName: "Second" };
  render(
    <MediaDefaultsFields
      kind="image"
      models={[...models, second]}
      value={{ model: "image/test", size: "1024x1024" }}
      onChange={onChange}
    />,
  );
  await userEvent.click(screen.getByRole("combobox", { name: "Model" }));
  await userEvent.click(screen.getByRole("option", { name: /Second/ }));
  expect(onChange).toHaveBeenCalledWith({ model: "image/second" });
});

it("falls back to the first usable model when the chosen one is gone", () => {
  expect(generationModel(models, "image", "gone")?.id).toBe("image/test");
  expect(generationModel(models, "speech")?.id).toBe("speech/test");
  expect(generationModel([], "video")).toBeUndefined();
});
