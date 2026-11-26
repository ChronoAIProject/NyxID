import { useCallback, useEffect, useRef } from "react";
import { AppState } from "react-native";

const DELAYS = [2000, 4000, 8000, 16000, 30000] as const;
const BUDGET_MS = 60_000;

/** The screen owns preview/decision exclusion and handles failures. A numeric
 * result waits for its throttle or cooldown without spending an attempt. */
export function useUncertainLoginPreview(
  enabled: boolean,
  scope: string,
  refresh: () => number | Promise<void>,
) {
  const latest = useRef(refresh);
  latest.current = refresh;
  const restart = useRef<(() => void) | undefined>(undefined);
  const reset = useCallback(() => restart.current?.(), []);

  useEffect(() => {
    if (!enabled) return;
    let active = AppState.currentState === "active";
    let disposed = false;
    let revision = 0;
    let attempt = 0;
    let deadline = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const cancel = () => {
      revision++;
      clearTimeout(timer);
      timer = undefined;
    };
    const schedule = (delay: number | undefined) => {
      if (
        disposed ||
        !active ||
        delay === undefined ||
        Date.now() + delay > deadline
      )
        return;
      timer = setTimeout(() => {
        void run();
      }, delay);
    };
    const run = async () => {
      timer = undefined;
      if (disposed || !active || Date.now() > deadline) return;
      const current = revision;
      const result = latest.current();
      if (typeof result === "number") {
        schedule(Math.max(1, result));
        return;
      }
      attempt++;
      await result;
      if (current === revision) schedule(DELAYS[attempt]);
    };
    const begin = () => {
      cancel();
      attempt = 0;
      deadline = Date.now() + BUDGET_MS;
      schedule(DELAYS[0]);
    };
    restart.current = begin;
    const subscription = AppState.addEventListener("change", (state) => {
      const wasActive = active;
      active = state === "active";
      if (!active) cancel();
      else if (!wasActive) begin();
    });
    begin();
    return () => {
      disposed = true;
      cancel();
      restart.current = undefined;
      subscription.remove();
    };
  }, [enabled, scope]);
  return { reset };
}
