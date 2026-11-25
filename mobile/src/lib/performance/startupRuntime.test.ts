import assert from "node:assert/strict";
import test from "node:test";
import { createStartupObserver } from "./startupRuntime";
import { deferred } from "../../features/assistant/assistantHookTestHarness";
import type { StartupTiming } from "./nativePerformance";
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
function setup() {
  const grant = deferred<number | null>(),
    timing = deferred<StartupTiming | null>();
  let current: number | null = null;
  const measures: string[] = [],
    reports: unknown[][] = [];
  const observer = createStartupObserver({
    native: {
      measureReady: async (kind) => {
        measures.push(kind);
        return timing.promise;
      },
      startFrames: async () => false,
      stopFrames: async () => null,
      cancelFrames: async () => {},
    },
    launchEpoch: () => grant.promise,
    begin: () => current,
    current: () => current,
    submit: (...args) => reports.push(args),
  });
  return {
    observer,
    grant,
    timing,
    measures,
    reports,
    consent: (epoch: number | null) => {
      current = epoch;
    },
  };
}
test("cold startup waits for persisted launch authority before starting any native readiness observer", async () => {
  const x = setup();
  x.observer.ready("nyxbot_home");
  x.timing.resolve({
    duration_ms: 352,
    startup_source: "android_process_uptime",
  });
  await tick();
  assert.deepEqual(x.reports, []);
  assert.deepEqual(x.measures, []);
  x.consent(7);
  x.grant.resolve(7);
  await tick();
  assert.deepEqual(x.reports, [
    [
      7,
      "cold",
      "nyxbot_home",
      { duration_ms: 352, startup_source: "android_process_uptime" },
    ],
  ]);
  x.observer.ready("nyxbot_home");
  await tick();
  assert.equal(x.measures.length, 1);
});
test("initial unknown or inactive AppState preserves the first active cold readiness", async () => {
  for (const repeat of [false, true]) {
    const x = setup();
    x.consent(7);
    x.grant.resolve(7);
    x.observer.foreground(false, false);
    if (repeat) x.observer.foreground(false, false);
    x.observer.ready("auth");
    await tick();
    assert.deepEqual(x.measures, []);
    x.observer.foreground(true);
    x.observer.ready("chat");
    x.timing.resolve({
      duration_ms: 600,
      startup_source: "ios_did_finish_launching",
    });
    await tick();
    assert.deepEqual(x.measures, ["cold"]);
    assert.deepEqual(x.reports, [
      [
        7,
        "cold",
        "chat",
        { duration_ms: 600, startup_source: "ios_did_finish_launching" },
      ],
    ]);
  }
});
test("no initial grant, account change, interrupted screen and background never become retroactive startup consent", async () => {
  for (const transition of [
    "no-initial-grant",
    "account",
    "background",
    "screen",
    "invalidate",
  ]) {
    const x = setup();
    x.observer.ready("chat");
    x.consent(7);
    if (transition === "account") x.consent(8);
    if (transition === "background") x.observer.foreground(false);
    if (transition === "screen") x.observer.ready("auth");
    if (transition === "invalidate") x.observer.invalidate();
    x.grant.resolve(transition === "no-initial-grant" ? null : 7);
    x.timing.resolve({
      duration_ms: 600,
      startup_source: "ios_did_finish_launching",
    });
    await tick();
    assert.deepEqual(x.reports, [], transition);
    assert.deepEqual(x.measures, [], transition);
  }
});
test("resume requires a real background and ignores later inactive-only cycles even with a stale native origin", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const x = setup();
  x.consent(9);
  x.observer.foreground(false, true);
  x.observer.foreground(true);
  x.observer.ready("chat");
  x.timing.resolve({
    duration_ms: 52,
    startup_source: "ios_will_enter_foreground",
  });
  await tick();
  assert.deepEqual(x.measures, ["resume"]);
  assert.deepEqual(x.reports, [
    [
      9,
      "resume",
      "chat",
      { duration_ms: 52, startup_source: "ios_will_enter_foreground" },
    ],
  ]);
  context.mock.timers.tick(20000);
  x.observer.foreground(false, false);
  x.observer.foreground(true);
  x.observer.ready("chat");
  await tick();
  assert.equal(x.reports.length, 1);
  assert.equal(x.measures.length, 1);
});
test("inactive then background still records a true resume, while invalid native timing cannot fabricate a cold sample", async () => {
  const x = setup();
  x.consent(9);
  x.observer.foreground(false, false);
  x.observer.foreground(false, true);
  x.observer.foreground(true);
  x.observer.ready("chat");
  x.timing.resolve({
    duration_ms: 30,
    startup_source: "android_activity_resume",
  });
  await tick();
  assert.equal(x.reports.length, 1);
  for (const timing of [
    null,
    { duration_ms: NaN, startup_source: "android_process_uptime" },
    { duration_ms: 45, startup_source: "ios_will_enter_foreground" },
    { duration_ms: 120001, startup_source: "android_process_uptime" },
  ] as (StartupTiming | null)[]) {
    const y = setup();
    y.consent(1);
    y.grant.resolve(1);
    y.observer.ready("chat");
    y.timing.resolve(timing);
    await tick();
    assert.deepEqual(y.reports, []);
  }
});
