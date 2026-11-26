import type {
  FrameScenario,
  FrameTiming,
  NativePerformance,
} from "./nativePerformance";

type Sample = { stop: (report?: boolean) => void };
let sequence = 0;
export const FRAME_WINDOW_MS = 10000;
export function safeFrameTiming(value: FrameTiming | null): FrameTiming | null {
  if (
    !value ||
    !["android_frame_metrics", "ios_display_link"].includes(
      value.measurement_source,
    )
  )
    return null;
  if (
    ![
      value.duration_ms,
      value.sample_count,
      value.interval_p50_ms,
      value.interval_p95_ms,
      value.interval_max_ms,
      value.over_budget_count,
    ].every(Number.isFinite)
  )
    return null;
  if (
    value.duration_ms <= 0 ||
    value.duration_ms > 15000 ||
    !Number.isInteger(value.sample_count) ||
    value.sample_count < 2 ||
    value.sample_count > 10000 ||
    value.over_budget_count < 0 ||
    value.over_budget_count > value.sample_count
  )
    return null;
  if (
    value.interval_p50_ms < 0 ||
    value.interval_p95_ms < value.interval_p50_ms ||
    value.interval_p95_ms > value.interval_max_ms + 1 ||
    value.interval_max_ms > 15000
  )
    return null;
  const result: FrameTiming = {
    measurement_source: value.measurement_source,
    duration_ms: value.duration_ms,
    sample_count: value.sample_count,
    interval_p50_ms: value.interval_p50_ms,
    interval_p95_ms: value.interval_p95_ms,
    interval_max_ms: value.interval_max_ms,
    over_budget_count: value.over_budget_count,
  };
  if (
    value.measurement_source === "android_frame_metrics" &&
    Number.isInteger(value.dropped_callback_count) &&
    value.dropped_callback_count! >= 0 &&
    value.dropped_callback_count! <= 100000000
  )
    result.dropped_callback_count = value.dropped_callback_count;
  const fps = value.observed_fps;
  if (typeof fps === "number" && Number.isFinite(fps) && fps > 0 && fps <= 1000)
    result.observed_fps = fps;
  return result;
}

/** A late native admission or completion can never cross consent/visibility epochs. */
export function beginFrameWindow(
  scenario: FrameScenario,
  deps: {
    native: NativePerformance | null;
    begin: () => number | null;
    current: () => number | null;
    visible: () => boolean;
    submit: (
      epoch: number,
      scenario: FrameScenario,
      timing: FrameTiming,
    ) => void;
  },
): Sample {
  const epoch = deps.begin();
  if (!deps.native || epoch === null || !deps.visible()) return { stop() {} };
  const native = deps.native;
  const token = `perf-${++sequence}`;
  let open = true;
  const allowed = () => epoch === deps.current() && deps.visible();
  const started = native.startFrames(token).catch(() => false);
  const stop = (report = true) => {
    if (!open) return;
    open = false;
    clearTimeout(timeout);
    void started.then(async (accepted) => {
      if (!accepted) return;
      const timing = safeFrameTiming(
        await native.stopFrames(token).catch(() => null),
      );
      if (report && allowed() && timing) deps.submit(epoch, scenario, timing);
    });
  };
  const timeout = setTimeout(() => stop(), FRAME_WINDOW_MS);
  void started.then((accepted) => {
    if (open && (!accepted || !allowed())) stop(false);
  });
  return { stop };
}

/** One streaming window per visible episode; native observers stop after 10s. */
export function createChatFrameObserver(
  start: (scenario: FrameScenario) => Sample,
) {
  let visible = false,
    streaming = false,
    dragging = false;
  let stream: Sample | undefined, drag: Sample | undefined;
  let scope: unknown;
  let momentumTimer: ReturnType<typeof setTimeout> | undefined;
  const endDrag = (report = true) => {
    clearTimeout(momentumTimer);
    momentumTimer = undefined;
    dragging = false;
    drag?.stop(report);
    drag = undefined;
  };
  const beginDrag = () => {
    endDrag();
    if (!visible) return;
    dragging = true;
    drag = start(streaming ? "chat_drag_during_stream" : "chat_drag");
  };
  return {
    update(next: { visible: boolean; streaming: boolean; scope: unknown }) {
      if (!next.visible || scope !== next.scope) {
        endDrag(false);
        stream?.stop(false);
        stream = undefined;
      }
      if (
        next.visible &&
        (!visible || scope !== next.scope || streaming !== next.streaming)
      ) {
        if (next.streaming) {
          stream?.stop();
          stream = start("chat_streaming");
        } else {
          stream?.stop();
          stream = undefined;
        }
        if (dragging && streaming !== next.streaming) {
          drag?.stop();
          drag = start(
            next.streaming ? "chat_drag_during_stream" : "chat_drag",
          );
        }
      }
      visible = next.visible;
      streaming = next.streaming;
      scope = next.scope;
    },
    beginDrag,
    endDrag() {
      // RN sends momentum-begin after release. Keep one session through the fling.
      clearTimeout(momentumTimer);
      momentumTimer = setTimeout(() => endDrag(), 120);
    },
    momentumBegin() {
      clearTimeout(momentumTimer);
    },
    momentumEnd() {
      endDrag();
    },
    // Consent changes stop observers but preserve visibility for the next drag.
    // Streaming is not restarted in the middle of an existing episode.
    cancel() {
      endDrag(false);
      stream?.stop(false);
      stream = undefined;
    },
    dispose() {
      endDrag(false);
      stream?.stop(false);
      stream = undefined;
      visible = false;
    },
  };
}
