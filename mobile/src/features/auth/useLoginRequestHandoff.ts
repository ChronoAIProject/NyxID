import { useCallback, useEffect, useRef } from "react";
import { handoffLoginRequest, type LoginRequest } from "./loginRequestResolver";

/** Destinations consume in layout, before Navigation's passive focus events.
 * Blur/unmount immediately releases any preview the destination did not take. */
export function useLoginRequestHandoff(navigation: {
  addListener?: (event: "blur" | "focus", listener: () => void) => () => void;
}) {
  const owned = useRef<(() => void) | undefined>(undefined);
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    const release = () => {
      active.current = false;
      const cancel = owned.current;
      owned.current = undefined;
      cancel?.();
    };
    const unsubscribe = navigation.addListener?.("blur", release);
    const focus = navigation.addListener?.("focus", () => {
      active.current = true;
    });
    return () => {
      unsubscribe?.();
      focus?.();
      release();
    };
  }, [navigation]);
  return useCallback((code: string, result: LoginRequest) => {
    if (!active.current) return false;
    owned.current?.();
    owned.current = handoffLoginRequest(code, result);
    return true;
  }, []);
}
