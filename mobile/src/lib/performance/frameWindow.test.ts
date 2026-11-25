import assert from "node:assert/strict";
import test from "node:test";
import {
  beginFrameWindow,
  createChatFrameObserver,
  safeFrameTiming,
  FRAME_WINDOW_MS,
} from "./frameWindow";
import { deferred } from "../../features/assistant/assistantHookTestHarness";
import type { FrameTiming } from "./nativePerformance";
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
const timing: FrameTiming = {
  measurement_source: "android_frame_metrics",
  duration_ms: 300,
  sample_count: 12,
  interval_p50_ms: 8,
  interval_p95_ms: 20,
  interval_max_ms: 35,
  over_budget_count: 1,
  dropped_callback_count: 2,
};
function setup() {
  let epoch: number | null = 7,
    visible = true;
  const started = deferred<boolean>(),
    stopped = deferred<FrameTiming | null>();
  const starts: string[] = [],
    stops: string[] = [],
    reports: unknown[] = [];
  const deps = {
    native: {
      startFrames: async (id: string) => {
        starts.push(id);
        return started.promise;
      },
      stopFrames: async (id: string) => {
        stops.push(id);
        return stopped.promise;
      },
      cancelFrames: async () => {},
    },
    begin: () => epoch,
    current: () => epoch,
    visible: () => visible,
    submit: (...args: unknown[]) => reports.push(args),
  };
  return {
    deps,
    started,
    stopped,
    starts,
    stops,
    reports,
    consent: (value: number | null) => {
      epoch = value;
    },
    visible: (value: boolean) => {
      visible = value;
    },
  };
}
test("unconsented samples never start native observers", () => {
  const x = setup();
  x.consent(null);
  beginFrameWindow("chat_drag", x.deps).stop();
  assert.deepEqual(x.starts, []);
});
test("late native admission is stopped after navigation and cannot cross a new permission epoch", async () => {
  const x = setup(),
    sample = beginFrameWindow("chat_drag_during_stream", x.deps);
  sample.stop(false);
  x.consent(8);
  x.started.resolve(true);
  await tick();
  assert.equal(x.stops.length, 1);
  x.stopped.resolve(timing);
  await tick();
  assert.deepEqual(x.reports, []);
});
test("completion rechecks consent and visibility and publishes only closed numeric properties", async () => {
  for (const transition of ["none", "revoke", "new-account", "hidden"]) {
    const x = setup(),
      sample = beginFrameWindow("chat_streaming", x.deps);
    x.started.resolve(true);
    await tick();
    sample.stop();
    if (transition === "revoke") x.consent(null);
    if (transition === "new-account") x.consent(8);
    if (transition === "hidden") x.visible(false);
    x.stopped.resolve({ ...timing, unexpected_text: "private" } as FrameTiming);
    await tick();
    assert.deepEqual(
      x.reports,
      transition === "none" ? [[7, "chat_streaming", timing]] : [],
    );
  }
});
test("the native window stops at ten seconds even when streaming stays active", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const x = setup();
  beginFrameWindow("chat_streaming", x.deps);
  x.started.resolve(true);
  await tick();
  context.mock.timers.tick(FRAME_WINDOW_MS);
  await tick();
  assert.equal(x.stops.length, 1);
  x.stopped.resolve(timing);
  await tick();
  assert.equal(x.reports.length, 1);
});
test("drag momentum and streaming overlap have distinct scenarios and navigation cancels both", () => {
  const reports: unknown[][] = [];
  const observer = createChatFrameObserver((scenario) => {
    reports.push(["start", scenario]);
    return { stop: (value = true) => reports.push(["stop", scenario, value]) };
  });
  observer.update({ visible: true, streaming: false, scope: "a" });
  observer.beginDrag();
  observer.update({ visible: true, streaming: true, scope: "a" });
  observer.endDrag();
  observer.momentumBegin();
  observer.momentumEnd();
  observer.update({ visible: true, streaming: true, scope: "b" });
  observer.cancel();
  assert.deepEqual(reports, [
    ["start", "chat_drag"],
    ["start", "chat_streaming"],
    ["stop", "chat_drag", true],
    ["start", "chat_drag_during_stream"],
    ["stop", "chat_drag_during_stream", true],
    ["stop", "chat_streaming", false],
    ["start", "chat_streaming"],
    ["stop", "chat_streaming", false],
  ]);
});
test("iOS cadence cannot claim callback drops and malformed summaries are refused", () => {
  assert.equal(
    safeFrameTiming({ ...timing, measurement_source: "ios_display_link" })
      ?.dropped_callback_count,
    undefined,
  );
  for (const invalid of [
    { sample_count: 0 },
    { interval_p95_ms: 40 },
    { duration_ms: 20000 },
    { over_budget_count: 13 },
    { interval_max_ms: NaN },
  ])
    assert.equal(safeFrameTiming({ ...timing, ...invalid }), null);
});
test("forwards bounded FPS and a platform-matched process RSS peak independently", () => {
  const sample: FrameTiming = {
    ...timing,
    observed_fps: 32.5,
    peak_rss_bytes: 84_934_656,
    peak_rss_source: "android_proc_status_vmhwm",
  };
  assert.deepEqual(safeFrameTiming(sample), sample);
  const ios = { ...sample, measurement_source: "ios_display_link" } as const;
  assert.equal(safeFrameTiming(ios)?.peak_rss_bytes, undefined);
  assert.equal(
    safeFrameTiming({ ...ios, peak_rss_source: "ios_getrusage_maxrss" })
      ?.peak_rss_bytes,
    sample.peak_rss_bytes,
  );
  assert.deepEqual(
    safeFrameTiming({ ...sample, observed_fps: Infinity, peak_rss_bytes: -1 }),
    timing,
  );
});
test("regrant on the same focused chat permits the next drag without restarting a live stream midway", () => {
  const events: unknown[][] = [];
  const observer = createChatFrameObserver((scenario) => {
    events.push(["start", scenario]);
    return { stop: (report = true) => events.push(["stop", scenario, report]) };
  });
  observer.update({ visible: true, streaming: true, scope: "same-chat" });
  observer.beginDrag();
  observer.cancel(); // revoke
  observer.cancel(); // regrant: no visibility/render/stream transition
  observer.update({ visible: true, streaming: true, scope: "same-chat" });
  assert.deepEqual(events, [
    ["start", "chat_streaming"],
    ["start", "chat_drag_during_stream"],
    ["stop", "chat_drag_during_stream", false],
    ["stop", "chat_streaming", false],
  ]);
  observer.beginDrag();
  observer.momentumEnd();
  assert.deepEqual(events.slice(4), [
    ["start", "chat_drag_during_stream"],
    ["stop", "chat_drag_during_stream", true],
  ]);
  observer.update({ visible: true, streaming: false, scope: "same-chat" });
  observer.update({ visible: true, streaming: true, scope: "same-chat" });
  assert.deepEqual(events.at(-1), ["start", "chat_streaming"]);
  observer.dispose();
});
