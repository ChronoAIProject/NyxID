import { useCallback, useEffect, useRef } from "react";
import { AppState } from "react-native";
import { useFocusEffect } from "@react-navigation/native";
import { getAuthEpoch } from "../../lib/auth/authEpoch";

/** UI mutations and browser returns belong to one mounted account and screen. */
export function useAccountSecurityRefresh(
  refresh: () => void,
  clear?: () => void,
) {
  const epoch = useRef(getAuthEpoch()).current;
  const active = useRef(false);
  const focused = useRef(false);
  const latest = useRef({ refresh, clear });
  latest.current = { refresh, clear };
  const current = useCallback(
    () => active.current && epoch === getAuthEpoch(),
    [epoch],
  );
  useFocusEffect(
    useCallback(() => {
      focused.current = true;
      active.current = AppState.currentState === "active";
      if (current()) latest.current.refresh();
      return () => {
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
      else latest.current.clear?.();
    });
    return () => {
      focused.current = false;
      active.current = false;
      subscription.remove();
    };
  }, [current]);
  return current;
}
