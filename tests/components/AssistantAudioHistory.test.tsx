import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantAudioInput } from "@/components/assistant/AssistantAudioInput";
import type { AssistantAudioResult, AssistantModel } from "@/lib/api/assistant";
const api = vi.hoisted(() => ({
  pickAudio: vi.fn(),
  discardAudio: vi.fn(),
  transcribe: vi.fn(),
  stop: vi.fn(),
  synthesize: vi.fn(),
  saveMedia: vi.fn(),
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
const asr = {
  id: "openai/gpt-transcribe",
  name: "GPT Transcribe",
  kind: "transcription",
  audio: { pricing: { unit: "seconds", inputAudioPerSecond: "0.000075" } },
} as AssistantModel;
const originalScroll = Object.getOwnPropertyDescriptor(
  Element.prototype,
  "scrollIntoView",
);
beforeEach(async () => {
  Object.defineProperty(Element.prototype, "scrollIntoView", {
    configurable: true,
    value: vi.fn(),
  });
  Object.values(api).forEach((f) => f.mockReset());
  api.discardAudio.mockResolvedValue(true);
  api.stop.mockResolvedValue(null);
  api.pickAudio.mockResolvedValue({
    inputId: "input-a",
    name: "voice.wav",
    mimeType: "audio/wav",
    sizeBytes: 16044,
    durationMs: 1000,
    sampleRate: 8000,
    channels: 1,
  });
  api.transcribe.mockResolvedValue({
    operationId: "op",
    status: "completed",
    cost: null,
    params: {},
    audio: { text: "Hello", usage: null },
  });
  vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(() => {});
  vi.spyOn(HTMLMediaElement.prototype, "canPlayType").mockReturnValue(
    "probably",
  );
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  if (originalScroll)
    Object.defineProperty(Element.prototype, "scrollIntoView", originalScroll);
  else Reflect.deleteProperty(Element.prototype, "scrollIntoView");
});
it("keeps originals temporary by default and retains them only through an explicit choice", async () => {
  render(
    <AssistantAudioInput
      target={target}
      models={[asr]}
      onInsert={() => true}
      onClose={() => {}}
    />,
  );
  const toggle = screen.getByRole("switch", {
    name: "Keep original audio in history and backups",
  });
  expect(toggle).not.toBeChecked();
  await userEvent.click(screen.getByRole("button", { name: "Import audio" }));
  await userEvent.click(toggle);
  await userEvent.click(screen.getByRole("button", { name: "Transcribe" }));
  expect(api.transcribe).toHaveBeenCalledWith(
    expect.objectContaining({ keepOriginal: true, inputId: "input-a" }),
  );
  expect(
    await screen.findByRole("textbox", { name: "Transcription draft" }),
  ).toHaveValue("Hello");
});
it("reopens a retained original from ASR history without transcribing it again", async () => {
  const history = [
    {
      operationId: "old",
      runId: "old-run",
      kind: "transcription",
      model: asr.id,
      params: { inputName: "original.wav", keepOriginal: true },
      status: "completed",
      cost: null,
      inputIds: ["original-id"],
      audio: {
        pricing: asr.audio!.pricing,
        input: {
          mimeType: "audio/wav",
          durationMs: 1000,
          sampleRate: 8000,
          channels: 1,
        },
        text: "Saved draft",
        usage: null,
        requestId: null,
        requestIdSource: null,
        errorCode: null,
      },
    },
  ] as AssistantAudioResult[];
  render(
    <AssistantAudioInput
      target={target}
      models={[asr]}
      history={history}
      onInsert={() => true}
      onClose={() => {}}
    />,
  );
  await userEvent.click(
    screen.getByRole("button", { name: "View transcription 1" }),
  );
  expect(screen.getByRole("button", { name: "Play audio" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "Play audio" }));
  expect(document.querySelector("audio")?.getAttribute("src")).toContain(
    "original-id",
  );
  expect(api.transcribe).not.toHaveBeenCalled();
  expect(api.synthesize).not.toHaveBeenCalled();
});
it("keeps an active transcription's model and duration quote frozen during catalog refresh", async () => {
  let settle!: (value: unknown) => void;
  api.transcribe.mockReturnValue(
    new Promise((resolve) => {
      settle = resolve;
    }),
  );
  const props = {
    target,
    models: [asr],
    onInsert: () => true,
    onClose: () => {},
  };
  const view = render(<AssistantAudioInput {...props} />);
  await userEvent.click(screen.getByRole("button", { name: "Import audio" }));
  await userEvent.click(screen.getByRole("button", { name: "Transcribe" }));
  view.rerender(
    <AssistantAudioInput
      {...props}
      models={[
        {
          ...asr,
          name: "Updated ASR",
          audio: {
            ...asr.audio!,
            pricing: { ...asr.audio!.pricing, inputAudioPerSecond: "0.5" },
          },
        },
      ]}
    />,
  );
  expect(
    screen.getByRole("combobox", { name: "Transcription model" }),
  ).toHaveTextContent("GPT Transcribe");
  expect(screen.getByText(/Approximately \$0.000075 /)).toBeVisible();
  expect(api.transcribe).toHaveBeenCalledTimes(1);
  await act(async () =>
    settle({
      operationId: "op",
      runId: "run",
      kind: "transcription",
      status: "unknown",
      params: {},
      cost: null,
      audio: { errorCode: "assistant_audio_outcome_unknown" },
    }),
  );
});
