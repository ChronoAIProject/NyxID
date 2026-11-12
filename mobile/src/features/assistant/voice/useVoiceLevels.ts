import { useCallback, useEffect, useState } from "react";
import { useSharedValue, withTiming } from "react-native-reanimated";
import type { NativeVoiceMedia } from "./VoiceMediaBridge";
import {
  OUTPUT_SPEAKING_RELEASE_MS,
  levelTransitionMs,
  nextOutputActivity,
  type OutputActivity,
} from "./voiceAuraState";

/**
 * Mic and playback loudness for the call screen. Levels go straight into
 * shared values (no React render per frame); only the coarse "assistant is
 * speaking" flag is React state, and it changes once per turn.
 */
export function useVoiceLevels(media: NativeVoiceMedia | null) {
  const input = useSharedValue(0);
  const output = useSharedValue(0);
  const [speaking, setSpeaking] = useState(false);
  useEffect(() => {
    input.value = 0;
    output.value = 0;
    setSpeaking(false);
    if (!media) return;
    let lastInput = 0;
    let lastOutput = 0;
    let activity: OutputActivity = { active: false };
    let release: ReturnType<typeof setTimeout> | undefined;
    const observe = (level: number, now: number) => {
      activity = nextOutputActivity(activity, level, now);
      setSpeaking(activity.active);
      clearTimeout(release);
      release = undefined;
      // The engine only reports changes, so silence needs its own release.
      if (activity.active && activity.quietSince !== undefined)
        release = setTimeout(
          () => observe(0, Date.now()),
          OUTPUT_SPEAKING_RELEASE_MS - (now - activity.quietSince) + 10,
        );
    };
    const unsubscribe = media.subscribe((event) => {
      if (event.type !== "level") return;
      input.value = withTiming(event.input, {
        duration: levelTransitionMs(lastInput, event.input),
      });
      output.value = withTiming(event.output, {
        duration: levelTransitionMs(lastOutput, event.output),
      });
      lastInput = event.input;
      lastOutput = event.output;
      observe(event.output, Date.now());
    });
    return () => {
      unsubscribe();
      clearTimeout(release);
      input.value = 0;
      output.value = 0;
    };
  }, [media]);
  const subscribeLevel = useCallback(
    (listener: (level: number) => void) => {
      if (!media) return () => {};
      return media.subscribe((event) => {
        if (event.type === "level")
          listener(Math.max(event.input, event.output));
      });
    },
    [media],
  );
  return { input, output, speaking, subscribeLevel };
}
