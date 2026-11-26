import { useEffect, useLayoutEffect, useRef } from "react";
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
  const { focused, scope, streaming } = options;
  const latest = useRef(options);
  const observer = useRef<
    ReturnType<typeof createChatFrameObserver> | undefined
  >(undefined);
  if (!observer.current)
    observer.current = createChatFrameObserver((scenario, isCurrent) =>
      beginFrameWindow(scenario, {
        native: nativePerformance,
        begin: beginPerformanceSample,
        current: getPerformanceConsentEpoch,
        visible: () =>
          isCurrent() &&
          latest.current.focused &&
          AppState.currentState === "active",
        submit: (epoch, scenario, timing) =>
          submitPerformanceSample(epoch, "mobile.perf_frames", {
            scenario,
            ...timing,
          }),
      }),
    );
  useLayoutEffect(() => {
    const sample = observer.current!;
    const sub = AppState.addEventListener("change", (state) =>
      sample.update({
        ...latest.current,
        visible: latest.current.focused && state === "active",
      }),
    );
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
  useLayoutEffect(() => {
    latest.current = { focused, scope, streaming };
    observer.current!.update({
      ...latest.current,
      visible: focused && AppState.currentState === "active",
    });
  }, [focused, scope, streaming]);
  return observer.current;
}
