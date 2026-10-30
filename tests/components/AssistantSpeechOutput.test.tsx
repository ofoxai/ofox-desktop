import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantSpeechOutput } from "@/components/assistant/AssistantSpeechOutput";
import type { AssistantModel, AssistantAudioResult } from "@/lib/api/assistant";
const api = vi.hoisted(() => ({
  synthesize: vi.fn(),
  stop: vi.fn(),
  send: vi.fn(),
}));
vi.mock("@/lib/api/assistant", async (original) => ({
  ...(await original<typeof import("@/lib/api/assistant")>()),
  assistantApi: api,
}));
const target = {
  sessionId: "session-a",
  expectedOwner: "a".repeat(64),
  expectedGeneration: 7,
};
const models = [
  {
    id: "google/gemini-3.8-flash-tts",
    name: "Gemini TTS",
    kind: "speech",
    audio: {
      family: "gemini-tts",
      responseFormats: ["wav", "pcm"],
      defaultFormat: "wav",
      suggestedVoices: ["Kore", "Puck", "Zephyr"],
      instructions: true,
      voiceKind: "name",
      pricing: { unit: "tokens" },
    },
  },
  {
    id: "elevenlabs/eleven_v4",
    name: "ElevenLabs",
    kind: "speech",
    audio: {
      family: "elevenlabs-tts",
      responseFormats: ["mp3", "opus", "wav", "pcm"],
      defaultFormat: "mp3",
      suggestedVoices: ["21m00Tcm4TlvDq8ikWAM"],
      instructions: false,
      voiceKind: "id",
      pricing: { unit: "characters", inputTextPerCharacter: "0.0002" },
    },
  },
] as AssistantModel[];
beforeEach(async () => {
  Object.values(api).forEach((fn) => fn.mockReset());
  api.stop.mockResolvedValue(null);
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
it("previews supplied reply text and never synthesizes or plays on opening", () => {
  render(
    <AssistantSpeechOutput
      target={target}
      models={models}
      initialText="Read this reply."
      sourceMessageId="reply-a"
      onClose={() => {}}
    />,
  );
  expect(screen.getByRole("textbox", { name: "Text to speak" })).toHaveValue(
    "Read this reply.",
  );
  expect(screen.getByRole("button", { name: "Generate speech" })).toBeEnabled();
  expect(api.synthesize).not.toHaveBeenCalled();
  expect(document.querySelector("audio")).toBeNull();
});
it("edits text before one explicit generation and removes stale reply attribution", async () => {
  const user = userEvent.setup();
  api.synthesize.mockResolvedValue({
    operationId: "op",
    runId: "run",
    kind: "speech",
    status: "completed",
    cost: null,
    outputs: ["output-a"],
    audio: {
      text: "Edited text",
      estimate: null,
      output: { mimeType: "audio/wav", sizeBytes: 16044, durationMs: 1000 },
    },
  });
  render(
    <AssistantSpeechOutput
      target={target}
      models={models}
      initialText="Read this reply."
      sourceMessageId="reply-a"
      onClose={() => {}}
    />,
  );
  fireEvent.change(screen.getByRole("textbox", { name: "Text to speak" }), {
    target: { value: "Edited text" },
  });
  await user.click(screen.getByRole("button", { name: "Generate speech" }));
  expect(await screen.findByText("Speech generated")).toBeVisible();
  expect(api.synthesize).toHaveBeenCalledTimes(1);
  expect(api.synthesize).toHaveBeenCalledWith(
    expect.objectContaining({
      ...target,
      input: "Edited text",
      modelId: models[0].id,
      voice: "Kore",
      responseFormat: "wav",
    }),
  );
  expect(api.synthesize.mock.calls[0][0].sourceMessageId).toBeUndefined();
  expect(api.send).not.toHaveBeenCalled();
  expect(
    screen.getByText(
      "Actual cost is unknown; the audio response did not include usage.",
    ),
  ).toBeVisible();
});
it("shows only family-supported controls and refuses text over the Unicode character limit", async () => {
  render(
    <AssistantSpeechOutput
      target={target}
      models={models}
      rememberedModel={models[1].id}
      initialText={"中".repeat(4001)}
      onClose={() => {}}
    />,
  );
  expect(
    screen.queryByRole("textbox", { name: "Tone and style" }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Voice ID" })).toHaveValue(
    "21m00Tcm4TlvDq8ikWAM",
  );
  expect(
    screen.getByRole("combobox", { name: "Audio format" }),
  ).toHaveTextContent("MP3");
  expect(
    screen.getByRole("button", { name: "Generate speech" }),
  ).toBeDisabled();
});
it("stops an in-flight request once on unmount without replaying it", async () => {
  let resolve!: (value: unknown) => void;
  api.synthesize.mockReturnValue(
    new Promise((r) => {
      resolve = r;
    }),
  );
  const user = userEvent.setup();
  const view = render(
    <AssistantSpeechOutput
      target={target}
      models={models}
      initialText="Hello"
      onClose={() => {}}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Generate speech" }));
  const request = api.synthesize.mock.calls[0][0];
  expect(
    screen.getByRole("button", { name: "Stop generating speech" }),
  ).toBeEnabled();
  view.unmount();
  await waitFor(() =>
    expect(api.stop).toHaveBeenCalledWith(
      target.sessionId,
      request.runId,
      request.operationId,
    ),
  );
  await act(async () =>
    resolve({
      status: "unknown",
      audio: { errorCode: "assistant_audio_outcome_unknown" },
    }),
  );
  expect(api.synthesize).toHaveBeenCalledTimes(1);
  expect(api.stop).toHaveBeenCalledTimes(1);
});

it("does not offer an unchanged paid replay after an uncertain outcome", async () => {
  api.synthesize.mockResolvedValue({
    kind: "speech",
    status: "unknown",
    cost: null,
    audio: { errorCode: "assistant_audio_outcome_unknown" },
  });
  const user = userEvent.setup();
  render(
    <AssistantSpeechOutput
      target={target}
      models={models}
      initialText="Hello"
      onClose={() => {}}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Generate speech" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "The audio request outcome is unknown",
  );
  expect(
    screen.getByRole("button", { name: "Generate speech" }),
  ).toBeDisabled();
  expect(api.synthesize).toHaveBeenCalledTimes(1);
});

it("previews the saved text and frozen estimate when selecting a historical result", async () => {
  const history: AssistantAudioResult[] = [
    {
      operationId: "old-op",
      runId: "old-run",
      kind: "speech",
      model: models[1].id,
      status: "completed",
      cost: null,
      params: {
        voice: "21m00Tcm4TlvDq8ikWAM",
        responseFormat: "mp3",
        language: "en",
      },
      audio: {
        text: "Previous speech.",
        estimate: "0.25",
        pricing: models[1].audio!.pricing,
        input: null,
        usage: null,
        requestId: null,
        requestIdSource: null,
        errorCode: null,
        output: {
          mimeType: "audio/mpeg",
          sizeBytes: 1024,
          durationMs: 1000,
          sampleRate: 44100,
          channels: 1,
        },
      },
    },
  ];
  render(
    <AssistantSpeechOutput
      target={target}
      models={models}
      initialText="Current draft"
      history={history}
      onClose={() => {}}
    />,
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Speech result 1" }),
  );
  expect(screen.getByRole("textbox", { name: "Text to speak" })).toHaveValue(
    "Previous speech.",
  );
  expect(
    screen.getByRole("combobox", { name: "Speech model" }),
  ).toHaveTextContent("ElevenLabs");
  expect(
    screen.getByText(
      "Estimated price: $0.25. Actual cost needs a usage receipt.",
    ),
  ).toBeVisible();
  expect(api.synthesize).not.toHaveBeenCalled();
});
