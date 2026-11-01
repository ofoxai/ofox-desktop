// The worklet outputs silence: microphone samples never feed the speakers.
class AssistantPcmCapture extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const requested = options.processorOptions?.maxFrames;
    this.maxFrames =
      Number.isSafeInteger(requested) && requested > 0
        ? Math.min(requested, sampleRate * 300)
        : sampleRate * 300;
    this.samples = new Int16Array(4096);
    this.used = 0;
    this.frames = 0;
    this.ended = false;
    this.port.onmessage = ({ data }) => {
      if (data?.type === "stop" && !this.ended) this.finish("stopped");
    };
  }
  flush() {
    if (!this.used) return;
    const samples = this.samples.slice(0, this.used);
    this.port.postMessage({ type: "chunk", samples }, [samples.buffer]);
    this.used = 0;
  }
  finish(type) {
    this.ended = true;
    this.flush();
    this.port.postMessage({ type, frames: this.frames });
  }
  process(inputs, outputs) {
    for (const output of outputs) for (const channel of output) channel.fill(0);
    if (this.ended) return false;
    const channels = inputs[0] ?? [];
    const length = channels[0]?.length ?? 0;
    for (let i = 0; i < length; i++) {
      let value = 0;
      for (const channel of channels) value += channel[i] ?? 0;
      value /= channels.length;
      value = Number.isFinite(value) ? Math.max(-1, Math.min(1, value)) : 0;
      this.samples[this.used++] = Math.round(
        value < 0 ? value * 32768 : value * 32767,
      );
      this.frames++;
      if (this.used === this.samples.length) this.flush();
      if (this.frames >= this.maxFrames) {
        this.finish("limit");
        return false;
      }
    }
    return true;
  }
}
registerProcessor("assistant-pcm-capture", AssistantPcmCapture);
