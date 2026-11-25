import { useEffect, useRef, useState } from "react";
import { AppState, type LayoutChangeEvent } from "react-native";
import {
  beginPerformanceSample,
  getPerformanceConsentEpoch,
  resolvePerformanceLaunchEpoch,
  submitPerformanceSample,
  subscribePerformanceConsent,
} from "./metrics";
import { nativePerformance, type PerformanceScreen } from "./nativePerformance";
import { beginFrameWindow, createChatFrameObserver } from "./frameWindow";
import { createStartupObserver } from "./startupRuntime";

const startup = createStartupObserver({
  native: nativePerformance,
  launchEpoch: resolvePerformanceLaunchEpoch,
  begin: beginPerformanceSample,
  current: getPerformanceConsentEpoch,
  submit: (epoch, startup_kind, screen, timing) =>
    submitPerformanceSample(epoch, "mobile.perf_startup", {
      ...timing,
      startup_kind,
      screen,
      readiness: screen === "auth" ? "screen_layout" : "input_layout",
    }),
});
// Resolve the persisted-at-launch grant before any future in-session opt-in.
void resolvePerformanceLaunchEpoch().catch(() => null);

/** App owns foreground state; cleanup never reports a partially observed flow. */
export function usePerformanceLifecycle() {
  useEffect(() => {
    startup.foreground(
      AppState.currentState === "active",
      AppState.currentState === "background",
    );
    const subscription = AppState.addEventListener("change", (state) => {
      const active = state === "active";
      startup.foreground(active, state === "background");
      if (!active) void nativePerformance?.cancelFrames().catch(() => {});
    });
    let previousEpoch = getPerformanceConsentEpoch();
    const unsubscribe = subscribePerformanceConsent(() => {
      // Hydration resolving an initial grant must not invalidate a cold stamp.
      const epoch = getPerformanceConsentEpoch();
      if (previousEpoch !== null && epoch !== previousEpoch) {
        startup.invalidate();
        void nativePerformance?.cancelFrames().catch(() => {});
      }
      previousEpoch = epoch;
    });
    return () => {
      subscription.remove();
      unsubscribe();
      startup.invalidate();
      void nativePerformance?.cancelFrames().catch(() => {});
    };
  }, []);
}

/** A real, enabled composer has laid out; cached layout also works on resume. */
export function usePerformanceInputReady(
  screen: PerformanceScreen,
  enabled: boolean,
  scope: unknown,
) {
  const [layout, setLayout] = useState(false);
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const mounted = useRef(true);
  const current = useRef({ screen, enabled, scope });
  current.current = { screen, enabled, scope };
  const ready = () =>
    startup.ready(
      screen,
      () =>
        mounted.current &&
        current.current.enabled &&
        current.current.screen === screen &&
        current.current.scope === scope &&
        AppState.currentState === "active",
    );
  useEffect(() => {
    mounted.current = true;
    const sub = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => {
      mounted.current = false;
      sub.remove();
    };
  }, []);
  useEffect(() => {
    if (layout && enabled && foreground) ready();
  }, [layout, enabled, foreground, screen, scope]);
  return (event: LayoutChangeEvent) => {
    const { width, height } = event.nativeEvent.layout;
    if (width > 0 && height > 0) {
      setLayout(true);
      if (current.current.enabled) ready();
    }
  };
}

export function useChatNativeFrames(options: {
  focused: boolean;
  streaming: boolean;
  scope: unknown;
}) {
  const latest = useRef(options);
  latest.current = options;
  const observer = useRef<
    ReturnType<typeof createChatFrameObserver> | undefined
  >(undefined);
  if (!observer.current)
    observer.current = createChatFrameObserver((scenario) =>
      beginFrameWindow(scenario, {
        native: nativePerformance,
        begin: beginPerformanceSample,
        current: getPerformanceConsentEpoch,
        visible: () =>
          latest.current.focused && AppState.currentState === "active",
        submit: (epoch, scenario, timing) =>
          submitPerformanceSample(epoch, "mobile.perf_frames", {
            scenario,
            ...timing,
          }),
      }),
    );
  useEffect(() => {
    const sample = observer.current!;
    const update = () =>
      sample.update({
        ...latest.current,
        visible: latest.current.focused && AppState.currentState === "active",
      });
    update();
    const sub = AppState.addEventListener("change", update);
    let previousEpoch = getPerformanceConsentEpoch();
    const unsubscribe = subscribePerformanceConsent(() => {
      const epoch = getPerformanceConsentEpoch();
      if (epoch !== previousEpoch) {
        sample.cancel();
        void nativePerformance?.cancelFrames().catch(() => {});
      }
      previousEpoch = epoch;
      // New permission starts at the next interaction/stream edge, not midway.
    });
    return () => {
      sub.remove();
      unsubscribe();
      sample.dispose();
    };
  }, []);
  useEffect(() => {
    observer.current!.update({
      ...options,
      visible: options.focused && AppState.currentState === "active",
    });
  }, [options.focused, options.scope, options.streaming]);
  return observer.current;
}
