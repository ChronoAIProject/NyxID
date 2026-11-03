import { useCallback, useEffect, useMemo, useRef } from "react";
import { AppState } from "react-native";
import { useFocusEffect } from "@react-navigation/native";
import { getAuthEpoch } from "../../lib/auth/authEpoch";

export type AccountSecurityTicket = {
  isCurrent: () => boolean;
  assertCurrent: () => void;
};

/** UI mutations and browser returns belong to one mounted account and screen. */
export function useAccountSecurityRefresh(
  refresh: () => void,
  clear?: () => void,
) {
  const epoch = useRef(getAuthEpoch()).current;
  const active = useRef(false);
  const focused = useRef(false);
  const activation = useRef(0);
  const latest = useRef({ refresh, clear });
  latest.current = { refresh, clear };
  const current = useCallback(
    () => focused.current && active.current && epoch === getAuthEpoch(),
    [epoch],
  );
  const capture = useCallback((): AccountSecurityTicket => {
    const expectedActivation = activation.current;
    const admitted = current();
    const isCurrent = () =>
      admitted && expectedActivation === activation.current && current();
    return {
      isCurrent,
      assertCurrent: () => {
        if (!isCurrent())
          throw Object.assign(new Error("Account action cancelled"), {
            name: "AbortError",
          });
      },
    };
  }, [current]);
  useFocusEffect(
    useCallback(() => {
      focused.current = true;
      active.current = AppState.currentState === "active";
      if (current()) latest.current.refresh();
      return () => {
        activation.current += 1;
        focused.current = false;
        active.current = false;
        latest.current.clear?.();
      };
    }, [current]),
  );
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      // Focus cleanup below prevents background screens from becoming active.
      if (!focused.current) return;
      active.current = state === "active";
      if (current()) latest.current.refresh();
      else {
        activation.current += 1;
        latest.current.clear?.();
      }
    });
    return () => {
      activation.current += 1;
      focused.current = false;
      active.current = false;
      subscription.remove();
    };
  }, [current]);
  return useMemo(() => Object.assign(current, { capture }), [current, capture]);
}
