import { useEffect, useRef, useState } from "react";
import {
  AppState,
  Platform,
  Dimensions,
  type TextInput,
  type LayoutChangeEvent,
} from "react-native";
import { createLaunchMonitor, type InputTarget } from "./launchRuntime";
import { nativePerformance } from "./nativePerformance";
import {
  beginPerformanceSample,
  getPerformanceColdGrant,
  getPerformanceConsentEpoch,
  subscribePerformanceConsent,
  submitPerformanceSample,
} from "./metrics";

const launch = createLaunchMonitor({
  native: nativePerformance,
  androidActivityPause: Platform.OS === "android",
  current: getPerformanceConsentEpoch,
  begin: beginPerformanceSample,
  coldGrant: getPerformanceColdGrant,
  submit: (epoch, event) =>
    submitPerformanceSample(epoch, "mobile.perf_launch", event),
});
export const launchSplashHidden = launch.splash;

export function useLaunchLifecycle() {
  useEffect(() => {
    launch.state(AppState.currentState);
    const sub = AppState.addEventListener("change", launch.state);
    const unsubscribe = subscribePerformanceConsent(launch.consent);
    return () => {
      sub.remove();
      unsubscribe();
      launch.reset();
    };
  }, []);
}

export function useLaunchShell(route: string | undefined, clear: boolean) {
  useEffect(() => {
    launch.shell(route, clear);
    return () => launch.shell(undefined, false);
  }, [route, clear]);
}

// A measurable input must be mounted, enabled and focused on a committed route.
// Give the committed view a frame opportunity, then recheck every gate again
// when native returns its launch-relative monotonic time.
export function useLaunchInput(
  target: InputTarget,
  enabled: boolean,
  identity: string,
) {
  const [layoutScope, setLayoutScope] = useState<string>();
  const key = useRef({});
  const input = useRef<(() => TextInput | null) | undefined>(undefined);
  const visible = () =>
    new Promise<boolean>((resolve) => {
      const node = input.current?.();
      if (!node || typeof node.measureInWindow !== "function") {
        resolve(false);
        return;
      }
      const timer = setTimeout(() => resolve(false), 300);
      node.measureInWindow((x, y, width, height) => {
        clearTimeout(timer);
        const window = Dimensions.get("window");
        resolve(
          node === input.current?.() &&
            width > 0 &&
            height > 0 &&
            x < window.width &&
            y < window.height &&
            x + width > 0 &&
            y + height > 0,
        );
      });
    });
  const stable = useRef(false);
  stable.current = enabled && layoutScope === identity;
  useEffect(() => {
    if (!enabled || layoutScope !== identity) return;
    let cleanup: (() => void) | undefined;
    const frame = requestAnimationFrame(() => {
      cleanup = launch.input(
        key.current,
        target,
        () => stable.current && AppState.currentState === "active",
        visible,
      );
    });
    return () => {
      cancelAnimationFrame(frame);
      cleanup?.();
    };
  }, [target, identity, enabled, layoutScope]);
  return (event: LayoutChangeEvent, currentInput: () => TextInput | null) => {
    input.current = currentInput;
    const { width, height } = event.nativeEvent.layout;
    setLayoutScope(width > 0 && height > 0 ? identity : undefined);
  };
}
