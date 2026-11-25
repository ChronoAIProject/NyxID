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
function fixture() {
  let epoch: number | null = 1;
  let initial: number | null = 1;
  let sampleCalls = 0;
  let snapshot = async (): Promise<LaunchSnapshot | null> => cold;
  let measure = async (_id: number): Promise<number | null> => 525;
  const events: LaunchTiming[] = [];
  const launch = createLaunchMonitor({
    native: {
      getLaunchSnapshot: () => snapshot(),
      measureLaunchMilestone: (id) => measure(id),
    },
    current: () => epoch,
    begin: () => {
      sampleCalls++;
      return epoch;
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
