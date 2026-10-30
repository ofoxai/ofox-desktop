import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import type { AssistantAudioResult } from "@/lib/api/assistant";
import { AssistantAudioHistory } from "@/components/assistant/AssistantAudioHistory";

const player = vi.hoisted(() => vi.fn());
vi.mock("@/components/assistant/AssistantAudioPlayer", () => ({
  AssistantAudioPlayer: (props: Record<string, unknown>) => {
    player(props);
    return <div data-testid="saved-player">{String(props.attachmentId)}</div>;
  },
}));
const target = {
  sessionId: "saved-session",
  expectedOwner: "a".repeat(64),
  expectedGeneration: 7,
};
const pricing: AssistantAudioResult["audio"]["pricing"] = {
  unit: "tokens",
  inputAudioPerSecond: null,
  inputTextPerCharacter: null,
  inputTextPerMillion: null,
  inputAudioPerMillion: null,
  outputTextPerMillion: null,
  outputAudioPerMillion: null,
};
const item = (
  kind: "speech" | "transcription",
  extra: Partial<AssistantAudioResult> = {},
): AssistantAudioResult => ({
  operationId: kind,
  runId: `run-${kind}`,
  kind,
  model: `retired/${kind}`,
  params: {},
  status: "completed",
  cost: null,
  outputs: kind === "speech" ? ["generated"] : [],
  inputIds: kind === "transcription" ? ["original"] : [],
  audio: {
    pricing,
    text: "Saved words",
    usage: null,
    input: {
      mimeType: "audio/mpeg",
      durationMs: 900,
      sampleRate: 16000,
      channels: 1,
    },
    output: {
      mimeType: "audio/wav",
      durationMs: 1000,
      sizeBytes: 32044,
      sampleRate: 16000,
      channels: 1,
    },
    requestId: null,
    requestIdSource: null,
    errorCode: null,
  },
  ...extra,
});
const view = (history: AssistantAudioResult[], onClose = vi.fn()) => (
  <AssistantAudioHistory
    target={target}
    history={history}
    attachments={[]}
    returnFocusRef={createRef<HTMLButtonElement>()}
    onClose={onClose}
  />
);
beforeEach(async () => {
  player.mockReset();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
it("switching saved records mounts one player and releases the previous result", async () => {
  const user = userEvent.setup();
  render(view([item("speech"), item("transcription")]));
  expect(screen.getAllByTestId("saved-player")).toHaveLength(1);
  expect(screen.getByTestId("saved-player")).toHaveTextContent("original");
  await user.click(screen.getByRole("button", { name: /retired\/speech/ }));
  expect(screen.getAllByTestId("saved-player")).toHaveLength(1);
  expect(screen.getByTestId("saved-player")).toHaveTextContent("generated");
  expect(player).toHaveBeenCalledWith(
    expect.objectContaining({
      target,
      attachmentId: "generated",
      mimeType: "audio/wav",
      durationMs: 1000,
    }),
  );
  expect(player).toHaveBeenCalledWith(
    expect.objectContaining({
      target,
      attachmentId: "original",
      mimeType: "audio/mpeg",
      durationMs: 900,
    }),
  );
  expect(screen.queryByRole("combobox")).toBeNull();
  expect(screen.queryByRole("textbox")).toBeNull();
  for (const name of [
    "Generate speech",
    "Transcribe",
    "Record",
    "Import audio",
  ])
    expect(screen.queryByRole("button", { name })).toBeNull();
  expect(
    screen.getByText(/Opening history makes no new model request/),
  ).toBeVisible();
});

it("keeps failed and unknown records readable without inventing a playable file", () => {
  render(view([item("speech", { status: "unknown", outputs: [] })]));
  expect(screen.getByText(/Unknown outcome/)).toBeVisible();
  expect(screen.getByText("Saved words")).toBeVisible();
  expect(
    screen.getByText("No saved audio file for this request."),
  ).toBeVisible();
  expect(player).not.toHaveBeenCalled();
});
it("does not offer temporary or discarded originals that have no retained reference", () => {
  render(view([item("transcription", { inputIds: [] })]));
  expect(player).not.toHaveBeenCalled();
});
it("falls back to saved attachment MIME for an older speech result", () => {
  const old = item("speech");
  old.audio.output = null;
  render(
    <AssistantAudioHistory
      target={target}
      history={[old]}
      attachments={[
        {
          id: "generated",
          name: "voice.mp3",
          mimeType: "audio/mpeg",
          sizeBytes: 100,
          status: "ready",
          reason: null,
          previewUrl: null,
          details: null,
        },
      ]}
      returnFocusRef={createRef<HTMLButtonElement>()}
      onClose={() => {}}
    />,
  );
  expect(player).toHaveBeenCalledWith(
    expect.objectContaining({
      mimeType: "audio/mpeg",
      attachmentId: "generated",
    }),
  );
});
it("closes from the explicit button and Escape", async () => {
  const user = userEvent.setup();
  const close = vi.fn();
  render(view([item("speech")], close));
  await user.click(screen.getByRole("button", { name: "Close" }));
  expect(close).toHaveBeenCalledTimes(1);
  await user.keyboard("{Escape}");
  expect(close).toHaveBeenCalledTimes(2);
});
