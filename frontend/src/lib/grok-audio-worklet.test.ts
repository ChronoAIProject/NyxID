import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import source from "./grok-audio-worklet.js?raw";

type Packet = { type: string; bytes?: ArrayBuffer; end_ms?: number };
interface Processor {
  port: {
    onmessage: (event: { data: unknown }) => void;
    postMessage: ReturnType<typeof vi.fn>;
  };
  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean;
}
let ProcessorClass: new () => Processor;
let rate = 48000;
beforeEach(() => {
  vi.stubGlobal(
    "AudioWorkletProcessor",
    class {
      port = { onmessage: () => {}, postMessage: vi.fn() };
    },
  );
  vi.stubGlobal(
    "registerProcessor",
    (_: string, processor: new () => Processor) => {
      ProcessorClass = processor;
    },
  );
  vi.stubGlobal("sampleRate", rate);
  vi.stubGlobal("currentTime", 0);
  // Executes the exact module shipped to the browser in the fixture worklet realm.
  new Function(source)();
});
afterEach(() => {
  vi.unstubAllGlobals();
  rate = 48000;
});
function process(p: Processor, blocks: number) {
  for (let i = 0; i < blocks; i++)
    p.process([[new Float32Array(128).fill(0.5)]], [[new Float32Array(128)]]);
}
function packets(p: Processor, type: string) {
  return p.port.postMessage.mock.calls
    .map(([v]) => v as Packet)
    .filter((v) => v.type === type);
}
describe("Grok PCM worklet", () => {
  it("resamples microphone blocks into contiguous 20ms PCM16 packets only while held", () => {
    const p = new ProcessorClass();
    process(p, 15);
    expect(packets(p, "pcm")).toHaveLength(0);
    p.port.onmessage({
      data: { type: "capture", enabled: true, generation: 7 },
    });
    process(p, 15); // 1920 input frames / 2 = 960 frames at 24 kHz.
    expect(packets(p, "pcm")).toHaveLength(2);
    expect(packets(p, "pcm")[0]).toMatchObject({ generation: 7 });
    const pcm = new DataView(packets(p, "pcm")[1]!.bytes!);
    expect(pcm.byteLength).toBe(960);
    expect(pcm.getInt16(0, true)).toBe(16384);
    p.port.onmessage({ data: { type: "capture", enabled: false } });
    process(p, 15);
    expect(packets(p, "pcm")).toHaveLength(2);
  });
  it("plays only provider audio, reports completed packets and discards flushed PCM", () => {
    const p = new ProcessorClass();
    const pcm = new Int16Array(480).fill(8192);
    p.port.onmessage({
      data: { type: "audio", bytes: pcm.buffer, end_ms: 300 },
    });
    const output = new Float32Array(128);
    p.process([[new Float32Array(128).fill(1)]], [[output]]);
    expect(output[0]).toBe(0.25); // Never monitor the microphone.
    process(p, 7);
    expect(packets(p, "played")).toMatchObject([{ end_ms: 300 }]);
    p.port.onmessage({
      data: { type: "audio", bytes: pcm.buffer, end_ms: 320 },
    });
    p.port.onmessage({ data: { type: "flush" } });
    p.process([], [[output]]);
    expect(output.every((v) => v === 0)).toBe(true);
    expect(packets(p, "played")).toHaveLength(1);
  });
  it("bounds playback buffering instead of allowing unbounded lag", () => {
    const p = new ProcessorClass();
    p.port.onmessage({
      data: {
        type: "audio",
        bytes: new ArrayBuffer(24000 * 6 * 2),
        end_ms: 6000,
      },
    });
    expect(packets(p, "overflow")).toHaveLength(1);
    const output = new Float32Array(128);
    p.process([], [[output]]);
    expect(output.every((v) => v === 0)).toBe(true);
  });
});

it("resamples low-rate playback without NaNs across tiny provider chunks", () => {
  vi.stubGlobal("sampleRate", 8000);
  const p = new ProcessorClass();
  for (let i = 0; i < 30; i++)
    p.port.onmessage({
      data: { type: "audio", bytes: new Int16Array([8192]).buffer, end_ms: i },
    });
  const output = new Float32Array(128);
  p.process([], [[output]]);
  expect(output.every(Number.isFinite)).toBe(true);
  expect(packets(p, "played")).toHaveLength(30);
});
