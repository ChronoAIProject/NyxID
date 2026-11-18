import { useEffect, useRef, useState } from "react";
import { Platform } from "react-native";
import { useReducedMotionPreference } from "../../lib/accessibility/useReducedMotion";

/** Keep conditional sheets mounted until their native down animation completes. */
export function useNativeSheetDismiss(onClosed: () => void) {
  const preference = useReducedMotionPreference();
  const reduced = preference !== false;
  const [visible, setVisible] = useState(true);
  const closing = useRef(false);
  const mounted = useRef(true);
  const completed = useRef(false);
  const after = useRef<(() => void) | undefined>(undefined);
  const latest = useRef(onClosed);
  latest.current = onClosed;
  const finish = () => {
    if (!mounted.current || !closing.current || completed.current) return;
    completed.current = true;
    after.current?.();
    latest.current();
  };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (visible || !closing.current) return;
    if (reduced) {
      finish();
      return;
    }
    // Android Modal has no onDismiss event. Its native slide runs for300ms;
    // keep the owner mounted beyond that window, including rapid re-presentation.
    if (Platform.OS !== "ios") {
      const timer = setTimeout(finish, 350);
      return () => clearTimeout(timer);
    }
  }, [visible, reduced]);
  return {
    modalProps: {
      visible: visible && preference !== undefined,
      animationType: reduced ? ("none" as const) : ("slide" as const),
      onDismiss: finish,
    },
    close(afterDismiss?: () => void) {
      if (closing.current) return;
      closing.current = true;
      after.current = afterDismiss;
      setVisible(false);
    },
  };
}
