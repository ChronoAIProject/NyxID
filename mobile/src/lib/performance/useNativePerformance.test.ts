import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  deferred,
  hookHarness,
} from "../../features/assistant/assistantHookTestHarness";
import { beginFrameWindow, createChatFrameObserver } from "./frameWindow";
import { createStartupObserver } from "./startupRuntime";
import type { StartupTiming } from "./nativePerformance";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
const layout = { nativeEvent: { layout: { width: 300, height: 40 } } };
function setup(
  initialState: string | null = "active",
  initialEpoch: number | null = 7,
  lifecycleOwner = true,
) {
  const launch = deferred<number | null>();
  let epoch = initialEpoch;
  const stateListeners = new Set<(state: string) => void>();
  const consentListeners = new Set<() => void>();
  const measures: string[] = [],
    starts: string[] = [],
    stops: string[] = [],
    reports: any[][] = [];
  let cancellations = 0;
  let measure: (kind: string) => Promise<StartupTiming | null> = async (
    kind,
  ) => ({
    duration_ms: 100,
    startup_source:
      kind === "cold"
        ? "ios_did_finish_launching"
        : "ios_will_enter_foreground",
  });
  const appState = {
    currentState: initialState,
    addEventListener: (_: string, fn: (state: string) => void) => {
      stateListeners.add(fn);
      return { remove: () => stateListeners.delete(fn) };
    },
  };
  const options = { focused: true, streaming: false, scope: "chat" };
  const harness = hookHarness<any>({
    "react-native": { AppState: appState },
    "./metrics": {
      beginPerformanceSample: () => epoch,
      getPerformanceConsentEpoch: () => epoch,
      resolvePerformanceLaunchEpoch: () => launch.promise,
      subscribePerformanceConsent: (fn: () => void) => {
        consentListeners.add(fn);
        return () => consentListeners.delete(fn);
      },
      submitPerformanceSample: (...args: any[]) => reports.push(args),
    },
    "./nativePerformance": {
      nativePerformance: {
        measureReady: (kind: string) => {
          measures.push(kind);
          return measure(kind);
        },
        startFrames: async (token: string) => {
          starts.push(token);
          return true;
        },
        stopFrames: async (token: string) => {
          stops.push(token);
          return {
            measurement_source: "ios_display_link",
            duration_ms: 300,
            sample_count: 20,
            interval_p50_ms: 9,
            interval_p95_ms: 17,
            interval_max_ms: 17,
            over_budget_count: 1,
          };
        },
        cancelFrames: async () => {
          cancellations++;
        },
      },
    },
    "./frameWindow": { beginFrameWindow, createChatFrameObserver },
    "./startupRuntime": { createStartupObserver },
  });
  const hooks = harness.load(
    fileURLToPath(new URL("./useNativePerformance.ts", import.meta.url)),
  ) as {
    usePerformanceLifecycle(): void;
    usePerformanceInputReady(
      screen: string,
      enabled: boolean,
      scope: string,
    ): (event: typeof layout) => void;
    useChatNativeFrames(input: typeof options): {
      beginDrag(): void;
      momentumEnd(): void;
    };
  };
  harness.mount(() => {
    if (lifecycleOwner) hooks.usePerformanceLifecycle();
    return {
      ready: hooks.usePerformanceInputReady(
        "chat",
        options.focused,
        options.scope,
      ),
      frames: hooks.useChatNativeFrames(options),
    };
  });
  return {
    harness,
    launch,
    measures,
    starts,
    stops,
    reports,
    options,
    cancellations: () => cancellations,
    measure: (value: typeof measure) => {
      measure = value;
    },
    state: (value: string) => {
      appState.currentState = value;
      for (const fn of stateListeners) fn(value);
      harness.flush();
    },
    consent: (value: number | null) => {
      epoch = value;
      for (const fn of consentListeners) fn();
      harness.flush();
    },
    subscriptions: () => [stateListeners.size, consentListeners.size],
  };
}

test("initial unknown/inactive AppState waits for active input layout and retains the cold boundary", async () => {
  for (const state of [null, "unknown", "inactive"]) {
    const x = setup(state);
    x.launch.resolve(7);
    x.harness.value().ready(layout);
    x.harness.flush();
    await tick();
    assert.deepEqual(x.measures, []);
    x.state("active");
    await tick();
    assert.deepEqual(x.measures, ["cold"]);
    assert.equal(x.reports[0][1], "mobile.perf_startup");
    assert.equal(x.reports[0][2].readiness, "input_layout");
    x.harness.unmount();
    assert.deepEqual(x.subscriptions(), [0, 0]);
  }
});

test("missing initial permission never starts native readiness; later opt-in only permits a new drag", async () => {
  const x = setup("active", null);
  x.launch.resolve(null);
  x.harness.value().ready(layout);
  x.harness.flush();
  await tick();
  x.harness.value().frames.beginDrag();
  await tick();
  assert.deepEqual(x.measures, []);
  assert.deepEqual(x.starts, []);
  x.consent(8);
  await tick();
  assert.deepEqual(x.measures, []);
  x.harness.value().frames.beginDrag();
  await tick();
  x.harness.value().frames.momentumEnd();
  await tick();
  assert.equal(x.starts.length, 1);
  assert.equal(x.reports[0][1], "mobile.perf_frames");
  x.harness.unmount();
});

test("revocation cancels in-flight readiness and frames; same-chat regrant waits for the next interaction", async () => {
  const x = setup();
  const readiness = deferred<StartupTiming | null>();
  x.measure(() => readiness.promise);
  x.launch.resolve(7);
  x.harness.value().ready(layout);
  x.harness.flush();
  await tick();
  x.options.streaming = true;
  x.harness.render();
  await tick();
  assert.equal(x.starts.length, 1);
  assert.equal(x.measures.length, 1);
  x.consent(null);
  await tick();
  assert.ok(x.cancellations() > 0);
  assert.equal(x.stops.length, 1);
  readiness.resolve({
    duration_ms: 500,
    startup_source: "ios_did_finish_launching",
  });
  await tick();
  assert.deepEqual(x.reports, []);
  x.consent(9);
  x.harness.render();
  await tick();
  assert.equal(x.starts.length, 1);
  x.harness.value().frames.beginDrag();
  await tick();
  x.harness.value().frames.momentumEnd();
  await tick();
  assert.equal(x.starts.length, 2);
  assert.equal(x.reports[0][0], 9);
  assert.equal(x.reports[0][2].scenario, "chat_drag_during_stream");
  x.harness.unmount();
  assert.deepEqual(x.subscriptions(), [0, 0]);
});

test("background resumes use cached real input layout, while inactive-only interruptions do not resume", async () => {
  const x = setup();
  x.launch.resolve(7);
  x.harness.value().ready(layout);
  x.harness.flush();
  await tick();
  x.state("inactive");
  x.state("active");
  await tick();
  assert.deepEqual(x.measures, ["cold"]);
  x.state("inactive");
  x.state("background");
  x.state("active");
  await tick();
  assert.deepEqual(x.measures, ["cold", "resume"]);
  assert.equal(x.reports[1][2].startup_kind, "resume");
  x.harness.unmount();
});

test("focus loss and same-screen conversation replacement cannot turn a stale layout into startup readiness", async () => {
  for (const transition of ["hidden", "new-scope"]) {
    const x = setup();
    x.harness.value().ready(layout);
    x.harness.flush();
    if (transition === "hidden") x.options.focused = false;
    else x.options.scope = "replacement-chat";
    x.harness.render();
    x.launch.resolve(7);
    await tick();
    assert.deepEqual(x.measures, []);
    assert.deepEqual(x.reports, []);
    x.harness.unmount();
  }
});

test("unmounting the input screen alone fences both delayed grant admission and native completion", async () => {
  const pendingGrant = setup("active", 7, false);
  pendingGrant.harness.value().ready(layout);
  pendingGrant.harness.flush();
  pendingGrant.harness.unmount();
  pendingGrant.launch.resolve(7);
  await tick();
  assert.deepEqual(pendingGrant.measures, []);
  assert.deepEqual(pendingGrant.reports, []);

  const pendingNative = setup("active", 7, false);
  const timing = deferred<StartupTiming | null>();
  pendingNative.measure(() => timing.promise);
  pendingNative.launch.resolve(7);
  pendingNative.harness.value().ready(layout);
  pendingNative.harness.flush();
  await tick();
  assert.deepEqual(pendingNative.measures, ["cold"]);
  pendingNative.harness.unmount();
  timing.resolve({
    duration_ms: 500,
    startup_source: "ios_did_finish_launching",
  });
  await tick();
  assert.deepEqual(pendingNative.reports, []);
});

test("disabled and zero-size inputs cannot trigger readiness; unmount cancels an active frame window", async () => {
  const x = setup();
  x.launch.resolve(7);
  x.options.focused = false;
  x.harness.render();
  x.harness.value().ready(layout);
  x.harness.flush();
  await tick();
  assert.deepEqual(x.measures, []);
  // A fresh enabled input still needs a nonzero native layout.
  x.options.focused = true;
  x.harness.render();
  await tick();
  assert.deepEqual(x.measures, ["cold"]); // the previously laid-out native input remains real
  x.harness.value().frames.beginDrag();
  await tick();
  x.harness.unmount();
  await tick();
  assert.equal(x.stops.length, 1);
  assert.equal(
    x.reports.filter((event) => event[1] === "mobile.perf_frames").length,
    0,
  );
  const y = setup();
  y.launch.resolve(7);
  y.harness
    .value()
    .ready({ nativeEvent: { layout: { width: 0, height: 40 } } });
  y.harness.flush();
  await tick();
  assert.deepEqual(y.measures, []);
  y.harness.unmount();
});
