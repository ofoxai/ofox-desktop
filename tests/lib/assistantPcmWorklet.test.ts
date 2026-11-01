// @vitest-environment node
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { expect, it } from "vitest";

function capture(maxFrames: number) {
  const messages: Array<{ type: string; frames?: number; samples?: number[] }> =
    [];
  let Processor: new (options: unknown) => {
    process: (inputs: Float32Array[][], outputs: Float32Array[][]) => boolean;
    port: { onmessage: (event: unknown) => void };
  };
  class Base {
    port = {
      onmessage: (_event: unknown) => undefined,
      postMessage: (data: {
        type: string;
        frames?: number;
        samples?: Int16Array;
      }) =>
        messages.push({
          ...data,
          samples: data.samples ? Array.from(data.samples) : undefined,
        }),
    };
  }
  runInNewContext(
    readFileSync(
      new URL("../../src/lib/assistantPcmWorklet.js", import.meta.url),
      "utf8",
    ),
    {
      AudioWorkletProcessor: Base,
      sampleRate: 16000,
      Int16Array,
      registerProcessor: (_name: string, value: typeof Processor) => {
        Processor = value;
      },
    },
  );
  return {
    processor: new Processor!({ processorOptions: { maxFrames } }),
    messages,
  };
}
it("captures bounded PCM, clamps samples and emits silence to the output graph", () => {
  const { processor, messages } = capture(6);
  const output = new Float32Array(6).fill(1);
  expect(
    processor.process([[new Float32Array([-2, -1, 0, 1, 2, NaN])]], [[output]]),
  ).toBe(false);
  expect([...output]).toEqual([0, 0, 0, 0, 0, 0]);
  expect(messages[0].samples).toEqual([-32768, -32768, 0, 32767, 32767, 0]);
  expect(messages[1]).toMatchObject({ type: "limit", frames: 6 });
  expect(processor.process([[new Float32Array([1])]], [[output]])).toBe(false);
  expect(messages).toHaveLength(2);
});
it("flushes the final partial chunk once without assuming fixed render quanta", () => {
  const { processor, messages } = capture(100);
  processor.process([[new Float32Array([1, 0, -1])]], []);
  processor.process([[new Float32Array([0, 1])]], []);
  processor.port.onmessage({ data: { type: "stop" } });
  processor.port.onmessage({ data: { type: "stop" } });
  expect(messages).toHaveLength(2);
  expect(messages[0].samples).toEqual([32767, 0, -32768, 0, 32767]);
  expect(messages[1]).toMatchObject({ type: "stopped", frames: 5 });
});
