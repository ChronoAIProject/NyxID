import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistantHookTestHarness";
import type { VoiceMediaEvent } from "./AssistantVoiceClient";
import * as auraState from "./voiceAuraState";

type Timed = { to: number; duration: number };
type Levels = {
  input: { value: Timed | number };
  output: { value: Timed | number };
  speaking: boolean;
  subscribeLevel: (listener: (level: number) => void) => () => void;
};

function fakeMedia() {
  const listeners = new Set<(event: VoiceMediaEvent) => void>();
  return {
    listeners,
    subscribe(listener: (event: VoiceMediaEvent) => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    emit(event: VoiceMediaEvent) {
      for (const listener of listeners) listener(event);
    },
  };
}

function setup(initial: ReturnType<typeof fakeMedia> | null) {
  let media = initial;
  const shared: { value: unknown }[] = [];
  let calls = 0;
  const harness = hookHarness<Levels>({
    "react-native-reanimated": {
      // Two stable shared values per render, as Reanimated guarantees.
      useSharedValue: (value: unknown) => (shared[calls++ % 2] ??= { value }),
      withTiming: (to: number, config: { duration: number }) => ({
        to,
        duration: config.duration,
      }),
    },
    "./voiceAuraState": auraState,
  });
  const { useVoiceLevels } = harness.load(
    fileURLToPath(new URL("./useVoiceLevels.ts", import.meta.url)),
  ) as { useVoiceLevels: (media: unknown) => Levels };
  harness.mount(() => useVoiceLevels(media));
  return {
    harness,
    value: () => harness.flush(),
    setMedia(next: ReturnType<typeof fakeMedia> | null) {
      media = next;
      harness.render();
    },
  };
}

test("level events ease shared values with a fast attack and slow release", () => {
  const media = fakeMedia();
  const f = setup(media);
  media.emit({ type: "level", input: 0.6, output: 0 });
  let levels = f.value();
  assert.deepEqual(levels.input.value, { to: 0.6, duration: 90 });
  assert.deepEqual(levels.output.value, { to: 0, duration: 320 });
  media.emit({ type: "level", input: 0.2, output: 0 });
  levels = f.value();
  assert.deepEqual(levels.input.value, { to: 0.2, duration: 320 });
  // PCM and other media events never touch the meters.
  media.emit({ type: "pcm", bytes: new ArrayBuffer(960) });
  assert.deepEqual(f.value().input.value, { to: 0.2, duration: 320 });
  f.harness.unmount();
});

test("loud playback marks the assistant speaking until a quiet release", async () => {
  const media = fakeMedia();
  const f = setup(media);
  assert.equal(f.value().speaking, false);
  media.emit({ type: "level", input: 0, output: 0.5 });
  assert.equal(f.value().speaking, true);
  // The engine sends one silent frame and then nothing; the hook releases itself.
  media.emit({ type: "level", input: 0, output: 0 });
  assert.equal(f.value().speaking, true);
  await new Promise((resolve) =>
    setTimeout(resolve, auraState.OUTPUT_SPEAKING_RELEASE_MS + 60),
  );
  assert.equal(f.value().speaking, false);
  f.harness.unmount();
});

test("a new media engine resets the meters and drops the old subscription", () => {
  const first = fakeMedia();
  const f = setup(first);
  first.emit({ type: "level", input: 0.7, output: 0.7 });
  assert.equal(f.value().speaking, true);
  const second = fakeMedia();
  f.setMedia(second);
  assert.equal(first.listeners.size, 0);
  assert.equal(second.listeners.size, 1);
  assert.equal(f.value().input.value, 0);
  assert.equal(f.value().speaking, false);
  f.setMedia(null);
  assert.equal(second.listeners.size, 0);
  f.harness.unmount();
});

test("the dictation waveform subscription receives live mic and playback levels and releases its listener", () => {
  const media = fakeMedia();
  const f = setup(media);
  const samples: number[] = [];
  const unsubscribe = f.value().subscribeLevel((level) => samples.push(level));
  media.emit({ type: "level", input: 0.7, output: 0.2 });
  media.emit({ type: "level", input: 0.1, output: 0.8 });
  assert.deepEqual(samples, [0.7, 0.8]);
  unsubscribe();
  media.emit({ type: "level", input: 1, output: 1 });
  assert.deepEqual(samples, [0.7, 0.8]);
  f.harness.unmount();
  assert.equal(media.listeners.size, 0);
});
