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
