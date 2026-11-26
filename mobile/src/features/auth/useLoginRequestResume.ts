import { useEffect, useRef } from "react";
import { AppState } from "react-native";

/** A positive return value defers the preview until its shared throttle opens.
 * The screen checks expiry and in-flight decisions before requesting that wait. */
export function useLoginRequestResume(
  enabled: boolean,
  refresh: () => void | number,
) {
  const latest = useRef(refresh);
  latest.current = refresh;
  useEffect(() => {
    if (!enabled) return;
    let previous = AppState.currentState;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = () => {
      timer = undefined;
      const delay = latest.current();
      if (typeof delay === "number" && delay > 0)
        timer = setTimeout(run, delay);
    };
    const subscription = AppState.addEventListener("change", (state) => {
      const resumed = state === "active" && previous !== "active";
      previous = state;
      if (state !== "active" && timer !== undefined) {
        clearTimeout(timer);
        timer = undefined;
      }
      if (resumed) run();
    });
    return () => {
      if (timer !== undefined) clearTimeout(timer);
      subscription.remove();
    };
  }, [enabled]);
}
