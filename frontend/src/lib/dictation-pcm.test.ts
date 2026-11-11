import { afterEach, expect, it, vi } from "vitest";
import { DictationPcm } from "./dictation-pcm";
afterEach(() => vi.unstubAllGlobals());
it("activates the encoding context on construction and closes it once even after rejected close", async () => {
  const resume = vi.fn().mockResolvedValue(undefined),
    close = vi.fn().mockRejectedValue(new Error("already closed"));
  vi.stubGlobal(
    "AudioContext",
    class {
      resume = resume;
      close = close;
      audioWorklet = { addModule: vi.fn().mockResolvedValue(undefined) };
    },
  );
  const pcm = new DictationPcm();
  expect(resume).toHaveBeenCalledOnce();
  pcm.close();
  pcm.close();
  await Promise.resolve();
  expect(close).toHaveBeenCalledOnce();
});
it("cancelling while worklet loads cannot attach capture afterward", async () => {
  let loaded!: () => void;
  const node = vi.fn();
  vi.stubGlobal(
    "AudioContext",
    class {
      resume = vi.fn().mockResolvedValue(undefined);
      close = vi.fn().mockResolvedValue(undefined);
      audioWorklet = {
        addModule: () =>
          new Promise<void>((resolve) => {
            loaded = resolve;
          }),
      };
    },
  );
  vi.stubGlobal("AudioWorkletNode", node);
  const pcm = new DictationPcm();
  const starting = pcm.start({} as MediaStream, vi.fn());
  await Promise.resolve();
  pcm.close();
  loaded();
  await starting;
  expect(node).not.toHaveBeenCalled();
});

it("loads the encoder during microphone setup without connecting or recording audio", async () => {
  const addModule = vi.fn().mockResolvedValue(undefined);
  const source = vi.fn();
  const node = vi.fn();
  vi.stubGlobal(
    "AudioContext",
    class {
      resume = vi.fn().mockResolvedValue(undefined);
      close = vi.fn().mockResolvedValue(undefined);
      audioWorklet = { addModule };
      createMediaStreamSource = source;
    },
  );
  vi.stubGlobal("AudioWorkletNode", node);
  const pcm = new DictationPcm();
  try {
    await Promise.resolve();
    expect(addModule).toHaveBeenCalledOnce();
    expect(source).not.toHaveBeenCalled();
    expect(node).not.toHaveBeenCalled();
  } finally {
    pcm.close();
  }
});

it("retains an early encoder failure for startup and still releases the audio context", async () => {
  const failure = new Error("Encoder unavailable");
  const close = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal(
    "AudioContext",
    class {
      resume = vi.fn().mockResolvedValue(undefined);
      close = close;
      audioWorklet = {
        addModule: vi.fn(() => {
          throw failure;
        }),
      };
    },
  );
  const pcm = new DictationPcm();
  await Promise.resolve();
  await expect(pcm.start({} as MediaStream, vi.fn())).rejects.toBe(failure);
  pcm.close();
  expect(close).toHaveBeenCalledOnce();
});
