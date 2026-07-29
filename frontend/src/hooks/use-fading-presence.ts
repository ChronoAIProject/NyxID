import { useEffect, useState } from "react";

export interface FadingPresence {
  readonly present: boolean;
  readonly visible: boolean;
}

/** Keep an exiting element mounted long enough for its opacity transition. */
export function useFadingPresence(
  active: boolean,
  exitMs: number,
): FadingPresence {
  const [retained, setRetained] = useState(active);
  const [entered, setEntered] = useState(false);

  useEffect(() => {
    if (active) {
      const enterTimer = window.setTimeout(() => {
        setRetained(true);
        setEntered(true);
      }, 0);
      return () => window.clearTimeout(enterTimer);
    }

    const exitTimer = window.setTimeout(() => {
      setRetained(false);
      setEntered(false);
    }, exitMs);
    return () => window.clearTimeout(exitTimer);
  }, [active, exitMs]);

  return {
    present: active || retained,
    visible: active && entered,
  };
}
