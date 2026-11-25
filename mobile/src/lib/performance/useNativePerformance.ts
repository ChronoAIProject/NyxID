import { useEffect, useRef } from "react";
import { AppState } from "react-native";
import {
  beginPerformanceSample,
  getPerformanceConsentEpoch,
  hydratePerformanceConsent,
  submitPerformanceSample,
  subscribePerformanceConsent,
} from "./metrics";
import { nativePerformance } from "./nativePerformance";
import { beginFrameWindow, createChatFrameObserver } from "./frameWindow";
import { createJsTaskObserver } from "./jsTaskWindow";
/** App owns foreground state; cleanup never reports a partially observed flow. */
export function usePerformanceLifecycle() {
  useEffect(() => {
    const jsTasks = createJsTaskObserver({
      active: () => AppState.currentState === "active",
      begin: beginPerformanceSample,
      current: getPerformanceConsentEpoch,
      submit: (epoch, result) =>
        submitPerformanceSample(epoch, "mobile.perf_js_tasks", result),
    });
    jsTasks.update();
    void hydratePerformanceConsent().catch(() => {});
    const subscription = AppState.addEventListener("change", (state) => {
      const active = state === "active";
      jsTasks.update();
      if (!active) void nativePerformance?.cancelFrames().catch(() => {});
    });
    let previousEpoch = getPerformanceConsentEpoch();
    const unsubscribe = subscribePerformanceConsent(() => {
      const epoch = getPerformanceConsentEpoch();
      jsTasks.update();
      if (previousEpoch !== null && epoch !== previousEpoch) {
        void nativePerformance?.cancelFrames().catch(() => {});
      }
      previousEpoch = epoch;
    });
    return () => {
      subscription.remove();
      unsubscribe();
      jsTasks.cancel();
      void nativePerformance?.cancelFrames().catch(() => {});
    };
  }, []);
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
