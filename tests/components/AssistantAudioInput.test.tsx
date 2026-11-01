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
import { AssistantAudioInput } from "@/components/assistant/AssistantAudioInput";
import type { AssistantModel } from "@/lib/api/assistant";

const api = vi.hoisted(() => ({
  pickAudio: vi.fn(),
  importAudio: vi.fn(),
  discardAudio: vi.fn(),
  transcribe: vi.fn(),
  stop: vi.fn(),
  send: vi.fn(),
}));
vi.mock("@/lib/api/assistant", async (original) => ({
  ...(await original<typeof import("@/lib/api/assistant")>()),
  assistantApi: api,
}));
const recorder = vi.hoisted(() => ({ start: vi.fn() }));
vi.mock("@/lib/assistantRecorder", () => ({ startRecording: recorder.start }));
const target = {
  sessionId: "session-a",
  expectedOwner: "a".repeat(64),
  expectedGeneration: 7,
};
const model = {
  id: "openai/gpt-transcribe",
  name: "GPT Transcribe",
  kind: "transcription",
  audio: { pricing: { unit: "seconds", inputAudioPerSecond: "0.000075" } },
} as AssistantModel;
const input = {
  inputId: "input-a",
  name: "voice.wav",
  sizeBytes: 8044,
  mimeType: "audio/wav",
  durationMs: 250,
  sampleRate: 16000,
  channels: 1,
};
beforeEach(async () => {
  Object.values(api).forEach((fn) => fn.mockReset());
  recorder.start.mockReset();
  api.discardAudio.mockResolvedValue(true);
  api.stop.mockResolvedValue(null);
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

it("waits for a recording/import gesture and never submits on opening", () => {
  render(
    <AssistantAudioInput
      target={target}
      models={[model]}
      onInsert={() => true}
      onClose={() => {}}
    />,
  );
  expect(screen.getByRole("button", { name: "Import audio" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Record" })).toBeEnabled();
  expect(api.pickAudio).not.toHaveBeenCalled();
  expect(api.transcribe).not.toHaveBeenCalled();
  expect(recorder.start).not.toHaveBeenCalled();
});

it("transcribes only explicitly, allows editing, then inserts without sending a chat turn", async () => {
  const user = userEvent.setup();
  const insert = vi.fn(() => true);
  const close = vi.fn();
  api.pickAudio.mockResolvedValue(input);
  api.transcribe.mockResolvedValue({
    operationId: "operation",
    runId: "run",
    kind: "transcription",
    model: model.id,
    status: "completed",
    cost: "0.00001875",
    audio: {
      text: "今天下午三点开会。",
      usage: { type: "duration", seconds: 0.25 },
    },
  });
  render(
    <AssistantAudioInput
      target={target}
      models={[model]}
      onInsert={insert}
      onClose={close}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Import audio" }));
  expect(await screen.findByText("voice.wav")).toBeVisible();
  expect(api.transcribe).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Transcribe" }));
  const draft = await screen.findByRole("textbox", {
    name: "Transcription draft",
  });
  expect(draft).toHaveValue("今天下午三点开会。");
  fireEvent.change(draft, { target: { value: "改为下午四点。" } });
  await user.click(screen.getByRole("button", { name: "Insert into draft" }));
  expect(api.transcribe).toHaveBeenCalledTimes(1);
  expect(api.transcribe).toHaveBeenCalledWith(
    expect.objectContaining({
      ...target,
      inputId: "input-a",
      modelId: model.id,
      runId: expect.any(String),
      operationId: expect.any(String),
    }),
  );
  expect(insert).toHaveBeenCalledWith("改为下午四点。");
  expect(close).toHaveBeenCalled();
  expect(api.send).not.toHaveBeenCalled();
});

it("discards a late file-picker result under its original authority after unmount", async () => {
  let resolve!: (value: typeof input) => void;
  api.pickAudio.mockReturnValue(
    new Promise((r) => {
      resolve = r;
    }),
  );
  const user = userEvent.setup();
  const view = render(
    <AssistantAudioInput
      target={target}
      models={[model]}
      onInsert={() => true}
      onClose={() => {}}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Import audio" }));
  view.unmount();
  await act(async () => resolve(input));
  await waitFor(() =>
    expect(api.discardAudio).toHaveBeenCalledWith({
      ...target,
      inputId: "input-a",
    }),
  );
  expect(api.transcribe).not.toHaveBeenCalled();
});

it("imports a finished recording without submitting and releases it on close", async () => {
  const stop = vi.fn().mockResolvedValue(new File(["PCM"], "recording.wav"));
  const cancel = vi.fn();
  recorder.start.mockResolvedValue({ stop, cancel });
  api.importAudio.mockResolvedValue({ ...input, name: "recording.wav" });
  const user = userEvent.setup();
  const view = render(
    <AssistantAudioInput
      target={target}
      models={[model]}
      onInsert={() => true}
      onClose={() => {}}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Record" }));
  await user.click(
    await screen.findByRole("button", { name: "Finish recording" }),
  );
  expect(await screen.findByText("recording.wav")).toBeVisible();
  expect(stop).toHaveBeenCalledTimes(1);
  expect(api.importAudio).toHaveBeenCalledWith(expect.any(File), target);
  expect(api.transcribe).not.toHaveBeenCalled();
  view.unmount();
  expect(api.discardAudio).toHaveBeenCalledWith({
    ...target,
    inputId: input.inputId,
  });
});

it("cancels a pending microphone permission and owns its late result", async () => {
  let resolve!: (value: unknown) => void;
  recorder.start.mockReturnValue(
    new Promise((r) => {
      resolve = r;
    }),
  );
  const user = userEvent.setup();
  render(
    <AssistantAudioInput
      target={target}
      models={[model]}
      onInsert={() => true}
      onClose={() => {}}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Record" }));
  const signal = recorder.start.mock.calls[0][1] as AbortSignal;
  await user.click(screen.getByRole("button", { name: "Discard recording" }));
  expect(signal.aborted).toBe(true);
  const cancel = vi.fn();
  await act(async () => resolve({ stop: vi.fn(), cancel }));
  expect(cancel).toHaveBeenCalledTimes(1);
  expect(api.importAudio).not.toHaveBeenCalled();
  expect(api.transcribe).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Record" })).toBeEnabled();
});
