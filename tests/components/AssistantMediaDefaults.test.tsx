import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
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

it("starts images on gpt-image-2 and videos on Wan 3.0 while nothing is chosen", () => {
  const as = (id: string, of: AssistantModel) => ({ ...of, id }) as AssistantModel;
  const catalog = [
    as("google/gemini-2.5-flash-image", image),
    as("openai/gpt-image-2", image),
    as("alibaba/happyhorse-1.0", video),
    as("alibaba/wan-3.0", video),
  ];
  expect(generationModel(catalog, "image")?.id).toBe("openai/gpt-image-2");
  expect(generationModel(catalog, "video")?.id).toBe("alibaba/wan-3.0");
  // A choice still wins.
  expect(generationModel(catalog, "video", "alibaba/happyhorse-1.0")?.id).toBe(
    "alibaba/happyhorse-1.0",
  );
});

describe("speech models and voices", () => {
  const tts = (
    id: string,
    family: "gemini-tts" | "elevenlabs-tts",
    pricing: Record<string, string | null>,
  ) =>
    ({
      id,
      name: id,
      shortName: id,
      kind: "speech",
      audio: {
        family,
        responseFormats: ["wav", "mp3"],
        defaultFormat: "wav",
        voiceKind: family === "gemini-tts" ? "name" : "id",
        suggestedVoices:
          family === "gemini-tts" ? ["Kore", "Puck"] : ["21m00Tcm4TlvDq8ikWAM"],
        instructions: false,
        prompt: false,
        language: true,
        wordTimestamps: false,
        pricing: {
          unit: family === "gemini-tts" ? "tokens" : "characters",
          inputAudioPerSecond: null,
          inputTextPerCharacter: null,
          inputTextPerMillion: null,
          inputAudioPerMillion: null,
          outputTextPerMillion: null,
          outputAudioPerMillion: null,
          ...pricing,
        },
      },
      inputPricePerMillion: "0",
      outputPricePerMillion: "0",
    }) as unknown as AssistantModel;
  const gemini = tts("google/tts", "gemini-tts", {
    inputTextPerMillion: "0.5",
    outputAudioPerMillion: "10",
  });
  const eleven = tts("elevenlabs/eleven_v4", "elevenlabs-tts", {
    inputTextPerCharacter: "0.00008",
  });

  it("prices each speech model by how it is billed", async () => {
    render(
      <MediaDefaultsFields
        kind="speech"
        models={[gemini, eleven]}
        value={{ model: "google/tts" }}
        onChange={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("combobox", { name: "Model" }));
    const [first, second] = screen.getAllByRole("option");
    expect(first).toHaveTextContent("In $0.5 · audio $10/M");
    expect(second).toHaveTextContent("$0.08/1K chars");
    expect(second).not.toHaveTextContent("$0 / $0");
  });

  it("takes a voice the list does not have, in the form the model accepts", async () => {
    const onChange = vi.fn();
    render(
      <MediaDefaultsFields
        kind="speech"
        models={[gemini]}
        value={{ model: "google/tts" }}
        onChange={onChange}
      />,
    );
    await userEvent.click(screen.getByRole("combobox", { name: "Voice" }));
    const search = screen.getByRole("combobox", {
      name: "Search or type a voice name",
    });
    await userEvent.type(search, "Charon");
    await userEvent.click(screen.getByRole("option", { name: "Use “Charon”" }));
    expect(onChange).toHaveBeenCalledWith({ model: "google/tts", voice: "Charon" });
  });

  it("offers no ElevenLabs voice that is not a voice ID", async () => {
    render(
      <MediaDefaultsFields
        kind="speech"
        models={[eleven]}
        value={{ model: "elevenlabs/eleven_v4" }}
        onChange={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("combobox", { name: "Voice" }));
    const search = screen.getByRole("combobox", {
      name: "Search or paste a voice ID",
    });
    await userEvent.type(search, "Rachel");
    expect(screen.queryByRole("option", { name: /Use/ })).toBeNull();
    await userEvent.clear(search);
    await userEvent.type(search, "pNInz6obpgDQGcFmaJgB");
    expect(
      screen.getByRole("option", { name: "Use “pNInz6obpgDQGcFmaJgB”" }),
    ).toBeInTheDocument();
  });
});
