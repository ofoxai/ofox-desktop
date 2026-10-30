import {
  act,
  fireEvent,
  render,
  screen,
  cleanup,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  afterEach,
  beforeEach,
  expect,
  it,
  vi,
  type MockInstance,
} from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantAudioPlayer } from "@/components/assistant/AssistantAudioPlayer";
import { InteractionScope } from "@/components/ui/interaction-scope";
const api = vi.hoisted(() => ({
  saveMedia: vi.fn(),
  synthesize: vi.fn(),
  transcribe: vi.fn(),
  stop: vi.fn(),
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
const props = {
  target,
  attachmentId: "audio-a",
  mimeType: "audio/wav",
  durationMs: 4000,
};
let play: ReturnType<typeof vi.spyOn>,
  pause: ReturnType<typeof vi.spyOn>,
  load: ReturnType<typeof vi.spyOn>,
  canPlay: MockInstance<HTMLMediaElement["canPlayType"]>;
beforeEach(async () => {
  Object.values(api).forEach((fn) => fn.mockReset());
  api.saveMedia.mockResolvedValue(true);
  play = vi
    .spyOn(HTMLMediaElement.prototype, "play")
    .mockResolvedValue(undefined);
  pause = vi
    .spyOn(HTMLMediaElement.prototype, "pause")
    .mockImplementation(() => {});
  load = vi
    .spyOn(HTMLMediaElement.prototype, "load")
    .mockImplementation(() => {});
  canPlay = vi
    .spyOn(HTMLMediaElement.prototype, "canPlayType")
    .mockReturnValue("probably");
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it("opens a stored result without autoplay or loading audio bytes", () => {
  const view = render(<AssistantAudioPlayer {...props} />);
  const audio = view.container.querySelector("audio")!;
  expect(audio).not.toBeNull();
  expect(audio).toHaveAttribute("preload", "none");
  expect(audio).not.toHaveAttribute("src");
  expect(play).not.toHaveBeenCalled();
  expect(api.synthesize).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Play audio" })).toBeEnabled();
});
it("keeps the validated duration until the decoder provides a positive finite duration", async () => {
  const view = render(<AssistantAudioPlayer {...props} />);
  const audio = view.container.querySelector("audio")!;
  await userEvent.click(screen.getByRole("button", { name: "Play audio" }));
  Object.defineProperty(audio, "duration", { configurable: true, value: 0 });
  fireEvent.loadedMetadata(audio);
  expect(screen.getByRole("timer")).toHaveTextContent("0:00 / 0:04");
  Object.defineProperty(audio, "duration", { value: Infinity });
  fireEvent.durationChange(audio);
  expect(screen.getByRole("timer")).toHaveTextContent("0:00 / 0:04");
  Object.defineProperty(audio, "duration", { value: 5 });
  fireEvent.durationChange(audio);
  expect(screen.getByRole("timer")).toHaveTextContent("0:00 / 0:05");
  Object.defineProperty(audio, "duration", { value: Infinity });
  fireEvent.durationChange(audio);
  expect(screen.getByRole("timer")).toHaveTextContent("0:00 / 0:05");
  await userEvent.click(screen.getByRole("button", { name: "Stop audio" }));
  Object.defineProperty(audio, "duration", { value: 99 });
  fireEvent.durationChange(audio);
  expect(screen.getByRole("timer")).toHaveTextContent("0:00 / 0:05");
  expect(api.synthesize).not.toHaveBeenCalled();
  expect(api.transcribe).not.toHaveBeenCalled();
});
it("keeps a zero decoder duration unknown when no validated duration was saved", async () => {
  const view = render(
    <AssistantAudioPlayer {...props} durationMs={undefined} />,
  );
  const audio = view.container.querySelector("audio")!;
  await userEvent.click(screen.getByRole("button", { name: "Play audio" }));
  Object.defineProperty(audio, "duration", { value: 0 });
  fireEvent.loadedMetadata(audio);
  expect(screen.getByRole("timer")).toHaveTextContent("0:00 / --:--");
});
it("plays, pauses, stops and replays the same private file without another model request", async () => {
  const user = userEvent.setup();
  const view = render(<AssistantAudioPlayer {...props} />);
  const audio = view.container.querySelector("audio")!;
  await user.click(screen.getByRole("button", { name: "Play audio" }));
  expect(
    await screen.findByRole("button", { name: "Pause audio" }),
  ).toBeEnabled();
  expect(audio.getAttribute("src")).toContain("assistant-media");
  expect(audio.getAttribute("src")).toContain("generation=7");
  audio.currentTime = 2;
  fireEvent.timeUpdate(audio);
  expect(screen.getByText("0:02 / 0:04")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Pause audio" }));
  expect(audio.currentTime).toBe(2);
  await user.click(screen.getByRole("button", { name: "Play audio" }));
  expect(play).toHaveBeenCalledTimes(2);
  await user.click(screen.getByRole("button", { name: "Stop audio" }));
  expect(audio.currentTime).toBe(0);
  expect(audio).not.toHaveAttribute("src");
  expect(load).toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Play audio" }));
  expect(play).toHaveBeenCalledTimes(3);
  expect(api.synthesize).not.toHaveBeenCalled();
  expect(api.transcribe).not.toHaveBeenCalled();
  expect(api.stop).not.toHaveBeenCalled();
});
it("keeps native save available for unsupported Opus without a paid format conversion", async () => {
  canPlay.mockReturnValue("");
  render(<AssistantAudioPlayer {...props} mimeType="audio/ogg" />);
  expect(screen.getByRole("button", { name: "Play audio" })).toBeDisabled();
  expect(
    screen.getByText(
      "This device cannot play this audio format. You can save the file instead.",
    ),
  ).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "Save audio" }));
  expect(api.saveMedia).toHaveBeenCalledWith({
    ...target,
    attachmentId: "audio-a",
  });
  expect(await screen.findByText("Audio saved")).toBeVisible();
  expect(api.synthesize).not.toHaveBeenCalled();
});
it("aborts playback and ignores an old play promise when the owner generation changes", async () => {
  let resolve!: () => void;
  play.mockReturnValue(
    new Promise<void>((r) => {
      resolve = r;
    }),
  );
  const user = userEvent.setup();
  const view = render(<AssistantAudioPlayer {...props} />);
  await user.click(screen.getByRole("button", { name: "Play audio" }));
  view.rerender(
    <AssistantAudioPlayer
      {...props}
      target={{ ...target, expectedGeneration: 8 }}
    />,
  );
  await act(async () => resolve());
  expect(screen.getByRole("button", { name: "Play audio" })).toBeEnabled();
  expect(view.container.querySelector("audio")).not.toHaveAttribute("src");
  expect(pause).toHaveBeenCalled();
});
it("releases the source when its retained page becomes inactive and never auto-resumes", async () => {
  const user = userEvent.setup();
  const view = render(
    <InteractionScope active>
      <AssistantAudioPlayer {...props} />
    </InteractionScope>,
  );
  await user.click(screen.getByRole("button", { name: "Play audio" }));
  view.rerender(
    <InteractionScope active={false}>
      <AssistantAudioPlayer {...props} />
    </InteractionScope>,
  );
  expect(view.container.querySelector("audio")).not.toHaveAttribute("src");
  expect(screen.getByRole("button", { name: "Play audio" })).toBeDisabled();
  view.rerender(
    <InteractionScope active>
      <AssistantAudioPlayer {...props} />
    </InteractionScope>,
  );
  expect(play).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("button", { name: "Play audio" })).toBeEnabled();
});
it("reports a local read/play failure and leaves saving available without regeneration", async () => {
  render(<AssistantAudioPlayer {...props} />);
  await userEvent.click(screen.getByRole("button", { name: "Play audio" }));
  fireEvent.error(document.querySelector("audio")!);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Unable to play this saved audio",
  );
  expect(screen.getByRole("button", { name: "Save audio" })).toBeEnabled();
  expect(api.synthesize).not.toHaveBeenCalled();
});
it("ignores a native-save completion that belongs to a previous generation", async () => {
  let resolve!: (saved: boolean) => void;
  api.saveMedia.mockReturnValue(
    new Promise((r) => {
      resolve = r;
    }),
  );
  const user = userEvent.setup();
  const view = render(<AssistantAudioPlayer {...props} />);
  await user.click(screen.getByRole("button", { name: "Save audio" }));
  view.rerender(
    <AssistantAudioPlayer
      {...props}
      target={{ ...target, expectedGeneration: 8 }}
    />,
  );
  await act(async () => resolve(true));
  expect(screen.queryByText("Audio saved")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save audio" })).toBeEnabled();
});

it("recovers from a rejected play permission without requesting new speech", async () => {
  play.mockRejectedValueOnce(new DOMException("Blocked", "NotAllowedError"));
  render(<AssistantAudioPlayer {...props} />);
  await userEvent.click(screen.getByRole("button", { name: "Play audio" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Playback was blocked",
  );
  expect(screen.getByRole("button", { name: "Play audio" })).toBeEnabled();
  expect(document.querySelector("audio")).not.toHaveAttribute("src");
  await userEvent.click(screen.getByRole("button", { name: "Play audio" }));
  expect(
    await screen.findByRole("button", { name: "Pause audio" }),
  ).toBeEnabled();
  expect(api.synthesize).not.toHaveBeenCalled();
});
