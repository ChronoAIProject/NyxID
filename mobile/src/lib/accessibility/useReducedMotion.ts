import { useEffect, useState } from "react";
import { AccessibilityInfo } from "react-native";

let cachedPreference: boolean | undefined;

/** Undefined means native settings have not answered yet; sheets wait to present. */
export function useReducedMotionPreference() {
  const [reduced, setReduced] = useState(cachedPreference);
  useEffect(() => {
    let active = true;
    let changed = false;
    const accept = (value: boolean) => {
      cachedPreference = value;
      if (active) setReduced(value);
    };
    const subscription = AccessibilityInfo.addEventListener(
      "reduceMotionChanged",
      (value) => {
        changed = true;
        accept(value);
      },
    );
    void AccessibilityInfo.isReduceMotionEnabled()
      .then((value) => {
        if (active && !changed) accept(value);
      })
      .catch(() => {
        if (active && !changed) accept(true);
      });
    return () => {
      active = false;
      subscription.remove();
    };
  }, []);
  return reduced;
}

export function useReducedMotion() {
  // Unknown preferences never introduce motion while the initial app loads.
  return useReducedMotionPreference() ?? true;
}
