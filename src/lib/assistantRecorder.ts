import workletUrl from "./assistantPcmWorklet.js?url&no-inline";

/** A local, bounded PCM recording. No network or model request is made here. */
export interface RecordingCallbacks {
  progress: (seconds: number) => void;
  limit: (file: File) => void;
  error: (code: string) => void;
}
export interface Recording {
  stop: () => Promise<File>;
  cancel: () => void;
}
const RATE = 16000;
const MAX_FRAMES = RATE * 300;
const aborted = () => new DOMException("Recording cancelled", "AbortError");

function microphoneError(reason: unknown): string {
  if (typeof reason === "string" && reason.startsWith("assistant_"))
    return reason;
  const name =
    reason && typeof reason === "object" && "name" in reason ? reason.name : "";
  return name === "NotAllowedError" || name === "SecurityError"
    ? "assistant_microphone_denied"
    : "assistant_microphone_unavailable";
}
function wav(chunks: readonly Int16Array[], frames: number): File {
  if (!frames) throw "assistant_audio_empty";
  if (frames > MAX_FRAMES) throw "assistant_audio_too_long";
  const bytes = new ArrayBuffer(44 + frames * 2);
  const data = new DataView(bytes);
  const text = (at: number, value: string) => {
    for (let i = 0; i < value.length; i++)
      data.setUint8(at + i, value.charCodeAt(i));
  };
  text(0, "RIFF");
  data.setUint32(4, 36 + frames * 2, true);
  text(8, "WAVEfmt ");
  data.setUint32(16, 16, true);
  data.setUint16(20, 1, true);
  data.setUint16(22, 1, true);
  data.setUint32(24, RATE, true);
  data.setUint32(28, RATE * 2, true);
  data.setUint16(32, 2, true);
  data.setUint16(34, 16, true);
  text(36, "data");
  data.setUint32(40, frames * 2, true);
  let offset = 44;
  for (const chunk of chunks)
    for (const sample of chunk) {
      data.setInt16(offset, sample, true);
      offset += 2;
    }
  return new File([bytes], "recording.wav", { type: "audio/wav" });
}

/** Call from a recording gesture. Cancellation also owns late permission replies. */
export async function startRecording(
  callbacks: RecordingCallbacks,
  signal: AbortSignal,
): Promise<Recording> {
  if (signal.aborted) throw aborted();
  if (
    !navigator.mediaDevices?.getUserMedia ||
    typeof AudioContext === "undefined" ||
    typeof AudioWorkletNode === "undefined"
  )
    throw "assistant_microphone_unavailable";
  let context: AudioContext | null = null;
  let stream: MediaStream | null = null;
  let source: MediaStreamAudioSourceNode | null = null;
  let processor: AudioWorkletNode | null = null;
  let released = false;
  let frames = 0;
  const chunks: Int16Array[] = [];
  let finished: File | null = null;
  let stopping: Promise<File> | null = null;
  let resolveStop: ((file: File) => void) | undefined;
  let rejectStop: ((reason: unknown) => void) | undefined;
  let timeout: ReturnType<typeof setTimeout> | undefined;
  const stopTracks = (value: MediaStream) => {
    for (const track of value.getTracks()) {
      track.onended = null;
      try {
        track.stop();
      } catch {
        /* Continue releasing the other resources. */
      }
    }
  };
  const release = () => {
    if (released) return;
    released = true;
    signal.removeEventListener("abort", cancel);
    if (timeout) clearTimeout(timeout);
    if (stream) stopTracks(stream);
    if (processor) {
      processor.port.onmessage = null;
      processor.onprocessorerror = null;
      processor.port.close();
      try {
        processor.disconnect();
      } catch {
        /* Already disconnected. */
      }
    }
    try {
      source?.disconnect();
    } catch {
      /* Already disconnected. */
    }
    if (context) {
      context.onstatechange = null;
      void context.close().catch(() => undefined);
    }
    chunks.length = 0;
  };
  const cancel = () => {
    release();
    finished = null;
    rejectStop?.(aborted());
  };
  const fail = (code: string) => {
    release();
    rejectStop?.(code);
    callbacks.error(code);
  };
  const complete = (limited: boolean, expectedFrames: unknown) => {
    if (released) return;
    if (expectedFrames !== frames) {
      fail("assistant_recording_interrupted");
      return;
    }
    try {
      finished = wav(chunks, frames);
      release();
      resolveStop?.(finished);
      if (limited) callbacks.limit(finished);
    } catch (reason) {
      fail(
        typeof reason === "string" ? reason : "assistant_recording_interrupted",
      );
    }
  };
  signal.addEventListener("abort", cancel, { once: true });
  try {
    context = new AudioContext({ sampleRate: RATE });
    if (context.sampleRate !== RATE || !context.audioWorklet)
      throw "assistant_microphone_unavailable";
    const acquired = navigator.mediaDevices
      .getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
        },
        video: false,
      })
      .then((value) => {
        stream = value;
        if (released || signal.aborted) {
          stopTracks(value);
          stream = null;
        }
        return value;
      });
    const [, activeStream] = await Promise.all([context.resume(), acquired]);
    if (released || signal.aborted) throw aborted();
    await context.audioWorklet.addModule(workletUrl);
    if (released || signal.aborted) throw aborted();
    if (
      context.state !== "running" ||
      activeStream.getAudioTracks().length === 0 ||
      activeStream
        .getAudioTracks()
        .some((track) => track.readyState === "ended")
    )
      throw "assistant_recording_interrupted";
    processor = new AudioWorkletNode(context, "assistant-pcm-capture", {
      channelCount: 1,
      channelCountMode: "explicit",
      processorOptions: { maxFrames: MAX_FRAMES },
    });
    processor.port.onmessage = ({ data }) => {
      if (released) return;
      if (data?.type === "chunk") {
        if (
          !(data.samples instanceof Int16Array) ||
          frames + data.samples.length > MAX_FRAMES
        ) {
          fail("assistant_recording_interrupted");
          return;
        }
        chunks.push(data.samples);
        frames += data.samples.length;
        callbacks.progress(frames / RATE);
      } else if (data?.type === "stopped" || data?.type === "limit")
        complete(data.type === "limit", data.frames);
    };
    processor.onprocessorerror = () => fail("assistant_recording_interrupted");
    for (const track of activeStream.getAudioTracks())
      track.onended = () => fail("assistant_recording_interrupted");
    context.onstatechange = () => {
      if (!released && context?.state !== "running")
        fail("assistant_recording_interrupted");
    };
    source = context.createMediaStreamSource(activeStream);
    source.connect(processor);
    processor.connect(context.destination);
    return {
      cancel,
      stop: () => {
        if (finished) return Promise.resolve(finished);
        if (stopping) return stopping;
        if (released) return Promise.reject(aborted());
        stopping = new Promise<File>((resolve, reject) => {
          resolveStop = resolve;
          rejectStop = reject;
        });
        timeout = setTimeout(
          () => fail("assistant_recording_interrupted"),
          1500,
        );
        processor!.port.postMessage({ type: "stop" });
        return stopping;
      },
    };
  } catch (reason) {
    release();
    if (
      signal.aborted ||
      (reason instanceof DOMException && reason.name === "AbortError")
    )
      throw aborted();
    throw microphoneError(reason);
  }
}
