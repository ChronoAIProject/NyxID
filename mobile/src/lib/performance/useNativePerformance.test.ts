import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { beginFrameWindow, createChatFrameObserver } from "./frameWindow";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
function setup(
  initialState: string | null = "active",
  initialEpoch: number | null = 7,
) {
  let epoch = initialEpoch;
  const stateListeners = new Set<(state: string) => void>();
  const consentListeners = new Set<() => void>();
  const starts: string[] = [],
    stops: string[] = [],
    reports: any[][] = [];
  let cancellations = 0;
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
      hydratePerformanceConsent: async () => {},
      subscribePerformanceConsent: (fn: () => void) => {
        consentListeners.add(fn);
        return () => consentListeners.delete(fn);
      },
      submitPerformanceSample: (...args: any[]) => reports.push(args),
    },
    "./nativePerformance": {
      nativePerformance: {
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
  });
  const hooks = harness.load(
    fileURLToPath(new URL("./useNativePerformance.ts", import.meta.url)),
  ) as {
    usePerformanceLifecycle(): void;
    useChatNativeFrames(input: typeof options): {
      beginDrag(): void;
      momentumEnd(): void;
    };
  };
  harness.mount(() => {
    hooks.usePerformanceLifecycle();
    return {
      frames: hooks.useChatNativeFrames(options),
    };
  });
  return {
    harness,
    starts,
    stops,
    reports,
    options,
    cancellations: () => cancellations,
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

test("an active grant permits a drag and background discards an unfinished frame window", async () => {
  const x = setup("active", null);
  x.harness.value().frames.beginDrag();
  await tick();
  assert.deepEqual(x.starts, []);
  x.consent(8);
  x.harness.value().frames.beginDrag();
  await tick();
  x.harness.value().frames.momentumEnd();
  await tick();
  assert.equal(x.reports.length, 1);
  assert.equal(x.reports[0][1], "mobile.perf_frames");
  x.harness.value().frames.beginDrag();
  await tick();
  x.state("background");
  await tick();
  assert.equal(x.stops.length, 2);
  assert.equal(x.reports.length, 1);
  x.harness.unmount();
  assert.deepEqual(x.subscriptions(), [0, 0]);
});

test("withdrawal cancels a running stream window and a new grant starts at the next drag", async () => {
  const x = setup();
  x.options.streaming = true;
  x.harness.render();
  await tick();
  assert.equal(x.starts.length, 1);
  x.consent(null);
  await tick();
  assert.ok(x.cancellations() > 0);
  assert.equal(x.stops.length, 1);
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
