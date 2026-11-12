import { useEffect, useMemo, useState } from "react";
import { AppState } from "react-native";
import {
  EMPTY,
  PAUSE_AFTER_MS,
  finalScene,
  type Beat,
  type Scene,
} from "../../../../shared/onboardingReplay";

/** A local demonstration only; no chat sender or service is connected. */
export function useReplayScene(beats: readonly Beat[], reducedMotion: boolean) {
  const still = useMemo(() => finalScene(beats), [beats]);
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [frame, setFrame] = useState<{ beats: readonly Beat[]; scene: Scene }>({
    beats,
    scene: EMPTY,
  });
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => subscription.remove();
  }, []);
  useEffect(() => {
    if (reducedMotion || !foreground) return;
    setFrame({ beats, scene: EMPTY });
    let index = 0;
    let timer: ReturnType<typeof setTimeout>;
    const play = () => {
      const beat = beats[index];
      if (!beat) {
        index = 0;
        timer = setTimeout(play, PAUSE_AFTER_MS);
        return;
      }
      index++;
      timer = setTimeout(() => {
        setFrame((current) => ({ beats, scene: beat.apply(current.scene) }));
        play();
      }, beat.wait);
    };
    play();
    return () => clearTimeout(timer);
  }, [beats, reducedMotion, foreground]);
  return reducedMotion || !foreground
    ? still
    : frame.beats === beats
      ? frame.scene
      : EMPTY;
}
