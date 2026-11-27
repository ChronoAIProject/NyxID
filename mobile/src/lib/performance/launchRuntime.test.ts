import test from "node:test";
import assert from "node:assert/strict";
import {
  createLaunchMonitor,
  type LaunchSnapshot,
  type LaunchTiming,
} from "./launchRuntime";
import { deferred } from "../../features/assistant/assistantHookTestHarness";
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
const cold: LaunchSnapshot = {
  launch_id: 1,
  startup_kind: "cold",
  origin_source: "android_process_uptime",
  app_active_ms: 300,
  active_source: "android_activity_resumed",
  first_frame_ms: 410,
  first_frame_source: "android_window_frame_metrics_callback",
};
const hot: LaunchSnapshot = {
  ...cold,
  launch_id: 2,
  startup_kind: "hot",
  origin_source: "android_activity_started_after_background",
};
function fixture(androidActivityPause = false, sampled = true) {
  let epoch: number | null = 1;
  let initial: number | null = 1;
  let sampleCalls = 0;
  let snapshot = async (): Promise<LaunchSnapshot | null> => cold;
  let measure = async (_id: number): Promise<number | null> => 525;
  const events: LaunchTiming[] = [];
  const launch = createLaunchMonitor({
    androidActivityPause,
    native: {
      getLaunchSnapshot: () => snapshot(),
      measureLaunchMilestone: (id) => measure(id),
    },
    current: () => epoch,
    begin: () => {
      sampleCalls++;
      return sampled ? epoch : null;
    },
    coldGrant: async () => initial,
    submit: (_epoch, e) => events.push(e),
  });
  return {
    launch,
    events,
    calls: () => sampleCalls,
    epoch(value: number | null) {
      epoch = value;
      launch.consent();
    },
    initial(value: number | null) {
      initial = value;
    },
    read(fn: typeof snapshot) {
      snapshot = fn;
    },
    measure(fn: typeof measure) {
      measure = fn;
    },
  };
}

test("cold launch samples once and independently reports the first visible enabled input after the splash and overlays clear", async () => {
  const x = fixture();
  let visible = false;
  x.launch.input(
    {},
    "login_email",
    () => true,
    async () => visible,
  );
  x.launch.state("inactive");
  x.launch.state("active");
  x.launch.shell("auth", false);
  await tick();
  assert.deepEqual(
    x.events.map((e) => e.milestone),
    ["app_active", "first_frame"],
  );
  x.launch.splash();
  x.launch.shell("auth", true);
  await tick();
  assert.equal(x.events.length, 2);
  visible = true;
  x.launch.shell("auth", true);
  await tick();
  x.launch.shell("auth", true);
  assert.equal(x.calls(), 1);
  assert.equal(x.events.length, 3);
  assert.deepEqual(x.events[2], {
    milestone: "interaction_ready",
    target: "login_email",
    duration_ms: 525,
    origin_source: "android_process_uptime",
    measurement_source: "enabled_input_in_visible_window",
    startup_kind: "cold",
  });
  x.launch.reset();
});

test("a transient offscreen measurement can settle without another React layout", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const x = fixture();
  let probes = 0;
  x.launch.splash();
  x.launch.shell("login", true);
  x.launch.input(
    {},
    "login_email",
    () => true,
    async () => ++probes > 1,
  );
  x.launch.state("active");
  await tick();
  assert.equal(probes, 1);
  assert.equal(x.events.length, 2);
  t.mock.timers.tick(100);
  await tick();
  assert.equal(probes, 2);
  assert.equal(x.events[2]?.milestone, "interaction_ready");
  x.launch.reset();
});

test("in-session opt-in cannot backfill cold or a hot session begun without that grant", async () => {
  const x = fixture();
  x.initial(null);
  x.epoch(null);
  x.launch.state("active");
  await tick();
  x.epoch(2);
  x.launch.state("inactive");
  x.launch.state("active");
  await tick();
  assert.equal(x.events.length, 0);
  x.read(async () => hot);
  x.launch.state("background");
  x.epoch(null);
  x.epoch(3);
  x.launch.state("active");
  await tick();
  assert.equal(x.events.length, 0);
  x.launch.state("background");
  x.launch.state("active");
  await tick();
  assert.deepEqual(
    x.events.map((e) => e.startup_kind),
    ["hot", "hot"],
  );
  x.launch.reset();
});

test("late snapshots, measurements and unsubscribed inputs cannot cross navigation, app background or regrant", async () => {
  for (const interruption of ["navigation", "background", "regrant", "input"]) {
    const x = fixture();
    const delayed = deferred<number | null>();
    x.measure(() => delayed.promise);
    x.launch.splash();
    x.launch.shell("a", true);
    const remove = x.launch.input(
      {},
      "chat_composer",
      () => true,
      async () => true,
    );
    x.launch.state("active");
    await tick();
    if (interruption === "navigation") x.launch.shell("b", true);
    if (interruption === "background") x.launch.state("background");
    if (interruption === "regrant") {
      x.epoch(null);
      x.epoch(2);
    }
    if (interruption === "input") remove();
    delayed.resolve(600);
    await tick();
    assert.equal(x.events.length, 2, interruption);
    x.launch.reset();
  }
  const x = fixture();
  const delayed = deferred<LaunchSnapshot>();
  x.read(() => delayed.promise);
  x.launch.state("active");
  await tick();
  x.epoch(null);
  delayed.resolve(cold);
  await tick();
  assert.deepEqual(x.events, []);
  x.launch.reset();
});

test("Android temporary pause suspends an existing sample and resumes only the same active launch", async () => {
  const x = fixture(true);
  const delayed = deferred<number | null>();
  x.launch.splash();
  x.launch.shell("chat", true);
  x.measure(() => delayed.promise);
  x.launch.input({}, "chat_composer", () => true, async () => true);
  x.launch.state("active");
  await tick();
  x.launch.state("background");
  delayed.resolve(500);
  await tick();
  assert.equal(x.events.length, 2);
  x.measure(async () => 580);
  x.read(async () => ({ ...cold, continuation_only: true }));
  x.launch.state("active");
  await tick();
  assert.deepEqual(x.events.map(e => e.milestone), ["app_active", "first_frame", "interaction_ready"]);
  assert.ok(x.events[2]);
  assert.equal(x.events[2].duration_ms, 580);
  assert.equal(x.events[2].startup_kind, "cold");
  assert.equal(x.calls(), 1);
  x.launch.reset();
});

test("Android resumes the original cold sample after a pause before the first snapshot", async (t) => {
  const x = fixture(true);
  t.after(x.launch.reset);
  x.read(async () => null);
  x.launch.splash();
  x.launch.shell("chat", true);
  x.launch.input({}, "chat_composer", () => true, async () => true);
  x.launch.state("active");
  await tick();
  assert.equal(x.calls(), 1);
  assert.deepEqual(x.events, []);

  x.launch.state("background");
  x.read(async () => ({ ...cold, continuation_only: true }));
  x.launch.state("active");
  await tick();
  assert.deepEqual(
    x.events.map(({ startup_kind, milestone }) => [startup_kind, milestone]),
    [["cold", "app_active"], ["cold", "first_frame"], ["cold", "interaction_ready"]],
  );
  assert.equal(x.calls(), 1);
});

test("Android will not attach a suspended hot sample to a replacement launch ID", async (t) => {
  const x = fixture(true);
  t.after(x.launch.reset);
  x.read(async () => null);
  x.launch.state("active");
  await tick();
  x.launch.state("background");
  // A true Activity stop replaces the unbound cold launch with a fresh hot ID.
  x.read(async () => ({ ...hot, first_frame_ms: undefined }));
  x.launch.state("active");
  await tick();
  assert.deepEqual(x.events.map((event) => event.startup_kind), ["hot"]);

  x.launch.state("background");
  x.read(async () => ({ ...hot, launch_id: 3, continuation_only: true }));
  x.launch.state("active");
  await tick();
  assert.deepEqual(x.events.map((event) => event.milestone), ["app_active"]);
  assert.equal(x.calls(), 2);
});

test("Android fresh activity start discards prior partial measurements and a permission resume cannot resample a hot launch", async () => {
  const x = fixture(true);
  x.launch.state("active");
  await tick();
  x.launch.state("background");
  x.read(async () => hot);
  x.launch.state("active");
  await tick();
  assert.deepEqual(x.events.map(e => e.startup_kind), ["cold", "cold", "hot", "hot"]);
  x.launch.splash();
  x.launch.shell("chat", true);
  x.launch.input({}, "chat_composer", () => true, async () => true);
  await tick();
  assert.ok(x.events[4]);
  assert.equal(x.events[4].startup_kind, "hot");
  x.launch.state("background");
  x.read(async () => ({ ...hot, continuation_only: true }));
  x.launch.state("active");
  await tick();
  assert.equal(x.events.length, 5);
  assert.equal(x.calls(), 2);
  x.launch.reset();
  const unsampled = fixture(true, false);
  unsampled.launch.state("active");
  await tick();
  unsampled.launch.state("background");
  unsampled.read(async () => hot);
  unsampled.launch.state("active");
  await tick();
  assert.equal(unsampled.calls(), 2);
  unsampled.launch.state("background");
  unsampled.read(async () => ({ ...hot, continuation_only: true }));
  unsampled.launch.state("active");
  await tick();
  assert.equal(unsampled.calls(), 2);
  assert.deepEqual(unsampled.events, []);
  unsampled.launch.reset();
});
