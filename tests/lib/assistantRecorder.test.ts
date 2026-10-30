import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { startRecording } from "@/lib/assistantRecorder";

let context: {
  sampleRate: number;
  state: string;
  audioWorklet: { addModule: ReturnType<typeof vi.fn> };
  resume: ReturnType<typeof vi.fn>;
  close: ReturnType<typeof vi.fn>;
  createMediaStreamSource: ReturnType<typeof vi.fn>;
  destination: object;
  onstatechange: (() => void) | null;
};
let track: {
  stop: ReturnType<typeof vi.fn>;
  readyState: string;
  onended: (() => void) | null;
};
let node: {
  port: {
    onmessage: ((event: { data: unknown }) => void) | null;
    postMessage: ReturnType<typeof vi.fn>;
    close: ReturnType<typeof vi.fn>;
  };
  connect: ReturnType<typeof vi.fn>;
  disconnect: ReturnType<typeof vi.fn>;
  onprocessorerror: (() => void) | null;
};
const media = vi.fn();
const callbacks = () => ({ progress: vi.fn(), limit: vi.fn(), error: vi.fn() });
function emit(data: unknown) {
  node.port.onmessage?.({ data });
}
beforeEach(() => {
  track = { stop: vi.fn(), readyState: "live", onended: null };
  node = {
    port: { onmessage: null, postMessage: vi.fn(), close: vi.fn() },
    connect: vi.fn(),
    disconnect: vi.fn(),
    onprocessorerror: null,
  };
  context = {
    sampleRate: 16000,
    state: "running",
    audioWorklet: { addModule: vi.fn().mockResolvedValue(undefined) },
    resume: vi.fn().mockResolvedValue(undefined),
    close: vi.fn().mockResolvedValue(undefined),
    createMediaStreamSource: vi.fn(() => ({
      connect: vi.fn(),
      disconnect: vi.fn(),
    })),
    destination: {},
    onstatechange: null,
  };
  media.mockReset().mockResolvedValue({
    getTracks: () => [track],
    getAudioTracks: () => [track],
  });
  vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: media } });
  vi.stubGlobal(
    "AudioContext",
    vi.fn(function () {
      return context;
    }),
  );
  vi.stubGlobal(
    "AudioWorkletNode",
    vi.fn(function () {
      return node;
    }),
  );
});
afterEach(() => vi.unstubAllGlobals());

it("records only after explicit start and flushes bounded WAV before releasing every resource", async () => {
  const cb = callbacks();
  const recording = await startRecording(cb, new AbortController().signal);
  expect(media).toHaveBeenCalledTimes(1);
  expect(media).toHaveBeenCalledWith({
    audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    video: false,
  });
  emit({ type: "chunk", samples: new Int16Array([0, 32767, -32768]) });
  const stopped = recording.stop();
  expect(node.port.postMessage).toHaveBeenCalledWith({ type: "stop" });
  emit({ type: "stopped", frames: 3 });
  const file = await stopped;
  expect(file.type).toBe("audio/wav");
  expect(file.size).toBe(50);
  const bytes = await new Promise<ArrayBuffer>((resolve) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as ArrayBuffer);
    reader.readAsArrayBuffer(file);
  });
  const view = new DataView(bytes);
  expect(view.getUint32(24, true)).toBe(16000);
  expect(view.getInt16(46, true)).toBe(32767);
  expect(track.stop).toHaveBeenCalledTimes(1);
  expect(node.disconnect).toHaveBeenCalled();
  expect(context.close).toHaveBeenCalled();
  expect(node.port.close).toHaveBeenCalled();
  expect(cb.error).not.toHaveBeenCalled();
  expect(cb.limit).not.toHaveBeenCalled();
});

it("releases a late permission result after cancellation without capturing anything", async () => {
  let resolve!: (stream: unknown) => void;
  media.mockReturnValue(
    new Promise((r) => {
      resolve = r;
    }),
  );
  const controller = new AbortController();
  const result = startRecording(callbacks(), controller.signal).catch((e) => e);
  controller.abort();
  resolve({ getTracks: () => [track], getAudioTracks: () => [track] });
  expect((await result).name).toBe("AbortError");
  expect(track.stop).toHaveBeenCalledTimes(1);
  expect(context.close).toHaveBeenCalled();
});

it("reports denied and missing microphones without leaking an audio context", async () => {
  for (const [name, code] of [
    ["NotAllowedError", "assistant_microphone_denied"],
    ["NotFoundError", "assistant_microphone_unavailable"],
  ]) {
    media.mockRejectedValueOnce(
      new DOMException("private device detail", name),
    );
    await expect(
      startRecording(callbacks(), new AbortController().signal),
    ).rejects.toBe(code);
  }
  expect(context.close).toHaveBeenCalledTimes(2);
});

it("discards empty, interrupted and cancelled recordings and never leaves a track live", async () => {
  const cb = callbacks();
  const recording = await startRecording(cb, new AbortController().signal);
  const stopped = recording.stop();
  emit({ type: "stopped", frames: 0 });
  await expect(stopped).rejects.toBe("assistant_audio_empty");
  const other = await startRecording(cb, new AbortController().signal);
  node.onprocessorerror?.();
  expect(cb.error).toHaveBeenCalledWith("assistant_recording_interrupted");
  other.cancel();
  expect(track.stop).toHaveBeenCalledTimes(2);
});

it("honors the captured sample rate rather than mislabelling a browser fallback", async () => {
  context.sampleRate = 48000;
  await expect(
    startRecording(callbacks(), new AbortController().signal),
  ).rejects.toBe("assistant_microphone_unavailable");
  expect(context.close).toHaveBeenCalled();
});

it("stops capturing when the track ends or the audio context is interrupted", async () => {
  const cb = callbacks();
  await startRecording(cb, new AbortController().signal);
  track.onended?.();
  expect(cb.error).toHaveBeenCalledWith("assistant_recording_interrupted");
  expect(track.stop).toHaveBeenCalledTimes(1);
  await startRecording(cb, new AbortController().signal);
  context.state = "interrupted";
  context.onstatechange?.();
  expect(cb.error).toHaveBeenCalledTimes(2);
  expect(track.stop).toHaveBeenCalledTimes(2);
  expect(cb.limit).not.toHaveBeenCalled();
});

it("rejects a stop waiting for its worklet acknowledgement when cancelled", async () => {
  const cb = callbacks();
  const recording = await startRecording(cb, new AbortController().signal);
  emit({ type: "chunk", samples: new Int16Array([1, 2]) });
  const stopped = recording.stop();
  recording.cancel();
  await expect(stopped).rejects.toMatchObject({ name: "AbortError" });
  expect(cb.limit).not.toHaveBeenCalled();
  expect(cb.error).not.toHaveBeenCalled();
  expect(track.stop).toHaveBeenCalledTimes(1);
});
